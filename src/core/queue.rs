//! A session's comment queue: sequence numbers, polls after a cursor and
//! acknowledgements. No timers and no waiting: whoever waits for comments
//! asks [`CommentQueue::has_after`] when woken.

use std::collections::{HashSet, VecDeque};

use crate::core::ids::{CommentId, Seq};

/// Maximum number of queued (unacknowledged) comments per session.
pub(crate) const MAX_QUEUED: usize = 500;
/// How many acknowledged comment ids are remembered for status queries.
pub(crate) const DELIVERED_MEMORY: usize = 4096;

/// Something with a comment id.
pub(crate) trait Identified {
    fn id(&self) -> CommentId;
}

/// A queued item and its sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sequenced<T> {
    pub(crate) seq: Seq,
    pub(crate) item: T,
}

/// What viewers see about one of their comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    Queued,
    Delivered,
    /// Never seen (or forgotten, e.g. after a restart).
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueError {
    /// The queue holds [`MAX_QUEUED`] comments.
    Full,
    /// An acknowledgement beyond the last assigned sequence number. Nothing
    /// is changed.
    AckOutOfRange { last: Seq },
}

/// Result of an accepted acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Acked {
    /// This many comments were dropped from the queue.
    Removed(usize),
    /// At or below an earlier acknowledgement; nothing changed.
    Stale,
}

#[derive(Debug)]
pub(crate) struct CommentQueue<T> {
    /// Unacknowledged items, in increasing sequence order.
    queued: VecDeque<Sequenced<T>>,
    /// The last assigned sequence number.
    last: Seq,
    /// Everything at or below this is acknowledged.
    acked: Seq,
    delivered: HashSet<CommentId>,
    delivered_order: VecDeque<CommentId>,
}

impl<T> Default for CommentQueue<T> {
    fn default() -> Self {
        Self {
            queued: VecDeque::new(),
            last: Seq::ZERO,
            acked: Seq::ZERO,
            delivered: HashSet::new(),
            delivered_order: VecDeque::new(),
        }
    }
}

impl<T: Identified + Clone> CommentQueue<T> {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.queued.len()
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    /// The last assigned sequence number.
    #[must_use]
    pub(crate) fn last_seq(&self) -> Seq {
        self.last
    }

    /// The highest acknowledged sequence number.
    #[must_use]
    pub(crate) fn acked_through(&self) -> Seq {
        self.acked
    }

    /// Appends `item` and returns its sequence number.
    pub(crate) fn push(&mut self, item: T) -> Result<Seq, QueueError> {
        if self.queued.len() >= MAX_QUEUED {
            return Err(QueueError::Full);
        }
        self.last = self.last.next();
        self.queued.push_back(Sequenced {
            seq: self.last,
            item,
        });
        Ok(self.last)
    }

    /// Whether any unacknowledged item has a sequence number above `cursor`.
    #[must_use]
    pub(crate) fn has_after(&self, cursor: Seq) -> bool {
        self.queued.back().is_some_and(|last| last.seq > cursor)
    }

    /// Up to `limit` unacknowledged items after `cursor`, in order.
    #[must_use]
    pub(crate) fn poll_after(&self, cursor: Seq, limit: usize) -> Vec<Sequenced<T>> {
        let start = self.queued.partition_point(|queued| queued.seq <= cursor);
        self.queued.range(start..).take(limit).cloned().collect()
    }

    /// Acknowledges everything up to and including `up_to`.
    pub(crate) fn ack(&mut self, up_to: Seq) -> Result<Acked, QueueError> {
        if up_to > self.last {
            return Err(QueueError::AckOutOfRange { last: self.last });
        }
        if up_to <= self.acked {
            return Ok(Acked::Stale);
        }
        self.acked = up_to;
        let mut removed = 0;
        while let Some(front) = self.queued.pop_front_if(|front| front.seq <= up_to) {
            self.remember_delivered(front.item.id());
            removed += 1;
        }
        Ok(Acked::Removed(removed))
    }

    fn remember_delivered(&mut self, id: CommentId) {
        if self.delivered.insert(id) {
            self.delivered_order.push_back(id);
        }
        while self.delivered_order.len() > DELIVERED_MEMORY {
            if let Some(oldest) = self.delivered_order.pop_front() {
                self.delivered.remove(&oldest);
            }
        }
    }

    /// Delivery state of the comment `id`.
    #[must_use]
    pub(crate) fn status(&self, id: CommentId) -> Delivery {
        if self.queued.iter().any(|queued| queued.item.id() == id) {
            Delivery::Queued
        } else if self.delivered.contains(&id) {
            Delivery::Delivered
        } else {
            Delivery::Unknown
        }
    }
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
