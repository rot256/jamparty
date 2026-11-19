//! Request streams: one HTTP/1.1 exchange (or upgrade) per iroh stream,
//! answered from the directory or the dev server, with the client
//! injected (the joiner serves the client itself).

use std::{convert::Infallible, sync::Arc};

use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use iroh::endpoint::{RecvStream, SendStream};
use tokio::{io, sync::Semaphore};

use crate::{
    adapters::http::{AppBody, HEADER_READ_TIMEOUT, RequestError, json_response, read_upload},
    core::text::Untrusted,
    protocol::{
        control::SiteInfo,
        resource::{UPLOAD_PATH, Uploaded},
    },
    share::{
        Shared, files::Files, hub::Hub, live::ReloadState, proxy::Proxy, resources::StoreError,
    },
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
pub(crate) async fn serve_stream(
    send: SendStream,
    recv: RecvStream,
    shared: Arc<Shared>,
    upload_slot: Arc<Semaphore>,
) {
    let io = TokioIo::new(io::join(recv, send));
    let service = service_fn(move |request| {
        let shared = Arc::clone(&shared);
        let upload_slot = Arc::clone(&upload_slot);
        async move { Ok::<_, Infallible>(handle(request, &shared, &upload_slot).await) }
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

async fn handle(
    request: Request<Incoming>,
    shared: &Shared,
    upload_slot: &Semaphore,
) -> Response<AppBody> {
    let path = request.uri().path();
    let result = if path == UPLOAD_PATH {
        upload(request, &shared.hub, upload_slot).await
    } else if path.starts_with(RESERVED_PREFIX) {
        Err(RequestError::new(
            StatusCode::NOT_FOUND,
            "reserved jamparty resource not found",
        ))
    } else {
        match &shared.site {
            Site::Proxy(proxy) => proxy.handle(request).await,
            Site::Files { files, reload } => files.handle(request, reload.current()).await,
        }
    };
    result.unwrap_or_else(|error| {
        tracing::debug!(status = %error.status, error = %Untrusted(&error.message), "request refused");
        error.into()
    })
}

async fn upload(
    request: Request<Incoming>,
    hub: &Hub,
    upload_slot: &Semaphore,
) -> Result<Response<AppBody>, RequestError> {
    if request.method() != Method::POST {
        return Err(RequestError::method_not_allowed("POST"));
    }
    let _slot = upload_slot.try_acquire().map_err(|_| {
        RequestError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "this connection already has an attachment upload in progress",
        )
    })?;
    let _permit = hub.uploads.try_acquire().map_err(|_| {
        RequestError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "the share is busy receiving attachments",
        )
    })?;
    let bytes = read_upload(request.into_body()).await?;
    let id = hub.store_resource(bytes).map_err(|error| match error {
        StoreError::FileTooLarge => RequestError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "an attachment can be at most 8 MiB",
        ),
        StoreError::Full => RequestError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "the share's attachment storage is full",
        ),
    })?;
    Ok(json_response(StatusCode::CREATED, &Uploaded { id }))
}
