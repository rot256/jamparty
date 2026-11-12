//! The share's state: the comment queue, the rate limiter and who is
//! connected; broadcasts to every joiner's control stream.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::broadcast;

use crate::{
    core::{
        clock::Clock,
        ids::{CommentId, EndpointKey},
        queue::{DeliveryQueue, QueueError},
        ratelimit::{RateLimit, RateLimiter},
        rng::Rng,
    },
    protocol::{
        comment::{Author, Comment, Draft},
        control::{AckOutcome, ShareMessage},
    },
};

/// Broadcast messages a slow joiner may fall behind by before it is
/// disconnected (it reconnects and starts afresh).
const BROADCAST_CAPACITY: usize = 256;

pub(crate) struct Hub {
    inner: Mutex<Inner>,
    broadcast: broadcast::Sender<ShareMessage>,
    clock: Arc<dyn Clock>,
    rng: Arc<dyn Rng>,
}

struct Inner {
    queue: DeliveryQueue<Comment>,
    limiter: RateLimiter,
    /// Connected joiners, by connection.
    joiners: Vec<(u64, Author)>,
    next_connection: u64,
}

impl Hub {
    pub(crate) fn new(clock: Arc<dyn Clock>, rng: Arc<dyn Rng>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                queue: DeliveryQueue::new(),
                limiter: RateLimiter::new(RateLimit::default()),
                joiners: Vec::new(),
                next_connection: 0,
            }),
            broadcast: broadcast::channel(BROADCAST_CAPACITY).0,
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
        match inner.queue.push(Comment::new(id, author.clone(), draft)) {
            Ok(()) => {
                tracing::info!(%id, author = %author.name, fingerprint = %author.fingerprint, "comment queued");
                AckOutcome::Queued { id }
            }
            Err(QueueError::Full | QueueError::NotInFlight) => AckOutcome::QueueFull,
        }
    }

    /// The undelivered comments.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn queued(&self) -> Vec<Comment> {
        self.lock().queue.iter().cloned().collect()
    }
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
