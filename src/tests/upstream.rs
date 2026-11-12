//! A fixture dev server: an HTML page, a streamed body held back until
//! released, and an upgrade that echoes.

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::{
    Request, Response, StatusCode, body::Frame, body::Incoming, header, server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::JoinHandle,
};

use crate::{share::body::AppBody, tests::harness::FIXTURE_HTML};

pub(crate) struct Upstream {
    pub(crate) addr: SocketAddr,
    /// Lets `/stream` send its second chunk.
    pub(crate) release: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn start() -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let release = Arc::new(Notify::new());
    let gate = Arc::clone(&release);
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let gate = Arc::clone(&gate);
                    async move { Ok::<_, Infallible>(respond(request, gate)) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    Upstream {
        addr,
        release,
        task,
    }
}

fn respond(request: Request<Incoming>, gate: Arc<Notify>) -> Response<AppBody> {
    match request.uri().path() {
        "/" => {
            let mut response = full(FIXTURE_HTML);
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response
        }
        "/stream" => {
            let chunks = stream::unfold(0, move |step| {
                let gate = Arc::clone(&gate);
                async move {
                    match step {
                        0 => Some((Ok(Frame::data(Bytes::from_static(b"first\n"))), 1)),
                        1 => {
                            gate.notified().await;
                            Some((Ok(Frame::data(Bytes::from_static(b"second\n"))), 2))
                        }
                        _ => None,
                    }
                }
            });
            let body = StreamBody::new(chunks)
                .map_err(|never: Infallible| match never {})
                .boxed_unsync();
            Response::new(body)
        }
        "/echo" => {
            tokio::spawn(async move {
                let Ok(upgraded) = hyper::upgrade::on(request).await else {
                    return;
                };
                let mut io = TokioIo::new(upgraded);
                let mut buffer = [0; 64];
                while let Ok(read) = io.read(&mut buffer).await {
                    let Some(bytes) = buffer.get(..read).filter(|bytes| !bytes.is_empty()) else {
                        return;
                    };
                    if io.write_all(bytes).await.is_err() {
                        return;
                    }
                }
            });
            let mut response = full("");
            *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
            let headers = response.headers_mut();
            headers.insert(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            );
            headers.insert(header::UPGRADE, header::HeaderValue::from_static("echo"));
            response
        }
        _ => {
            let mut response = full("missing");
            *response.status_mut() = StatusCode::NOT_FOUND;
            response
        }
    }
}

fn full(body: &'static str) -> Response<AppBody> {
    Response::new(
        Full::new(Bytes::from_static(body.as_bytes()))
            .map_err(|never: Infallible| match never {})
            .boxed_unsync(),
    )
}
