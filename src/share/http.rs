//! Request streams: one HTTP/1.1 exchange (or upgrade) per iroh stream,
//! answered from the directory or the dev server, with the client
//! injected.

use std::convert::Infallible;

use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use iroh::endpoint::{RecvStream, SendStream};

use crate::share::{
    body::{AppBody, RequestError, javascript_response},
    files::Files,
    live::ReloadState,
    proxy::Proxy,
};

const RESERVED_PREFIX: &str = "/_jamjam/";
const CLIENT_PATH: &str = "/_jamjam/client.js";
pub(crate) const CLIENT_JS: &str = include_str!("../../assets/client.js");

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
}

/// Serves HTTP/1.1 on one request stream until it ends.
pub(crate) async fn serve_stream(send: SendStream, recv: RecvStream, site: Site) {
    let io = TokioIo::new(tokio::io::join(recv, send));
    let service = service_fn(move |request| {
        let site = site.clone();
        async move { Ok::<_, Infallible>(handle(request, site).await) }
    });
    let served = http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await;
    if let Err(error) = served {
        tracing::debug!(%error, "request stream ended with an error");
    }
}

async fn handle(request: Request<Incoming>, site: Site) -> Response<AppBody> {
    let path = request.uri().path();
    let result = if path == CLIENT_PATH {
        if request.method() == Method::GET || request.method() == Method::HEAD {
            Ok(javascript_response(CLIENT_JS))
        } else {
            Err(RequestError::method_not_allowed("GET, HEAD"))
        }
    } else if path.starts_with(RESERVED_PREFIX) {
        Err(RequestError::new(
            StatusCode::NOT_FOUND,
            "reserved jamjam resource not found",
        ))
    } else {
        match site {
            Site::Proxy(proxy) => proxy.handle(request).await,
            Site::Files { files, reload } => files.handle(request, reload.current()).await,
        }
    };
    result.unwrap_or_else(|error| {
        tracing::debug!(status = %error.status, error = %error.message, "request refused");
        error.into()
    })
}
