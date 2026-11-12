//! Comments, presence, the share stopping and reconnection.

use std::time::Duration;

use futures_util::FutureExt;
use http_body_util::{BodyExt, Full};
use iroh::endpoint::VarInt;
use tokio::net::UnixStream;

use crate::{
    adapters::endpoint::generate_key,
    agent,
    core::{ids::EndpointKey, rng::SystemRng},
    protocol::comment::Draft,
    share::hub::MAX_JOINERS_PER_KEY,
    tests::harness::{
        DRAFT, connected, fixture_dir, get, join, joiner_with_key, local, post_comment, send,
        share_files, until, within,
    },
};

#[tokio::test(flavor = "multi_thread")]
async fn comments_are_queued_with_the_author_from_admission() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let key = generate_key(&SystemRng);
    let public = EndpointKey::from(key.public());
    let joiner = joiner_with_key(share.token().clone(), share.direct_addrs(), "alice", key).await;
    connected(&joiner).await;

    let reply = post_comment(&joiner, DRAFT).await;
    assert_eq!(reply.status, 202, "{}", reply.body);
    assert!(reply.body.contains("\"pending\":1"), "{}", reply.body);

    let queued = share.queued();
    let [comment] = queued.as_slice() else {
        panic!("expected one queued comment, got {queued:?}");
    };
    assert_eq!(comment.author.name.to_string(), "alice");
    assert_eq!(comment.author.key, public);
    assert_eq!(comment.author.fingerprint, public.fingerprint());
    let you = joiner.watch().borrow().you.clone().unwrap();
    assert_eq!(comment.author, you);
    assert_eq!(comment.comment, "Bigger, please");
    assert_eq!(comment.selected_text.as_deref(), Some("Fixture"));
    assert!(reply.body.contains(&comment.id.to_string()));

    // A draft cannot claim an author.
    let forged = DRAFT.replace(
        '}',
        r#","author":{"name":"bob","fingerprint":"0000000000"}}"#,
    );
    assert_eq!(post_comment(&joiner, &forged).await.status, 400);
    assert_eq!(post_comment(&joiner, "{}").await.status, 400);
    assert_eq!(share.queued().len(), 1);

    let status = get(&joiner, "/_jamjam/api/status").await;
    assert!(status.body.contains("\"pending\":1"), "{}", status.body);
    assert!(
        status.body.contains("\"connected\":true"),
        "{}",
        status.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_limit_answers_429() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    for _ in 0..10 {
        assert_eq!(post_comment(&joiner, DRAFT).await.status, 202);
    }
    let limited = post_comment(&joiner, DRAFT).await;
    assert_eq!(limited.status, 429);
    assert!(limited.body.contains("retryAfterSecs"), "{}", limited.body);
    assert_eq!(share.queued().len(), 10);
}

#[tokio::test(flavor = "multi_thread")]
async fn presence_follows_two_joiners() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;

    let mut view = alice.watch();
    within(
        "both in presence",
        view.wait_for(|view| view.joiners.len() == 2),
    )
    .await
    .unwrap();
    let names: Vec<String> = view
        .borrow()
        .joiners
        .iter()
        .map(|author| author.name.to_string())
        .collect();
    assert_eq!(names, ["alice", "bob"]);
    assert_eq!(share.presence().len(), 2);

    // Presence also streams to pages.
    let events = local(&alice, "GET", "/_jamjam/api/events")
        .body(Full::default())
        .unwrap();
    let response = send(alice.local_addr(), events).await;
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let first = within("the first event", body.frame())
        .await
        .unwrap()
        .unwrap();
    let first = String::from_utf8(first.into_data().unwrap().to_vec()).unwrap();
    assert_eq!(first, "data: {\"type\":\"share\",\"connected\":true}\n\n");

    bob.stop().await;
    within("bob gone", view.wait_for(|view| view.joiners.len() == 1))
        .await
        .unwrap();
    let next = within("the presence event", body.frame())
        .await
        .unwrap()
        .unwrap();
    let next = String::from_utf8(next.into_data().unwrap().to_vec()).unwrap();
    assert!(next.contains("\"type\":\"presence\""), "{next}");
    assert!(!next.contains("bob"), "{next}");
    until("the share to drop bob", || share.presence().len() == 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_share_disconnects_joiners() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    assert_eq!(get(&joiner, "/").await.status, 200);
    share.stop().await;
    let mut view = joiner.watch();
    within("the disconnect", view.wait_for(|view| !view.connected))
        .await
        .unwrap();
    assert_eq!(get(&joiner, "/").await.status, 502);
    assert_eq!(post_comment(&joiner, DRAFT).await.status, 503);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiner_reconnects_after_losing_the_connection() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let first = joiner.connection().unwrap();
    first.close(VarInt::from_u32(0), b"test drop");
    let mut view = joiner.watch();
    within("the disconnect", view.wait_for(|view| !view.connected))
        .await
        .unwrap();
    connected(&joiner).await;
    let second = joiner.connection().unwrap();
    assert_ne!(first.stable_id(), second.stable_id());
    assert_eq!(get(&joiner, "/").await.status, 200);
    until("presence to settle", || share.presence().len() == 1).await;
    assert_eq!(post_comment(&joiner, DRAFT).await.status, 202);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnecting_joiner_learns_what_was_delivered_meanwhile() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    assert_eq!(post_comment(&joiner, DRAFT).await.status, 202);
    let mut view = joiner.watch();
    assert_eq!(view.borrow().pending, 1);

    joiner
        .connection()
        .unwrap()
        .close(VarInt::from_u32(0), b"test drop");
    within("the disconnect", view.wait_for(|view| !view.connected))
        .await
        .unwrap();
    // Delivered while the joiner is away (it waits a second to reconnect).
    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    agent::recv(stream, &mut Vec::new()).await.unwrap();
    until("the delivery", || share.queued().is_empty()).await;
    assert!(!view.borrow().connected, "reconnected too early to tell");
    assert_eq!(view.borrow().pending, 1);

    within(
        "reconnection with the delivered state",
        view.wait_for(|view| view.connected && view.pending == 0 && view.undelivered.is_empty()),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_comment_sent_again_is_queued_once() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let body = DRAFT.replacen('{', r#"{"ref":"page-ref-1","#, 1);
    let first = post_comment(&joiner, &body).await;
    assert_eq!(first.status, 202, "{}", first.body);
    let again = post_comment(&joiner, &body).await;
    assert_eq!(again.status, 202, "{}", again.body);
    assert_eq!(first.body, again.body);
    assert_eq!(share.queued().len(), 1);

    // A wait given up (the page went away, a timeout) leaves nothing behind.
    let control = joiner.control().unwrap();
    let draft: Draft = serde_json::from_str(DRAFT).unwrap();
    let reference = "page-ref-2".to_owned().try_into().unwrap();
    // Polled once (the line goes out), then dropped.
    let given_up = control.comment(reference, draft).now_or_never();
    assert!(given_up.is_none(), "{given_up:?}");
    assert_eq!(control.waiting(), 0);
    // The comment may still have reached the share; sending it again is safe.
    let body = DRAFT.replacen('{', r#"{"ref":"page-ref-2","#, 1);
    assert_eq!(post_comment(&joiner, &body).await.status, 202);
    until("both queued", || share.queued().len() == 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_share_is_retried_not_final() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let key = generate_key(&SystemRng);
    let mut joiners = Vec::new();
    for _ in 0..MAX_JOINERS_PER_KEY {
        let joiner = joiner_with_key(
            share.token().clone(),
            share.direct_addrs(),
            "alice",
            key.clone(),
        )
        .await;
        connected(&joiner).await;
        joiners.push(joiner);
    }
    let extra = joiner_with_key(share.token().clone(), share.direct_addrs(), "alice", key).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!extra.watch().borrow().connected);
    assert_eq!(extra.watch().borrow().refused, None);
    assert_eq!(share.presence().len(), 1);
    joiners.pop().unwrap().stop().await;
    connected(&extra).await;
}
