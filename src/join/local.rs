//! The joiner's local HTTP server on `127.0.0.1`: checks each request with
//! the core decision, answers jamparty's own paths (the browser client
//! included) and forwards everything else to the share, one iroh stream
//! per request. What it forwards is bounded on the joiner's side, whatever
//! the share does: so many requests and tunnels at once
//! ([`ProxyLimits`]), a deadline for the share's response headers, and an
//! upgraded tunnel ends when the browser's side does, or goes idle.

use std::{
    convert::Infallible,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::{
    Method as HttpMethod, Request, Response, StatusCode,
    body::{Body, Incoming},
    client::conn::http1 as client_http1,
    header::{self, HeaderMap, HeaderValue},
    server::conn::http1,
    service::service_fn,
    upgrade,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use rootcause::prelude::report;
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, oneshot},
    time::{self, Instant},
};

use crate::{
    Report,
    adapters::http::{
        AppBody, BodyError, HEADER_READ_TIMEOUT, RequestError, escape_html, full_body,
        json_response, map_incoming_response, read_upload,
    },
    core::{
        request::{
            FAVICON_PATH, FRAME_ANCESTORS, LocalOrigin, Method, PDF_CMAP_PREFIX, Refusal,
            RequestFacts, Route, Upgrade, is_navigation, is_pdf_path,
        },
        text::{OneLineReport, Untrusted},
    },
    join::{control::Control, session::Session, socket},
    protocol::resource::{MAX_UPLOAD_REPLY_BYTES, ResourceId, UPLOAD_PATH, Uploaded},
};

mod assets;

use assets::{
    CLIENT, FAVICON, JAVASCRIPT, PDF_CMAPS, PDF_SCRIPT, PDF_STYLE, PDF_VIEWER, PDF_WORKER,
};

/// The pause after a failed accept.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// What an upgraded tunnel copies at a time, each way.
const TUNNEL_BUFFER: usize = 16 * 1024;
const MAX_ACTIVE_UPLOADS: usize = 2;

/// What the joiner allows the requests it forwards to the share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProxyLimits {
    /// Requests and upgraded tunnels to the share at once, each on a
    /// stream of its own; past this a request is answered 503. Below the
    /// streams a share allows a joiner (64, its control stream included).
    pub(crate) concurrent: usize,
    /// How long the share has to send a response's headers (a dev
    /// server's first build can be slow); then 504.
    pub(crate) response_wait: Duration,
    /// How long an upgraded tunnel may carry nothing either way before it
    /// is closed. Live-reload sockets that never speak reconnect after.
    pub(crate) idle: Duration,
    /// How long one side of a tunnel may still send after the other
    /// closed.
    pub(crate) half_close: Duration,
}

impl ProxyLimits {
    /// What `jamparty join` allows.
    pub(crate) const JOIN: Self = Self {
        concurrent: 48,
        response_wait: Duration::from_secs(120),
        idle: Duration::from_secs(30 * 60),
        half_close: Duration::from_secs(2),
    };
}

/// The local server's way to the share: its limits, and the permits for
/// requests at once.
pub(crate) struct Proxy {
    limits: ProxyLimits,
    permits: Arc<Semaphore>,
    uploads: Semaphore,
}

impl Proxy {
    #[must_use]
    pub(crate) fn new(limits: ProxyLimits) -> Self {
        Self {
            limits,
            permits: Arc::new(Semaphore::new(limits.concurrent)),
            uploads: Semaphore::new(MAX_ACTIVE_UPLOADS),
        }
    }

    /// Requests and tunnels to the share open now.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn open(&self) -> usize {
        self.limits
            .concurrent
            .saturating_sub(self.permits.available_permits())
    }
}

/// What a page load gets while the share cannot be reached: a short note
/// that retries by itself (the joiner reconnects after a drop).
const UNREACHABLE_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"refresh\" content=\"2\"><title>jamparty: connecting</title>\
<link rel=\"icon\" href=\"/_jamparty/favicon.svg\" type=\"image/svg+xml\">\
<style>body{font:15px/1.5 system-ui,sans-serif;max-width:32em;margin:4em auto;padding:0 1em;color:#222}</style>\
</head><body><h1>Connecting to the share…</h1>\
<p>jamparty join cannot reach the share yet. This page retries every few seconds.</p>\
<p>If it keeps failing, the share may have stopped; ask for a new token.</p></body></html>\n";

/// Accepts browser connections for `origin` until the task is dropped. A
/// request for the share waits up to `connect_wait` for the connection to
/// it.
pub(crate) async fn serve(
    listener: TcpListener,
    session: Arc<Session>,
    origin: Arc<LocalOrigin>,
    proxy: Arc<Proxy>,
    connect_wait: Duration,
) {
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
        let origin = Arc::clone(&origin);
        let proxy = Arc::clone(&proxy);
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let session = Arc::clone(&session);
                let origin = Arc::clone(&origin);
                let proxy = Arc::clone(&proxy);
                async move {
                    Ok::<_, Infallible>(
                        handle(request, &session, &origin, &proxy, connect_wait).await,
                    )
                }
            });
            let served = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
            if let Err(error) = served {
                tracing::debug!(error = %Untrusted(&error), "local connection ended with an error");
            }
        });
    }
}

async fn handle(
    request: Request<Incoming>,
    session: &Arc<Session>,
    origin: &LocalOrigin,
    proxy: &Proxy,
    connect_wait: Duration,
) -> Response<AppBody> {
    let request_facts = facts(&request);
    let page_load = is_navigation(&request_facts);
    let pdf = is_pdf_path(request_facts.path);
    let decision = origin.decide(&request_facts);
    let headers = request.headers();
    let mut response = match decision {
        Ok(Route::Client) => CLIENT.response(headers, JAVASCRIPT),
        Ok(Route::Favicon) => nosniff(FAVICON.response(headers, "image/svg+xml")),
        Ok(Route::PdfViewer) => pdf_viewer_response(origin, headers),
        Ok(Route::PdfScript) => PDF_SCRIPT.response(headers, JAVASCRIPT),
        Ok(Route::PdfWorker) => {
            let mut response = PDF_WORKER.response(headers, JAVASCRIPT);
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'"),
            );
            response
        }
        Ok(Route::PdfStyle) => nosniff(PDF_STYLE.response(headers, "text/css; charset=utf-8")),
        Ok(Route::PdfCMap) => pdf_cmap_response(request_facts.path),
        Ok(Route::Socket) => socket::open(request, session),
        Ok(Route::Upload) => upload(request, session, proxy, connect_wait)
            .await
            .unwrap_or_else(Response::from),
        Ok(Route::Unknown) => {
            RequestError::new(StatusCode::NOT_FOUND, "no such jamparty path").into()
        }
        Ok(Route::Proxy) => match forward(request, session, proxy, connect_wait).await {
            Ok(response) => map_incoming_response(response),
            Err(error) if page_load && error.status == StatusCode::BAD_GATEWAY => {
                unreachable_page()
            }
            Err(error) => error.into(),
        },
        Err(Refusal::CrossSite | Refusal::BadOrigin) if page_load => {
            let target = request
                .uri()
                .path_and_query()
                .map_or("/", |uri| uri.as_str());
            let url = escape_html(&format!("{}{target}", origin.origin()));
            let html = format!(
                "<!doctype html><meta charset=\"utf-8\"><title>Navigation refused</title>\
                 <link rel=\"icon\" href=\"/_jamparty/favicon.svg\" type=\"image/svg+xml\">\
                 <h1>Navigation refused</h1>\
                 <p>Open this page from the address bar or from within the share.</p>\
                 <p>Copy this URL: <code>{url}</code></p>"
            );
            local_html(StatusCode::FORBIDDEN, Bytes::from(html))
        }
        Err(refusal) => refused(refusal),
    };
    if pdf {
        // A cached response for a fetch must not satisfy a navigation.
        response.headers_mut().append(
            header::VARY,
            HeaderValue::from_static("Sec-Fetch-Dest, Sec-Fetch-Mode"),
        );
    }
    response.headers_mut().append(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(FRAME_ANCESTORS),
    );
    response
}

fn pdf_cmap_response(path: &str) -> Response<AppBody> {
    fn cmap(name: &str) -> Option<&'static [u8]> {
        let cmap_range = include!("../../assets/pdf-cmaps.rs");
        let range = cmap_range(name)?;
        PDF_CMAPS.identity().get(range)
    }

    let Some(data) = path.strip_prefix(PDF_CMAP_PREFIX).and_then(cmap) else {
        return RequestError::new(StatusCode::NOT_FOUND, "no such PDF CMap").into();
    };
    // The embedded stream encodes the whole table. Each URL serves one slice.
    assets::asset_response(data, "application/octet-stream")
}

fn pdf_viewer_response(origin: &LocalOrigin, headers: &HeaderMap) -> Response<AppBody> {
    let mut response = nosniff(PDF_VIEWER.response(headers, "text/html; charset=utf-8"));
    response.headers_mut().insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("noopener-allow-popups"),
    );
    let http = origin.origin();
    let ws = http.replacen("http:", "ws:", 1);
    let csp = format!(
        "default-src 'none'; script-src {http}/_jamparty/pdf.js {http}/_jamparty/pdf-worker.js; \
         worker-src {http}/_jamparty/pdf-worker.js; style-src 'self' 'unsafe-inline'; \
         font-src blob:; img-src {http}{FAVICON_PATH} blob: data:; connect-src 'self' {ws}; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&csp)
            .unwrap_or_else(|_| HeaderValue::from_static("default-src 'none'")),
    );
    response
}

fn nosniff(mut response: Response<AppBody>) -> Response<AppBody> {
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
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
/// response back; an upgrade keeps both ends open, as a tunnel. Each holds
/// a permit of `proxy` until its stream is done.
async fn forward<B>(
    mut request: Request<B>,
    session: &Session,
    proxy: &Proxy,
    connect_wait: Duration,
) -> Result<Response<Incoming>, RequestError>
where
    B: Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<BodyError>,
{
    let Ok(permit) = Arc::clone(&proxy.permits).try_acquire_owned() else {
        tracing::debug!("too many requests to the share at once");
        return Err(RequestError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "too many requests to the share at once",
        ));
    };
    let permit = Arc::new(permit);
    let limits = proxy.limits;
    let unreachable = |error: Report| {
        tracing::debug!(error = %OneLineReport(&error), "could not forward to the share");
        RequestError::new(StatusCode::BAD_GATEWAY, "the share is not reachable")
    };
    let control = connection(session, connect_wait)
        .await
        .ok_or_else(|| unreachable(report!("not connected")))?;
    let local_upgrade = request
        .headers()
        .contains_key(header::UPGRADE)
        .then(|| upgrade::on(&mut request));
    // A share can keep the connection alive yet withhold stream credit.
    let (send, recv) = time::timeout(connect_wait, control.connection.open_bi())
        .await
        .map_err(|_| {
            tracing::debug!("the share did not open a stream in time");
            RequestError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "the share did not answer in time",
            )
        })?
        .map_err(|error| unreachable(error.into()))?;
    let io = TokioIo::new(io::join(recv, send));
    let (mut sender, connection) = client_http1::handshake(io)
        .await
        .map_err(|error| unreachable(error.into()))?;
    // The stream is driven until its response is done, unless this
    // request is given up first (the browser went, or the share was too
    // slow): then `answered` is dropped unsent, and so is the stream.
    let (answered, mut abandoned) = oneshot::channel::<()>();
    let held = Arc::clone(&permit);
    tokio::spawn(async move {
        let _held = held;
        tokio::select! {
            result = connection.with_upgrades() => {
                if let Err(error) = result {
                    tracing::debug!(error = %Untrusted(&error), "request stream ended with an error");
                }
            }
            Err(_) = &mut abandoned => {
                tracing::debug!("request given up before the share answered");
            }
        }
    });
    let mut response = match time::timeout(limits.response_wait, sender.send_request(request)).await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return Err(unreachable(error.into())),
        Err(_) => {
            tracing::debug!("the share did not answer in time");
            return Err(RequestError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "the share did not answer in time",
            ));
        }
    };
    // Nobody waits for it any more if the stream ended meanwhile.
    let _ = answered.send(());
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(local_upgrade) = local_upgrade
    {
        let share_upgrade = upgrade::on(&mut response);
        tokio::spawn(async move {
            let _held = permit;
            match tokio::try_join!(local_upgrade, share_upgrade) {
                Ok((local, share)) => {
                    tunnel(TokioIo::new(local), TokioIo::new(share), limits).await;
                }
                Err(error) => tracing::debug!(error = %Untrusted(&error), "upgrade failed"),
            }
        });
    }
    Ok(response)
}

/// Carries an upgraded connection between the browser and the share until
/// either side closes it (the other then has [`ProxyLimits::half_close`]
/// to finish), or nothing moves either way for [`ProxyLimits::idle`].
async fn tunnel<L, S>(local: L, share: S, limits: ProxyLimits)
where
    L: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (local_read, local_write) = io::split(local);
    let (share_read, share_write) = io::split(share);
    let moved = Mutex::new(Instant::now());
    let up = pipe(local_read, share_write, &moved);
    let down = pipe(share_read, local_write, &moved);
    tokio::pin!(up, down);
    tokio::select! {
        ended = &mut up => {
            if let Err(error) = ended {
                tracing::debug!(error = %Untrusted(&error), "the browser's side of a tunnel failed");
            }
            let _ = time::timeout(limits.half_close, &mut down).await;
        }
        ended = &mut down => {
            if let Err(error) = ended {
                tracing::debug!(error = %Untrusted(&error), "the share's side of a tunnel failed");
            }
            let _ = time::timeout(limits.half_close, &mut up).await;
        }
        () = idle(&moved, limits.idle) => {
            tracing::debug!("closing an idle tunnel");
        }
    }
}

/// Copies `from` to `to` until `from` ends, then shuts `to` down; notes
/// in `moved` when bytes last went through.
async fn pipe<R, W>(mut from: R, mut to: W, moved: &Mutex<Instant>) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0; TUNNEL_BUFFER];
    loop {
        let read = from.read(&mut buffer).await?;
        let Some(bytes) = buffer.get(..read).filter(|bytes| !bytes.is_empty()) else {
            return to.shutdown().await;
        };
        to.write_all(bytes).await?;
        to.flush().await?;
        *moved.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
}

/// Ends once nothing went through for `limit`, as `moved` says.
async fn idle(moved: &Mutex<Instant>, limit: Duration) {
    loop {
        let last = *moved.lock().unwrap_or_else(PoisonError::into_inner);
        let deadline = last + limit;
        if Instant::now() >= deadline {
            return;
        }
        time::sleep_until(deadline).await;
    }
}

/// The connection to the share, waiting up to `wait` for one while the
/// joiner is (re)connecting; none if it does not come, or the share refused
/// this joiner.
async fn connection(session: &Session, wait: Duration) -> Option<Arc<Control>> {
    if let Some(control) = session.control() {
        return Some(control);
    }
    let mut snapshot = session.watch();
    // Timing out is the "not reachable" answer the caller gives.
    let _ = time::timeout(
        wait,
        snapshot.wait_for(|snapshot| snapshot.link.connected || snapshot.link.refused.is_some()),
    )
    .await;
    session.control()
}

/// The page a browser gets when it loads a page the share cannot serve now.
fn unreachable_page() -> Response<AppBody> {
    let mut response = local_html(
        StatusCode::SERVICE_UNAVAILABLE,
        Bytes::from_static(UNREACHABLE_PAGE.as_bytes()),
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    response
}

fn local_html(status: StatusCode, html: Bytes) -> Response<AppBody> {
    let mut response = Response::new(full_body(html));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Rebuilds the peer request and response. Browser request headers and
/// share response headers have no role in the attachment API.
async fn upload(
    request: Request<Incoming>,
    session: &Session,
    proxy: &Proxy,
    connect_wait: Duration,
) -> Result<Response<AppBody>, RequestError> {
    let _permit = proxy.uploads.try_acquire().map_err(|_| {
        RequestError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "the joiner is busy uploading attachments",
        )
    })?;
    let bytes = read_upload(request.into_body()).await?;
    let expected = ResourceId::digest(&bytes);
    let mut request = Request::new(Full::new(bytes));
    *request.method_mut() = HttpMethod::POST;
    *request.uri_mut() = hyper::Uri::from_static(UPLOAD_PATH);
    request
        .headers_mut()
        .insert(header::HOST, HeaderValue::from_static("jamparty"));
    let response = forward(request, session, proxy, connect_wait).await?;
    let status = response.status();
    let invalid = || {
        RequestError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the share returned an invalid attachment reply",
        )
    };
    match status {
        StatusCode::CREATED => {
            let collected = time::timeout(
                connect_wait,
                Limited::new(response.into_body(), MAX_UPLOAD_REPLY_BYTES).collect(),
            )
            .await
            .map_err(|_| {
                RequestError::new(
                    StatusCode::GATEWAY_TIMEOUT,
                    "the share did not answer in time",
                )
            })?
            .map_err(|error| {
                if error.is::<LengthLimitError>() {
                    invalid()
                } else {
                    RequestError::new(StatusCode::BAD_GATEWAY, "the upload reply was interrupted")
                }
            })?;
            let uploaded: Uploaded =
                serde_json::from_slice(&collected.to_bytes()).map_err(|_| invalid())?;
            if uploaded.id != expected {
                return Err(invalid());
            }
            Ok(json_response(status, &uploaded))
        }
        StatusCode::INSUFFICIENT_STORAGE => Err(RequestError::new(
            status,
            "the share's attachment storage is full",
        )),
        StatusCode::SERVICE_UNAVAILABLE => Err(RequestError::new(
            status,
            "the share is busy receiving attachments",
        )),
        StatusCode::PAYLOAD_TOO_LARGE => Err(RequestError::new(
            status,
            "an attachment can be at most 8 MiB",
        )),
        StatusCode::REQUEST_TIMEOUT => {
            Err(RequestError::new(status, "attachment upload timed out"))
        }
        _ => Err(invalid()),
    }
}
