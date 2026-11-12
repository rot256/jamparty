use std::collections::BTreeMap;

use proptest::prelude::*;

use crate::core::{
    ids::{CommentId, Seq},
    queue::{Acked, CommentQueue, DELIVERED_MEMORY, Delivery, Identified, MAX_QUEUED, QueueError},
    rng::SeededRng,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Item(CommentId);

impl Identified for Item {
    fn id(&self) -> CommentId {
        self.0
    }
}

fn item(rng: &SeededRng) -> Item {
    Item(CommentId::generate(rng))
}

fn seqs(items: &[crate::core::queue::Sequenced<Item>]) -> Vec<u64> {
    items.iter().map(|queued| queued.seq.0).collect()
}

#[test]
fn push_poll_ack() {
    let rng = SeededRng::new(1);
    let mut queue = CommentQueue::new();
    let first = item(&rng);
    assert_eq!(queue.push(first.clone()), Ok(Seq(1)));
    assert_eq!(queue.push(item(&rng)), Ok(Seq(2)));
    assert_eq!(queue.push(item(&rng)), Ok(Seq(3)));

    assert!(queue.has_after(Seq::ZERO));
    assert!(queue.has_after(Seq(2)));
    assert!(!queue.has_after(Seq(3)));
    assert_eq!(seqs(&queue.poll_after(Seq::ZERO, 10)), [1, 2, 3]);
    assert_eq!(seqs(&queue.poll_after(Seq(1), 1)), [2]);
    assert_eq!(seqs(&queue.poll_after(Seq(3), 10)), Vec::<u64>::new());

    assert_eq!(queue.status(first.0), Delivery::Queued);
    assert_eq!(queue.ack(Seq(2)), Ok(Acked::Removed(2)));
    assert_eq!(queue.status(first.0), Delivery::Delivered);
    assert_eq!(queue.status(item(&rng).0), Delivery::Unknown);
    assert_eq!(seqs(&queue.poll_after(Seq::ZERO, 10)), [3]);
    assert_eq!(queue.acked_through(), Seq(2));
}

#[test]
fn stale_and_out_of_range_acks_change_nothing() {
    let rng = SeededRng::new(2);
    let mut queue = CommentQueue::new();
    assert_eq!(queue.ack(Seq::ZERO), Ok(Acked::Stale));
    assert_eq!(
        queue.ack(Seq(1)),
        Err(QueueError::AckOutOfRange { last: Seq::ZERO })
    );
    for _ in 0..3 {
        queue.push(item(&rng)).unwrap();
    }
    assert_eq!(queue.ack(Seq(2)), Ok(Acked::Removed(2)));
    assert_eq!(queue.ack(Seq(1)), Ok(Acked::Stale));
    assert_eq!(queue.ack(Seq(2)), Ok(Acked::Stale));
    assert_eq!(
        queue.ack(Seq(4)),
        Err(QueueError::AckOutOfRange { last: Seq(3) })
    );
    assert_eq!(seqs(&queue.poll_after(Seq::ZERO, 10)), [3]);
    assert_eq!(queue.ack(Seq(3)), Ok(Acked::Removed(1)));
    assert!(queue.is_empty());
    assert_eq!(queue.ack(Seq(3)), Ok(Acked::Stale));
}

#[test]
fn cap_is_enforced_and_freed_by_acks() {
    let rng = SeededRng::new(3);
    let mut queue = CommentQueue::new();
    for _ in 0..MAX_QUEUED {
        queue.push(item(&rng)).unwrap();
    }
    assert_eq!(queue.push(item(&rng)), Err(QueueError::Full));
    assert_eq!(queue.last_seq(), Seq(MAX_QUEUED as u64));
    queue.ack(Seq(1)).unwrap();
    assert_eq!(queue.push(item(&rng)), Ok(Seq(MAX_QUEUED as u64 + 1)));
}

#[test]
fn delivered_memory_is_bounded() {
    let rng = SeededRng::new(4);
    let mut queue = CommentQueue::new();
    let first = item(&rng);
    queue.push(first.clone()).unwrap();
    queue.ack(Seq(1)).unwrap();
    for _ in 0..DELIVERED_MEMORY {
        let seq = queue.push(item(&rng)).unwrap();
        queue.ack(seq).unwrap();
    }
    assert_eq!(queue.status(first.0), Delivery::Unknown);
}

#[derive(Clone, Debug)]
enum Op {
    Push,
    /// Poll after a cursor: 0, the client's cursor, or a random number.
    Poll(CursorChoice, usize),
    /// Acknowledge: relative to the last seen sequence number, may be
    /// stale (negative) or out of range (positive past the end).
    Ack(i64),
    /// The client forgets its cursor (a restart).
    Restart,
}

#[derive(Clone, Copy, Debug)]
enum CursorChoice {
    Zero,
    Client,
    Raw(u64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => Just(Op::Push),
        3 => (
            prop_oneof![
                Just(CursorChoice::Zero),
                Just(CursorChoice::Client),
                (0u64..80).prop_map(CursorChoice::Raw),
            ],
            1usize..20,
        )
            .prop_map(|(cursor, limit)| Op::Poll(cursor, limit)),
        3 => (-5i64..5).prop_map(Op::Ack),
        1 => Just(Op::Restart),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn queue_model(ops in proptest::collection::vec(op(), 1..200)) {
        let rng = SeededRng::new(5);
        let mut queue = CommentQueue::new();
        // Model: every pushed item by sequence number, and the acked bound.
        let mut pushed: BTreeMap<u64, CommentId> = BTreeMap::new();
        let mut acked = 0u64;
        let mut client_cursor = 0u64;

        for op in ops {
            match op {
                Op::Push => {
                    let next = item(&rng);
                    let unacked = pushed.range(acked + 1..).count();
                    match queue.push(next.clone()) {
                        Ok(seq) => {
                            prop_assert!(unacked < MAX_QUEUED);
                            prop_assert_eq!(seq.0, pushed.keys().next_back().copied().unwrap_or(0) + 1);
                            pushed.insert(seq.0, next.0);
                        }
                        Err(error) => {
                            prop_assert_eq!(error, QueueError::Full);
                            prop_assert_eq!(unacked, MAX_QUEUED);
                        }
                    }
                }
                Op::Poll(choice, limit) => {
                    let cursor = match choice {
                        CursorChoice::Zero => 0,
                        CursorChoice::Client => client_cursor,
                        CursorChoice::Raw(raw) => raw,
                    };
                    let polled = queue.poll_after(Seq(cursor), limit);
                    let expected: Vec<(u64, CommentId)> = pushed
                        .range(acked.max(cursor) + 1..)
                        .take(limit)
                        .map(|(seq, id)| (*seq, *id))
                        .collect();
                    let actual: Vec<(u64, CommentId)> =
                        polled.iter().map(|queued| (queued.seq.0, queued.item.0)).collect();
                    // Nothing acked handed out again; order kept; nothing lost.
                    prop_assert_eq!(&actual, &expected);
                    prop_assert_eq!(queue.has_after(Seq(cursor)), pushed.range(acked.max(cursor) + 1..).next().is_some());
                    if let Some((last, _)) = actual.last() {
                        client_cursor = client_cursor.max(*last);
                    }
                }
                Op::Ack(offset) => {
                    let target = client_cursor.saturating_add_signed(offset);
                    let last = pushed.keys().next_back().copied().unwrap_or(0);
                    match queue.ack(Seq(target)) {
                        Ok(Acked::Stale) => prop_assert!(target <= acked),
                        Ok(Acked::Removed(removed)) => {
                            prop_assert!(target > acked && target <= last);
                            prop_assert_eq!(removed, pushed.range(acked + 1..=target).count());
                            acked = target;
                        }
                        Err(error) => {
                            prop_assert_eq!(error, QueueError::AckOutOfRange { last: Seq(last) });
                            prop_assert!(target > last);
                        }
                    }
                }
                Op::Restart => client_cursor = 0,
            }

            prop_assert!(queue.len() <= MAX_QUEUED);
            prop_assert_eq!(queue.len(), pushed.range(acked + 1..).count());
            for (seq, id) in &pushed {
                let expected = if *seq > acked { Delivery::Queued } else { Delivery::Delivered };
                prop_assert_eq!(queue.status(*id), expected);
            }
        }
    }
}
