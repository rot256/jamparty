//! Requests through the joiner's local server to the share: static files,
//! a proxied dev server, streaming and upgrades, live reload.

use std::{fs, os::unix::fs::symlink, time::Duration};

use bytes::Bytes;
use futures_util::future;
use http_body_util::{BodyExt, Full};
use hyper::{StatusCode, header, upgrade};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    adapters::endpoint::generate_key,
    core::rng::SystemRng,
    join::Joiner,
    protocol::control::Revision,
    tests::{
        harness::{
            Reply, fetch, fixture_dir, get, join, joiner, joiner_waiting, local, send, share_files,
            share_proxy, until, within,
        },
        upstream,
    },
};

#[tokio::test(flavor = "multi_thread")]
async fn static_files_with_the_client_injected() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;

    let page = get(&joiner, "/").await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("<h1 id=\"title\">Fixture</h1>"));
    assert!(
        page.body.contains("/_jamparty/client.js?live=0"),
        "{}",
        page.body
    );
    assert_eq!(
        page.headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        "frame-ancestors 'self'"
    );

    let css = get(&joiner, "/style.css").await;
    assert_eq!(css.status, 200);
    assert_eq!(css.body, "h1 { color: red }");

    for hidden in ["/.secret", "/%2esecret", "/missing", "/_jamparty/other"] {
        assert_eq!(get(&joiner, hidden).await.status, 404, "{hidden}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn symlinks_to_dotfiles_or_out_of_the_root_are_not_served() {
    let dir = fixture_dir();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("other.txt"), "outside").unwrap();
    symlink(".secret", dir.path().join("public.txt")).unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/index.html"), "hidden").unwrap();
    symlink(".git", dir.path().join("docs")).unwrap();
    symlink(outside.path(), dir.path().join("out")).unwrap();
    symlink("style.css", dir.path().join("link.css")).unwrap();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    for refused in [
        "/public.txt",
        "/docs/",
        "/docs/index.html",
        "/out/other.txt",
    ] {
        let reply = get(&joiner, refused).await;
        assert_eq!(reply.status, 404, "{refused}: {}", reply.body);
    }
    assert_eq!(get(&joiner, "/link.css").await.body, "h1 { color: red }");
}

#[tokio::test(flavor = "multi_thread")]
async fn live_reload_reaches_the_joiner() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let mut view = joiner.watch();
    within(
        "the first revision",
        view.wait_for(|view| view.revision == Some(Revision::default())),
    )
    .await
    .unwrap();
    fs::write(dir.path().join("style.css"), "h1 { color: blue }").unwrap();
    within(
        "a new revision",
        view.wait_for(|view| {
            view.revision
                .is_some_and(|revision| revision > Revision::default())
        }),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn proxied_pages_and_streams_are_not_buffered() {
    let upstream = upstream::start().await;
    let share = share_proxy(upstream.addr).await;
    let joiner = join(&share, "alice").await;

    let page = get(&joiner, "/").await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Fixture"));
    assert!(
        page.body.contains("/_jamparty/client.js\""),
        "{}",
        page.body
    );
    assert_eq!(get(&joiner, "/nowhere").await.status, 404);

    // The first chunk arrives while the upstream still holds the second.
    let request = local(&joiner, "GET", "/stream")
        .body(Full::default())
        .unwrap();
    let response = send(joiner.local_addr(), request).await;
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let first = within("the first chunk", body.frame())
        .await
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    assert_eq!(first, Bytes::from_static(b"first\n"));
    upstream.release.notify_one();
    let rest = within("the rest", body.collect()).await.unwrap().to_bytes();
    assert_eq!(rest, Bytes::from_static(b"second\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrades_echo_through_join_and_share() {
    let upstream = upstream::start().await;
    let share = share_proxy(upstream.addr).await;
    let joiner = join(&share, "alice").await;

    let request = local(&joiner, "GET", "/echo")
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "echo")
        .header(header::ORIGIN, joiner.origin().origin())
        .body(Full::default())
        .unwrap();
    let response = send(joiner.local_addr(), request).await;
    assert_eq!(response.status(), 101);
    let upgraded = within("the upgrade", upgrade::on(response)).await.unwrap();
    let mut io = TokioIo::new(upgraded);
    for message in [b"ping".as_slice(), b"pong again"] {
        io.write_all(message).await.unwrap();
        let mut echoed = vec![0; message.len()];
        within("the echo", io.read_exact(&mut echoed))
            .await
            .unwrap();
        assert_eq!(echoed, message);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_requests_each_get_a_stream() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let requests = (0..8).map(|_| {
        fetch(
            &joiner,
            local(&joiner, "GET", "/style.css")
                .body(Full::default())
                .unwrap(),
        )
    });
    for reply in future::join_all(requests).await {
        assert_eq!(reply.status, 200);
    }
}

/// A page load as a browser sends it: a top-level navigation.
async fn load_page(joiner: &Joiner, path: &str) -> Reply {
    let request = local(joiner, "GET", path)
        .header("sec-fetch-site", "none")
        .header("sec-fetch-mode", "navigate")
        .header("sec-fetch-dest", "document")
        .body(Full::default())
        .unwrap();
    fetch(joiner, request).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_loaded_while_joining_waits_for_the_share() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    // `jamparty join` opens the browser before it is connected: the first
    // load must wait for the share rather than fail.
    let joiner = joiner(share.token().clone(), share.direct_addrs(), "alice").await;
    let page = load_page(&joiner, "/").await;
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(page.body.contains("Fixture"), "{}", page.body);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_share_a_page_load_gets_a_page_that_retries() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = joiner_waiting(
        share.token().clone(),
        share.direct_addrs(),
        "alice",
        generate_key(&SystemRng),
        Duration::from_millis(300),
    )
    .await;
    let mut view = joiner.watch();
    within("admission", view.wait_for(|view| view.link.connected))
        .await
        .unwrap();
    share.stop().await;
    until("the joiner notices", || {
        !joiner.watch().borrow().link.connected
    })
    .await;

    let page = load_page(&joiner, "/").await;
    assert_eq!(page.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        page.headers.get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    assert_eq!(page.headers.get(header::RETRY_AFTER).unwrap(), "2");
    assert!(
        page.body.contains("http-equiv=\"refresh\""),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("Connecting to the share"),
        "{}",
        page.body
    );

    assert!(
        page.body
            .contains(r#"<link rel="icon" href="/_jamparty/favicon.svg" type="image/svg+xml">"#)
    );

    // Anything else (the page's own fetches) keeps the JSON answer.
    let asset = get(&joiner, "/style.css").await;
    assert_eq!(asset.status, StatusCode::BAD_GATEWAY);
    assert_eq!(asset.body, r#"{"error":"the share is not reachable"}"#);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiner_serves_the_client_itself() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = joiner_waiting(
        share.token().clone(),
        share.direct_addrs(),
        "alice",
        generate_key(&SystemRng),
        Duration::from_millis(300),
    )
    .await;
    let mut view = joiner.watch();
    within("admission", view.wait_for(|view| view.link.connected))
        .await
        .unwrap();
    // With the share gone, the client still comes: from the joiner.
    share.stop().await;
    until("the joiner notices", || {
        !joiner.watch().borrow().link.connected
    })
    .await;
    let client = get(&joiner, "/_jamparty/client.js?live=3&version=0.0.1").await;
    assert_eq!(client.status, 200);
    assert_eq!(
        client.headers.get(header::CONTENT_TYPE).unwrap(),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        client.headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(client.body, include_str!("../../assets/client.js"));
    let head = local(&joiner, "HEAD", "/_jamparty/client.js")
        .body(Full::default())
        .unwrap();
    assert_eq!(fetch(&joiner, head).await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn embedded_assets_negotiate_encoding_and_head_lengths() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let cmaps = include_bytes!("../../assets/pdf-cmaps.bin");
    let cmap_range = include!("../../assets/pdf-cmaps.rs");
    let cmap = cmaps.get(cmap_range("78-EUC-H.bcmap").unwrap()).unwrap();

    for (path, raw, content_type) in [
        (
            "/_jamparty/client.js",
            include_bytes!("../../assets/client.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        (
            "/_jamparty/favicon.svg",
            include_bytes!("../../assets/favicon.svg").as_slice(),
            "image/svg+xml",
        ),
        (
            "/_jamparty/pdf.js",
            include_bytes!("../../assets/pdf.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        (
            "/_jamparty/pdf-worker.js",
            include_bytes!("../../assets/pdf-worker.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        (
            "/_jamparty/pdf.css",
            include_bytes!("../../assets/pdf.css").as_slice(),
            "text/css; charset=utf-8",
        ),
        (
            "/fixture.pdf",
            include_bytes!("../../assets/pdf.html").as_slice(),
            "text/html; charset=utf-8",
        ),
        (
            "/_jamparty/pdf-cmaps/78-EUC-H.bcmap",
            cmap,
            "application/octet-stream",
        ),
    ] {
        for accept in ["gzip, br", "*;q=1, br;q=0"] {
            let request = |method| {
                let mut request =
                    local(&joiner, method, path).header(header::ACCEPT_ENCODING, accept);
                if path == "/fixture.pdf" {
                    request = request
                        .header("sec-fetch-mode", "navigate")
                        .header("sec-fetch-dest", "document");
                }
                request.body(Full::default()).unwrap()
            };
            let get = send(joiner.local_addr(), request("GET")).await;
            let head = send(joiner.local_addr(), request("HEAD")).await;
            assert_eq!(get.status(), StatusCode::OK, "{path}");
            assert_eq!(head.status(), get.status(), "{path}");
            let headers = get.headers().clone();
            for name in [
                header::CONTENT_LENGTH,
                header::CONTENT_ENCODING,
                header::CONTENT_TYPE,
                header::CACHE_CONTROL,
                header::VARY,
                header::CONTENT_SECURITY_POLICY,
                header::X_CONTENT_TYPE_OPTIONS,
            ] {
                assert_eq!(
                    head.headers().get_all(&name),
                    headers.get_all(&name),
                    "{path} {name}"
                );
            }
            assert_eq!(headers[header::CONTENT_TYPE], content_type);
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            assert!(
                headers
                    .get_all(header::CONTENT_SECURITY_POLICY)
                    .iter()
                    .any(|v| v == "frame-ancestors 'self'")
            );
            if path == "/_jamparty/pdf-worker.js" {
                assert_eq!(
                    headers[header::CONTENT_SECURITY_POLICY],
                    "default-src 'none'"
                );
            }
            if path == "/fixture.pdf" || path == "/_jamparty/pdf.css" {
                assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            }
            if path == "/fixture.pdf" {
                assert_eq!(
                    headers["cross-origin-opener-policy"],
                    "noopener-allow-popups"
                );
                assert!(
                    headers
                        .get_all(header::VARY)
                        .iter()
                        .any(|v| v == "Sec-Fetch-Dest, Sec-Fetch-Mode")
                );
            }
            let body = get.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(headers[header::CONTENT_LENGTH], body.len().to_string());
            assert!(
                head.into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );

            let negotiates = content_type != "application/octet-stream";
            assert_eq!(
                headers
                    .get_all(header::VARY)
                    .iter()
                    .any(|v| v == "Accept-Encoding"),
                negotiates,
            );
            if negotiates && accept == "gzip, br" {
                assert_eq!(headers[header::CONTENT_ENCODING], "br");
                let mut decoded = Vec::new();
                brotli_decompressor::BrotliDecompress(&mut body.as_ref(), &mut decoded).unwrap();
                assert_eq!(decoded, raw, "{path}");
            } else {
                assert!(!headers.contains_key(header::CONTENT_ENCODING), "{path}");
                assert_eq!(body, raw, "{path}");
            }
        }
    }
    assert_eq!(
        get(&joiner, "/_jamparty/pdf-cmaps/78-EUC-H.bcmap.bak")
            .await
            .status,
        StatusCode::NOT_FOUND,
    );

    // Explicit weights decide, also across header lines; excluding both
    // representations is refused.
    for (lines, status, encoding) in [
        (&["identity;q=1, br;q=0.1"][..], StatusCode::OK, None),
        (&["identity;q=1", "br;q=0.1"][..], StatusCode::OK, None),
        (&["identity;q=0.5", "br"][..], StatusCode::OK, Some("br")),
        (
            &["br;q=0", "identity;q=0"][..],
            StatusCode::NOT_ACCEPTABLE,
            None,
        ),
        (&["*;q=0"][..], StatusCode::NOT_ACCEPTABLE, None),
    ] {
        for method in ["GET", "HEAD"] {
            let mut request = local(&joiner, method, "/_jamparty/client.js");
            for line in lines {
                request = request.header(header::ACCEPT_ENCODING, *line);
            }
            let response = send(joiner.local_addr(), request.body(Full::default()).unwrap()).await;
            assert_eq!(response.status(), status, "{method} {lines:?}");
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .map(|value| value.to_str().unwrap()),
                encoding,
                "{method} {lines:?}"
            );
            assert!(
                response
                    .headers()
                    .get_all(header::VARY)
                    .iter()
                    .any(|v| v == "Accept-Encoding"),
                "{method} {lines:?}"
            );
        }
    }
}
