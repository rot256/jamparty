//! The share's comment queue and delivery state machine.
//!
//! Comments wait in order. One batch is out at a time: [`start`] hands out
//! everything waiting, and comments arriving meanwhile form the next
//! batch. [`finish`] reports the result: `Ok` drops the batch, `Error`
//! keeps it and backs off on the injected clock, `Returned` keeps it with
//! no backoff (the taker went away before it was delivered, e.g. a
//! `jampot recv` that died), `Gone` keeps it and holds the queue for good
//! (the share must be restarted to deliver elsewhere). A kept batch is
//! sent again unchanged, never merged with newer comments, so a target
//! can tell a retry from new comments. No timers here: the caller retries
//! [`start`] at [`DeliveryQueue::wake_at`].
//!
//! [`start`]: DeliveryQueue::start
//! [`finish`]: DeliveryQueue::finish

use std::collections::VecDeque;

use jiff::{SignedDuration, Timestamp};

/// Maximum number of undelivered comments (waiting and in flight).
pub(crate) const MAX_QUEUED: usize = 500;
/// Backoff after the first failure; doubled per failure up to [`MAX_BACKOFF`].
pub(crate) const FIRST_BACKOFF: SignedDuration = SignedDuration::from_secs(1);
pub(crate) const MAX_BACKOFF: SignedDuration = SignedDuration::from_secs(60);

/// What the target said about a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryResult {
    Ok,
    /// A failure worth retrying.
    Error,
    /// The batch came back undelivered through no fault of the target
    /// (a `jampot recv` died before printing it): no backoff.
    Returned,
    /// The target reports itself gone (e.g. the thread was deleted).
    Gone,
}

/// The queue's state, for the status line and the joiners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryState {
    /// Nothing in flight; [`DeliveryQueue::start`] sends if anything waits.
    Ready,
    InFlight,
    /// Waiting until `at` after a failure.
    Retrying {
        at: Timestamp,
    },
    /// The target is gone; nothing is sent again.
    Held,
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
    /// The batch out (`InFlight`) or to be sent again unchanged; empty when
    /// the last one was delivered.
    batch: Vec<T>,
    state: DeliveryState,
    /// Failures since the last success.
    failures: u32,
}

impl<T> Default for DeliveryQueue<T> {
    fn default() -> Self {
        Self {
            waiting: VecDeque::new(),
            batch: Vec::new(),
            state: DeliveryState::Ready,
            failures: 0,
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

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub(crate) fn state(&self) -> DeliveryState {
        self.state
    }

    /// When to call [`DeliveryQueue::start`] again while retrying.
    #[must_use]
    pub(crate) fn wake_at(&self) -> Option<Timestamp> {
        match self.state {
            DeliveryState::Retrying { at } => Some(at),
            DeliveryState::Ready | DeliveryState::InFlight | DeliveryState::Held => None,
        }
    }

    /// Queues `item` behind everything undelivered.
    pub(crate) fn push(&mut self, item: T) -> Result<(), QueueError> {
        if self.len() >= MAX_QUEUED {
            return Err(QueueError::Full);
        }
        self.waiting.push_back(item);
        Ok(())
    }

    /// The batch to send, if nothing is in flight, the backoff has passed
    /// and the target is not gone: the last batch again if it failed,
    /// otherwise everything waiting.
    pub(crate) fn start(&mut self, now: Timestamp) -> Option<Vec<T>> {
        let may_send = match self.state {
            DeliveryState::Ready => true,
            DeliveryState::Retrying { at } => at <= now,
            DeliveryState::InFlight | DeliveryState::Held => false,
        };
        if !may_send {
            return None;
        }
        if self.batch.is_empty() {
            if self.waiting.is_empty() {
                return None;
            }
            self.batch = self.waiting.drain(..).collect();
        }
        self.state = DeliveryState::InFlight;
        Some(self.batch.clone())
    }

    /// Records the in-flight batch's result; returns the delivered items
    /// (the batch on `Ok`, nothing otherwise).
    pub(crate) fn finish(
        &mut self,
        result: DeliveryResult,
        now: Timestamp,
    ) -> Result<Vec<T>, QueueError> {
        if self.state != DeliveryState::InFlight {
            return Err(QueueError::NotInFlight);
        }
        match result {
            DeliveryResult::Ok => {
                self.failures = 0;
                self.state = DeliveryState::Ready;
                return Ok(std::mem::take(&mut self.batch));
            }
            DeliveryResult::Error => {
                self.failures = self.failures.saturating_add(1);
                let at = now
                    .checked_add(backoff(self.failures))
                    .unwrap_or(Timestamp::MAX);
                self.state = DeliveryState::Retrying { at };
            }
            DeliveryResult::Returned => self.state = DeliveryState::Ready,
            DeliveryResult::Gone => self.state = DeliveryState::Held,
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
