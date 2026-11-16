//! `jamparty recv` and `jamparty send` over the runtime socket: comments reach
//! the agent and count as delivered once printed; answers reach every
//! joiner.

use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time,
};

use crate::{
    adapters::endpoint::generate_key,
    agent,
    core::{
        ids::CommentId,
        queue::DeliveryState,
        rng::{SeededRng, SystemRng},
    },
    join::Joiner,
    protocol::{agent::Addressee, comment::Comment},
    tests::harness::{
        DRAFT, PageSocket, commented, connected, fixture_dir, join, joiner_with_key, post_comment,
        share_files, until, within,
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
    send_to(path, message, anchor, Some(Addressee::Everyone)).await
}

async fn send_to(
    path: &Path,
    message: &str,
    anchor: Option<CommentId>,
    to: Option<Addressee>,
) -> Result<(), String> {
    let stream = UnixStream::connect(path).await.unwrap();
    let message = message.to_owned().try_into().unwrap();
    agent::send(stream, message, anchor, to)
        .await
        .map_err(|error| format!("{error:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_socket_is_private_and_goes_away_with_the_share() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let path = share.socket_path().to_owned();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
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
async fn recv_waits_for_comments_and_delivery_reaches_only_the_author() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    let (mut alice_page, _) = PageSocket::hello(&alice).await;
    let (mut bob_page, _) = PageSocket::hello(&bob).await;

    let path = share.socket_path().to_owned();
    let mut waiting = tokio::spawn(async move { recv(&path).await });
    // Nothing pending: recv keeps waiting.
    assert!(
        time::timeout(Duration::from_millis(300), &mut waiting)
            .await
            .is_err()
    );

    let mut body: Value = serde_json::from_str(DRAFT).unwrap();
    body["type"] = "comment".into();
    body["ref"] = "recv-ref".into();
    let mut seen = alice_page.request(body).await;
    let id = commented(&seen.pop().unwrap()["outcome"]).unwrap();
    let comments = within("recv", waiting).await.unwrap();
    let [comment] = comments.as_slice() else {
        panic!("expected one comment, got {comments:?}");
    };
    assert_eq!(comment.author.name.to_string(), "alice");
    assert_eq!(comment.comment, "Bigger, please");
    assert_eq!(comment.id.to_string(), id);

    // Alice's page hears of the delivery: nothing pending any more. With
    // recv already waiting, that can come before the reply.
    let delivered =
        |message: &Value| message["type"] == "mine" && message["undelivered"] == json!([]);
    if !seen.iter().any(delivered) {
        let mine = alice_page.next_of("mine").await;
        assert!(delivered(&mine), "{mine}");
    }
    // Bob's hears nothing of alice's comments: an answer sent to everyone
    // afterwards arrives first.
    send(share.socket_path(), "after", None).await.unwrap();
    let seen = bob_page.until("answer").await;
    assert!(
        seen.iter().all(|message| message["type"] != "mine"),
        "{seen:?}"
    );
    let mut view = alice.watch();
    within(
        "alice's pending to clear",
        view.wait_for(|view| view.pending() == 0),
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
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());

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
    let (mut bob_page, _) = PageSocket::hello(&bob).await;
    assert!(commented(&post_comment(&alice, DRAFT).await).is_some());
    let id = share.queued().first().unwrap().id;

    let path = share.socket_path();
    send(
        path,
        "**Done**: see [the title](jamparty:%23title)",
        Some(id),
    )
    .await
    .unwrap();
    let answer = bob_page.next_of("answer").await;
    assert!(
        answer["html"]
            .as_str()
            .unwrap()
            .contains("<strong>Done</strong>"),
        "{answer}"
    );
    assert_eq!(answer["anchor"], "#title", "{answer}");
    for joiner in [&alice, &bob] {
        let mut view = joiner.watch();
        let view = within("the answer", view.wait_for(|view| view.answers.len() == 1))
            .await
            .unwrap()
            .clone();
        let answer = view.answers.first().unwrap();
        assert_eq!(answer.anchor.as_ref().map(AsRef::as_ref), Some("#title"));
        assert_eq!(
            answer.page.as_ref().map(AsRef::as_ref),
            Some("http://x.localhost/")
        );
    }

    let unknown = CommentId::generate(&SeededRng::new(9));
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
    let html = last.html.as_str();
    assert!(html.contains("&lt;script&gt;"), "{html}");
    assert!(last.anchor.is_none());
}

/// The answers `joiner` has, as (text, to you).
fn answers(joiner: &Joiner) -> Vec<(String, bool)> {
    joiner
        .watch()
        .borrow()
        .answers
        .iter()
        .map(|answer| (answer.html.as_str().to_owned(), answer.to_you))
        .collect()
}

fn has(joiner: &Joiner, text: &str) -> bool {
    answers(joiner).iter().any(|(html, _)| html.contains(text))
}

#[tokio::test(flavor = "multi_thread")]
async fn addressed_answers_reach_only_their_recipient() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let bob = join(&share, "bob").await;
    // Another alice: the name alone is ambiguous now.
    let key = generate_key(&SystemRng);
    let other_alice =
        joiner_with_key(share.token().clone(), share.direct_addrs(), "alice", key).await;
    connected(&other_alice).await;
    until("three here", || share.hub().presence().len() == 3).await;
    let path = share.socket_path();

    let error = send_to(path, "hi", None, Some("alice".parse().unwrap()))
        .await
        .unwrap_err();
    let alice_fp = alice.watch().borrow().link.you.clone().unwrap().fingerprint;
    let other_fp = other_alice
        .watch()
        .borrow()
        .link
        .you
        .clone()
        .unwrap()
        .fingerprint;
    assert!(
        error.contains("several people here are called alice"),
        "{error}"
    );
    assert!(
        error.contains(&format!("alice (key:{alice_fp})")),
        "{error}"
    );
    assert!(
        error.contains(&format!("alice (key:{other_fp})")),
        "{error}"
    );
    let error = send_to(path, "hi", None, Some("zed".parse().unwrap()))
        .await
        .unwrap_err();
    assert!(error.contains("nobody called zed is here"), "{error}");
    assert!(error.contains("bob ("), "{error}");

    // By key when names collide; by name otherwise.
    send_to(
        path,
        "for alice",
        None,
        Some(format!("key:{alice_fp}").parse().unwrap()),
    )
    .await
    .unwrap();
    send_to(path, "for bob", None, Some("bob".parse().unwrap()))
        .await
        .unwrap();
    until("alice's answer", || has(&alice, "for alice")).await;
    until("bob's answer", || has(&bob, "for bob")).await;

    // An answer to a comment goes to its author by default; `all` and no
    // anchor go to everyone.
    assert!(commented(&post_comment(&bob, DRAFT).await).is_some());
    let id = share.queued().first().unwrap().id;
    send_to(path, "about your comment", Some(id), None)
        .await
        .unwrap();
    send_to(
        path,
        "everyone, anchored",
        Some(id),
        Some(Addressee::Everyone),
    )
    .await
    .unwrap();
    send_to(path, "everyone", None, None).await.unwrap();
    for joiner in [&alice, &bob, &other_alice] {
        until("the broadcasts", || {
            has(joiner, "everyone, anchored") && has(joiner, "<p>everyone</p>")
        })
        .await;
    }
    until("bob's reply", || has(&bob, "about your comment")).await;
    assert_eq!(
        answers(&bob),
        [
            ("<p>for bob</p>\n".to_owned(), true),
            ("<p>about your comment</p>\n".to_owned(), true),
            ("<p>everyone, anchored</p>\n".to_owned(), false),
            ("<p>everyone</p>\n".to_owned(), false),
        ]
    );
    assert_eq!(
        answers(&alice),
        [
            ("<p>for alice</p>\n".to_owned(), true),
            ("<p>everyone, anchored</p>\n".to_owned(), false),
            ("<p>everyone</p>\n".to_owned(), false),
        ]
    );
    assert!(!has(&other_alice, "for alice"));

    // A later connection of bob's gets bob's answers; carol only the
    // broadcasts.
    let carol = join(&share, "carol").await;
    until("carol's catch-up", || answers(&carol).len() == 2).await;
    assert!(!has(&carol, "for bob") && !has(&carol, "about your comment"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_cannot_take_a_fingerprint_address() {
    let dir = fixture_dir();
    let share = share_files(&dir).await;
    let alice = join(&share, "alice").await;
    let other_alice = join(&share, "alice").await;
    let alice_fp = alice.watch().borrow().link.you.clone().unwrap().fingerprint;
    // Mallory names herself after alice's fingerprint.
    let mallory = join(&share, &alice_fp.to_string()).await;
    until("four here", || share.hub().presence().len() == 3).await;
    let path = share.socket_path();

    // The ambiguous name lists the candidates by key.
    let error = send_to(path, "hi", None, Some("alice".parse().unwrap()))
        .await
        .unwrap_err();
    assert!(error.contains(&format!("key:{alice_fp}")), "{error}");

    // key: addresses the key, never a name.
    let to_alice: Addressee = format!("key:{alice_fp}").parse().unwrap();
    send_to(path, "for alice", None, Some(to_alice))
        .await
        .unwrap();
    let alice_key = alice.watch().borrow().link.you.clone().unwrap().key;
    let by_key: Addressee = format!("key:{alice_key}").parse().unwrap();
    send_to(path, "for alice again", None, Some(by_key))
        .await
        .unwrap();
    // A bare name matches names only: here, mallory's.
    send_to(
        path,
        "for mallory",
        None,
        Some(alice_fp.to_string().parse().unwrap()),
    )
    .await
    .unwrap();
    send_to(path, "everyone", None, None).await.unwrap();
    for joiner in [&alice, &other_alice, &mallory] {
        until("the broadcast", || has(joiner, "<p>everyone</p>")).await;
    }
    assert!(has(&alice, "for alice") && has(&alice, "for alice again"));
    assert!(!has(&alice, "for mallory"));
    assert!(!has(&mallory, "for alice"));
    assert!(has(&mallory, "for mallory"));
    assert!(!has(&other_alice, "for alice") && !has(&other_alice, "for mallory"));

    // Malformed keys are refused before reaching the share.
    for bad in ["key:", "key:xyz", "key:0123", "a b"] {
        assert!(bad.parse::<Addressee>().is_err(), "{bad}");
    }
}
