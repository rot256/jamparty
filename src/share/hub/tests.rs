use std::{slice, sync::Arc};

use futures_util::FutureExt;
use jiff::{SignedDuration, Timestamp};

use crate::{
    core::{
        clock::ManualClock,
        ids::{CommentId, EndpointKey},
        queue::Attempt,
        rng::SeededRng,
        settings::Switch,
    },
    protocol::{
        comment::{Author, Draft, DraftFields},
        control::{AckOutcome, Choices, CommentRef, MAX_QUEUED_PER_KEY, ShareMessage},
        presence::{Position, Whereabouts},
    },
    share::hub::{
        ACK_QUEUE, Hub, Ignored, MAX_ANSWERS, MAX_JOINERS, MAX_JOINERS_PER_KEY, Outbox, Outgoing,
    },
};

fn hub() -> (Arc<Hub>, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let hub = Hub::new(Arc::clone(&clock) as _, Arc::new(SeededRng::new(7)));
    (hub, clock)
}

fn author(name: &str, key: u8) -> Author {
    Author::new(name.parse().unwrap(), EndpointKey([key; 32]))
}

fn draft() -> Draft {
    serde_json::from_value(serde_json::json!({"page": "/", "selector": "h1", "comment": "hi"}))
        .unwrap()
}

fn fresh() -> CommentRef {
    uuid::Uuid::new_v4().to_string().try_into().unwrap()
}

fn queued(outcome: AckOutcome) -> bool {
    matches!(outcome, AckOutcome::Queued { .. })
}

#[test]
fn joiners_are_capped_in_all_and_per_key_and_shown_once() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let mine: Vec<_> = (0..MAX_JOINERS_PER_KEY)
        .map(|_| hub.join(alice.clone(), true).unwrap())
        .collect();
    assert!(
        hub.join(alice.clone(), true).is_none(),
        "one key, too often"
    );
    assert_eq!(hub.presence(), slice::from_ref(&alice));

    let others: Vec<_> = (2..)
        .take(MAX_JOINERS - MAX_JOINERS_PER_KEY)
        .map(|key| hub.join(author("bob", key), true).unwrap())
        .collect();
    assert!(
        hub.join(author("carol", 255), true).is_none(),
        "the share is full"
    );
    assert_eq!(hub.presence().len(), 1 + others.len());
    drop(mine);
    assert!(hub.join(author("carol", 255), true).is_some());
}

#[test]
fn the_same_ref_is_queued_once() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let reference = fresh();
    let first = hub.comment(&alice, reference.clone(), draft());
    let AckOutcome::Queued { id } = first else {
        panic!("{first:?}");
    };
    assert_eq!(
        hub.comment(&alice, reference.clone(), draft()),
        AckOutcome::Queued { id }
    );
    assert_eq!(hub.queued().len(), 1);
    // Another joiner's ref is its own.
    assert_ne!(
        hub.comment(&author("bob", 2), reference, draft()),
        AckOutcome::Queued { id }
    );
    assert_eq!(hub.queued().len(), 2);
}

#[tokio::test]
async fn a_ref_sent_again_after_delivery_is_reported_delivered() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let reference = fresh();
    let AckOutcome::Queued { id } = hub.comment(&alice, reference.clone(), draft()) else {
        panic!("not queued");
    };
    hub.next_batch().await;
    hub.finish(Attempt::Delivered);
    assert_eq!(
        hub.comment(&alice, reference, draft()),
        AckOutcome::Delivered { id }
    );
    assert!(hub.queued().is_empty());
}

#[test]
fn one_key_cannot_fill_the_queue() {
    let (hub, clock) = hub();
    let alice = author("alice", 1);
    for _ in 0..MAX_QUEUED_PER_KEY {
        assert!(queued(hub.comment(&alice, fresh(), draft())));
        clock.advance(SignedDuration::from_secs(6));
    }
    assert_eq!(hub.comment(&alice, fresh(), draft()), AckOutcome::QueueFull);
    assert!(queued(hub.comment(&author("bob", 2), fresh(), draft())));
}

#[test]
fn comments_are_rate_limited_in_all_too() {
    let (hub, _) = hub();
    let mut accepted = 0;
    for key in 1..=7 {
        let joiner = author("joiner", key);
        for _ in 0..10 {
            if queued(hub.comment(&joiner, fresh(), draft())) {
                accepted += 1;
            }
        }
    }
    assert_eq!(accepted, 60);
}

#[test]
fn undelivered_comments_are_matched_by_the_full_key() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    // Same fingerprint (first five bytes), another key.
    let mut bytes = [1; 32];
    bytes[31] = 2;
    let forger = Author::new("alice".parse().unwrap(), EndpointKey(bytes));
    assert_eq!(forger.fingerprint, alice.fingerprint);
    let AckOutcome::Queued { id } = hub.comment(&alice, fresh(), draft()) else {
        panic!("not queued");
    };
    assert_eq!(hub.undelivered_by(&alice), [id]);
    assert!(hub.undelivered_by(&forger).is_empty());
    let comment = hub.queued().pop().unwrap();
    assert_eq!(comment.author.key, alice.key);
}

fn whereabouts(name: &str, url: Option<&str>, cursor: Switch) -> Whereabouts {
    Whereabouts {
        name: name.parse().unwrap(),
        url: url.map(|url| url.to_owned().try_into().unwrap()),
        cursor,
    }
}

#[test]
fn presence_updates_rename_and_move_joiners_within_a_rate() {
    let (hub, clock) = hub();
    let alice = hub.join(author("alice", 1), true).unwrap().0;
    let _bob = hub.join(author("bob", 2), true).unwrap();
    let renamed = alice
        .update(whereabouts("alicia", Some("/a"), Switch::On))
        .unwrap();
    assert_eq!(renamed.name.to_string(), "alicia");
    assert_eq!(renamed.key, EndpointKey([1; 32]));
    let here = hub.here();
    let [first, second] = here.as_slice() else {
        panic!("{here:?}");
    };
    assert_eq!(first.name.to_string(), "alicia");
    assert_eq!(first.url.clone().map(String::from).as_deref(), Some("/a"));
    assert_eq!(first.cursor, Switch::On);
    assert_eq!(second.url, None);
    assert_eq!(second.cursor, Switch::Off);
    assert_eq!(hub.presence().first().unwrap().name.to_string(), "alicia");

    // Every update is recorded, the last one wins: state is never dropped.
    for index in 1..100 {
        let url = format!("/r{index}");
        alice
            .update(whereabouts("alicia", Some(&url), Switch::On))
            .unwrap();
    }
    assert_eq!(
        hub.here()
            .first()
            .unwrap()
            .url
            .clone()
            .map(String::from)
            .as_deref(),
        Some("/r99")
    );
    // Past 100 per 10 s the joiner is flooding: still recorded, but the
    // connection is to be closed.
    assert_eq!(
        alice.update(whereabouts("mallory", None, Switch::Off)),
        Err(Ignored::Flood)
    );
    assert_eq!(hub.here().first().unwrap().name.to_string(), "mallory");
    clock.advance(SignedDuration::from_secs(1));
    assert!(alice.update(whereabouts("al", None, Switch::Off)).is_ok());
}

#[test]
fn presence_shows_a_key_as_its_latest_connection_says() {
    let (hub, _) = hub();
    let first = hub.join(author("alice", 1), true).unwrap().0;
    let second = hub.join(author("alice", 1), true).unwrap().0;
    second
        .update(whereabouts("alice", Some("/two"), Switch::Off))
        .unwrap();
    first
        .update(whereabouts("alice", Some("/one"), Switch::On))
        .unwrap();
    let here = hub.here();
    let [only] = here.as_slice() else {
        panic!("{here:?}");
    };
    assert_eq!(only.url.clone().map(String::from).as_deref(), Some("/one"));
    drop(first);
    let here = hub.here();
    let [only] = here.as_slice() else {
        panic!("{here:?}");
    };
    assert_eq!(only.url.clone().map(String::from).as_deref(), Some("/two"));
}

fn position(selector: &str) -> Position {
    serde_json::from_value(serde_json::json!({"selector": selector, "x": 5000, "y": 10000}))
        .unwrap()
}

#[test]
fn cursors_go_only_to_other_people_on_the_same_page() {
    let (hub, clock) = hub();
    let (alice, mut alice_rx) = hub.join(author("alice", 1), true).unwrap();
    let (bob, mut bob_rx) = hub.join(author("bob", 2), true).unwrap();
    let (carol, mut carol_rx) = hub.join(author("carol", 3), true).unwrap();
    // Bob's second connection: his own cursor is not sent back to him.
    let (bob_again, mut bob_again_rx) = hub.join(author("bob", 2), true).unwrap();
    alice
        .update(whereabouts("alice", Some("/a"), Switch::Off))
        .unwrap();
    carol
        .update(whereabouts("carol", Some("/c"), Switch::Off))
        .unwrap();
    bob_again
        .update(whereabouts("bob", Some("/a"), Switch::Off))
        .unwrap();

    // Off (the default) or nowhere yet: nothing is forwarded.
    assert_eq!(bob.cursor(Some(position("h1"))), Err(Ignored::CursorOff));
    bob.update(whereabouts("bob", None, Switch::On)).unwrap();
    assert_eq!(bob.cursor(Some(position("h1"))), Err(Ignored::Nowhere));
    assert!(alice_rx.cursors.try_recv().is_err());

    bob.update(whereabouts("bob", Some("/a"), Switch::On))
        .unwrap();
    assert_eq!(bob.cursor(Some(position("h1"))), Ok(1));
    let Ok(ShareMessage::Cursor {
        key,
        url,
        position: Some(at),
    }) = alice_rx.cursors.try_recv()
    else {
        panic!("alice got no cursor");
    };
    assert_eq!(key, EndpointKey([2; 32]));
    assert_eq!(String::from(url), "/a");
    assert_eq!(at, position("h1"));
    assert!(carol_rx.cursors.try_recv().is_err(), "carol is elsewhere");
    assert!(bob_rx.cursors.try_recv().is_err() && bob_again_rx.cursors.try_recv().is_err());

    // Leaving the page is a cursor without a position.
    assert_eq!(bob.cursor(None), Ok(1));
    assert!(matches!(
        alice_rx.cursors.try_recv(),
        Ok(ShareMessage::Cursor { position: None, .. })
    ));

    // Bob moves to carol's page; then turns his cursor off.
    bob.update(whereabouts("bob", Some("/c"), Switch::On))
        .unwrap();
    assert_eq!(bob.cursor(Some(position("p"))), Ok(1));
    assert!(carol_rx.cursors.try_recv().is_ok());
    assert!(alice_rx.cursors.try_recv().is_err());
    bob.update(whereabouts("bob", Some("/c"), Switch::Off))
        .unwrap();
    assert_eq!(bob.cursor(Some(position("p"))), Err(Ignored::CursorOff));
    assert!(carol_rx.cursors.try_recv().is_err());

    // At most 25 per second.
    clock.advance(SignedDuration::from_secs(1));
    bob.update(whereabouts("bob", Some("/c"), Switch::On))
        .unwrap();
    let forwarded = (0..40)
        .filter(|_| bob.cursor(Some(position("p"))).is_ok())
        .count();
    assert_eq!(forwarded, 25);
    clock.advance(SignedDuration::from_secs(1));
    assert!(bob.cursor(Some(position("p"))).is_ok());
}

/// What `outbox` has ready, in the order its writer sends it, but
/// presence.
fn sent(outbox: &mut Outbox<'_>) -> Vec<ShareMessage> {
    let mut sent = Vec::new();
    while let Some(message) = outbox.next().now_or_never() {
        match message.unwrap() {
            ShareMessage::Presence { .. } => {}
            other @ (ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. }
            | ShareMessage::CommentAck { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::AnswerResolved { .. }
            | ShareMessage::Delivered { .. }
            | ShareMessage::Reload { .. }
            | ShareMessage::Status { .. }
            | ShareMessage::Cursor { .. }) => sent.push(other),
        }
    }
    sent
}

#[tokio::test]
async fn the_ack_goes_to_its_connection_before_the_delivery() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let mut heard = hub.subscribe();
    let broadcasts = hub.subscribe();
    let (presence, inbox) = hub.join(alice.clone(), true).unwrap();
    let mut outbox = Outbox::new(&presence, alice.key, inbox, broadcasts);
    let other_broadcasts = hub.subscribe();
    let (other, other_inbox) = hub.join(alice.clone(), true).unwrap();
    let mut other_outbox = Outbox::new(&other, alice.key, other_inbox, other_broadcasts);
    let reference = fresh();
    // Delivered at once: the delivery task may finish before the
    // connection's writer runs; the ack was stamped first.
    let AckOutcome::Queued { id } = presence
        .comment(&alice, reference.clone(), draft())
        .unwrap()
    else {
        panic!("not queued");
    };
    hub.next_batch().await;
    hub.finish(Attempt::Delivered);
    assert_eq!(
        sent(&mut outbox),
        [
            ShareMessage::CommentAck {
                reference,
                outcome: AckOutcome::Queued { id }
            },
            ShareMessage::Delivered { ids: vec![id] },
        ]
    );
    assert_eq!(
        sent(&mut other_outbox),
        [ShareMessage::Delivered { ids: vec![id] }],
        "acks are per connection"
    );
    // Only the delivery is broadcast.
    let mut broadcast = Vec::new();
    while let Ok(Outgoing { message, .. }) = heard.try_recv() {
        match message {
            ShareMessage::Presence { .. } => {}
            other @ (ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. }
            | ShareMessage::CommentAck { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::AnswerResolved { .. }
            | ShareMessage::Delivered { .. }
            | ShareMessage::Reload { .. }
            | ShareMessage::Status { .. }
            | ShareMessage::Cursor { .. }) => broadcast.push(other),
        }
    }
    assert_eq!(broadcast, [ShareMessage::Delivered { ids: vec![id] }]);
}

#[tokio::test]
async fn new_acks_do_not_hold_back_an_earlier_broadcast() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let broadcasts = hub.subscribe();
    let (presence, inbox) = hub.join(alice.clone(), true).unwrap();
    let mut outbox = Outbox::new(&presence, alice.key, inbox, broadcasts);
    assert_eq!(sent(&mut outbox), [], "only presence so far");
    let AckOutcome::Queued { id } = presence.comment(&alice, fresh(), draft()).unwrap() else {
        panic!("not queued");
    };
    hub.next_batch().await;
    hub.finish(Attempt::Delivered);
    // A new ack comes before each message the writer takes.
    let mut taken = Vec::new();
    for _ in 0..ACK_QUEUE {
        assert!(presence.comment(&alice, fresh(), draft()).is_ok());
        taken.push(outbox.next().await.unwrap());
    }
    let delivered = ShareMessage::Delivered { ids: vec![id] };
    assert!(
        matches!(taken.as_slice(), [ShareMessage::CommentAck { .. }, second, ..] if *second == delivered),
        "{taken:?}"
    );
}

#[test]
fn comments_faster_than_their_acks_are_read_are_a_flood() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let mut heard = hub.subscribe();
    let (presence, _inbox) = hub.join(alice.clone(), true).unwrap();
    // Rate-limited or not, each comment line is acknowledged; none of it
    // reaches the broadcast that every joiner reads.
    for _ in 0..ACK_QUEUE {
        assert!(presence.comment(&alice, fresh(), draft()).is_ok());
    }
    assert_eq!(
        presence.comment(&alice, fresh(), draft()),
        Err(Ignored::Flood)
    );
    assert!(Ignored::Flood.disconnects());
    while let Ok(Outgoing { message, .. }) = heard.try_recv() {
        assert!(
            matches!(message, ShareMessage::Presence { .. }),
            "{message:?}"
        );
    }
}

fn question(hub: &Hub, anchor: Option<CommentId>, to: Option<&str>) -> CommentId {
    hub.answer(
        "Which size?".to_owned().try_into().unwrap(),
        anchor,
        to.map(|to| to.parse().unwrap()),
        vec!["Keep it".to_owned(), "Make it bigger".to_owned()]
            .try_into()
            .unwrap(),
    )
    .unwrap();
    let inner = hub.lock();
    let ShareMessage::Answer { id, .. } = &inner.answers.back().unwrap().message else {
        panic!("expected question");
    };
    *id
}

fn pick(id: CommentId, choice: &str) -> Draft {
    serde_json::from_value(serde_json::json!({
        "page": "http://example.localhost/", "selector": format!("jamparty-agent:{id}"),
        "comment": choice, "choice": choice,
    }))
    .unwrap()
}

#[test]
fn first_pick_wins_and_retries_are_deduplicated() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let bob = author("bob", 2);
    let id = question(&hub, None, None);
    let reference = fresh();
    let outcome = hub.comment(&alice, reference.clone(), pick(id, "Make it bigger"));
    assert!(queued(outcome));
    assert_eq!(
        hub.comment(&alice, reference, pick(id, "Make it bigger")),
        outcome
    );
    assert_eq!(
        hub.comment(&bob, fresh(), pick(id, "Keep it")),
        AckOutcome::ChoiceRefused
    );
    assert!(hub.catch_up(alice.key).is_empty());
    assert!(hub.catch_up(bob.key).is_empty());
    let comments = hub.queued();
    let [comment] = comments.as_slice() else {
        panic!("one pick expected")
    };
    let json = serde_json::to_value(comment).unwrap();
    assert_eq!(
        json.get("selector").and_then(serde_json::Value::as_str),
        Some(format!("jamparty-agent:{id}").as_str())
    );
    assert_eq!(json.get("choice").unwrap(), "Make it bigger");
    assert_eq!(json.get("comment").unwrap(), "Make it bigger");
    assert_eq!(comment.author, alice);
}

#[test]
fn picks_validate_audience_and_offered_label_but_typed_replies_do_not_resolve() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let bob = author("bob", 2);
    let _alice = hub.join(alice.clone(), true).unwrap();
    let _bob = hub.join(bob.clone(), true).unwrap();
    let AckOutcome::Queued { id: anchor } = hub.comment(&alice, fresh(), draft()) else {
        panic!("anchor")
    };
    let id = question(&hub, Some(anchor), None);
    assert!(
        hub.catch_up(bob.key)
            .iter()
            .all(|message| !matches!(message, ShareMessage::Answer { .. }))
    );
    assert_eq!(
        hub.comment(&bob, fresh(), pick(id, "Keep it")),
        AckOutcome::ChoiceRefused
    );
    assert_eq!(
        hub.comment(&alice, fresh(), pick(id, "Other")),
        AckOutcome::ChoiceRefused
    );
    let mut typed = DraftFields::from(pick(id, "Keep it"));
    typed.choice = None;
    assert!(queued(hub.comment(
        &alice,
        fresh(),
        typed.try_into().unwrap()
    )));
    assert!(
        hub.catch_up(alice.key)
            .iter()
            .any(|message| matches!(message, ShareMessage::Answer { .. }))
    );
    assert!(queued(hub.comment(&alice, fresh(), pick(id, "Keep it"))));
    let all = question(&hub, Some(anchor), Some("all"));
    assert!(queued(hub.comment(&bob, fresh(), pick(all, "Keep it"))));
    let private = question(&hub, None, Some("alice"));
    assert_eq!(
        hub.comment(&bob, fresh(), pick(private, "Keep it")),
        AckOutcome::ChoiceRefused
    );
}

#[tokio::test]
async fn pick_ack_precedes_resolution_and_resolved_records_are_bounded() {
    let (hub, clock) = hub();
    let alice = author("alice", 1);
    let broadcasts = hub.subscribe();
    let (presence, inbox) = hub.join(alice.clone(), true).unwrap();
    let mut outbox = Outbox::new(&presence, alice.key, inbox, broadcasts);
    let id = question(&hub, None, None);
    let _ = sent(&mut outbox);
    presence
        .comment(&alice, fresh(), pick(id, "Keep it"))
        .unwrap();
    assert!(
        matches!(sent(&mut outbox).as_slice(), [ShareMessage::CommentAck { .. }, ShareMessage::AnswerResolved { id: resolved }] if *resolved == id)
    );
    let reference = fresh();
    let id = question(&hub, None, None);
    let _ = sent(&mut outbox);
    let outcome = presence
        .comment(&alice, reference.clone(), pick(id, "Keep it"))
        .unwrap();
    let _ = sent(&mut outbox);
    for _ in 0..10 {
        assert_eq!(
            presence
                .comment(&alice, reference.clone(), pick(id, "Keep it"))
                .unwrap(),
            outcome
        );
        assert!(matches!(
            sent(&mut outbox).as_slice(),
            [ShareMessage::CommentAck { .. }]
        ));
    }
    for _ in 0..MAX_ANSWERS {
        clock.advance(SignedDuration::from_secs(6));
        let id = question(&hub, None, None);
        assert!(queued(hub.comment(&alice, fresh(), pick(id, "Keep it"))));
        let _ = hub.next_batch().await;
        hub.finish(Attempt::Delivered);
    }
    assert_eq!(hub.lock().answers.len(), MAX_ANSWERS);
    assert!(hub.catch_up(alice.key).is_empty());
}

#[tokio::test]
async fn legacy_joiners_never_receive_questions_or_resolutions() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let bob = author("bob", 2);
    let broadcasts = hub.subscribe();
    let (legacy, inbox) = hub.join(bob.clone(), false).unwrap();
    let mut outbox = Outbox::new(&legacy, bob.key, inbox, broadcasts);
    let id = question(&hub, None, None);
    assert!(
        sent(&mut outbox)
            .iter()
            .all(|message| !matches!(message, ShareMessage::Answer { .. }))
    );
    assert!(
        hub.catch_up(bob.key)
            .iter()
            .all(|message| !legacy.supports(message))
    );
    assert!(queued(hub.comment(&alice, fresh(), pick(id, "Keep it"))));
    assert!(sent(&mut outbox).is_empty());
    assert!(hub.catch_up(bob.key).is_empty());
}

#[tokio::test]
async fn concurrent_picks_queue_one_comment_and_ack_before_resolution() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let bob = author("bob", 2);
    let a_broadcasts = hub.subscribe();
    let (a, a_inbox) = hub.join(alice.clone(), true).unwrap();
    let b_broadcasts = hub.subscribe();
    let (b, b_inbox) = hub.join(bob.clone(), true).unwrap();
    let mut a_out = Outbox::new(&a, alice.key, a_inbox, a_broadcasts);
    let mut b_out = Outbox::new(&b, bob.key, b_inbox, b_broadcasts);
    let id = question(&hub, None, None);
    let _ = sent(&mut a_out);
    let _ = sent(&mut b_out);
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| a.comment(&alice, fresh(), pick(id, "Keep it")).unwrap());
        let second = scope.spawn(|| {
            b.comment(&bob, fresh(), pick(id, "Make it bigger"))
                .unwrap()
        });
        (first.join().unwrap(), second.join().unwrap())
    });
    assert!(matches!(
        (first, second),
        (AckOutcome::Queued { .. }, AckOutcome::ChoiceRefused)
            | (AckOutcome::ChoiceRefused, AckOutcome::Queued { .. })
    ));
    assert_eq!(hub.queued().len(), 1);
    for messages in [sent(&mut a_out), sent(&mut b_out)] {
        let ack = messages.iter().position(|message| {
            matches!(
                message,
                ShareMessage::CommentAck {
                    outcome: AckOutcome::Queued { .. },
                    ..
                }
            )
        });
        let resolved = messages
            .iter()
            .position(|message| matches!(message, ShareMessage::AnswerResolved { .. }))
            .unwrap();
        if let Some(ack) = ack {
            assert!(ack < resolved);
        }
    }
}

#[test]
fn targeted_questions_check_connected_capabilities() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let (mut presence, _inbox) = hub.join(alice.clone(), false).unwrap();
    let AckOutcome::Queued { id: anchor } = hub.comment(&alice, fresh(), draft()) else {
        panic!("anchor")
    };
    let choices = || vec!["Yes".to_owned(), "No".to_owned()].try_into().unwrap();
    for to in [
        Some("alice".to_owned()),
        Some(format!("key:{}", alice.fingerprint)),
        None,
    ] {
        let error = hub
            .answer(
                "Pick".to_owned().try_into().unwrap(),
                Some(anchor),
                to.map(|to| to.parse().unwrap()),
                choices(),
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "alice runs an older jamparty that cannot show choices; send it without --choice"
        );
    }
    question(&hub, Some(anchor), Some("all"));
    hub.answer(
        "Typed".to_owned().try_into().unwrap(),
        Some(anchor),
        None,
        Choices::default(),
    )
    .unwrap();
    let (supporting, _supporting_inbox) = hub.join(alice.clone(), true).unwrap();
    question(&hub, None, Some("alice"));
    drop(supporting);
    presence.set_choices(true);
    question(&hub, None, Some("alice"));
    drop(presence);
    let id = question(&hub, Some(anchor), None);
    let (reconnected, _inbox) = hub.join(alice.clone(), true).unwrap();
    assert!(hub.catch_up(alice.key).iter().any(
        |message| matches!(message, ShareMessage::Answer { id: found, .. } if *found == id)
            && reconnected.supports(message)
    ));
    drop(reconnected);
    let (legacy, _inbox) = hub.join(alice.clone(), false).unwrap();
    assert!(
        hub.catch_up(alice.key)
            .iter()
            .all(|message| !legacy.supports(message)
                || !matches!(message, ShareMessage::Answer { choices, .. } if !choices.is_empty()))
    );
}

#[test]
fn mismatched_comment_and_choice_is_refused() {
    let (hub, _) = hub();
    let alice = author("alice", 1);
    let id = question(&hub, None, None);
    let mut fields = DraftFields::from(pick(id, "Keep it"));
    fields.comment = "Different".to_owned();
    assert_eq!(
        hub.comment(&alice, fresh(), fields.try_into().unwrap()),
        AckOutcome::ChoiceRefused
    );
    assert_eq!(hub.catch_up(alice.key).len(), 1);
}

#[test]
fn catch_up_on_either_side_of_pick_and_reconnect_after_missed_resolution() {
    // Force both lock orderings with channels: no scheduler-dependent races.
    for pick_first in [false, true] {
        let (hub, _) = hub();
        let alice = author("alice", 1);
        let (presence, _inbox) = hub.join(alice.clone(), true).unwrap();
        let id = question(&hub, None, None);
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let worker_hub = &hub;
            let worker_alice = &alice;
            let worker = scope.spawn(move || {
                if !pick_first {
                    rx.recv().unwrap();
                }
                assert!(queued(worker_hub.comment(
                    worker_alice,
                    fresh(),
                    pick(id, "Keep it")
                )));
            });
            if pick_first {
                worker.join().unwrap();
                assert!(hub.catch_up(alice.key).is_empty());
            } else {
                assert_eq!(hub.catch_up(alice.key).len(), 1);
                tx.send(()).unwrap();
                worker.join().unwrap();
            }
        });
        // Drop without consuming the resolution, then reconnect under the same key.
        drop(presence);
        let (_presence, _inbox) = hub.join(alice.clone(), true).unwrap();
        assert!(hub.catch_up(alice.key).is_empty());
    }
}

#[test]
fn a_legacy_joiner_is_never_briefly_able_to_show_choices() {
    // The capability is set as the joiner is seated: a targeted question
    // sent right after admission is refused, with no window where the hub
    // still believed it could show choices.
    let (hub, _) = hub();
    let bob = author("bob", 2);
    let (_legacy, _inbox) = hub.join(bob.clone(), false).unwrap();
    let refused = hub.answer(
        "Which size?".to_owned().try_into().unwrap(),
        None,
        Some("bob".parse().unwrap()),
        vec!["Keep it".to_owned(), "Make it bigger".to_owned()]
            .try_into()
            .unwrap(),
    );
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("cannot show choices")
    );
}
