//! `api/socket`: one WebSocket per page. The joiner writes the page's
//! queue (its `hello`, then every change) and a `ping` every 15 s; it
//! reads the page's messages in order, each comment in a task of its own
//! so the share's acknowledgement does not hold up cursors.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use data_encoding::BASE64;
use futures_util::{SinkExt, StreamExt};
use hyper::{
    Request, Response, StatusCode,
    body::Incoming,
    header::{self, HeaderMap, HeaderValue},
    upgrade,
};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use sha1_smol::Sha1;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    task::JoinSet,
    time::{self, MissedTickBehavior},
};
use tokio_websockets::{CloseCode, Limits, Message, ServerBuilder, WebSocketStream};

use crate::{
    adapters::http::{AppBody, RequestError, empty_response},
    core::text::Untrusted,
    join::{
        api::{CommentOutcome, FromPage, RequestNumber, SettingsOutcome, ToPage},
        pages::{Attached, PageId, frame},
        session::Session,
    },
    protocol::MAX_LINE_BYTES,
};

/// How often the joiner pings a page; a page that hears nothing for more
/// than twice as long reconnects.
const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// How long one frame to a page may take to go out; a page that takes
/// longer is stuck, and its socket is closed.
const SEND_TIMEOUT: Duration = KEEP_ALIVE;
/// Comments one page may have waiting for the share at once; more are
/// refused. A page sends one at a time, and its unsent ones one by one.
pub(crate) const MAX_IN_FLIGHT: usize = 8;

/// RFC 6455's key suffix for `Sec-WebSocket-Accept`.
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// `GET api/socket` with `Upgrade: websocket` (the request rules have
/// checked `Host`, `Origin` and `Sec-Fetch-Site`): attaches the page and
/// switches protocols; 400 for anything but a version-13 WebSocket
/// handshake, 503 when the joiner has as many pages as it takes.
pub(crate) fn open(mut request: Request<Incoming>, session: &Arc<Session>) -> Response<AppBody> {
    let accept = match accept_key(request.headers()) {
        Ok(accept) => accept,
        Err(error) => return error.into(),
    };
    let Ok(attached) = session.attach() else {
        return RequestError::new(StatusCode::SERVICE_UNAVAILABLE, "too many pages").into();
    };
    let upgrading = upgrade::on(&mut request);
    let session = Arc::clone(session);
    tokio::spawn(async move {
        let page = attached.id;
        match upgrading.await {
            Ok(upgraded) => {
                let socket = ServerBuilder::new()
                    .limits(Limits::default().max_payload_len(Some(MAX_LINE_BYTES)))
                    .serve(TokioIo::new(upgraded));
                run(socket, &session, attached).await;
            }
            Err(error) => tracing::debug!(error = %Untrusted(&error), "the socket upgrade failed"),
        }
        session.detach(page);
    });
    let mut response = empty_response(StatusCode::SWITCHING_PROTOCOLS);
    let headers = response.headers_mut();
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    headers.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
    response
}

/// The `Sec-WebSocket-Accept` answer to a valid handshake.
fn accept_key(headers: &HeaderMap) -> Result<HeaderValue, RequestError> {
    let bad = |what: &str| RequestError::new(StatusCode::BAD_REQUEST, what.to_owned());
    let value = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    let websocket = value(header::UPGRADE).is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let upgrade = value(header::CONNECTION).is_some_and(|v| {
        v.split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
    });
    if !websocket || !upgrade {
        return Err(bad("a WebSocket upgrade is expected"));
    }
    if value(header::SEC_WEBSOCKET_VERSION) != Some("13") {
        return Err(bad("WebSocket version 13 is expected"));
    }
    let key = value(header::SEC_WEBSOCKET_KEY).ok_or_else(|| bad("no Sec-WebSocket-Key"))?;
    let mut sha1 = Sha1::new();
    sha1.update(key.as_bytes());
    sha1.update(WEBSOCKET_GUID.as_bytes());
    HeaderValue::from_str(&BASE64.encode(&sha1.digest().bytes()))
        .map_err(|_| bad("invalid Sec-WebSocket-Key"))
}

/// Why a socket ended, as its close frame says.
struct Ended(CloseCode, &'static str);

/// Runs one page's socket until either side closes it, or the joiner drops
/// the page for falling behind.
async fn run(
    socket: WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>,
    session: &Arc<Session>,
    attached: Attached,
) {
    let Attached {
        id,
        mut queue,
        mut dropped,
    } = attached;
    let (mut sink, mut stream) = socket.split();
    let mut comments = JoinSet::new();
    let reader = async {
        while let Some(message) = stream.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    tracing::debug!(error = %Untrusted(&error), "page socket failed");
                    return Ended(CloseCode::PROTOCOL_ERROR, "invalid frame");
                }
            };
            if message.is_close() {
                return Ended(CloseCode::NORMAL_CLOSURE, "bye");
            }
            if message.is_ping() || message.is_pong() {
                continue;
            }
            let Some(text) = message.as_text() else {
                return Ended(CloseCode::UNSUPPORTED_DATA, "text frames only");
            };
            if let Err(ended) = handle(session, id, text, &mut comments).await {
                return ended;
            }
            // Reap finished comments so the set does not grow.
            while comments.try_join_next().is_some() {}
        }
        Ended(CloseCode::NORMAL_CLOSURE, "bye")
    };
    let writer = async {
        let mut keep_alive = time::interval_at(time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
        keep_alive.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let Ok(ping) = frame(&ToPage::Ping) else {
            return Ended(CloseCode::INTERNAL_SERVER_ERROR, "no ping");
        };
        loop {
            let payload: Bytes = tokio::select! {
                biased;
                _ = &mut dropped => {
                    return Ended(CloseCode::SERVICE_OVERLOAD, "fell behind");
                }
                next = queue.recv() => match next {
                    Some(payload) => payload,
                    None => return Ended(CloseCode::SERVICE_OVERLOAD, "fell behind"),
                },
                _ = keep_alive.tick() => ping.clone(),
            };
            match time::timeout(SEND_TIMEOUT, sink.send(Message::text(payload))).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Ended(CloseCode::NORMAL_CLOSURE, "gone"),
                Err(_) => return Ended(CloseCode::SERVICE_OVERLOAD, "fell behind"),
            }
        }
    };
    let Ended(code, reason) = tokio::select! {
        ended = reader => ended,
        ended = writer => ended,
    };
    // Comments still waiting fail with the socket: the page sends them
    // again, under the same ref, once it is back.
    comments.abort_all();
    tracing::debug!(?id, reason, "page socket closed");
    let _ = time::timeout(SEND_TIMEOUT, sink.send(Message::close(Some(code), reason))).await;
}

/// A request that could not be read: its kind and number, to answer it.
#[derive(Deserialize)]
struct Numbered {
    #[serde(rename = "type")]
    kind: RequestKind,
    n: RequestNumber,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequestKind {
    Comment,
    Settings,
}

/// Handles one message from the page; an invalid one ends the socket
/// (unless it is a request, which is answered "refused").
async fn handle(
    session: &Arc<Session>,
    page: PageId,
    text: &str,
    comments: &mut JoinSet<()>,
) -> Result<(), Ended> {
    let message = match serde_json::from_str::<FromPage>(text) {
        Ok(message) => message,
        Err(error) => {
            if let Ok(Numbered { kind, n }) = serde_json::from_str(text) {
                let error = error.to_string();
                match kind {
                    RequestKind::Comment => {
                        let outcome = CommentOutcome::Refused {
                            error,
                            retry_after: None,
                        };
                        session.reply_comment(page, n, outcome);
                    }
                    RequestKind::Settings => {
                        session.reply_settings(page, n, SettingsOutcome::Refused { error });
                    }
                }
                return Ok(());
            }
            tracing::debug!(error = %Untrusted(&error), "invalid page message");
            return Err(Ended(CloseCode::POLICY_VIOLATION, "invalid message"));
        }
    };
    match message {
        FromPage::Page { url } => session.visit(page, url),
        FromPage::Cursor { position } => session.cursor(page, position),
        FromPage::Comment(submission) => {
            while comments.try_join_next().is_some() {}
            if comments.len() >= MAX_IN_FLIGHT {
                let outcome = CommentOutcome::Refused {
                    error: "too many comments on their way; try again shortly".to_owned(),
                    retry_after: None,
                };
                session.reply_comment(page, submission.n, outcome);
                return Ok(());
            }
            let session = Arc::clone(session);
            comments.spawn(async move { session.comment(page, submission).await });
        }
        FromPage::Settings(change) => session.change_settings(page, change),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{future, sync::Arc, time::Duration};

    use futures_util::{StreamExt, poll};
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, duplex},
        task::JoinSet,
        time::{Instant, advance},
    };
    use tokio_websockets::{ClientBuilder, ServerBuilder};

    use crate::join::{
        api::SettingsOutcome,
        pages::PAGE_QUEUE,
        session::tests::session,
        socket::{MAX_IN_FLIGHT, SEND_TIMEOUT, handle, run},
    };

    #[tokio::test]
    async fn a_page_that_falls_behind_is_closed_and_comes_back() {
        let session = Arc::new(session());
        let slow = session.attach().unwrap();
        let mut quick = session.attach().unwrap();
        let id = slow.id;
        // No socket writer is running yet: overflow the real page queue
        // without relying on TCP buffer sizes or racing a send deadline.
        quick.queue.recv().await.unwrap(); // The quick page's hello.
        for i in 0..PAGE_QUEUE {
            let name = format!("n{i}");
            session.change_settings(
                id,
                serde_json::from_value(json!({"n": i, "change": {"name": name}})).unwrap(),
            );
            let broadcast: Value =
                serde_json::from_slice(&quick.queue.try_recv().unwrap()).unwrap();
            assert_eq!(broadcast.pointer("/type"), Some(&json!("settings")));
            assert_eq!(broadcast.pointer("/name"), Some(&json!(name)));
            assert!(quick.queue.try_recv().is_err());
            assert!(!quick.queue.is_closed());
        }
        assert!(slow.queue.is_closed());
        let (server, client) = duplex(4096);
        let socket = ServerBuilder::new().serve(server);
        let mut page = ClientBuilder::new().take_over(client);
        let reader = async {
            let closed = page.next().await.unwrap().unwrap();
            assert_eq!(
                closed.as_close().map(|(code, _)| u16::from(code)),
                Some(1013)
            );
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(run(socket, &session, slow), reader);
        })
        .await
        .expect("the lagging page closes");
        session.detach(id);
        // Reattaching sends a new hello with the current settings, rather
        // than the updates that were left in the old queue.
        let again = session.attach().unwrap();
        let (server, client) = duplex(4096);
        let socket = ServerBuilder::new().serve(server);
        let mut page = ClientBuilder::new().take_over(client);
        let reader = async {
            let message = page.next().await.unwrap().unwrap();
            let hello: Value = serde_json::from_str(message.as_text().unwrap()).unwrap();
            assert_eq!(hello.pointer("/type"), Some(&json!("hello")));
            assert_eq!(
                hello.pointer("/settings/name"),
                Some(&json!(format!("n{}", PAGE_QUEUE - 1)))
            );
            drop(page);
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(run(socket, &session, again), reader);
        })
        .await
        .expect("the reconnected page receives its hello");
        // The page drained above stayed attached throughout.
        session.reply_settings(
            quick.id,
            serde_json::from_str("2").unwrap(),
            SettingsOutcome::Saved,
        );
        let reply: Value = serde_json::from_slice(&quick.queue.recv().await.unwrap()).unwrap();
        assert_eq!(reply.pointer("/type"), Some(&json!("settings_reply")));
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_writer_expires_even_when_close_cannot_fit() {
        let session = Arc::new(session());
        let attached = session.attach().unwrap();
        let id = attached.id;
        let url = Some("/stalled".to_owned().try_into().unwrap());
        session.visit(id, url.clone());
        assert_eq!(session.whereabouts().url, url);

        // One byte cannot hold even the hello's header. Polling run once
        // starts its send and fills the duplex before any settings arrive.
        let (server, mut client) = duplex(1);
        let socket = ServerBuilder::new().serve(server);
        let started = Instant::now();
        {
            let running = run(socket, &session, attached);
            tokio::pin!(running);
            assert!(poll!(&mut running).is_pending());
            assert_eq!(Instant::now(), started);

            for i in 0..PAGE_QUEUE {
                session.change_settings(
                    id,
                    serde_json::from_value(json!({"n": i, "change": {"name": format!("n{i}")}}))
                        .unwrap(),
                );
            }
            assert_eq!(
                session.whereabouts().url,
                None,
                "overflow detached the page"
            );
            assert!(poll!(&mut running).is_pending());

            // Dropping a page cannot interrupt an in-progress send. Its
            // deadline expires first; then the close has its own deadline.
            advance(SEND_TIMEOUT).await;
            assert!(poll!(&mut running).is_pending());
            advance(SEND_TIMEOUT).await;
            assert!(poll!(&mut running).is_ready());
            assert_eq!(Instant::now() - started, SEND_TIMEOUT * 2);
        }
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(
            bytes.len(),
            1,
            "only a partial hello fits, with no close frame"
        );
    }

    #[tokio::test]
    async fn comments_in_flight_are_capped_per_page() {
        let session = Arc::new(session());
        let mut attached = session.attach().unwrap();
        attached.queue.recv().await.unwrap();
        // As many comments as a page may have waiting, none answered yet.
        let mut comments = JoinSet::new();
        for _ in 0..MAX_IN_FLIGHT {
            comments.spawn(future::pending::<()>());
        }
        let comment = json!({
            "type": "comment",
            "n": 9,
            "ref": "one-more",
            "page": "http://x.localhost/",
            "selector": "h1",
            "comment": "hi",
        });
        handle(&session, attached.id, &comment.to_string(), &mut comments)
            .await
            .ok()
            .unwrap();
        assert_eq!(comments.len(), MAX_IN_FLIGHT);
        let reply: Value = serde_json::from_slice(&attached.queue.recv().await.unwrap()).unwrap();
        assert_eq!(
            reply.pointer("/type"),
            Some(&json!("comment_reply")),
            "{reply}"
        );
        assert_eq!(reply.pointer("/to"), Some(&json!(9)), "{reply}");
        let error = reply.pointer("/outcome/refused/error");
        assert!(error.is_some_and(Value::is_string), "{reply}");
    }
}
