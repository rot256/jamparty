//! Comments: queued once per ref, rate-limited, delivered in batches; the
//! status line follows the delivery state.

use tokio::time;

use crate::{
    core::{
        ids::{CommentId, EndpointKey},
        queue::{Attempt, DeliveryResult, DeliveryState, QueueError},
    },
    protocol::{
        comment::{Author, Comment, Draft},
        control::{AckOutcome, Anchor, CommentRef, MAX_QUEUED_PER_KEY, ShareMessage, StatusLine},
    },
    share::hub::{Audience, Hub, Inner, MAX_ANCHORS, MAX_REFS},
};

impl Hub {
    /// Rate-limits and queues a comment from `author`. A `reference` this
    /// joiner sent before gets the comment it queued then. Joiners'
    /// comments go through [`Presence::comment`].
    #[cfg(test)]
    pub(crate) fn comment(
        &self,
        author: &Author,
        reference: CommentRef,
        draft: Draft,
    ) -> AckOutcome {
        let outcome = self.queue_comment(&mut self.lock(), author, reference, draft);
        self.wake_for(outcome);
        outcome
    }

    /// Wakes the delivery task for a newly queued comment.
    pub(super) fn wake_for(&self, outcome: AckOutcome) {
        match outcome {
            AckOutcome::Queued { .. } => self.wakeup.notify_one(),
            AckOutcome::Delivered { .. }
            | AckOutcome::RateLimited { .. }
            | AckOutcome::QueueFull => {}
        }
    }

    pub(super) fn queue_comment(
        &self,
        inner: &mut Inner,
        author: &Author,
        reference: CommentRef,
        draft: Draft,
    ) -> AckOutcome {
        let earlier = inner
            .refs
            .iter()
            .find(|(key, known, _)| *key == author.key && *known == reference)
            .map(|(_, _, id)| *id);
        if let Some(id) = earlier {
            let held = inner.queue.iter().any(|comment| comment.id == id);
            return if held {
                AckOutcome::Queued { id }
            } else {
                AckOutcome::Delivered { id }
            };
        }
        let clock = self.clock.as_ref();
        let limited = inner
            .global
            .peek(&(), clock)
            .and_then(|()| inner.limiter.check(author.key, clock))
            .and_then(|()| inner.global.check((), clock));
        if let Err(limited) = limited {
            return AckOutcome::RateLimited {
                retry_after: limited.retry_after.unsigned_abs(),
            };
        }
        let mine = inner
            .queue
            .iter()
            .filter(|comment| comment.author.key == author.key)
            .count();
        if mine >= MAX_QUEUED_PER_KEY {
            return AckOutcome::QueueFull;
        }
        let id = CommentId::generate(self.rng.as_ref());
        let comment = Comment::new(id, author.clone(), draft);
        let anchor = Anchor {
            comment: id,
            page: comment.page.clone(),
            selector: comment.selector.clone(),
        };
        match inner.queue.push(comment) {
            Ok(()) => {
                if inner.anchors.len() == MAX_ANCHORS {
                    inner.anchors.pop_front();
                }
                inner.anchors.push_back((anchor, author.clone()));
                if inner.refs.len() == MAX_REFS {
                    inner.refs.pop_front();
                }
                inner.refs.push_back((author.key, reference, id));
                tracing::info!(%id, author = %author.name, fingerprint = %author.fingerprint, "comment queued");
                AckOutcome::Queued { id }
            }
            Err(QueueError::Full | QueueError::NotInFlight) => AckOutcome::QueueFull,
        }
    }

    /// Waits until a batch may be delivered and takes it (the queue is then
    /// in flight until [`Hub::finish`]).
    pub(crate) async fn next_batch(&self) -> Vec<Comment> {
        loop {
            let notified = self.wakeup.notified();
            let wake_at = {
                let mut inner = self.lock();
                if let Some(batch) = inner.queue.start(self.clock.now()) {
                    return batch;
                }
                inner.queue.wake_at()
            };
            let wait = wake_at
                .map(|at| self.clock.now().duration_until(at))
                .filter(|wait| wait.is_positive());
            match wait {
                Some(wait) => {
                    tokio::select! {
                        () = notified => {}
                        () = time::sleep(wait.unsigned_abs()) => {}
                    }
                }
                None => notified.await,
            }
        }
    }

    /// Records how the in-flight batch's attempt ended; what went wrong
    /// goes only to the share's log. Delivered ids go to each comment's
    /// author only; the status line follows the state.
    pub(crate) fn finish(&self, attempt: Attempt) {
        let result = DeliveryResult::from(&attempt);
        let now = self.clock.now();
        let delivered = {
            let mut inner = self.lock();
            let Ok(delivered) = inner.queue.finish(result, now) else {
                tracing::warn!("delivery finished with nothing in flight");
                return;
            };
            let status = match inner.queue.state() {
                DeliveryState::Ready | DeliveryState::InFlight => None,
                DeliveryState::Retrying { .. } => status_line(RETRYING),
                DeliveryState::Held => status_line(HELD),
            };
            let changed = inner.status != status;
            inner.status.clone_from(&status);
            // Stamped before the lock goes: a comment queued in a slot
            // these free is acknowledged after its author hears of them.
            for (key, ids) in by_author(&delivered) {
                self.send(Audience::Key(key), ShareMessage::Delivered { ids });
            }
            if changed {
                self.broadcast(ShareMessage::Status { line: status });
            }
            delivered.len()
        };
        match attempt {
            Attempt::Delivered => tracing::info!(count = delivered, "comments delivered"),
            Attempt::Failed { why } => tracing::warn!(%why, "delivery failed; retrying"),
            Attempt::Gone { why } => tracing::warn!(%why, "the target is gone; holding comments"),
            Attempt::Returned => tracing::info!("comments returned undelivered"),
        }
    }
}

/// Delivered comments' ids by author: each author hears of its own only.
fn by_author(delivered: &[Comment]) -> Vec<(EndpointKey, Vec<CommentId>)> {
    let mut authors: Vec<(EndpointKey, Vec<CommentId>)> = Vec::new();
    for comment in delivered {
        let key = comment.author.key;
        match authors.iter_mut().find(|(author, _)| *author == key) {
            Some((_, ids)) => ids.push(comment.id),
            None => authors.push((key, vec![comment.id])),
        }
    }
    authors
}

/// The status lines joiners see; what went wrong is in the share's log.
pub(crate) const RETRYING: &str = "delivery failing, retrying";
pub(crate) const HELD: &str = "agent unavailable; comments are kept until the share restarts";

fn status_line(text: &str) -> Option<StatusLine> {
    StatusLine::try_from(text.to_owned()).ok()
}
