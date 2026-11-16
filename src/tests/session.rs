//! Comments, presence, the share stopping and reconnection, and the page
//! socket: its hello, the order of its messages, slow pages.

use std::time::Duration;

use futures_util::FutureExt;
use iroh::endpoint::VarInt;
use serde_json::{Value, json};
use tokio::{net::UnixStream, time};

use crate::{
    adapters::endpoint::generate_key,
    agent,
    core::{ids::EndpointKey, rng::SystemRng},
    protocol::comment::Draft,
    share::hub::MAX_JOINERS_PER_KEY,
    tests::harness::{
        DRAFT, PageSocket, about, commented, connected, fixture_dir, get, join, joiner_with_key,
        post_comment, share_files, until, within,
    },
};

/// A page's comment request for `draft`, under `reference`.
fn comment_request(draft: &str, reference: &str) -> Value {
    let mut body: Value = serde_json::from_str(draft).unwrap();
    let object = body.as_object_mut().unwrap();
    object.insert("type".to_owned(), "comment".into());
    object.insert("ref".to_owned(), reference.into());
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_are_queued_with_the_author_from_admission() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let key = generate_key(&SystemRng);
    let public = EndpointKey::from(key.public());
    let joiner = joiner_with_key(share.token().clone(), share.direct_addrs(), "alice", key).await;
    connected(&joiner).await;

    let (mut page, _) = PageSocket::hello(&joiner).await;
    let seen = about(page.request(comment_request(DRAFT, "first-ref")).await);
    // The comment is in `mine` before the reply says it is queued.
    let [mine, reply] = seen.as_slice() else {
        panic!("{seen:?}");
    };
    let id = commented(&reply["outcome"]).unwrap();
    assert_eq!(mine["type"], "mine", "{mine}");
    assert_eq!(mine["undelivered"], json!([id]), "{mine}");

    let queued = share.queued();
    let [comment] = queued.as_slice() else {
        panic!("expected one queued comment, got {queued:?}");
    };
    assert_eq!(comment.author.name.to_string(), "alice");
    assert_eq!(comment.author.key, public);
    assert_eq!(comment.author.fingerprint, public.fingerprint());
    let you = joiner.watch().borrow().link.you.clone().unwrap();
    assert_eq!(comment.author, you);
    assert_eq!(comment.comment, "Bigger, please");
    assert_eq!(comment.selected_text.as_deref(), Some("Fixture"));
    assert_eq!(comment.id.to_string(), id);

    // A draft cannot claim an author, and must be one.
    let forged = DRAFT.replace(
        '}',
        r#","author":{"name":"bob","fingerprint":"0000000000"}}"#,
    );
    let refused = page.comment(&forged).await;
    assert!(refused["refused"]["error"].is_string(), "{refused}");
    let refused = page.comment(r#"{"comment":"no page"}"#).await;
    assert!(refused["refused"]["error"].is_string(), "{refused}");
    assert_eq!(share.queued().len(), 1);

    // A page that connects now is told all of it.
    let (_, hello) = PageSocket::hello(&joiner).await;
    assert_eq!(hello["mine"], json!([id]), "{hello}");
    assert_eq!(hello["link"]["connected"], true, "{hello}");
    assert_eq!(hello["link"]["path"], "direct", "{hello}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rate_limit_says_how_long_to_wait() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let (mut page, _) = PageSocket::hello(&joiner).await;
    for _ in 0..10 {
        assert!(commented(&page.comment(DRAFT).await).is_some());
    }
    let limited = page.comment(DRAFT).await;
    let wait = limited["refused"]["retryAfterSecs"].as_u64();
    assert!(wait.is_some_and(|secs| secs > 0), "{limited}");
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
        view.wait_for(|view| view.people.len() == 2),
    )
    .await
    .unwrap();
    let names: Vec<String> = view
        .borrow()
        .people
        .iter()
        .map(|person| person.name.to_string())
        .collect();
    assert_eq!(names, ["alice", "bob"]);
    assert_eq!(share.presence().len(), 2);

    // Presence also goes to pages: in the hello first.
    let (mut page, hello) = PageSocket::hello(&alice).await;
    assert_eq!(hello["people"].as_array().unwrap().len(), 2, "{hello}");

    bob.stop().await;
    within("bob gone", view.wait_for(|view| view.people.len() == 1))
        .await
        .unwrap();
    // Presence that still lists bob may be on its way when he leaves (it is
    // coalesced); the socket must come to one without him.
    within("people without bob", async {
        while page.next_of("people").await.to_string().contains("bob") {}
    })
    .await;
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
    within("the disconnect", view.wait_for(|view| !view.link.connected))
        .await
        .unwrap();
    assert_eq!(get(&joiner, "/").await.status, 502);
    let unsent = post_comment(&joiner, DRAFT).await;
    assert!(unsent["unsent"]["error"].is_string(), "{unsent}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_joiner_reconnects_after_losing_the_connection() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let first = joiner.connection().unwrap();
    first.close(VarInt::from_u32(0), b"test drop");
    let mut view = joiner.watch();
    within("the disconnect", view.wait_for(|view| !view.link.connected))
        .await
        .unwrap();
    connected(&joiner).await;
    let second = joiner.connection().unwrap();
    assert_ne!(first.stable_id(), second.stable_id());
    assert_eq!(get(&joiner, "/").await.status, 200);
    until("presence to settle", || share.presence().len() == 1).await;
    assert!(commented(&post_comment(&joiner, DRAFT).await).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnecting_joiner_learns_what_was_delivered_meanwhile() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    assert!(commented(&post_comment(&joiner, DRAFT).await).is_some());
    let mut view = joiner.watch();
    assert_eq!(view.borrow().pending(), 1);

    joiner
        .connection()
        .unwrap()
        .close(VarInt::from_u32(0), b"test drop");
    within("the disconnect", view.wait_for(|view| !view.link.connected))
        .await
        .unwrap();
    // Delivered while the joiner is away (it waits a second to reconnect).
    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    agent::recv(stream, &mut Vec::new()).await.unwrap();
    until("the delivery", || share.queued().is_empty()).await;
    assert!(
        !view.borrow().link.connected,
        "reconnected too early to tell"
    );
    assert_eq!(view.borrow().pending(), 1);

    within(
        "reconnection with the delivered state",
        view.wait_for(|view| view.link.connected && view.mine == Some(Vec::new())),
    )
    .await
    .unwrap();
}

// One thread: the given-up comment's single poll below must not race the
// writer, the share and the acknowledgement on other workers.
#[tokio::test]
async fn a_comment_sent_again_is_queued_once() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let joiner = join(&share, "alice").await;
    let (mut page, _) = PageSocket::hello(&joiner).await;
    let body = DRAFT.replacen('{', r#"{"ref":"page-ref-1","#, 1);
    let first = commented(&page.comment(&body).await).unwrap();
    // Again on another socket, as a page that reloaded does.
    let again = commented(&post_comment(&joiner, &body).await).unwrap();
    assert_eq!(first, again);
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
    assert!(commented(&post_comment(&joiner, &body).await).is_some());
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
    // The extra one keeps trying, without a seat, until one frees up.
    let mut extra = tokio::spawn(joiner_with_key(
        share.token().clone(),
        share.direct_addrs(),
        "alice",
        key,
    ));
    assert!(
        time::timeout(Duration::from_millis(500), &mut extra)
            .await
            .is_err()
    );
    assert_eq!(share.presence().len(), 1);
    joiners.pop().unwrap().stop().await;
    let extra = within("the extra joiner", extra).await.unwrap();
    connected(&extra).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_page_hears_every_change_in_order() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let (mut first, hello) = PageSocket::hello(&alice).await;
    assert_eq!(hello["mine"], json!([]), "{hello}");
    let (mut second, _) = PageSocket::hello(&alice).await;
    // A comment from the first page: its `mine` comes before the reply.
    let seen = about(first.request(comment_request(DRAFT, "in-order")).await);
    let [mine, reply] = seen.as_slice() else {
        panic!("{seen:?}");
    };
    let id = commented(&reply["outcome"]).unwrap();
    assert_eq!(mine["undelivered"], json!([id]));
    // Delivered: both pages hear it after the comment was queued.
    let stream = UnixStream::connect(share.socket_path()).await.unwrap();
    agent::recv(stream, &mut Vec::new()).await.unwrap();
    assert_eq!(first.next_of("mine").await["undelivered"], json!([]));
    assert_eq!(second.next_of("mine").await["undelivered"], json!([id]));
    assert_eq!(second.next_of("mine").await["undelivered"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_takes_what_is_pending_for_its_key_from_the_share() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let key = generate_key(&SystemRng);
    let first = joiner_with_key(
        share.token().clone(),
        share.direct_addrs(),
        "alice",
        key.clone(),
    )
    .await;
    connected(&first).await;
    let id = commented(&post_comment(&first, DRAFT).await).unwrap();
    first.stop().await;

    // The same key, a new connection: it knows nothing until admitted,
    // and then what the share holds for that key.
    let again = joiner_with_key(share.token().clone(), share.direct_addrs(), "alice", key).await;
    connected(&again).await;
    let (_, hello) = PageSocket::hello(&again).await;
    assert_eq!(hello["mine"], json!([id]), "{hello}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_message_that_is_not_one_closes_the_socket() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    for bad in [
        json!({"type": "nonsense"}),
        json!({"type": "page", "url": "http://evil/"}),
        json!({"type": "page", "url": "/", "tab": "x"}),
        json!({"type": "cursor", "position": {"selector": "h1", "x": 10001, "y": 0}}),
    ] {
        let (mut page, _) = PageSocket::hello(&alice).await;
        page.send(bad.clone()).await;
        assert_eq!(page.closed().await, Some(1008), "{bad}");
    }
    // An invalid request is answered instead, so the page can say why.
    let (mut page, _) = PageSocket::hello(&alice).await;
    let outcome = page.settings(r#"{"corner":"middle"}"#).await;
    assert!(outcome["refused"]["error"].is_string(), "{outcome}");
    // Text frames only; a frame too large closes it too.
    let (mut page, _) = PageSocket::hello(&alice).await;
    page.send(json!({"type": "page", "url": format!("/{}", "a".repeat(70_000))}))
        .await;
    assert!(page.closed().await.is_some_and(|code| code != 1000));
}
