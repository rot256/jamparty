//! Comments, presence, the share stopping and reconnection.

use http_body_util::{BodyExt, Full};
use iroh::endpoint::VarInt;

use crate::{
    adapters::endpoint::generate_key,
    core::{ids::EndpointKey, rng::SystemRng},
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
    let fingerprint = EndpointKey::from(key.public()).fingerprint();
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
    assert_eq!(comment.author.fingerprint, fingerprint);
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
