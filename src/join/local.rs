//! The joiner's local HTTP server on `127.0.0.1`: checks each request with
//! the core decision, answers jamjam's own paths and forwards everything
//! else to the share, one iroh stream per request.

use std::{convert::Infallible, sync::Arc, time::Duration};

use hyper::{
    Request, Response, StatusCode,
    body::Incoming,
    header::{self, HeaderMap, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::{io::copy_bidirectional, net::TcpListener};

use crate::{
    BoxError,
    core::request::{FRAME_ANCESTORS, Method, Refusal, RequestFacts, Route, Upgrade},
    join::{api, session::Session},
    share::{
        HEADER_READ_TIMEOUT,
        body::{AppBody, RequestError, map_incoming_response},
    },
};

/// The pause after a failed accept.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Accepts browser connections until the task is dropped.
pub(crate) async fn serve(listener: TcpListener, session: Arc<Session>) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "local accept failed");
                // Out of descriptors, say: let it pass rather than spin.
                tokio::time::sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let session = Arc::clone(&session);
                async move { Ok::<_, Infallible>(handle(request, &session).await) }
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

async fn handle(request: Request<Incoming>, session: &Session) -> Response<AppBody> {
    let decision = session.origin.decide(&facts(&request));
    let mut response = match decision {
        Ok(Route::Comments) => api::comment(request, session).await,
        Ok(Route::Status) => api::status(session),
        Ok(Route::Events) => api::events(session),
        Ok(Route::Proxy) => forward(request, session)
            .await
            .unwrap_or_else(Response::from),
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
) -> Result<Response<AppBody>, RequestError> {
    let unreachable = |error: BoxError| {
        tracing::debug!(%error, "could not forward to the share");
        RequestError::new(StatusCode::BAD_GATEWAY, "the share is not reachable")
    };
    let control = session
        .control()
        .ok_or_else(|| unreachable("not connected".into()))?;
    let local_upgrade = request
        .headers()
        .contains_key(header::UPGRADE)
        .then(|| hyper::upgrade::on(&mut request));
    let (send, recv) = control
        .connection
        .open_bi()
        .await
        .map_err(|error| unreachable(error.into()))?;
    let io = TokioIo::new(tokio::io::join(recv, send));
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
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
        let share_upgrade = hyper::upgrade::on(&mut response);
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
