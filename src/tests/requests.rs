//! Requests through the joiner's local server to the share: static files,
//! a proxied dev server, streaming and upgrades, live reload.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tests::{
    harness::{fetch, fixture_dir, get, join, local, send, share_files, share_proxy, within},
    upstream,
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
        page.body.contains("/_jamjam/client.js?live=0"),
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

    let client = get(&joiner, "/_jamjam/client.js").await;
    assert_eq!(client.status, 200);
    assert!(client.body.contains("api/events"));

    for hidden in ["/.secret", "/%2esecret", "/missing", "/_jamjam/other"] {
        assert_eq!(get(&joiner, hidden).await.status, 404, "{hidden}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn symlinks_to_dotfiles_or_out_of_the_root_are_not_served() {
    let dir = fixture_dir();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("other.txt"), "outside").unwrap();
    std::os::unix::fs::symlink(".secret", dir.path().join("public.txt")).unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git/index.html"), "hidden").unwrap();
    std::os::unix::fs::symlink(".git", dir.path().join("docs")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
    std::os::unix::fs::symlink("style.css", dir.path().join("link.css")).unwrap();
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
        view.wait_for(|view| view.revision == Some(0)),
    )
    .await
    .unwrap();
    std::fs::write(dir.path().join("style.css"), "h1 { color: blue }").unwrap();
    within(
        "a new revision",
        view.wait_for(|view| view.revision.is_some_and(|revision| revision > 0)),
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
    assert!(page.body.contains("/_jamjam/client.js\""), "{}", page.body);
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
    let upgraded = within("the upgrade", hyper::upgrade::on(response))
        .await
        .unwrap();
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
    for reply in futures_util::future::join_all(requests).await {
        assert_eq!(reply.status, 200);
    }
}
