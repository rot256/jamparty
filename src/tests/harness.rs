//! In-process shares and joiners over iroh with direct addresses, a
//! fixture site and a small HTTP client.

use std::{future::Future, net::SocketAddr, ops::Deref, path::PathBuf, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{HeaderMap, Request, Response, StatusCode, body::Incoming, header};
use hyper_util::rt::TokioIo;
use iroh::{Endpoint, SecretKey, endpoint::Connection};
use tempfile::TempDir;
use tokio::net::TcpStream;

use crate::{
    adapters::endpoint::{Accept, Discovery, NetConfig, bind, connect, generate_key},
    core::{rng::SystemRng, token::PartyToken},
    join::{JoinConfig, Joiner},
    share::{Share, ShareConfig, Source, delivery::Target, proxy::UpstreamUrl},
};

/// Every wait in these tests ends within this.
pub(crate) const PATIENCE: Duration = Duration::from_secs(15);

pub(crate) const FIXTURE_HTML: &str =
    "<!doctype html><html><body><h1 id=\"title\">Fixture</h1></body></html>";

/// Short keep-alive and idle timeout so lost peers show quickly.
pub(crate) fn net() -> NetConfig {
    NetConfig {
        discovery: Discovery::Direct,
        keep_alive: Duration::from_millis(200),
        idle_timeout: Duration::from_secs(2),
        relay: None,
    }
}

pub(crate) async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(PATIENCE, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// Polls `check` until it holds.
pub(crate) async fn until(what: &str, mut check: impl FnMut() -> bool) {
    within(what, async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
}

/// A directory with an `index.html`, a stylesheet and a dotfile.
pub(crate) fn fixture_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), FIXTURE_HTML).unwrap();
    std::fs::write(dir.path().join("style.css"), "h1 { color: red }").unwrap();
    std::fs::write(dir.path().join(".secret"), "hidden").unwrap();
    dir
}

/// A share's configuration for tests: direct addresses, short hello
/// timeout; [`start`] gives it a runtime directory.
pub(crate) fn config(source: Source, target: Target) -> ShareConfig {
    let mut config = ShareConfig::new(source, net(), target, PathBuf::new());
    config.admission_timeout = Duration::from_secs(2);
    config
}

/// A share with its own runtime directory, removed with it.
pub(crate) struct TestShare {
    share: Share,
    _runtime: TempDir,
}

impl Deref for TestShare {
    type Target = Share;

    fn deref(&self) -> &Share {
        &self.share
    }
}

impl TestShare {
    pub(crate) async fn stop(self) {
        self.share.stop().await;
    }
}

/// Starts a share from `config`, in a fresh runtime directory.
pub(crate) async fn start(mut config: ShareConfig) -> TestShare {
    let runtime = tempfile::tempdir().unwrap();
    config.runtime_dir = runtime.path().join("run");
    TestShare {
        share: Share::start(config).await.unwrap(),
        _runtime: runtime,
    }
}

pub(crate) async fn share(source: Source) -> TestShare {
    start(config(source, Target::Recv)).await
}

pub(crate) async fn share_files(dir: &TempDir) -> TestShare {
    let root: PathBuf = std::fs::canonicalize(dir.path()).unwrap();
    share(Source::Files(root)).await
}

pub(crate) async fn share_proxy(upstream: SocketAddr) -> TestShare {
    let url: UpstreamUrl = format!("http://{upstream}").parse().unwrap();
    share(Source::Proxy(url)).await
}

/// A joiner with a fresh key, not yet necessarily admitted.
pub(crate) async fn joiner(token: PartyToken, addrs: Vec<SocketAddr>, name: &str) -> Joiner {
    joiner_with_key(token, addrs, name, generate_key(&SystemRng)).await
}

pub(crate) async fn joiner_with_key(
    token: PartyToken,
    addrs: Vec<SocketAddr>,
    name: &str,
    key: SecretKey,
) -> Joiner {
    Joiner::start(JoinConfig {
        token,
        name: name.parse().unwrap(),
        key,
        net: net(),
        share_addrs: addrs,
    })
    .await
    .unwrap()
}

/// A joiner of `share`, admitted.
pub(crate) async fn join(share: &Share, name: &str) -> Joiner {
    let joiner = joiner(share.token().clone(), share.direct_addrs(), name).await;
    connected(&joiner).await;
    joiner
}

pub(crate) async fn connected(joiner: &Joiner) {
    let mut view = joiner.watch();
    within("admission", view.wait_for(|view| view.connected))
        .await
        .unwrap();
}

/// A raw iroh endpoint connected to `share`, for speaking the protocol by
/// hand.
pub(crate) async fn raw_connection(share: &Share) -> (Endpoint, Connection) {
    let endpoint = bind(&net(), generate_key(&SystemRng), Accept::Nothing)
        .await
        .unwrap();
    let connection = connect(&endpoint, share.token().share, &share.direct_addrs())
        .await
        .unwrap();
    (endpoint, connection)
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: String,
}

/// A request to the joiner's local server, `Host` set to its origin.
pub(crate) fn local(joiner: &Joiner, method: &str, path: &str) -> hyper::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, joiner.origin().host())
}

/// Sends `request` to `addr` and returns the response head, body unread.
pub(crate) async fn send(addr: SocketAddr, request: Request<Full<Bytes>>) -> Response<Incoming> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(connection.with_upgrades());
    within("a response", sender.send_request(request))
        .await
        .unwrap()
}

/// Sends `request` to the joiner and reads the whole response.
pub(crate) async fn fetch(joiner: &Joiner, request: Request<Full<Bytes>>) -> Reply {
    let response = send(joiner.local_addr(), request).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = within("a response body", response.into_body().collect())
        .await
        .unwrap()
        .to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

pub(crate) async fn get(joiner: &Joiner, path: &str) -> Reply {
    fetch(
        joiner,
        local(joiner, "GET", path).body(Full::default()).unwrap(),
    )
    .await
}

/// A comment draft on the fixture's title.
pub(crate) const DRAFT: &str = r##"{"page":"http://x.localhost/","selector":"#title","selectedText":"Fixture","comment":"Bigger, please"}"##;

/// Posts a comment the way the page does, with a fresh `ref` unless
/// `body` (a JSON object) has one.
pub(crate) async fn post_comment(joiner: &Joiner, body: &str) -> Reply {
    let mut json: serde_json::Value = serde_json::from_str(body).unwrap();
    if let Some(object) = json.as_object_mut() {
        object
            .entry("ref")
            .or_insert_with(|| uuid::Uuid::new_v4().to_string().into());
    }
    let body = json.to_string();
    let request = local(joiner, "POST", "/_jampot/api/comments")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, joiner.origin().origin())
        .header("sec-fetch-site", "same-origin")
        .body(Full::new(Bytes::from(body)))
        .unwrap();
    fetch(joiner, request).await
}

/// Opens the joiner's event stream.
pub(crate) async fn events(joiner: &Joiner) -> Incoming {
    let request = local(joiner, "GET", "/_jampot/api/events")
        .body(Full::default())
        .unwrap();
    let response = send(joiner.local_addr(), request).await;
    assert_eq!(response.status(), 200);
    response.into_body()
}

/// The next event of type `kind` on `body`, as its JSON text.
pub(crate) async fn next_event(body: &mut Incoming, kind: &str) -> String {
    let needle = format!("\"type\":\"{kind}\"");
    within(&format!("a {kind} event"), async {
        loop {
            let frame = body.frame().await.unwrap().unwrap();
            let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
            if let Some(data) = text.strip_prefix("data: ")
                && data.contains(&needle)
            {
                return data.trim_end().to_owned();
            }
        }
    })
    .await
}
