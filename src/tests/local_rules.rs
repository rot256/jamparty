//! The joiner's local server end to end: exact `Host`, cross-site rules,
//! `Origin` on writes and upgrades, frame-ancestors on every response.

use std::net::{Ipv6Addr, SocketAddr};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, header};

use crate::tests::harness::{Reply, fetch, fixture_dir, join, local, send, share_files};

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

    // Another site cannot read pages, the status or the event stream.
    for path in ["/", "/_jamjam/api/status", "/_jamjam/api/events"] {
        let reply = send("GET", path, &cross_fetch).await;
        assert_eq!(reply.status, 403, "{path}");
        assert!(reply.headers.contains_key(header::CONTENT_SECURITY_POLICY));
    }
    // Nor post a comment, even as a form navigation or with our Origin
    // claimed by a cross-site fetch.
    let post_navigate = [
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
        ("origin", "http://evil.example"),
    ];
    let cross_post = [
        ("sec-fetch-site", "cross-site"),
        ("origin", origin.as_str()),
    ];
    for headers in [&post_navigate[..], &cross_post[..]] {
        assert_eq!(
            send("POST", "/_jamjam/api/comments", headers).await.status,
            403
        );
    }
    // Same-origin writes need the exact Origin.
    let same_site_wrong_origin = [("sec-fetch-site", "same-site"), ("origin", origin.as_str())];
    assert_eq!(
        send("POST", "/_jamjam/api/comments", &same_site_wrong_origin)
            .await
            .status,
        403
    );
    assert_eq!(
        send(
            "POST",
            "/_jamjam/api/comments",
            &[("origin", "http://evil.example")]
        )
        .await
        .status,
        403
    );
    // Upgrades need the exact Origin too.
    let upgrade = [("connection", "upgrade"), ("upgrade", "websocket")];
    assert_eq!(send("GET", "/socket", &upgrade).await.status, 403);
    // Reserved paths take only their method.
    assert_eq!(send("GET", "/_jamjam/api/comments", &[]).await.status, 405);
    assert_eq!(
        send(
            "POST",
            "/_jamjam/api/status",
            &[("origin", origin.as_str())]
        )
        .await
        .status,
        405
    );
    // A same-origin POST with the right Origin reaches the handler, which
    // then wants JSON.
    assert_eq!(
        send(
            "POST",
            "/_jamjam/api/comments",
            &[
                ("origin", origin.as_str()),
                ("sec-fetch-site", "same-origin")
            ]
        )
        .await
        .status,
        415
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ipv6_side_of_the_port_is_taken_too() {
    if std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).is_err() {
        return; // No IPv6 loopback here: nobody can take that side.
    }
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let port = joiner.local_addr().port();
    let taken = std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, port)).unwrap_err();
    assert_eq!(taken.kind(), std::io::ErrorKind::AddrInUse);
    // And it is the joiner's: it answers there.
    let request = local(&joiner, "GET", "/").body(Full::default()).unwrap();
    let response = send(SocketAddr::from((Ipv6Addr::LOCALHOST, port)), request).await;
    assert_eq!(response.status(), 200);
}
