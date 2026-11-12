//! `nabe recv` and `nabe send` over the runtime socket: comments reach
//! the agent and count as delivered once printed; answers reach every
//! joiner.

use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

use crate::{
    agent,
    core::{ids::CommentId, queue::DeliveryState},
    protocol::comment::Comment,
    tests::harness::{
        DRAFT, events, fixture_dir, join, next_event, post_comment, share_files, until, within,
    },
};

async fn recv(path: &Path) -> Vec<Comment> {
    let stream = UnixStream::connect(path).await.unwrap();
    let mut out = Vec::new();
    agent::recv(stream, &mut out).await.unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn send(path: &Path, message: &str, anchor: Option<CommentId>) -> Result<(), String> {
    let stream = UnixStream::connect(path).await.unwrap();
    let message = message.to_owned().try_into().unwrap();
    agent::send(stream, message, anchor)
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_socket_is_private_and_goes_away_with_the_share() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let path = share.socket_path().to_owned();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        format!("{}.sock", share.id())
    );
    share.stop().await;
    assert!(!path.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn recv_waits_for_comments_and_delivery_reaches_every_joiner() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let mut bob_events = events(&bob).await;

    let path = share.socket_path().to_owned();
    let mut waiting = tokio::spawn(async move { recv(&path).await });
    // Nothing pending: recv keeps waiting.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut waiting)
            .await
            .is_err()
    );

    let reply = post_comment(&alice, DRAFT).await;
    assert_eq!(reply.status, 202, "{}", reply.body);
    let comments = within("recv", waiting).await.unwrap();
    let [comment] = comments.as_slice() else {
        panic!("expected one comment, got {comments:?}");
    };
    assert_eq!(comment.author.name.to_string(), "alice");
    assert_eq!(comment.comment, "Bigger, please");
    assert!(reply.body.contains(&comment.id.to_string()));

    let delivered = next_event(&mut bob_events, "delivered").await;
    assert!(delivered.contains(&comment.id.to_string()), "{delivered}");
    let mut view = alice.watch();
    within(
        "alice's pending to clear",
        view.wait_for(|view| view.pending == 0),
    )
    .await
    .unwrap();
    assert!(share.queued().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recv_that_dies_before_printing_loses_nothing() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);

    // Take the batch by hand and hang up without saying it was printed.
    let mut raw = BufReader::new(UnixStream::connect(share.socket_path()).await.unwrap());
    raw.get_mut()
        .write_all(b"{\"version\":1,\"message\":{\"type\":\"recv\"}}\n")
        .await
        .unwrap();
    let mut lines = Vec::new();
    while !lines
        .last()
        .is_some_and(|line: &String| line.contains("\"end\""))
    {
        let mut line = String::new();
        within("a reply line", raw.read_line(&mut line))
            .await
            .unwrap();
        lines.push(line);
    }
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(share.hub().delivery_state(), DeliveryState::InFlight);
    drop(raw);
    until("the batch to come back", || {
        share.hub().delivery_state() == DeliveryState::Ready
    })
    .await;
    assert_eq!(share.queued().len(), 1);

    let comments = within("recv", recv(share.socket_path())).await;
    assert_eq!(comments.len(), 1);
    // `printed` reaches the share after recv returns.
    until("delivery", || share.queued().is_empty()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_reach_every_joiner_and_later_ones() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let mut bob_events = events(&bob).await;
    assert_eq!(post_comment(&alice, DRAFT).await.status, 202);
    let id = share.queued().first().unwrap().id;

    let path = share.socket_path();
    send(path, "**Done**: see [the title](nabe:%23title)", Some(id))
        .await
        .unwrap();
    let event = next_event(&mut bob_events, "answer").await;
    assert!(event.contains("<strong>Done</strong>"), "{event}");
    assert!(event.contains("\"anchor\":\"#title\""), "{event}");
    for joiner in [&alice, &bob] {
        let mut view = joiner.watch();
        let view = within("the answer", view.wait_for(|view| view.answers.len() == 1))
            .await
            .unwrap()
            .clone();
        let answer = view.answers.first().unwrap();
        assert_eq!(answer.anchor.as_deref(), Some("#title"));
        assert_eq!(answer.page.as_deref(), Some("http://x.localhost/"));
    }

    let unknown = CommentId::generate(&crate::core::rng::SeededRng::new(9));
    let error = send(path, "hi", Some(unknown)).await.unwrap_err();
    assert!(error.contains("no comment"), "{error}");
    send(path, "<script>x</script> plain", None).await.unwrap();

    // A joiner that comes later sees the answers too.
    let carol = join(&share, "carol").await;
    let mut view = carol.watch();
    let view = within(
        "carol's answers",
        view.wait_for(|view| view.answers.len() == 2),
    )
    .await
    .unwrap()
    .clone();
    let last = view.answers.last().unwrap();
    assert!(last.html.contains("&lt;script&gt;"), "{}", last.html);
    assert!(last.anchor.is_none());
}
