use std::sync::Arc;

use futures_util::FutureExt;
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
        state::{State, TooMany, follow},
    },
    protocol::{
        comment::{Author, Draft},
        control::{
            AckOutcome, AnswerText, CommentRef, MAX_QUEUED_PER_KEY, Revision, ShareMessage,
            StatusLine,
        },
        presence::Person,
    },
    share::hub::{Hub, Outbox},
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
    // Nor may acknowledgements claim more.
    for n in 0..MAX_QUEUED_PER_KEY {
        state
            .acknowledged(AckOutcome::Queued { id: nth(n) })
            .unwrap();
    }
    let before = state.snapshot();
    assert_eq!(
        state.acknowledged(AckOutcome::Queued {
            id: nth(MAX_QUEUED_PER_KEY)
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

/// Takes what the outbox has ready, in its order, into `state`; returns
/// the messages other than presence.
fn take(outbox: &mut Outbox<'_>, state: &mut State) -> Vec<ShareMessage> {
    let mut taken = Vec::new();
    while let Some(message) = outbox.next().now_or_never() {
        let message = message.unwrap();
        match &message {
            ShareMessage::Presence { .. } => continue,
            ShareMessage::CommentAck { outcome, .. } => {
                state.acknowledged(*outcome).unwrap();
            }
            ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::Delivered { .. }
            | ShareMessage::Reload { .. }
            | ShareMessage::Status { .. }
            | ShareMessage::Cursor { .. } => {
                state.apply(message.clone());
            }
        }
        taken.push(message);
    }
    taken
}

/// A new comment's acknowledgement does not overtake the delivery of an
/// earlier batch: the joiner's view of a full queue never holds more than
/// the share does.
#[tokio::test]
async fn an_ack_ahead_of_a_delivery_at_a_full_queue_is_taken() {
    let clock = Arc::new(ManualClock::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let hub = Hub::new(Arc::clone(&clock) as _, Arc::new(SeededRng::new(7)));
    let alice = author(1);
    let broadcasts = hub.subscribe();
    let (presence, inbox) = hub.join(alice.clone()).unwrap();
    let mut outbox = Outbox::new(&presence, alice.key, inbox, broadcasts);
    let draft = || -> Draft {
        serde_json::from_str(r#"{"page":"/","selector":"h1","comment":"hi"}"#).unwrap()
    };
    let mut state = State::new(settings(Switch::Off));
    state.admitted(alice.clone(), &[]).unwrap();
    for n in 0..MAX_QUEUED_PER_KEY {
        let reference = format!("ref-{n}").try_into().unwrap();
        let outcome = presence.comment(&alice, reference, draft()).unwrap();
        assert!(matches!(outcome, AckOutcome::Queued { .. }), "{outcome:?}");
        take(&mut outbox, &mut state);
        clock.advance(SignedDuration::from_secs(6));
    }
    assert_eq!(state.snapshot().pending(), MAX_QUEUED_PER_KEY);
    let batch = hub.next_batch().await;
    hub.finish(Attempt::Delivered);
    let reference: CommentRef = "after-delivery".to_owned().try_into().unwrap();
    let outcome = presence
        .comment(&alice, reference.clone(), draft())
        .unwrap();
    assert!(matches!(outcome, AckOutcome::Queued { .. }), "{outcome:?}");
    let ids = batch.iter().map(|comment| comment.id).collect();
    assert_eq!(
        take(&mut outbox, &mut state),
        [
            ShareMessage::Delivered { ids },
            ShareMessage::CommentAck { reference, outcome },
        ]
    );
    assert_eq!(
        state.snapshot().pending(),
        MAX_QUEUED_PER_KEY - batch.len() + 1
    );
}

/// A comment queued in a slot a delivery just freed is acknowledged after
/// that delivery, also when it is queued while `finish` is still running:
/// the hook below queues it from another thread at the log line `finish`
/// writes once its lock is gone.
#[tokio::test]
async fn a_comment_queued_as_a_batch_finishes_is_acked_after_it() {
    use std::sync::Mutex;

    use tracing_subscriber::{Layer, layer::Context, prelude::*};

    /// Runs its action once, at the first event logged.
    struct AtFirstEvent(Mutex<Option<Box<dyn FnOnce() + Send>>>);
    impl<S: tracing::Subscriber> Layer<S> for AtFirstEvent {
        fn on_event(&self, _: &tracing::Event<'_>, _: Context<'_, S>) {
            let action = self.0.lock().unwrap().take();
            if let Some(action) = action {
                action();
            }
        }
    }

    let clock = Arc::new(ManualClock::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let hub = Hub::new(Arc::clone(&clock) as _, Arc::new(SeededRng::new(7)));
    let alice = author(1);
    let broadcasts = hub.subscribe();
    let (presence, inbox) = hub.join(alice.clone()).unwrap();
    let presence = Arc::new(presence);
    let mut outbox = Outbox::new(&presence, alice.key, inbox, broadcasts);
    let draft = || -> Draft {
        serde_json::from_str(r#"{"page":"/","selector":"h1","comment":"hi"}"#).unwrap()
    };
    let mut state = State::new(settings(Switch::Off));
    state.admitted(alice.clone(), &[]).unwrap();
    for n in 0..MAX_QUEUED_PER_KEY {
        let reference = format!("ref-{n}").try_into().unwrap();
        presence.comment(&alice, reference, draft()).unwrap();
        take(&mut outbox, &mut state);
        clock.advance(SignedDuration::from_secs(6));
    }
    assert_eq!(hub.next_batch().await.len(), MAX_QUEUED_PER_KEY);
    let reader = Arc::clone(&presence);
    let author = alice.clone();
    let during = AtFirstEvent(Mutex::new(Some(Box::new(move || {
        std::thread::spawn(move || {
            let reference = "during-finish".to_owned().try_into().unwrap();
            let outcome = reader.comment(&author, reference, draft()).unwrap();
            assert!(matches!(outcome, AckOutcome::Queued { .. }), "{outcome:?}");
        })
        .join()
        .unwrap();
    }))));
    tracing::subscriber::with_default(tracing_subscriber::registry().with(during), || {
        hub.finish(Attempt::Delivered);
    });
    assert_eq!(hub.queued().len(), 1, "the comment went in during finish");
    let taken = take(&mut outbox, &mut state);
    assert!(
        matches!(
            taken.as_slice(),
            [
                ShareMessage::Delivered { .. },
                ShareMessage::CommentAck { .. }
            ]
        ),
        "{taken:?}"
    );
    assert_eq!(state.snapshot().pending(), 1);
}
