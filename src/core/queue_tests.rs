use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;

use crate::core::queue::{
    DeliveryQueue, DeliveryResult, DeliveryState, MAX_BACKOFF, MAX_QUEUED, QueueError, backoff,
};

fn t0() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

#[test]
fn one_batch_in_flight_and_arrivals_form_the_next() {
    let mut queue = DeliveryQueue::new();
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    assert_eq!(queue.start(t0()), Some(vec![1, 2]));
    assert_eq!(queue.state(), DeliveryState::InFlight);
    queue.push(3).unwrap();
    assert_eq!(queue.start(t0()), None, "one delivery in flight");
    assert_eq!(queue.finish(DeliveryResult::Ok, t0()), Ok(vec![1, 2]));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.start(t0()), Some(vec![3]));
    assert_eq!(queue.finish(DeliveryResult::Ok, t0()), Ok(vec![3]));
    assert!(queue.is_empty());
    assert_eq!(queue.start(t0()), None);
    assert_eq!(
        queue.finish(DeliveryResult::Ok, t0()),
        Err(QueueError::NotInFlight)
    );
}

#[test]
fn errors_back_off_and_retry_in_order() {
    let mut queue = DeliveryQueue::new();
    queue.push(1).unwrap();
    queue.start(t0()).unwrap();
    queue.push(2).unwrap();
    assert_eq!(queue.finish(DeliveryResult::Error, t0()), Ok(vec![]));
    assert_eq!(queue.wake_at(), Some(t0() + secs(1)));
    assert_eq!(queue.start(t0()), None);
    assert_eq!(queue.start(t0() + secs(1)), Some(vec![1, 2]));
    queue.finish(DeliveryResult::Error, t0() + secs(1)).unwrap();
    assert_eq!(queue.wake_at(), Some(t0() + secs(3)));
    queue.start(t0() + secs(3)).unwrap();
    queue.finish(DeliveryResult::Ok, t0() + secs(3)).unwrap();
    assert_eq!(queue.state(), DeliveryState::Ready);
    // Success resets the backoff.
    queue.push(3).unwrap();
    queue.start(t0() + secs(3)).unwrap();
    queue.finish(DeliveryResult::Error, t0() + secs(3)).unwrap();
    assert_eq!(queue.wake_at(), Some(t0() + secs(4)));
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
fn gone_holds_everything() {
    let mut queue = DeliveryQueue::new();
    queue.push(1).unwrap();
    queue.start(t0()).unwrap();
    assert_eq!(queue.finish(DeliveryResult::Gone, t0()), Ok(vec![]));
    assert_eq!(queue.state(), DeliveryState::Held);
    queue.push(2).unwrap();
    assert_eq!(queue.start(t0() + secs(86_400)), None);
    assert_eq!(queue.wake_at(), None);
    assert_eq!(queue.len(), 2);
}

#[test]
fn cap_counts_the_batch_in_flight() {
    let mut queue = DeliveryQueue::new();
    for n in 0..MAX_QUEUED {
        queue.push(n).unwrap();
    }
    assert_eq!(queue.push(MAX_QUEUED), Err(QueueError::Full));
    queue.start(t0()).unwrap();
    assert_eq!(queue.push(MAX_QUEUED), Err(QueueError::Full));
    queue.finish(DeliveryResult::Ok, t0()).unwrap();
    assert_eq!(queue.push(MAX_QUEUED), Ok(()));
}

#[derive(Clone, Debug)]
enum Op {
    Push,
    Start,
    Finish(DeliveryResult),
    /// Advance the clock by this many milliseconds.
    Wait(i64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => Just(Op::Push),
        3 => Just(Op::Start),
        2 => Just(Op::Finish(DeliveryResult::Ok)),
        2 => Just(Op::Finish(DeliveryResult::Error)),
        1 => Just(Op::Finish(DeliveryResult::Gone)),
        3 => (0i64..90_000).prop_map(Op::Wait),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn delivery_model(ops in proptest::collection::vec(op(), 1..300), burst in 0usize..2) {
        let mut queue = DeliveryQueue::new();
        let mut now = t0();
        // Model: items accepted in order, how many are delivered, the batch
        // out, failures since the last success, earliest retry, gone.
        let mut accepted: Vec<u64> = Vec::new();
        let mut delivered = 0usize;
        let mut in_flight: Option<usize> = None;
        let mut failures = 0u32;
        let mut retry_at: Option<Timestamp> = None;
        let mut gone = false;
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
                    let may = in_flight.is_none()
                        && !gone
                        && retry_at.is_none_or(|at| at <= now)
                        && undelivered > 0;
                    match queue.start(now) {
                        Some(batch) => {
                            prop_assert!(may);
                            // Everything undelivered, in order, nothing skipped.
                            prop_assert_eq!(batch.as_slice(), accepted.get(delivered..).unwrap());
                            in_flight = Some(batch.len());
                        }
                        None => prop_assert!(!may),
                    }
                }
                Op::Finish(result) => {
                    let outcome = queue.finish(result, now);
                    let Some(size) = in_flight.take() else {
                        prop_assert_eq!(outcome, Err(QueueError::NotInFlight));
                        continue;
                    };
                    let done = outcome.unwrap();
                    match result {
                        DeliveryResult::Ok => {
                            prop_assert_eq!(done.as_slice(), accepted.get(delivered..delivered + size).unwrap());
                            delivered += size;
                            failures = 0;
                            retry_at = None;
                        }
                        DeliveryResult::Error => {
                            prop_assert!(done.is_empty());
                            failures += 1;
                            let at = now + backoff(failures);
                            prop_assert_eq!(queue.wake_at(), Some(at));
                            retry_at = Some(at);
                        }
                        DeliveryResult::Gone => {
                            prop_assert!(done.is_empty());
                            gone = true;
                        }
                    }
                }
                Op::Wait(millis) => now += SignedDuration::from_millis(millis),
            }

            // No loss, cap, and the state matches the model.
            prop_assert_eq!(queue.len(), accepted.len() - delivered);
            prop_assert!(queue.len() <= MAX_QUEUED);
            let expected = if in_flight.is_some() {
                DeliveryState::InFlight
            } else if gone {
                DeliveryState::Held
            } else if let Some(at) = retry_at {
                DeliveryState::Retrying { at }
            } else {
                DeliveryState::Ready
            };
            prop_assert_eq!(queue.state(), expected);
        }
    }
}
