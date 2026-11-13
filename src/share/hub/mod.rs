//! The share's state: the comment queue and its delivery state, the rate
//! limiter, who is connected (and where), the status line and recent
//! answers; sends to every joiner's control stream, or one joiner's.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(test)]
use jiff::Timestamp;

#[cfg(test)]
use crate::core::queue::DeliveryState;
use tokio::sync::{Notify, broadcast, mpsc};

use crate::{
    core::{
        clock::Clock,
        ids::{CommentId, EndpointKey},
        queue::DeliveryQueue,
        ratelimit::{RateLimit, RateLimiter},
        rng::Rng,
        settings::Switch,
    },
    protocol::{
        comment::{Author, Comment},
        control::{Anchor, CommentRef, ShareMessage, StatusLine},
    },
};

/// Broadcast messages a slow joiner may fall behind by before it is
/// disconnected (it reconnects and starts afresh).
const BROADCAST_CAPACITY: usize = 256;
/// Comments remembered for `jampot send --anchor`, delivered or not.
const MAX_ANCHORS: usize = 1000;
/// Answers replayed to joiners on admission.
const MAX_ANSWERS: usize = 100;
/// Admitted connections at once, in all and per joiner key. This also
/// bounds a presence line (64 authors of at most ~160 bytes).
pub(crate) const MAX_JOINERS: usize = 64;
pub(crate) const MAX_JOINERS_PER_KEY: usize = 4;
/// Undelivered comments one joiner key may have in the queue.
pub(crate) const MAX_QUEUED_PER_KEY: usize = 100;
/// Comment refs remembered, so a comment sent again is queued once.
const MAX_REFS: usize = 1000;
/// Cursor messages waiting for one joiner's control stream; more are
/// dropped (the next movement replaces them anyway).
const CURSOR_QUEUE: usize = 32;
/// Presence changes reach joiners at most this often, together.
const PRESENCE_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct Hub {
    inner: Mutex<Inner>,
    broadcast: broadcast::Sender<Outgoing>,
    /// Poked when a comment is queued (or by tests after moving the clock).
    wakeup: Notify,
    /// Poked when a joiner's whereabouts changed; see [`flush_presence`].
    presence_changed: Notify,
    clock: Arc<dyn Clock>,
    rng: Arc<dyn Rng>,
    /// Cursor messages joiners sent, forwarded or not: what tests check a
    /// joiner sends.
    #[cfg(test)]
    cursors_received: AtomicUsize,
}

struct Inner {
    queue: DeliveryQueue<Comment>,
    /// Per joiner key.
    limiter: RateLimiter<EndpointKey>,
    /// Everyone's comments together.
    global: RateLimiter<()>,
    /// Connected joiners, in order of admission.
    joiners: Vec<Seat>,
    /// Counts presence changes, to tell which of a key's connections is the
    /// latest.
    changes: u64,
    /// Recently queued comments by joiner key and the page's ref.
    refs: VecDeque<(EndpointKey, CommentRef, CommentId)>,
    next_connection: ConnectionId,
    status: Option<StatusLine>,
    /// Recent comments' anchors and authors.
    anchors: VecDeque<(Anchor, Author)>,
    answers: VecDeque<Outgoing>,
}

impl Hub {
    pub(crate) fn new(clock: Arc<dyn Clock>, rng: Arc<dyn Rng>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                queue: DeliveryQueue::new(),
                limiter: RateLimiter::new(RateLimit::PER_JOINER),
                global: RateLimiter::new(RateLimit::GLOBAL),
                joiners: Vec::new(),
                changes: 0,
                refs: VecDeque::new(),
                next_connection: ConnectionId(0),
                status: None,
                anchors: VecDeque::new(),
                answers: VecDeque::new(),
            }),
            broadcast: broadcast::channel(BROADCAST_CAPACITY).0,
            wakeup: Notify::new(),
            presence_changed: Notify::new(),
            clock,
            rng,
            #[cfg(test)]
            cursors_received: AtomicUsize::new(0),
        })
    }

    /// How many cursor messages joiners have sent this share.
    #[cfg(test)]
    pub(crate) fn cursors_received(&self) -> usize {
        self.cursors_received.load(Ordering::Relaxed)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Outgoing> {
        self.broadcast.subscribe()
    }

    /// Sends `message` to every joiner.
    pub(crate) fn broadcast(&self, message: ShareMessage) {
        self.send(Audience::Everyone, message);
    }

    fn send(&self, audience: Audience, message: ShareMessage) {
        // No receivers means no joiners: nothing to tell.
        let _ = self.broadcast.send(Outgoing { audience, message });
    }

    /// Adds an admitted joiner to presence until the returned guard drops;
    /// `None` if the share has [`MAX_JOINERS`] connections, or
    /// [`MAX_JOINERS_PER_KEY`] with this key.
    pub(crate) fn join(
        self: &Arc<Self>,
        author: Author,
    ) -> Option<(Presence, mpsc::Receiver<ShareMessage>)> {
        let (cursors, receiver) = mpsc::channel(CURSOR_QUEUE);
        let connection = {
            let mut inner = self.lock();
            let same_key = inner
                .joiners
                .iter()
                .filter(|seat| seat.author.key == author.key)
                .count();
            if inner.joiners.len() >= MAX_JOINERS || same_key >= MAX_JOINERS_PER_KEY {
                return None;
            }
            let connection = inner.next_connection;
            inner.next_connection = connection.next();
            inner.changes = inner.changes.wrapping_add(1);
            let changed = inner.changes;
            inner.joiners.push(Seat {
                connection,
                author,
                url: None,
                cursor: Switch::Off,
                changed,
                limiter: RateLimiter::new(RateLimit::PRESENCE_FLOOD),
                #[cfg(test)]
                updates: 0,
                cursor_limiter: RateLimiter::new(RateLimit::CURSOR),
                cursors,
            });
            connection
        };
        self.broadcast_presence();
        let presence = Presence {
            hub: Arc::clone(self),
            connection,
        };
        Some((presence, receiver))
    }

    /// What a newly admitted joiner is told besides presence: recent answers
    /// (those for everyone, and those for it) and the status line.
    #[must_use]
    pub(crate) fn catch_up(&self, key: EndpointKey) -> Vec<ShareMessage> {
        let inner = self.lock();
        let mut messages: Vec<ShareMessage> = inner
            .answers
            .iter()
            .filter(|answer| match answer.audience {
                Audience::Everyone => true,
                Audience::Key(only) => only == key,
                Audience::Connection(_) => false,
            })
            .map(|answer| answer.message.clone())
            .collect();
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
            .filter(|comment| comment.author.key == author.key)
            .map(|comment| comment.id)
            .collect()
    }

    /// Presence updates received from all connections so far.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn presence_updates(&self) -> u64 {
        self.lock().joiners.iter().map(|seat| seat.updates).sum()
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
/// Who a message from the hub is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Audience {
    Everyone,
    /// The connections of one joiner key: its delivered comments, answers
    /// addressed to it.
    Key(EndpointKey),
    /// One connection: the acknowledgement of a comment it sent.
    Connection(ConnectionId),
}

impl Audience {
    /// Whether connection `connection` of joiner `key` is in it.
    #[must_use]
    pub(crate) fn includes(self, key: EndpointKey, connection: ConnectionId) -> bool {
        match self {
            Self::Everyone => true,
            Self::Key(only) => only == key,
            Self::Connection(only) => only == connection,
        }
    }
}

impl From<Option<EndpointKey>> for Audience {
    fn from(to: Option<EndpointKey>) -> Self {
        to.map_or(Self::Everyone, Self::Key)
    }
}

/// A message from the hub to the joiners' control streams.
#[derive(Clone, Debug)]
pub(crate) struct Outgoing {
    pub(crate) audience: Audience,
    pub(crate) message: ShareMessage,
}

mod answers;
mod comments;
mod presence;

use crate::share::hub::presence::Seat;
pub(crate) use crate::share::hub::presence::{ConnectionId, Presence, flush_presence};
#[cfg(test)]
pub(crate) use crate::share::hub::{
    comments::{HELD, RETRYING},
    presence::Ignored,
};

#[cfg(test)]
mod tests;
