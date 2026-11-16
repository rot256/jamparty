//! Request streams: one HTTP/1.1 exchange (or upgrade) per iroh stream,
//! answered from the directory or the dev server, with the client
//! injected (the joiner serves the client itself).

use std::convert::Infallible;

use hyper::{
    Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use iroh::endpoint::{RecvStream, SendStream};
use tokio::io;

use crate::{
    adapters::http::{AppBody, HEADER_READ_TIMEOUT, RequestError},
    core::text::Untrusted,
    protocol::control::SiteInfo,
    share::{files::Files, live::ReloadState, proxy::Proxy},
};

const RESERVED_PREFIX: &str = "/_jamparty/";

/// What the share serves.
#[derive(Clone)]
pub(crate) enum Site {
    Proxy(Box<Proxy>),
    Files { files: Files, reload: ReloadState },
}

impl Site {
    /// The live-reload state, for a shared directory.
    #[must_use]
    pub(crate) fn reload(&self) -> Option<&ReloadState> {
        match self {
            Self::Proxy(_) => None,
            Self::Files { reload, .. } => Some(reload),
        }
    }

    /// What joiners are told about it.
    #[must_use]
    pub(crate) fn info(&self) -> SiteInfo {
        match self {
            Self::Proxy(proxy) => SiteInfo::Proxy { port: proxy.port() },
            Self::Files { .. } => SiteInfo::Files,
        }
    }
}

/// Serves HTTP/1.1 on one request stream until it ends.
pub(crate) async fn serve_stream(send: SendStream, recv: RecvStream, site: Site) {
    let io = TokioIo::new(io::join(recv, send));
    let service = service_fn(move |request| {
        let site = site.clone();
        async move { Ok::<_, Infallible>(handle(request, site).await) }
    });
    let served = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT)
        .serve_connection(io, service)
        .with_upgrades()
        .await;
    if let Err(error) = served {
        tracing::debug!(error = %Untrusted(&error), "request stream ended with an error");
    }
}

async fn handle(request: Request<Incoming>, site: Site) -> Response<AppBody> {
    let path = request.uri().path();
    let result = if path.starts_with(RESERVED_PREFIX) {
        Err(RequestError::new(
            StatusCode::NOT_FOUND,
            "reserved jamparty resource not found",
        ))
    } else {
        match site {
            Site::Proxy(proxy) => proxy.handle(request).await,
            Site::Files { files, reload } => files.handle(request, reload.current()).await,
        }
    };
    result.unwrap_or_else(|error| {
        tracing::debug!(status = %error.status, error = %Untrusted(&error.message), "request refused");
        error.into()
    })
}
