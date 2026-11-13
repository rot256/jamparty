//! The joiner's local HTTP server on `127.0.0.1`: checks each request with
//! the core decision, answers jampot's own paths and forwards everything
//! else to the share, one iroh stream per request.

use std::{convert::Infallible, sync::Arc, time::Duration};

use bytes::Bytes;
use hyper::{
    Request, Response, StatusCode,
    body::Incoming,
    client::conn::http1 as client_http1,
    header::{self, HeaderMap, HeaderValue},
    server::conn::http1,
    service::service_fn,
    upgrade,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use rootcause::prelude::report;
use tokio::{
    io::{self, copy_bidirectional},
    net::TcpListener,
    time,
};

use crate::{
    Report,
    core::request::{
        FRAME_ANCESTORS, Method, Refusal, RequestFacts, Route, Upgrade, is_navigation,
    },
    join::{
        api,
        session::{Control, Session},
    },
    share::{
        HEADER_READ_TIMEOUT,
        body::{AppBody, RequestError, full_body, map_incoming_response},
    },
};

/// The pause after a failed accept.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// What a page load gets while the share cannot be reached: a short note
/// that retries by itself (the joiner opens the browser before it is
/// connected, and reconnects after a drop).
const UNREACHABLE_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"refresh\" content=\"2\"><title>jampot: connecting</title>\
<style>body{font:15px/1.5 system-ui,sans-serif;max-width:32em;margin:4em auto;padding:0 1em;color:#222}</style>\
</head><body><h1>Connecting to the share…</h1>\
<p>jampot join cannot reach the share yet. This page retries every few seconds.</p>\
<p>If it keeps failing, the share may have stopped; ask for a new token.</p></body></html>\n";

/// Accepts browser connections until the task is dropped. A request for the
/// share waits up to `connect_wait` for the connection to it.
pub(crate) async fn serve(listener: TcpListener, session: Arc<Session>, connect_wait: Duration) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "local accept failed");
                // Out of descriptors, say: let it pass rather than spin.
                time::sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let session = Arc::clone(&session);
                async move { Ok::<_, Infallible>(handle(request, &session, connect_wait).await) }
            });
            let served = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
            if let Err(error) = served {
                tracing::debug!(%error, "local connection ended with an error");
            }
        });
    }
}

async fn handle(
    request: Request<Incoming>,
    session: &Arc<Session>,
    connect_wait: Duration,
) -> Response<AppBody> {
    let request_facts = facts(&request);
    let page_load = is_navigation(&request_facts);
    let decision = session.origin.decide(&request_facts);
    let mut response = match decision {
        Ok(Route::Comments) => api::comment(request, session).await,
        Ok(Route::Status) => api::status(session),
        Ok(Route::Events) => api::events(session, request.uri().query()),
        Ok(Route::Page) => api::page(request, session).await,
        Ok(Route::Cursor) => api::cursor(request, session).await,
        Ok(Route::Settings) => api::settings(request, session).await,
        Ok(Route::Proxy) => match forward(request, session, connect_wait).await {
            Ok(response) => response,
            Err(error) if page_load && error.status == StatusCode::BAD_GATEWAY => {
                unreachable_page()
            }
            Err(error) => error.into(),
        },
        Err(refusal) => refused(refusal),
    };
    response.headers_mut().append(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(FRAME_ANCESTORS),
    );
    response
}

fn facts(request: &Request<Incoming>) -> RequestFacts<'_> {
    let headers = request.headers();
    RequestFacts {
        host: text(headers, header::HOST.as_str()),
        method: Method::from(request.method().as_str()),
        path: request.uri().path(),
        sec_fetch_site: text(headers, "sec-fetch-site"),
        sec_fetch_mode: text(headers, "sec-fetch-mode"),
        sec_fetch_dest: text(headers, "sec-fetch-dest"),
        origin: text(headers, header::ORIGIN.as_str()),
        upgrade: if headers.contains_key(header::UPGRADE) {
            Upgrade::Yes
        } else {
            Upgrade::No
        },
    }
}

fn text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn refused(refusal: Refusal) -> Response<AppBody> {
    let message = match refusal {
        Refusal::WrongHost => "wrong host",
        Refusal::CrossSite => "cross-site request refused",
        Refusal::BadOrigin => "origin mismatch",
        Refusal::MethodNotAllowed => "method not allowed",
    };
    let status = StatusCode::from_u16(refusal.status()).unwrap_or(StatusCode::FORBIDDEN);
    RequestError::new(status, message).into()
}

/// Sends the request to the share on a fresh stream and streams the
/// response back; an upgrade keeps both ends open.
async fn forward(
    mut request: Request<Incoming>,
    session: &Session,
    connect_wait: Duration,
) -> Result<Response<AppBody>, RequestError> {
    let unreachable = |error: Report| {
        tracing::debug!(%error, "could not forward to the share");
        RequestError::new(StatusCode::BAD_GATEWAY, "the share is not reachable")
    };
    let control = connection(session, connect_wait)
        .await
        .ok_or_else(|| unreachable(report!("not connected")))?;
    let local_upgrade = request
        .headers()
        .contains_key(header::UPGRADE)
        .then(|| upgrade::on(&mut request));
    let (send, recv) = control
        .connection
        .open_bi()
        .await
        .map_err(|error| unreachable(error.into()))?;
    let io = TokioIo::new(io::join(recv, send));
    let (mut sender, connection) = client_http1::handshake(io)
        .await
        .map_err(|error| unreachable(error.into()))?;
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            tracing::debug!(%error, "request stream ended with an error");
        }
    });
    let mut response = sender
        .send_request(request)
        .await
        .map_err(|error| unreachable(error.into()))?;
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(local_upgrade) = local_upgrade
    {
        let share_upgrade = upgrade::on(&mut response);
        tokio::spawn(async move {
            match tokio::try_join!(local_upgrade, share_upgrade) {
                Ok((local, share)) => {
                    let mut local = TokioIo::new(local);
                    let mut share = TokioIo::new(share);
                    if let Err(error) = copy_bidirectional(&mut local, &mut share).await {
                        tracing::debug!(%error, "upgraded stream closed with an error");
                    }
                }
                Err(error) => tracing::debug!(%error, "upgrade failed"),
            }
        });
    }
    Ok(map_incoming_response(response))
}

/// The connection to the share, waiting up to `wait` for one while the
/// joiner is (re)connecting; none if it does not come, or the share refused
/// this joiner.
async fn connection(session: &Session, wait: Duration) -> Option<Arc<Control>> {
    if let Some(control) = session.control() {
        return Some(control);
    }
    let mut view = session.watch();
    // Timing out is the "not reachable" answer the caller gives.
    let _ = time::timeout(
        wait,
        view.wait_for(|view| view.connected || view.refused.is_some()),
    )
    .await;
    session.control()
}

/// The page a browser gets when it loads a page the share cannot serve now.
fn unreachable_page() -> Response<AppBody> {
    let mut response = Response::new(full_body(Bytes::from_static(UNREACHABLE_PAGE.as_bytes())));
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    response
}
