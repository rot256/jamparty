use jiff::SignedDuration;
use proptest::{collection, prelude::*};

use crate::core::queue::{
    DeliveryQueue, DeliveryResult, DeliveryState, MAX_BACKOFF, MAX_QUEUED, QueueError, backoff,
};

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

#[test]
fn one_batch_in_flight_and_arrivals_form_the_next() {
    let mut queue = DeliveryQueue::new();
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    assert_eq!(queue.start(usize::MAX, |_| 1), Some(vec![1, 2]));
    assert_eq!(queue.state(), DeliveryState::InFlight);
    queue.push(3).unwrap();
    assert_eq!(
        queue.start(usize::MAX, |_| 1),
        None,
        "one delivery in flight"
    );
    assert_eq!(queue.finish(DeliveryResult::Ok), Ok(vec![1, 2]));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.start(usize::MAX, |_| 1), Some(vec![3]));
    assert_eq!(queue.finish(DeliveryResult::Ok), Ok(vec![3]));
    assert!(queue.is_empty());
    assert_eq!(queue.start(usize::MAX, |_| 1), None);
    assert_eq!(
        queue.finish(DeliveryResult::Ok),
        Err(QueueError::NotInFlight)
    );
}

#[test]
fn backoff_doubles_up_to_a_minute() {
    let expected = [1, 2, 4, 8, 16, 32, 60, 60];
    for (failures, secs_expected) in (1..).zip(expected) {
        assert_eq!(backoff(failures), secs(secs_expected), "{failures}");
    }
    assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
}

#[test]
fn returned_batches_come_back_without_backoff() {
    let mut queue = DeliveryQueue::new();
    queue.push(1).unwrap();
    queue.start(usize::MAX, |_| 1).unwrap();
    queue.push(2).unwrap();
    assert_eq!(queue.finish(DeliveryResult::Returned), Ok(vec![]));
    assert_eq!(queue.state(), DeliveryState::Ready);
    assert_eq!(queue.start(usize::MAX, |_| 1), Some(vec![1]));
}

#[test]
fn cap_counts_the_batch_in_flight() {
    let mut queue = DeliveryQueue::new();
    for n in 0..MAX_QUEUED {
        queue.push(n).unwrap();
    }
    assert_eq!(queue.push(MAX_QUEUED), Err(QueueError::Full));
    queue.start(usize::MAX, |_| 1).unwrap();
    assert_eq!(queue.push(MAX_QUEUED), Err(QueueError::Full));
    queue.finish(DeliveryResult::Ok).unwrap();
    assert_eq!(queue.push(MAX_QUEUED), Ok(()));
}

#[derive(Clone, Debug)]
enum Op {
    Push,
    Start,
    Finish(DeliveryResult),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => Just(Op::Push),
        3 => Just(Op::Start),
        2 => Just(Op::Finish(DeliveryResult::Ok)),
        1 => Just(Op::Finish(DeliveryResult::Returned)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn delivery_model(ops in collection::vec(op(), 1..300), burst in 0usize..2) {
        let mut queue = DeliveryQueue::new();
        // Model: items accepted in order, how many are delivered, the batch
        // out.
        let mut accepted: Vec<u64> = Vec::new();
        let mut delivered = 0usize;
        // The size of the batch out or kept for a retry; whether it is out.
        let mut batch: Option<usize> = None;
        let mut in_flight = false;
        let mut next = 0u64;

        // Optionally start near the cap so `Full` is exercised.
        for _ in 0..burst * (MAX_QUEUED - 5) {
            queue.push(next).unwrap();
            accepted.push(next);
            next += 1;
        }

        for op in ops {
            let undelivered = accepted.len() - delivered;
            match op {
                Op::Push => match queue.push(next) {
                    Ok(()) => {
                        prop_assert!(undelivered < MAX_QUEUED);
                        accepted.push(next);
                        next += 1;
                    }
                    Err(error) => {
                        prop_assert_eq!(error, QueueError::Full);
                        prop_assert_eq!(undelivered, MAX_QUEUED);
                    }
                },
                Op::Start => {
                    let may = !in_flight
                        && undelivered > 0;
                    match queue.start(usize::MAX, |_| 1) {
                        Some(sent) => {
                            prop_assert!(may);
                            // A failed batch again unchanged, else everything
                            // undelivered; in order, nothing skipped.
                            let end = batch.map_or(accepted.len(), |size| delivered + size);
                            prop_assert_eq!(sent.as_slice(), accepted.get(delivered..end).unwrap());
                            batch = Some(sent.len());
                            in_flight = true;
                        }
                        None => prop_assert!(!may),
                    }
                }
                Op::Finish(result) => {
                    let outcome = queue.finish(result);
                    let (true, Some(size)) = (in_flight, batch) else {
                        prop_assert_eq!(outcome, Err(QueueError::NotInFlight));
                        continue;
                    };
                    in_flight = false;
                    let done = outcome.unwrap();
                    match result {
                        DeliveryResult::Ok => {
                            prop_assert_eq!(done.as_slice(), accepted.get(delivered..delivered + size).unwrap());
                            delivered += size;
                            batch = None;
                        }
                        DeliveryResult::Returned => {
                            prop_assert!(done.is_empty());
                        }
                    }
                }
            }

            // No loss, cap, and the state matches the model.
            prop_assert_eq!(queue.len(), accepted.len() - delivered);
            prop_assert!(queue.len() <= MAX_QUEUED);
            let expected = if in_flight {
                DeliveryState::InFlight
            } else {
                DeliveryState::Ready
            };
            prop_assert_eq!(queue.state(), expected);
        }
    }
}

#[test]
fn bounded_prefix_retries_unchanged_and_remainder_starts_immediately() {
    let mut queue = DeliveryQueue::new();
    for size in [4, 6, 1, 9] {
        queue.push(size).unwrap();
    }
    assert_eq!(queue.start(10, |n| *n), Some(vec![4, 6]));
    queue.finish(DeliveryResult::Returned).unwrap();
    queue.push(2).unwrap();
    // A retry ignores even a changed limit and size function.
    assert_eq!(queue.start(1, |_| 100), Some(vec![4, 6]));
    queue.finish(DeliveryResult::Ok).unwrap();
    assert_eq!(queue.start(10, |n| *n), Some(vec![1, 9]));
    queue.finish(DeliveryResult::Ok).unwrap();
    assert_eq!(queue.start(10, |n| *n), Some(vec![2]));
}

#[test]
fn oversized_first_item_still_makes_progress() {
    let mut queue = DeliveryQueue::new();
    queue.push(11).unwrap();
    queue.push(1).unwrap();
    assert_eq!(queue.start(10, |n| *n), Some(vec![11]));
    queue.finish(DeliveryResult::Ok).unwrap();
    assert_eq!(queue.start(0, |n| *n), Some(vec![1]));
}

#[test]
fn maximal_comments_fit_in_bounded_recv_batches() {
    use crate::{
        core::{
            ids::{CommentId, EndpointKey},
            name::MAX_NAME,
            queue::{DeliveryQueue, DeliveryResult},
            rng::SeededRng,
        },
        protocol::{
            comment::{
                Author, Comment, Draft, DraftFields, MAX_COMMENT, MAX_PAGE, MAX_SELECTED_TEXT,
                MAX_SELECTOR,
            },
            line_size,
        },
        share::delivery::MAX_BATCH_BYTES,
    };

    // U+0000 takes the longest JSON escape (six bytes) and is valid
    // in every free-text field, including a nonblank comment.
    let draft = Draft::try_from(DraftFields {
        attachments: Vec::new(),
        page: "\0".repeat(MAX_PAGE).try_into().unwrap(),
        selector: "\0".repeat(MAX_SELECTOR).try_into().unwrap(),
        choice: None,
        selected_text: Some("\0".repeat(MAX_SELECTED_TEXT)),
        comment: "\0".repeat(MAX_COMMENT),
    })
    .unwrap();
    let comment = Comment::new(
        CommentId::generate(&SeededRng::new(1)),
        Author::new(
            "x".repeat(MAX_NAME).try_into().unwrap(),
            EndpointKey([0xff; 32]),
        ),
        draft,
    );
    let size = line_size(&comment).unwrap();
    assert!(size <= MAX_BATCH_BYTES, "maximal comment is {size} bytes");
    eprintln!("maximal comment NDJSON: {size} bytes");
    let mut queue = DeliveryQueue::new();
    for _ in 0..3 {
        queue.push(comment.clone()).unwrap();
    }
    for _ in 0..3 {
        let batch = queue
            .start(MAX_BATCH_BYTES, |c| line_size(c).unwrap())
            .unwrap();
        assert_eq!(batch, vec![comment.clone()]);
        queue.finish(DeliveryResult::Ok).unwrap();
    }
    assert!(queue.is_empty());
}
