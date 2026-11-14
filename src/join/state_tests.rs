use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use proptest::{collection, prelude::*};

use crate::{
    core::{
        clock::ManualClock,
        ids::{CommentId, EndpointKey},
        queue::Attempt,
        rng::SeededRng,
        settings::{Corner, Keys, Switch},
    },
    join::{
        api::ToPage,
        preferences::PageSettings,
        state::{MAX_PENDING, State, TooMany, follow},
    },
    protocol::{
        comment::{Author, Draft},
        control::{AckOutcome, AnswerText, MAX_QUEUED_PER_KEY, Revision, ShareMessage, StatusLine},
        presence::Person,
    },
    share::hub::Hub,
};

fn settings(cursor: Switch) -> PageSettings {
    PageSettings {
        name: "alice".parse().unwrap(),
        ask_name: false,
        keys: Keys::default(),
        comment_mode: Switch::On,
        show_cursor: cursor,
        corner: Corner::BottomRight,
    }
}

fn id(index: u8) -> CommentId {
    format!("00000000-0000-4000-8000-0000000000{index:02x}")
        .parse()
        .unwrap()
}

fn author(key: u8) -> Author {
    Author::new(
        format!("joiner{key}").parse().unwrap(),
        EndpointKey([key; 32]),
    )
}

/// One change to the joiner's state.
#[derive(Clone, Debug)]
enum Op {
    Admitted(Vec<u8>),
    Disconnected,
    Queued(u8),
    AlreadyDelivered(u8),
    Delivered(Vec<u8>),
    Reload(u64),
    Presence(u8),
    Status(bool),
    Answer(u8),
    Settings(bool),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        collection::vec(0..8_u8, 0..4).prop_map(Op::Admitted),
        Just(Op::Disconnected),
        (0..8_u8).prop_map(Op::Queued),
        (0..8_u8).prop_map(Op::AlreadyDelivered),
        collection::vec(0..8_u8, 0..4).prop_map(Op::Delivered),
        (0..5_u64).prop_map(Op::Reload),
        (0..4_u8).prop_map(Op::Presence),
        any::<bool>().prop_map(Op::Status),
        (0..4_u8).prop_map(Op::Answer),
        any::<bool>().prop_map(Op::Settings),
    ]
}

fn run(state: &mut State, op: Op) -> Vec<ToPage> {
    match op {
        Op::Admitted(ids) => {
            let ids: Vec<CommentId> = ids.into_iter().map(id).collect();
            state.admitted(author(1), &ids).unwrap()
        }
        Op::Disconnected => state.disconnected(None),
        Op::Queued(index) => state
            .acknowledged(AckOutcome::Queued { id: id(index) })
            .unwrap(),
        Op::AlreadyDelivered(index) => state
            .acknowledged(AckOutcome::Delivered { id: id(index) })
            .unwrap(),
        Op::Delivered(ids) => state.apply(ShareMessage::Delivered {
            ids: ids.into_iter().map(id).collect(),
        }),
        Op::Reload(revision) => state.apply(ShareMessage::Reload {
            revision: Revision::from(revision),
        }),
        Op::Presence(count) => {
            let here: Vec<Person> = (1..=count)
                .map(|key| Person::new(author(key), None, Switch::Off))
                .collect();
            let joiners = (1..=count).map(author).collect();
            state.apply(ShareMessage::Presence { joiners, here })
        }
        Op::Status(set) => state.apply(ShareMessage::Status {
            line: set.then(|| StatusLine::try_from("retrying".to_owned()).unwrap()),
        }),
        Op::Answer(index) => state.apply(ShareMessage::Answer {
            id: id(index.wrapping_add(100)),
            message: AnswerText::try_from(format!("answer {index}")).unwrap(),
            anchor: None,
            to: None,
        }),
        Op::Settings(on) => state.set_settings(settings(if on { Switch::On } else { Switch::Off })),
    }
}

proptest! {
    /// A page that attaches at any point holds the joiner's state after
    /// each message: its `hello` and then the messages after it, in order.
    #[test]
    fn hello_and_the_messages_after_it_make_the_state(
        before in collection::vec(op(), 0..20),
        after in collection::vec(op(), 0..20),
    ) {
        let mut state = State::new(settings(Switch::Off));
        for op in before {
            run(&mut state, op);
        }
        let mut page = state.snapshot();
        let hello = ToPage::Hello(state.snapshot());
        follow(&mut page, &hello);
        for op in after {
            for message in run(&mut state, op) {
                follow(&mut page, &message);
            }
            prop_assert_eq!(&page, &state.snapshot());
        }
    }
}

#[test]
fn a_restarted_joiner_waits_for_the_share_before_saying_what_is_pending() {
    let mut state = State::new(settings(Switch::Off));
    // Not admitted yet: nothing is known, not "nothing pending".
    assert_eq!(state.snapshot().mine, None);
    assert!(state.disconnected(None).is_empty());
    assert_eq!(state.snapshot().mine, None);
    // The share holds one of this key's comments from before the restart.
    let messages = state.admitted(author(1), &[id(1)]).unwrap();
    assert!(messages.contains(&ToPage::Mine {
        undelivered: Some(vec![id(1)])
    }));
    // Losing the connection keeps what the share said.
    state.disconnected(None);
    assert_eq!(state.snapshot().mine, Some(vec![id(1)]));
    // Readmitted: the share's word replaces it (delivered meanwhile).
    state.admitted(author(1), &[]).unwrap();
    assert_eq!(state.snapshot().mine, Some(Vec::new()));
}

#[test]
fn a_comment_is_pending_from_its_acknowledgement_to_its_delivery() {
    let mut state = State::new(settings(Switch::Off));
    state.admitted(author(1), &[]).unwrap();
    state
        .acknowledged(AckOutcome::Queued { id: id(2) })
        .unwrap();
    assert_eq!(state.snapshot().mine, Some(vec![id(2)]));
    state.apply(ShareMessage::Delivered { ids: vec![id(2)] });
    assert_eq!(state.snapshot().mine, Some(Vec::new()));
    // Sent again after its delivery: nothing pending.
    state
        .acknowledged(AckOutcome::Delivered { id: id(2) })
        .unwrap();
    assert_eq!(state.snapshot().mine, Some(Vec::new()));
}

#[test]
fn a_readmitted_joiner_forgets_the_old_status_line() {
    let mut state = State::new(settings(Switch::Off));
    state.admitted(author(1), &[]).unwrap();
    let line = StatusLine::try_from("retrying".to_owned()).unwrap();
    state.apply(ShareMessage::Status { line: Some(line) });
    state.disconnected(None);
    // The share sends its status line on admission only when it has one:
    // the one from before is stale.
    state.admitted(author(1), &[]).unwrap();
    assert_eq!(state.snapshot().link.status, None);
}

fn nth(n: usize) -> CommentId {
    format!("00000000-0000-4000-8000-{n:012x}").parse().unwrap()
}

#[test]
fn no_more_comments_are_pending_than_a_share_can_claim() {
    let mut state = State::new(settings(Switch::Off));
    // An admission lists the share's queue as it is.
    let over: Vec<CommentId> = (0..=MAX_QUEUED_PER_KEY).map(nth).collect();
    assert_eq!(state.admitted(author(1), &over), Err(TooMany));
    assert_eq!(state.snapshot().mine, None, "nothing taken");
    state.admitted(author(1), &[]).unwrap();
    // Acknowledgements may run ahead of deliveries, up to a bound.
    for n in 0..MAX_PENDING {
        state
            .acknowledged(AckOutcome::Queued { id: nth(n) })
            .unwrap();
    }
    let before = state.snapshot();
    assert_eq!(
        state.acknowledged(AckOutcome::Queued {
            id: nth(MAX_PENDING)
        }),
        Err(TooMany)
    );
    assert_eq!(state.snapshot(), before, "nothing changed");
    // One it holds may be acknowledged again.
    assert!(
        state
            .acknowledged(AckOutcome::Queued { id: nth(0) })
            .is_ok()
    );
}

/// The share's writer sends a new comment's acknowledgement before an
/// earlier delivery: a joiner whose view of a full queue lags must not
/// take it for a share claiming too much.
#[tokio::test]
async fn an_ack_ahead_of_a_delivery_at_a_full_queue_is_taken() {
    let clock = Arc::new(ManualClock::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let hub = Hub::new(Arc::clone(&clock) as _, Arc::new(SeededRng::new(7)));
    let alice = author(1);
    let mut broadcasts = hub.subscribe();
    let (presence, mut inbox) = hub.join(alice.clone()).unwrap();
    let draft = || -> Draft {
        serde_json::from_str(r#"{"page":"/","selector":"h1","comment":"hi"}"#).unwrap()
    };
    let mut state = State::new(settings(Switch::Off));
    state.admitted(alice.clone(), &[]).unwrap();
    for n in 0..MAX_QUEUED_PER_KEY {
        let reference = format!("ref-{n}").try_into().unwrap();
        let outcome = presence.comment(&alice, reference, draft()).unwrap();
        inbox.acks.try_recv().unwrap();
        state.acknowledged(outcome).unwrap();
        clock.advance(SignedDuration::from_secs(6));
    }
    while broadcasts.try_recv().is_ok() {}
    hub.next_batch().await;
    hub.finish(Attempt::Delivered);
    let reference = "after-delivery".to_owned().try_into().unwrap();
    let outcome = presence.comment(&alice, reference, draft()).unwrap();
    // The writer's order: the acknowledgement first.
    let ShareMessage::CommentAck { outcome: acked, .. } = inbox.acks.recv().await.unwrap() else {
        panic!("an acknowledgement");
    };
    assert_eq!(acked, outcome);
    assert!(state.acknowledged(acked).is_ok());
    let delivered = broadcasts.recv().await.unwrap().message;
    assert!(
        matches!(delivered, ShareMessage::Delivered { .. }),
        "{delivered:?}"
    );
    state.apply(delivered);
    assert_eq!(state.snapshot().pending(), 1);
}
