//! What the joiner forwards to the share is bounded on its side, whatever
//! the share does: requests at once, the wait for response headers, and
//! upgraded tunnels, which end when the browser's side does or go idle.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::Endpoint;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task::JoinHandle,
};

use crate::{
    adapters::endpoint::{self, Accept, bind, direct_addrs, generate_key},
    core::{
        ids::EndpointKey,
        rng::SystemRng,
        token::{PartySecret, PartyToken},
    },
    join::{Joiner, ProxyLimits},
    tests::harness::{net, try_joiner_with, until, within},
};

const LIMITS: ProxyLimits = ProxyLimits {
    concurrent: 4,
    response_wait: Duration::from_secs(1),
    idle: Duration::from_secs(60),
    half_close: Duration::from_millis(200),
};

/// A share that answers every request with 101 and keeps its side open
/// for as long as the joiner does, except `/slow`, which it never
/// answers. Counts the streams it took and those the joiner ended.
struct HoldingShare {
    joiner: Joiner,
    taken: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
    _endpoint: Endpoint,
    _task: JoinHandle<()>,
}

impl HoldingShare {
    async fn start(limits: ProxyLimits) -> Self {
        let key = generate_key(&SystemRng);
        let token = PartyToken {
            share: EndpointKey::from(key.public()),
            secret: PartySecret::generate(&SystemRng),
        };
        let share = bind(&net(), key, Accept::Jampot).await.unwrap();
        let addrs = direct_addrs(&share);
        let accepting = share.clone();
        let taken = Arc::new(AtomicUsize::new(0));
        let ended = Arc::new(AtomicUsize::new(0));
        let (counted, closed) = (Arc::clone(&taken), Arc::clone(&ended));
        let task = tokio::spawn(async move {
            let connection = accepting.accept().await.unwrap().await.unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            let mut byte = [0];
            while recv.read_exact(&mut byte).await.is_ok() && byte != *b"\n" {}
            let admitted = format!(
                r#"{{"version":1,"message":{{"type":"admitted","you":{{"name":"alice","fingerprint":"ababababab","key":"{}"}},"undelivered":[],"presence":"supported","site":{{"kind":"files"}},"jampot":"{}"}}}}"#,
                "ab".repeat(32),
                env!("CARGO_PKG_VERSION")
            );
            send.write_all(format!("{admitted}\n").as_bytes())
                .await
                .unwrap();
            endpoint::admitted(&connection);
            let _control = (send, recv);
            while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                counted.fetch_add(1, Ordering::Relaxed);
                let closed = Arc::clone(&closed);
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        if recv.read_exact(&mut byte).await.is_err() {
                            closed.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        head.extend_from_slice(&byte);
                    }
                    if !head.starts_with(b"GET /slow ") {
                        let _ = send
                            .write_all(
                                b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
                            )
                            .await;
                    }
                    // Kept open until the joiner ends it.
                    let mut buffer = [0; 1024];
                    while let Ok(Some(_)) = recv.read(&mut buffer).await {}
                    closed.fetch_add(1, Ordering::Relaxed);
                    drop(send);
                });
            }
        });
        let joiner = try_joiner_with(token, addrs, limits).await.unwrap();
        Self {
            joiner,
            taken,
            ended,
            _endpoint: share,
            _task: task,
        }
    }

    fn taken(&self) -> usize {
        self.taken.load(Ordering::Relaxed)
    }

    fn ended(&self) -> usize {
        self.ended.load(Ordering::Relaxed)
    }

    /// A browser's request for `path`, as an upgrade: the socket and the
    /// status the joiner answered.
    async fn upgrade(&self, path: &str) -> (TcpStream, String) {
        let mut socket = TcpStream::connect(self.joiner.local_addr()).await.unwrap();
        let origin = self.joiner.origin();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nSec-Fetch-Site: same-origin\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==\r\n\r\n",
            origin.host(),
            origin.origin()
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut head = Vec::new();
        within("the joiner's answer", async {
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.unwrap());
            }
        })
        .await;
        let head = String::from_utf8(head).unwrap();
        let status = head.split(' ').nth(1).unwrap().to_owned();
        (socket, status)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnels_end_with_the_browser_and_are_bounded() {
    let share = HoldingShare::start(LIMITS).await;
    let mut sockets = Vec::new();
    for _ in 0..LIMITS.concurrent {
        let (socket, status) = share.upgrade("/hmr").await;
        assert_eq!(status, "101");
        sockets.push(socket);
    }
    assert_eq!(share.joiner.proxied(), LIMITS.concurrent);
    // One more is refused without a stream.
    let (_refused, status) = share.upgrade("/hmr").await;
    assert_eq!(status, "503");
    assert_eq!(share.taken(), LIMITS.concurrent);

    // The browser closes: so does every tunnel, though the share keeps
    // its side open.
    drop(sockets);
    until("the share's streams to end", || {
        share.ended() == LIMITS.concurrent
    })
    .await;
    until("the permits to come back", || share.joiner.proxied() == 0).await;
    let (_again, status) = share.upgrade("/hmr").await;
    assert_eq!(status, "101");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_that_does_not_answer_gets_a_deadline() {
    let share = HoldingShare::start(LIMITS).await;
    let started = tokio::time::Instant::now();
    let (_socket, status) = share.upgrade("/slow").await;
    assert_eq!(status, "504");
    assert!(started.elapsed() >= LIMITS.response_wait);
    until("the stream to end", || share.ended() == 1).await;
    until("the permit to come back", || share.joiner.proxied() == 0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_tunnel_is_closed() {
    let limits = ProxyLimits {
        idle: Duration::from_millis(300),
        ..LIMITS
    };
    let share = HoldingShare::start(limits).await;
    let (mut socket, status) = share.upgrade("/hmr").await;
    assert_eq!(status, "101");
    // Something goes through, then nothing: the joiner closes it.
    socket.write_all(b"ping").await.unwrap();
    let mut rest = Vec::new();
    within("the tunnel to close", socket.read_to_end(&mut rest))
        .await
        .unwrap();
    until("the share's stream to end", || share.ended() == 1).await;
    until("the permit to come back", || share.joiner.proxied() == 0).await;
}
