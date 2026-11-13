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
        page.body.contains("/_jampot/client.js?live=0"),
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

    let client = get(&joiner, "/_jampot/client.js").await;
    assert_eq!(client.status, 200);
    assert!(client.body.contains("api/events"));

    for hidden in ["/.secret", "/%2esecret", "/missing", "/_jampot/other"] {
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
    let script = format!("/_jampot/client.js?version={}\"", env!("CARGO_PKG_VERSION"));
    assert!(page.body.contains(&script), "{}", page.body);
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
    // `jampot join` opens the browser before it is connected: the first
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
    within("admission", view.wait_for(|view| view.connected))
        .await
        .unwrap();
    share.stop().await;
    until("the joiner notices", || !joiner.watch().borrow().connected).await;

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

    // Anything else (the page's own fetches) keeps the JSON answer.
    let asset = get(&joiner, "/style.css").await;
    assert_eq!(asset.status, StatusCode::BAD_GATEWAY);
    assert_eq!(asset.body, r#"{"error":"the share is not reachable"}"#);
}
