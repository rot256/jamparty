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
    task::JoinSet,
    time::{self, MissedTickBehavior},
};
use tokio_websockets::{CloseCode, Limits, Message, ServerBuilder, WebSocketStream};

use crate::{
    join::{
        api::{FromPage, Outcome, RequestNumber, ToPage},
        pages::{Attached, PageId, frame},
        session::Session,
    },
    protocol::MAX_LINE_BYTES,
    share::body::{AppBody, RequestError, empty_response},
};

/// How often the joiner pings a page; a page that hears nothing for more
/// than twice as long reconnects.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// RFC 6455's key suffix for `Sec-WebSocket-Accept`.
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

type Socket = WebSocketStream<TokioIo<upgrade::Upgraded>>;

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
            Err(error) => tracing::debug!(%error, "the socket upgrade failed"),
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
async fn run(socket: Socket, session: &Arc<Session>, attached: Attached) {
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
                    tracing::debug!(%error, "page socket failed");
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
            if let Err(ended) = handle(session, id, text, &mut comments) {
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
        let ping = frame(&ToPage::Ping);
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
            if sink.send(Message::text(payload)).await.is_err() {
                return Ended(CloseCode::NORMAL_CLOSURE, "gone");
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
    let _ = sink.send(Message::close(Some(code), reason)).await;
}

/// The request number of a message that could not be read, to answer it.
#[derive(Deserialize)]
struct Numbered {
    n: RequestNumber,
}

/// Handles one message from the page; an invalid one ends the socket
/// (unless it is a request, which is answered "refused").
fn handle(
    session: &Arc<Session>,
    page: PageId,
    text: &str,
    comments: &mut JoinSet<()>,
) -> Result<(), Ended> {
    let message = match serde_json::from_str::<FromPage>(text) {
        Ok(message) => message,
        Err(error) => {
            if let Ok(Numbered { n }) = serde_json::from_str(text) {
                let outcome = Outcome::Refused {
                    error: error.to_string(),
                    retry_after: None,
                };
                session.reply(page, n, outcome);
                return Ok(());
            }
            tracing::debug!(%error, "invalid page message");
            return Err(Ended(CloseCode::POLICY_VIOLATION, "invalid message"));
        }
    };
    match message {
        FromPage::Page { url } => session.visit(page, url),
        FromPage::Cursor { position } => session.cursor(page, position),
        FromPage::Comment(submission) => {
            let session = Arc::clone(session);
            comments.spawn(async move { session.comment(page, submission).await });
        }
        FromPage::Settings(change) => session.change_settings(page, change),
    }
    Ok(())
}
