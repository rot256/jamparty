//! In-process shares and joiners over iroh with direct addresses, a
//! fixture site and a small HTTP client.

use std::{
    env, fs, future::Future, net::SocketAddr, ops::Deref, path::PathBuf, sync::Arc, time::Duration,
};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::{
    HeaderMap, Request, Response, StatusCode,
    body::Incoming,
    client::conn::http1,
    header::{self, HeaderValue},
    http::request::Builder as RequestBuilder,
};
use hyper_util::rt::TokioIo;
use iroh::{Endpoint, SecretKey, endpoint::Connection};
use jiff::Timestamp;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    net::{TcpSocket, TcpStream},
    time,
};
use tokio_websockets::{ClientBuilder, Message, WebSocketStream};

use crate::{
    Report,
    adapters::endpoint::{Accept, Discovery, NetConfig, bind, connect, generate_key},
    core::{clock::ManualClock, port::Port, rng::SystemRng, token::PartyToken},
    join::{JoinConfig, Joiner},
    share::{Share, ShareConfig, Source, delivery::Target, proxy::UpstreamUrl},
};

/// Every wait in these tests ends within this.
const PATIENCE: Duration = Duration::from_secs(15);

pub(crate) const FIXTURE_HTML: &str =
    "<!doctype html><html><body><h1 id=\"title\">Fixture</h1></body></html>";

/// Timeouts with room for a loaded machine, well inside [`PATIENCE`]: the
/// tests that drop a connection close it (the peer hears at once), so none
/// waits for the idle timeout.
pub(crate) fn net() -> NetConfig {
    NetConfig {
        discovery: Discovery::Direct,
        keep_alive: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(15),
        relay: None,
    }
}

pub(crate) async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    time::timeout(PATIENCE, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// Polls `check` until it holds.
pub(crate) async fn until(what: &str, mut check: impl FnMut() -> bool) {
    within(what, async {
        while !check() {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
}

/// A directory with an `index.html`, a stylesheet and a dotfile.
pub(crate) fn fixture_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("index.html"), FIXTURE_HTML).unwrap();
    fs::write(dir.path().join("style.css"), "h1 { color: red }").unwrap();
    fs::write(dir.path().join(".secret"), "hidden").unwrap();
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
    let root: PathBuf = fs::canonicalize(dir.path()).unwrap();
    share(Source::Files(root)).await
}

/// A share of `dir` that delivers to `target`, on `clock`.
pub(crate) async fn share_files_to(
    dir: &TempDir,
    target: Target,
    clock: Arc<ManualClock>,
) -> TestShare {
    let root = fs::canonicalize(dir.path()).unwrap();
    let mut config = config(Source::Files(root), target);
    config.clock = clock;
    start(config).await
}

/// When the tests on a manual clock start.
pub(crate) fn t0() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

pub(crate) async fn share_proxy(upstream: SocketAddr) -> TestShare {
    let url: UpstreamUrl = format!("http://{upstream}").parse().unwrap();
    share(Source::Proxy(url)).await
}

/// A joiner with a fresh key, admitted.
pub(crate) async fn joiner(token: PartyToken, addrs: Vec<SocketAddr>, name: &str) -> Joiner {
    joiner_with_key(token, addrs, name, generate_key(&SystemRng)).await
}

/// Starts a joiner of `share` on `port` (none: as `jampot join` without
/// `--port`); fails if the share refuses it or the port cannot be had.
pub(crate) async fn joiner_on(share: &Share, port: Option<Port>) -> Result<Joiner, Report> {
    start_joiner(
        share.token().clone(),
        share.direct_addrs(),
        "alice",
        generate_key(&SystemRng),
        port,
    )
    .await
}

/// A test joiner's configuration: a settings file of its own, any port,
/// requests waiting [`CONNECT_WAIT`] for the share.
fn join_config(
    token: PartyToken,
    addrs: Vec<SocketAddr>,
    name: &str,
    key: SecretKey,
) -> JoinConfig {
    JoinConfig {
        token,
        name: Some(name.parse().unwrap()),
        default_name: "joiner".parse().unwrap(),
        settings: settings_path(),
        key,
        net: net(),
        share_addrs: addrs,
        port: None,
        connect_wait: CONNECT_WAIT,
    }
}

async fn start_joiner(
    token: PartyToken,
    addrs: Vec<SocketAddr>,
    name: &str,
    key: SecretKey,
    port: Option<Port>,
) -> Result<Joiner, Report> {
    let config = JoinConfig {
        port,
        ..join_config(token, addrs, name, key)
    };
    within("the joiner to start", Joiner::start(config)).await
}

/// How long a test joiner's requests wait for the share: well inside
/// [`PATIENCE`], so a test sees the answer rather than its own timeout.
pub(crate) const CONNECT_WAIT: Duration = Duration::from_secs(5);

pub(crate) async fn joiner_with_key(
    token: PartyToken,
    addrs: Vec<SocketAddr>,
    name: &str,
    key: SecretKey,
) -> Joiner {
    joiner_waiting(token, addrs, name, key, CONNECT_WAIT).await
}

/// A joiner whose requests wait `connect_wait` for the share.
pub(crate) async fn joiner_waiting(
    token: PartyToken,
    addrs: Vec<SocketAddr>,
    name: &str,
    key: SecretKey,
    connect_wait: Duration,
) -> Joiner {
    let config = JoinConfig {
        connect_wait,
        ..join_config(token, addrs, name, key)
    };
    Joiner::start(config).await.unwrap()
}

/// Starts a joiner that may be refused: the start's outcome.
pub(crate) async fn try_joiner(
    token: PartyToken,
    addrs: Vec<SocketAddr>,
) -> Result<Joiner, Report> {
    start_joiner(token, addrs, "alice", generate_key(&SystemRng), None).await
}

/// A settings file in a directory of its own, not created until saved.
fn settings_path() -> PathBuf {
    env::temp_dir()
        .join(format!("jampot-test-{}", uuid::Uuid::new_v4()))
        .join("settings.json")
}

/// A joiner of `share`, admitted.
pub(crate) async fn join(share: &Share, name: &str) -> Joiner {
    let joiner = joiner(share.token().clone(), share.direct_addrs(), name).await;
    connected(&joiner).await;
    joiner
}

pub(crate) async fn connected(joiner: &Joiner) {
    let mut snapshot = joiner.watch();
    within(
        "admission",
        snapshot.wait_for(|snapshot| snapshot.link.connected),
    )
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
pub(crate) fn local(joiner: &Joiner, method: &str, path: &str) -> RequestBuilder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, joiner.origin().host())
}

/// Sends `request` to `addr` and returns the response head, body unread.
pub(crate) async fn send(addr: SocketAddr, request: Request<Full<Bytes>>) -> Response<Incoming> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await.unwrap();
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

/// A page's socket to the joiner, spoken by hand.
pub(crate) struct PageSocket {
    socket: WebSocketStream<TcpStream>,
    /// The last request number used.
    n: u64,
}

/// The `type` of a page message.
fn kind(message: &Value) -> &str {
    message.get("type").and_then(Value::as_str).unwrap_or("")
}

impl PageSocket {
    /// Opens the joiner's socket as a page on its origin does; the `hello`
    /// is the first message waiting.
    pub(crate) async fn open(joiner: &Joiner) -> Self {
        let stream = TcpStream::connect(joiner.local_addr()).await.unwrap();
        Self::open_on(joiner, stream).await
    }

    /// Opens the socket with a small receive buffer, as a page that reads
    /// slowly.
    pub(crate) async fn open_slow(joiner: &Joiner) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(4096).unwrap();
        let stream = socket.connect(joiner.local_addr()).await.unwrap();
        Self::open_on(joiner, stream).await
    }

    async fn open_on(joiner: &Joiner, stream: TcpStream) -> Self {
        let uri = format!("ws://{}{SOCKET}", joiner.origin().host());
        let (socket, _) = within(
            "the socket to open",
            ClientBuilder::new()
                .uri(&uri)
                .unwrap()
                .add_header(
                    header::ORIGIN,
                    HeaderValue::from_str(joiner.origin().origin()).unwrap(),
                )
                .unwrap()
                .connect_on(stream),
        )
        .await
        .unwrap();
        Self { socket, n: 0 }
    }

    /// Opens the socket and reads its `hello`.
    pub(crate) async fn hello(joiner: &Joiner) -> (Self, Value) {
        let mut page = Self::open(joiner).await;
        let hello = page.next().await;
        assert_eq!(kind(&hello), "hello", "{hello}");
        (page, hello)
    }

    /// Opens the socket and says the page is on `url`.
    pub(crate) async fn on(joiner: &Joiner, url: &str) -> Self {
        let (mut page, _) = Self::hello(joiner).await;
        page.visit(Some(url)).await;
        page
    }

    pub(crate) async fn send(&mut self, message: Value) {
        self.socket
            .send(Message::text(message.to_string()))
            .await
            .unwrap();
    }

    /// The next message, pings included.
    pub(crate) async fn next(&mut self) -> Value {
        let message = within("a page message", self.socket.next())
            .await
            .expect("the socket closed")
            .unwrap();
        let text = message.as_text().expect("a text frame");
        serde_json::from_str(text).unwrap()
    }

    /// Every message up to and including the next of type `wanted`.
    pub(crate) async fn until(&mut self, wanted: &str) -> Vec<Value> {
        let mut seen = Vec::new();
        loop {
            let message = self.next().await;
            let done = kind(&message) == wanted;
            seen.push(message);
            if done {
                return seen;
            }
        }
    }

    /// The next message of type `wanted`.
    pub(crate) async fn next_of(&mut self, wanted: &str) -> Value {
        self.until(wanted).await.pop().unwrap()
    }

    /// Whether the socket carries a message of type `wanted` within
    /// `wait`.
    pub(crate) async fn hears(&mut self, wanted: &str, wait: Duration) -> bool {
        time::timeout(wait, async {
            while let Some(Ok(message)) = self.socket.next().await {
                if let Some(text) = message.as_text()
                    && kind(&serde_json::from_str(text).unwrap()) == wanted
                {
                    return;
                }
            }
            std::future::pending::<()>().await;
        })
        .await
        .is_ok()
    }

    /// How the socket ended: its close frame's code, once the joiner
    /// closes it.
    pub(crate) async fn closed(&mut self) -> Option<u16> {
        within("the socket to close", async {
            while let Some(message) = self.socket.next().await {
                match message {
                    Ok(message) => {
                        if let Some((code, _)) = message.as_close() {
                            return Some(u16::from(code));
                        }
                    }
                    Err(_) => return None,
                }
            }
            None
        })
        .await
    }

    /// Says the page is on `url` (none: it cannot say).
    pub(crate) async fn visit(&mut self, url: Option<&str>) {
        self.send(json!({"type": "page", "url": url})).await;
    }

    /// Moves the pointer (`position` none: it left).
    pub(crate) async fn cursor(&mut self, position: Option<Value>) {
        self.send(json!({"type": "cursor", "position": position}))
            .await;
    }

    /// Sends request `body` (its `n` filled in) and returns the messages
    /// up to its reply, the reply last.
    pub(crate) async fn request(&mut self, mut body: Value) -> Vec<Value> {
        self.n += 1;
        let n = self.n;
        body.as_object_mut()
            .unwrap()
            .insert("n".to_owned(), n.into());
        self.send(body).await;
        let mut seen = Vec::new();
        loop {
            let message = self.next().await;
            let reply = kind(&message) == "reply" && message["to"] == n;
            seen.push(message);
            if reply {
                return seen;
            }
        }
    }

    /// Sends a comment (a JSON draft, with a fresh `ref` unless it has
    /// one); the reply's outcome.
    pub(crate) async fn comment(&mut self, draft: &str) -> Value {
        let mut body: Value = serde_json::from_str(draft).unwrap();
        let object = body.as_object_mut().unwrap();
        object.insert("type".to_owned(), "comment".into());
        object
            .entry("ref")
            .or_insert_with(|| uuid::Uuid::new_v4().to_string().into());
        let mut seen = self.request(body).await;
        seen.pop().unwrap()["outcome"].take()
    }

    /// Changes the settings (a JSON change); the reply's outcome.
    pub(crate) async fn settings(&mut self, change: &str) -> Value {
        let change: Value = serde_json::from_str(change).unwrap();
        let mut seen = self
            .request(json!({"type": "settings", "change": change}))
            .await;
        seen.pop().unwrap()["outcome"].take()
    }
}

/// Sends a comment from a page of `joiner`, as [`PageSocket::comment`].
pub(crate) async fn post_comment(joiner: &Joiner, draft: &str) -> Value {
    let (mut page, _) = PageSocket::hello(joiner).await;
    page.comment(draft).await
}

/// The id of the comment an outcome says was queued, if it was.
pub(crate) fn commented(outcome: &Value) -> Option<String> {
    outcome["commented"]["id"].as_str().map(str::to_owned)
}

/// `messages` without presence, pings and live-reload revisions, which
/// come at any time (a shared directory's revision follows admission, so
/// it may reach the joiner after a page's hello).
pub(crate) fn about(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .filter(|message| !matches!(kind(message), "people" | "ping" | "reload"))
        .collect()
}

/// The joiner's page socket.
pub(crate) const SOCKET: &str = "/_jampot/api/socket";

/// A WebSocket handshake for the joiner's socket, `origin` as given; for
/// checking refusals.
pub(crate) fn handshake(joiner: &Joiner, origin: Option<&str>) -> RequestBuilder {
    let request = local(joiner, "GET", SOCKET)
        .header(header::UPGRADE, "websocket")
        .header(header::CONNECTION, "upgrade")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==");
    match origin {
        Some(origin) => request.header(header::ORIGIN, origin),
        None => request,
    }
}
