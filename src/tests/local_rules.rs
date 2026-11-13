//! The joiner's local server end to end: exact `Host`, cross-site rules,
//! `Origin` on writes and upgrades, frame-ancestors on every response.

use std::{
    io::ErrorKind,
    net::{Ipv6Addr, SocketAddr, TcpListener},
};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, header};

use crate::tests::{
    harness::{
        Reply, SOCKET, fetch, fixture_dir, get, handshake, join, local, send, share_files,
        share_proxy,
    },
    upstream,
};

#[tokio::test(flavor = "multi_thread")]
async fn host_and_cross_site_rules() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let origin = joiner.origin().origin().to_owned();
    let port = joiner.local_addr().port();

    let with_host = |host: String| {
        let request = Request::get("/")
            .header(header::HOST, host)
            .body(Full::default())
            .unwrap();
        fetch(&joiner, request)
    };
    // DNS rebinding and near misses.
    for host in [
        format!("evil.example:{port}"),
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("{}.evil:{port}", joiner.origin().host()),
        format!("{}.localhost:{}", "a".repeat(16), port),
        joiner.origin().host().replace(&port.to_string(), "1"),
    ] {
        assert_eq!(with_host(host.clone()).await.status, 421, "{host}");
    }
    assert_eq!(
        with_host(joiner.origin().host().to_owned()).await.status,
        200
    );

    let send = |method: &str, path: &str, headers: &[(&str, &str)]| {
        let mut request = local(&joiner, method, path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let body = if method == "POST" {
            Full::new(Bytes::from_static(b"{}"))
        } else {
            Full::default()
        };
        fetch(&joiner, request.body(body).unwrap())
    };
    let navigate = [
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
    ];
    let cross_fetch = [
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", "cors"),
        ("sec-fetch-dest", "empty"),
    ];

    // Top-level navigation from anywhere works; it carries frame-ancestors.
    let page: Reply = send("GET", "/", &navigate).await;
    assert_eq!(page.status, 200);
    let csp: Vec<_> = page
        .headers
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .collect();
    assert_eq!(csp, ["frame-ancestors 'self'"]);

    // Another site cannot read pages or the status.
    for path in ["/", "/_jampot/api/status"] {
        let reply = send("GET", path, &cross_fetch).await;
        assert_eq!(reply.status, 403, "{path}");
        assert!(reply.headers.contains_key(header::CONTENT_SECURITY_POLICY));
    }
    // Upgrades elsewhere need the exact Origin too.
    let upgrade = [("connection", "upgrade"), ("upgrade", "websocket")];
    assert_eq!(send("GET", "/socket", &upgrade).await.status, 403);
    // Reserved paths take only their method.
    assert_eq!(
        send(
            "POST",
            "/_jampot/api/status",
            &[("origin", origin.as_str())]
        )
        .await
        .status,
        405
    );
    assert_eq!(send("GET", SOCKET, &[]).await.status, 405);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_socket_needs_the_exact_origin() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let origin = joiner.origin().origin().to_owned();
    let port = joiner.local_addr().port();
    let open = |origin: Option<String>, headers: &[(&'static str, &'static str)]| {
        let mut request = handshake(&joiner, origin.as_deref());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        fetch(&joiner, request.body(Full::default()).unwrap())
    };
    // Another site's page, a forged or near-miss Origin, or none at all.
    for wrong in [
        None,
        Some("null".to_owned()),
        Some("http://evil.example".to_owned()),
        Some(origin.replace("http://", "https://")),
        Some(format!("{origin}/")),
        Some(origin.replace(&port.to_string(), "1")),
        Some(format!("http://localhost:{port}")),
    ] {
        assert_eq!(open(wrong.clone(), &[]).await.status, 403, "{wrong:?}");
    }
    // A cross-site fetch claiming our Origin.
    let cross = [("sec-fetch-site", "cross-site")];
    assert_eq!(open(Some(origin.clone()), &cross).await.status, 403);
    // Our Origin, from our page (or without fetch metadata): it opens.
    let same = [("sec-fetch-site", "same-origin")];
    let opened = open(Some(origin.clone()), &same).await;
    assert_eq!(opened.status, 101, "{}", opened.body);
    // Anything but a version-13 WebSocket handshake is refused.
    let mut request = handshake(&joiner, Some(&origin))
        .body(Full::default())
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-version", "8".parse().unwrap());
    assert_eq!(fetch(&joiner, request).await.status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ipv6_side_of_the_port_is_taken_too() {
    if TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).is_err() {
        return; // No IPv6 loopback here: nobody can take that side.
    }
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let port = joiner.local_addr().port();
    let taken = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).unwrap_err();
    assert_eq!(taken.kind(), ErrorKind::AddrInUse);
    // And it is the joiner's: it answers there.
    let request = local(&joiner, "GET", "/").body(Full::default()).unwrap();
    let response = send(SocketAddr::from((Ipv6Addr::LOCALHOST, port)), request).await;
    assert_eq!(response.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiners_paths_never_reach_the_site() {
    let upstream = upstream::start().await;
    let share = share_proxy(upstream.addr).await;
    let joiner = join(&share, "alice").await;
    // The site's own 404s come from the site.
    let site = get(&joiner, "/elsewhere").await;
    assert_eq!((site.status.as_u16(), site.body.as_str()), (404, "missing"));
    // Under /_jampot/api/ the joiner answers, whatever the path.
    for path in [
        "/_jampot/api/events",
        "/_jampot/api/comments",
        "/_jampot/api/status/",
        "/_jampot/api/",
        "/_jampot/api/socket/x",
    ] {
        let reply = get(&joiner, path).await;
        assert_eq!(reply.status, 404, "{path}");
        assert!(
            reply.body.contains("no such jampot path"),
            "{path}: {}",
            reply.body
        );
    }
    // Writes too (with the origin the rules want).
    let origin = joiner.origin().origin().to_owned();
    let post = local(&joiner, "POST", "/_jampot/api/comments")
        .header(header::ORIGIN, origin)
        .body(Full::new(Bytes::from_static(b"{}")))
        .unwrap();
    assert_eq!(fetch(&joiner, post).await.status, 404);
}
