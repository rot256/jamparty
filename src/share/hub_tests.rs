use std::{slice, sync::Arc};

use jiff::{SignedDuration, Timestamp};

use crate::{
    core::{clock::ManualClock, ids::EndpointKey, queue::DeliveryResult, rng::SeededRng},
    protocol::{
        comment::{Author, Draft},
        control::{AckOutcome, CommentRef},
    },
    share::hub::{Hub, MAX_JOINERS, MAX_JOINERS_PER_KEY, MAX_QUEUED_PER_KEY},
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
        .map(|_| hub.join(alice.clone()).unwrap())
        .collect();
    assert!(hub.join(alice.clone()).is_none(), "one key, too often");
    assert_eq!(hub.presence(), slice::from_ref(&alice));

    let others: Vec<_> = (2..)
        .take(MAX_JOINERS - MAX_JOINERS_PER_KEY)
        .map(|key| hub.join(author("bob", key)).unwrap())
        .collect();
    assert!(
        hub.join(author("carol", 255)).is_none(),
        "the share is full"
    );
    assert_eq!(hub.presence().len(), 1 + others.len());
    drop(mine);
    assert!(hub.join(author("carol", 255)).is_some());
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
    hub.finish(DeliveryResult::Ok, "");
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
