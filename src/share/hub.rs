//! The share's state: the comment queue and its delivery state, the rate
//! limiter, who is connected, the status line and recent answers;
//! broadcasts to every joiner's control stream.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

#[cfg(test)]
use jiff::Timestamp;
use tokio::sync::{Notify, broadcast};

use crate::{
    core::{
        clock::Clock,
        ids::{CommentId, EndpointKey},
        queue::{DeliveryQueue, DeliveryResult, DeliveryState, QueueError},
        ratelimit::{RateLimit, RateLimiter},
        rng::Rng,
    },
    protocol::{
        comment::{Author, Comment, Draft},
        control::{AckOutcome, Anchor, AnswerText, MAX_STATUS_LINE, ShareMessage, StatusLine},
    },
};

/// Broadcast messages a slow joiner may fall behind by before it is
/// disconnected (it reconnects and starts afresh).
const BROADCAST_CAPACITY: usize = 256;
/// Comments remembered for `jamjam send --anchor`, delivered or not.
const MAX_ANCHORS: usize = 1000;
/// Answers replayed to joiners on admission.
const MAX_ANSWERS: usize = 100;

pub(crate) struct Hub {
    inner: Mutex<Inner>,
    broadcast: broadcast::Sender<ShareMessage>,
    /// Poked when a comment is queued (or by tests after moving the clock).
    wakeup: Notify,
    clock: Arc<dyn Clock>,
    rng: Arc<dyn Rng>,
}

struct Inner {
    queue: DeliveryQueue<Comment>,
    limiter: RateLimiter,
    /// Connected joiners, by connection.
    joiners: Vec<(u64, Author)>,
    next_connection: u64,
    status: Option<StatusLine>,
    anchors: VecDeque<Anchor>,
    answers: VecDeque<ShareMessage>,
}

impl Hub {
    pub(crate) fn new(clock: Arc<dyn Clock>, rng: Arc<dyn Rng>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                queue: DeliveryQueue::new(),
                limiter: RateLimiter::new(RateLimit::default()),
                joiners: Vec::new(),
                next_connection: 0,
                status: None,
                anchors: VecDeque::new(),
                answers: VecDeque::new(),
            }),
            broadcast: broadcast::channel(BROADCAST_CAPACITY).0,
            wakeup: Notify::new(),
            clock,
            rng,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<ShareMessage> {
        self.broadcast.subscribe()
    }

    pub(crate) fn broadcast(&self, message: ShareMessage) {
        // No receivers means no joiners: nothing to tell.
        let _ = self.broadcast.send(message);
    }

    /// Adds an admitted joiner to presence until the returned guard drops.
    pub(crate) fn join(self: &Arc<Self>, author: Author) -> Presence {
        let connection = {
            let mut inner = self.lock();
            let connection = inner.next_connection;
            inner.next_connection = connection.wrapping_add(1);
            inner.joiners.push((connection, author));
            connection
        };
        self.broadcast_presence();
        Presence {
            hub: Arc::clone(self),
            connection,
        }
    }

    fn broadcast_presence(&self) {
        let joiners = self.presence();
        self.broadcast(ShareMessage::Presence { joiners });
    }

    /// Who is connected, in order of admission.
    #[must_use]
    pub(crate) fn presence(&self) -> Vec<Author> {
        self.lock()
            .joiners
            .iter()
            .map(|(_, author)| author.clone())
            .collect()
    }

    /// What a newly admitted joiner is told besides presence: recent answers
    /// and the status line.
    #[must_use]
    pub(crate) fn catch_up(&self) -> Vec<ShareMessage> {
        let inner = self.lock();
        let mut messages: Vec<ShareMessage> = inner.answers.iter().cloned().collect();
        if let Some(line) = &inner.status {
            messages.push(ShareMessage::Status {
                line: Some(line.clone()),
            });
        }
        messages
    }

    /// The ids of `author`'s comments not delivered yet.
    #[must_use]
    pub(crate) fn undelivered_by(&self, author: &Author) -> Vec<CommentId> {
        self.lock()
            .queue
            .iter()
            .filter(|comment| comment.author.fingerprint == author.fingerprint)
            .map(|comment| comment.id)
            .collect()
    }

    /// Rate-limits and queues a comment from `joiner`, signed as `author`.
    pub(crate) fn comment(&self, joiner: EndpointKey, author: &Author, draft: Draft) -> AckOutcome {
        let mut inner = self.lock();
        if let Err(limited) = inner.limiter.check(joiner, self.clock.as_ref()) {
            let retry = limited.retry_after;
            let secs = retry.as_secs() + i64::from(retry.subsec_nanos() > 0);
            return AckOutcome::RateLimited {
                retry_after_secs: u64::try_from(secs).unwrap_or(0),
            };
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
                inner.anchors.push_back(anchor);
                drop(inner);
                self.wakeup.notify_one();
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
                        () = tokio::time::sleep(wait.unsigned_abs()) => {}
                    }
                }
                None => notified.await,
            }
        }
    }

    /// Records the in-flight batch's result; `detail` says what went wrong.
    /// Delivered ids go to every joiner; the status line follows the state.
    pub(crate) fn finish(&self, result: DeliveryResult, detail: &str) {
        let now = self.clock.now();
        let (delivered, status) = {
            let mut inner = self.lock();
            let Ok(delivered) = inner.queue.finish(result, now) else {
                tracing::warn!("delivery finished with nothing in flight");
                return;
            };
            let status = match inner.queue.state() {
                DeliveryState::Ready | DeliveryState::InFlight => None,
                DeliveryState::Retrying { at } => status_line(&format!(
                    "delivery failed ({detail}); retrying in {}s",
                    now.duration_until(at).as_secs().max(1)
                )),
                DeliveryState::Held => status_line(&format!(
                    "the agent is gone ({detail}); comments are kept until the share restarts"
                )),
            };
            let changed = inner.status != status;
            inner.status.clone_from(&status);
            (delivered, changed.then_some(status))
        };
        match result {
            DeliveryResult::Ok => {
                tracing::info!(count = delivered.len(), "comments delivered");
            }
            DeliveryResult::Error => tracing::warn!(%detail, "delivery failed; retrying"),
            DeliveryResult::Gone => tracing::warn!(%detail, "the target is gone; holding comments"),
            DeliveryResult::Returned => tracing::info!("comments returned undelivered"),
        }
        if !delivered.is_empty() {
            let ids = delivered.iter().map(|comment| comment.id).collect();
            self.broadcast(ShareMessage::Delivered { ids });
        }
        if let Some(line) = status {
            self.broadcast(ShareMessage::Status { line });
        }
    }

    /// Sends an answer to every joiner; `anchor` must be a comment this
    /// share has seen recently.
    pub(crate) fn answer(
        &self,
        message: AnswerText,
        anchor: Option<CommentId>,
    ) -> Result<(), String> {
        let message = {
            let mut inner = self.lock();
            let anchor = match anchor {
                None => None,
                Some(id) => Some(
                    inner
                        .anchors
                        .iter()
                        .find(|anchor| anchor.comment == id)
                        .cloned()
                        .ok_or_else(|| format!("no comment with id {id} on this share"))?,
                ),
            };
            let message = ShareMessage::Answer {
                id: CommentId::generate(self.rng.as_ref()),
                message,
                anchor,
            };
            if inner.answers.len() == MAX_ANSWERS {
                inner.answers.pop_front();
            }
            inner.answers.push_back(message.clone());
            message
        };
        tracing::info!("answer sent");
        self.broadcast(message);
        Ok(())
    }

    /// Re-checks the queue now (tests move a manual clock, then call this).
    #[cfg(test)]
    pub(crate) fn wake(&self) {
        self.wakeup.notify_one();
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn delivery_state(&self) -> DeliveryState {
        self.lock().queue.state()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn wake_at(&self) -> Option<Timestamp> {
        self.lock().queue.wake_at()
    }

    /// The undelivered comments.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn queued(&self) -> Vec<Comment> {
        self.lock().queue.iter().cloned().collect()
    }
}

/// `text` as a status line: control characters become spaces, cut to fit.
fn status_line(text: &str) -> Option<StatusLine> {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_STATUS_LINE)
        .collect();
    StatusLine::try_from(line).ok()
}

/// A joiner's place in presence; leaving is announced on drop.
pub(crate) struct Presence {
    hub: Arc<Hub>,
    connection: u64,
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.hub
            .lock()
            .joiners
            .retain(|(connection, _)| *connection != self.connection);
        self.hub.broadcast_presence();
    }
}
