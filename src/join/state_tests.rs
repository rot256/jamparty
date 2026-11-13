use proptest::{collection, prelude::*};

use crate::{
    core::{
        ids::{CommentId, EndpointKey},
        settings::{Corner, Keys, Switch},
    },
    join::{
        api::{Hello, ToPage},
        preferences::PageSettings,
        state::{State, follow},
    },
    protocol::{
        comment::Author,
        control::{AckOutcome, AnswerText, Revision, ShareMessage, StatusLine},
        presence::Person,
    },
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
            state.admitted(author(1), &ids)
        }
        Op::Disconnected => state.disconnected(None),
        Op::Queued(index) => state.acknowledged(AckOutcome::Queued { id: id(index) }),
        Op::AlreadyDelivered(index) => state.acknowledged(AckOutcome::Delivered { id: id(index) }),
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
        let hello = ToPage::Hello(Hello::from(state.snapshot()));
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
    let messages = state.admitted(author(1), &[id(1)]);
    assert!(messages.contains(&ToPage::Mine {
        undelivered: Some(vec![id(1)])
    }));
    // Losing the connection keeps what the share said.
    state.disconnected(None);
    assert_eq!(state.snapshot().mine, Some(vec![id(1)]));
    // Readmitted: the share's word replaces it (delivered meanwhile).
    state.admitted(author(1), &[]);
    assert_eq!(state.snapshot().mine, Some(Vec::new()));
}

#[test]
fn a_delivery_that_overtakes_its_acknowledgement_is_kept() {
    let mut state = State::new(settings(Switch::Off));
    state.admitted(author(1), &[]);
    // Delivered before the joiner heard it was queued: never pending.
    state.apply(ShareMessage::Delivered { ids: vec![id(1)] });
    state.acknowledged(AckOutcome::Queued { id: id(1) });
    assert_eq!(state.snapshot().mine, Some(Vec::new()));
    // The usual order.
    state.acknowledged(AckOutcome::Queued { id: id(2) });
    assert_eq!(state.snapshot().mine, Some(vec![id(2)]));
    state.apply(ShareMessage::Delivered { ids: vec![id(2)] });
    assert_eq!(state.snapshot().mine, Some(Vec::new()));
}
