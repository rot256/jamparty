//! The share's comment queue and delivery state machine.
//!
//! Comments wait in order. One batch is out at a time: [`start`] hands out
//! the longest waiting prefix within the caller's encoded byte limit
//! (at least one comment). The share uses about 100 KB of NDJSON per batch;
//! large backlogs and new arrivals form later batches.
//! [`finish`] removes printed comments or returns an unchanged batch when a
//! recv client goes away. There is no delivery backoff: only nudges retry.
//!
//! [`start`]: DeliveryQueue::start
//! [`finish`]: DeliveryQueue::finish

use std::{collections::VecDeque, mem};

use jiff::SignedDuration;

/// Maximum number of undelivered comments (waiting and in flight).
pub(crate) const MAX_QUEUED: usize = 500;
/// Backoff after the first failure; doubled per failure up to [`MAX_BACKOFF`].
const FIRST_BACKOFF: SignedDuration = SignedDuration::from_secs(1);
pub(crate) const MAX_BACKOFF: SignedDuration = SignedDuration::from_secs(60);

/// What the target said about a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryResult {
    Ok,
    /// The batch came back undelivered through no fault of the target
    /// (a `jamparty recv` died before printing it): no backoff.
    Returned,
}

/// The queue's state, for the status line and the joiners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryState {
    /// Nothing in flight; [`DeliveryQueue::start`] sends if anything waits.
    Ready,
    InFlight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueError {
    /// [`MAX_QUEUED`] comments are undelivered.
    Full,
    /// [`DeliveryQueue::finish`] without a batch in flight.
    NotInFlight,
}

#[derive(Debug)]
pub(crate) struct DeliveryQueue<T> {
    /// Not in a batch yet, oldest first.
    waiting: VecDeque<T>,
    /// The batch out or returned unchanged; empty when
    /// the last one was delivered.
    batch: Vec<T>,
    state: DeliveryState,
}

impl<T> Default for DeliveryQueue<T> {
    fn default() -> Self {
        Self {
            waiting: VecDeque::new(),
            batch: Vec::new(),
            state: DeliveryState::Ready,
        }
    }
}

/// The wait after `failures` consecutive failures (at least 1).
#[must_use]
pub(crate) fn backoff(failures: u32) -> SignedDuration {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST_BACKOFF
        .checked_mul(1 << doublings)
        .unwrap_or(MAX_BACKOFF)
        .min(MAX_BACKOFF)
}

impl<T: Clone> DeliveryQueue<T> {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Undelivered comments, the batch first, then waiting.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.batch.iter().chain(self.waiting.iter())
    }

    /// Undelivered comments, waiting and in flight.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.waiting.len() + self.batch.len()
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub(crate) fn state(&self) -> DeliveryState {
        self.state
    }

    /// Queues `item` behind everything undelivered.
    pub(crate) fn push(&mut self, item: T) -> Result<(), QueueError> {
        if self.len() >= MAX_QUEUED {
            return Err(QueueError::Full);
        }
        self.waiting.push_back(item);
        Ok(())
    }

    /// The batch to send if nothing is in flight: a returned batch again,
    /// otherwise the longest prefix within `limit`, measured by `size`.
    /// Always takes at least one item, even if that item exceeds the limit.
    pub(crate) fn start(&mut self, limit: usize, size: impl Fn(&T) -> usize) -> Option<Vec<T>> {
        if self.state == DeliveryState::InFlight {
            return None;
        }
        if self.batch.is_empty() {
            if self.waiting.is_empty() {
                return None;
            }
            let mut bytes = 0usize;
            let mut count = 0;
            for item in &self.waiting {
                let next = size(item);
                if count > 0 && bytes.checked_add(next).is_none_or(|total| total > limit) {
                    break;
                }
                bytes = bytes.saturating_add(next);
                count += 1;
            }
            self.batch = self.waiting.drain(..count).collect();
        }
        self.state = DeliveryState::InFlight;
        Some(self.batch.clone())
    }

    /// Records the in-flight batch's result; returns the delivered items
    /// (the batch on `Ok`, nothing otherwise).
    pub(crate) fn finish(&mut self, result: DeliveryResult) -> Result<Vec<T>, QueueError> {
        if self.state != DeliveryState::InFlight {
            return Err(QueueError::NotInFlight);
        }
        match result {
            DeliveryResult::Ok => {
                self.state = DeliveryState::Ready;
                return Ok(mem::take(&mut self.batch));
            }
            DeliveryResult::Returned => self.state = DeliveryState::Ready,
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
