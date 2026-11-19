//! Comments: queued once per ref, rate-limited, delivered in batches; the
//! author hears delivery only after recv confirms printing.

use crate::{
    core::{
        ids::{CommentId, EndpointKey},
        queue::{DeliveryResult, QueueError},
    },
    protocol::{
        comment::{Author, Comment, Draft, DraftFields},
        control::{AckOutcome, Anchor, CommentRef, MAX_QUEUED_PER_KEY, ShareMessage},
        line_size,
    },
    share::{
        delivery::MAX_BATCH_BYTES,
        hub::{Addressed, Audience, Hub, Inner, MAX_ANCHORS, MAX_REFS},
    },
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
        let outcome = {
            let mut inner = self.lock();
            let (outcome, resolved) = self.queue_comment(&mut inner, author, reference, draft);
            if let Some(Addressed { audience, message }) = resolved {
                self.send(audience, message);
            }
            outcome
        };
        self.wake_for(outcome);
        outcome
    }

    /// Wakes the delivery task for a newly queued comment.
    pub(super) fn wake_for(&self, outcome: AckOutcome) {
        match outcome {
            AckOutcome::Queued { .. } => {
                self.wakeup.notify_one();
                self.nudge_changed.notify_one();
            }
            AckOutcome::Delivered { .. }
            | AckOutcome::RateLimited { .. }
            | AckOutcome::AttachmentUnavailable
            | AckOutcome::ChoiceRefused
            | AckOutcome::QueueFull => {}
        }
    }

    /// Queues once and, only for a new winning pick, returns its resolution.
    /// The caller stamps the acknowledgement before sending that resolution.
    pub(super) fn queue_comment(
        &self,
        inner: &mut Inner,
        author: &Author,
        reference: CommentRef,
        draft: Draft,
    ) -> (AckOutcome, Option<Addressed>) {
        let earlier = inner
            .refs
            .iter()
            .find(|(key, known, _)| *key == author.key && *known == reference)
            .map(|(_, _, id)| *id);
        if let Some(id) = earlier {
            let held = inner.queue.iter().any(|comment| comment.id == id);
            return (
                if held {
                    AckOutcome::Queued { id }
                } else {
                    AckOutcome::Delivered { id }
                },
                None,
            );
        }
        // Refs are checked first: retrying the winning pick gets its original id.
        let fields = DraftFields::from(draft.clone());
        if fields
            .attachments
            .iter()
            .any(|attachment| !inner.resources.contains(attachment.id()))
        {
            return (AckOutcome::AttachmentUnavailable, None);
        }
        let question = if let Some(choice) = &fields.choice {
            let id = fields
                .selector
                .as_ref()
                .strip_prefix("jamparty-agent:")
                .and_then(|id| id.parse::<CommentId>().ok());
            let offered = inner.answers.iter().find(|answer| {
                answer.audience.includes(author.key)
                    && matches!(&answer.message,
                    ShareMessage::Answer { id: known, choices, .. }
                    if Some(*known) == id && choices.contains(choice))
            });
            if fields.comment != *choice || offered.is_none() {
                return (AckOutcome::ChoiceRefused, None);
            }
            id
        } else {
            None
        };
        let clock = self.clock.as_ref();
        let limited = inner
            .global
            .peek(&(), clock)
            .and_then(|()| inner.limiter.check(author.key, clock))
            .and_then(|()| inner.global.check((), clock));
        if let Err(limited) = limited {
            return (
                AckOutcome::RateLimited {
                    retry_after: limited.retry_after.unsigned_abs(),
                },
                None,
            );
        }
        let mine = inner
            .queue
            .iter()
            .filter(|comment| comment.author.key == author.key)
            .count();
        if mine >= MAX_QUEUED_PER_KEY {
            return (AckOutcome::QueueFull, None);
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
                let resolved = if let Some(question) = question
                    && let Some(answer) = inner.answers.iter_mut().find(|answer| {
                        matches!(
                            &answer.message, ShareMessage::Answer { id, .. } if *id == question
                        )
                    }) {
                    answer.message = ShareMessage::AnswerResolved { id: question };
                    Some(answer.clone())
                } else {
                    None
                };
                (AckOutcome::Queued { id }, resolved)
            }
            Err(QueueError::Full | QueueError::NotInFlight) => (AckOutcome::QueueFull, None),
        }
    }

    /// Waits until a batch may be delivered and takes it (the queue is then
    /// in flight until [`Hub::finish`]).
    pub(crate) async fn next_batch(&self) -> Vec<Comment> {
        loop {
            let notified = self.wakeup.notified();
            {
                let mut inner = self.lock();
                if let Some(batch) = inner.queue.start(MAX_BATCH_BYTES, |comment| {
                    line_size(comment).unwrap_or(usize::MAX)
                }) {
                    return batch;
                }
            }
            notified.await;
        }
    }

    /// Installs a one-shot interleaving without depending on log filtering.
    #[cfg(test)]
    pub(crate) fn after_finish_unlock(&self, hook: impl FnOnce() + Send + 'static) {
        *self.after_finish_unlock.lock().unwrap() = Some(Box::new(hook));
    }

    /// Completes recv delivery. Delivered ids go only to their author.
    pub(crate) fn finish(&self, result: DeliveryResult) {
        let delivered = {
            let mut inner = self.lock();
            let Ok(delivered) = inner.queue.finish(result) else {
                tracing::warn!("delivery finished with nothing in flight");
                return;
            };
            // A confirmed batch ends this backlog, even if more remain.
            if result == DeliveryResult::Ok {
                inner.nudge = Default::default();
                inner.generation = inner.generation.wrapping_add(1);
            }
            // Stamped before the lock goes: a comment queued in a slot
            // these free is acknowledged after its author hears of them.
            for (key, ids) in by_author(&delivered) {
                self.send(Audience::Key(key), ShareMessage::Delivered { ids });
            }
            delivered.len()
        };
        #[cfg(test)]
        {
            let hook = self.after_finish_unlock.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.nudge_changed.notify_one();
        tracing::info!(count = delivered, "recv batch finished");
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
