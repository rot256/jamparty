//! The share's state: the comment queue and its delivery state, the rate
//! limiter, who is connected (and where), the status line and recent
//! answers; sends to every joiner's control stream, or one joiner's.

mod answers;
mod comments;
mod outbox;
mod presence;
#[cfg(test)]
mod tests;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

#[cfg(test)]
use jiff::Timestamp;
use tokio::sync::{Notify, broadcast, mpsc};

#[cfg(test)]
use crate::core::queue::DeliveryState;
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
    share::hub::presence::{ConnectionId, Seat},
};

#[cfg(test)]
pub(crate) use crate::share::hub::{
    comments::{HELD, RETRYING},
    presence::Ignored,
};
pub(crate) use crate::share::hub::{
    outbox::{Outbox, Stopped},
    presence::{Presence, flush_presence},
};

/// Broadcast messages a slow joiner may fall behind by before it is
/// disconnected (it reconnects and starts afresh).
const BROADCAST_CAPACITY: usize = 256;
/// Comments remembered for `jamparty send --anchor`, delivered or not.
const MAX_ANCHORS: usize = 1000;
/// Answers replayed to joiners on admission.
const MAX_ANSWERS: usize = 100;
/// Admitted connections at once, in all and per joiner key. This also
/// bounds a presence line (64 authors of at most ~160 bytes).
pub(crate) const MAX_JOINERS: usize = 64;
pub(crate) const MAX_JOINERS_PER_KEY: usize = 4;
/// Comment refs remembered, so a comment sent again is queued once.
const MAX_REFS: usize = 1000;
/// Cursor messages waiting for one joiner's control stream; more are
/// dropped (the next movement replaces them anyway).
const CURSOR_QUEUE: usize = 32;
/// Acknowledgements waiting for one joiner's control stream. A joiner
/// that sends comments faster than it reads their acknowledgements is
/// flooding, and is disconnected.
pub(crate) const ACK_QUEUE: usize = 64;
/// Presence changes reach joiners at most this often, together.
const PRESENCE_INTERVAL: Duration = Duration::from_millis(100);

/// Locks: `inner`, then `order`, never the other way round. What must be
/// stamped in step with a change to `inner` is stamped under it: a
/// comment's acknowledgement as it is queued (see [`Presence::comment`]),
/// a batch's delivery as it frees queue slots (see [`Hub::finish`]).
/// Nothing takes `inner` while holding `order`.
pub(crate) struct Hub {
    inner: Mutex<Inner>,
    /// The next stamp for a broadcast or an acknowledgement: taken and the
    /// message sent under this lock, together, so a connection's writer
    /// can tell what was sent before what (see [`Outbox`]).
    order: Mutex<Order>,
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
    /// One-shot interleaving for tests, after finish releases `inner`.
    #[cfg(test)]
    after_finish_unlock: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
    answers: VecDeque<Addressed>,
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
            order: Mutex::new(Order(0)),
            broadcast: broadcast::channel(BROADCAST_CAPACITY).0,
            wakeup: Notify::new(),
            presence_changed: Notify::new(),
            clock,
            rng,
            #[cfg(test)]
            cursors_received: AtomicUsize::new(0),
            #[cfg(test)]
            after_finish_unlock: Mutex::new(None),
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

    /// Takes `order`, possibly under `inner` (see [`Hub`]).
    fn send(&self, audience: Audience, message: ShareMessage) {
        self.stamped(|order| {
            // No receivers means no joiners: nothing to tell.
            let _ = self.broadcast.send(Outgoing {
                order,
                audience,
                message,
            });
        });
    }

    /// Runs `send` with the next stamp, under the order lock: whatever it
    /// sends is in its channel before anything stamped later.
    fn stamped<T>(&self, send: impl FnOnce(Order) -> T) -> T {
        let mut next = self.order.lock().unwrap_or_else(PoisonError::into_inner);
        let order = *next;
        *next = order.next();
        send(order)
    }

    /// Adds an admitted joiner to presence until the returned guard drops;
    /// `None` if the share has [`MAX_JOINERS`] connections, or
    /// [`MAX_JOINERS_PER_KEY`] with this key.
    pub(crate) fn join(self: &Arc<Self>, author: Author) -> Option<(Presence, Inbox)> {
        let (cursors, cursors_in) = mpsc::channel(CURSOR_QUEUE);
        let (acks, acks_in) = mpsc::channel(ACK_QUEUE);
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
                acks,
            });
            connection
        };
        self.broadcast_presence();
        let presence = Presence {
            hub: Arc::clone(self),
            connection,
        };
        let inbox = Inbox {
            acks: acks_in,
            cursors: cursors_in,
        };
        Some((presence, inbox))
    }

    /// What a newly admitted joiner is told besides presence: recent answers
    /// (those for everyone, and those for it) and the status line.
    #[must_use]
    pub(crate) fn catch_up(&self, key: EndpointKey) -> Vec<ShareMessage> {
        let inner = self.lock();
        let mut messages: Vec<ShareMessage> = inner
            .answers
            .iter()
            .filter(|answer| {
                answer.audience.includes(key)
                    && !matches!(answer.message, ShareMessage::AnswerResolved { .. })
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
}

impl Audience {
    /// Whether the connections of joiner `key` are in it.
    #[must_use]
    pub(crate) fn includes(self, key: EndpointKey) -> bool {
        match self {
            Self::Everyone => true,
            Self::Key(only) => only == key,
        }
    }
}

/// What one connection is sent besides the hub's broadcasts.
pub(crate) struct Inbox {
    /// The acknowledgements of its comments, each stamped and queued
    /// before anything the comment leads to is broadcast.
    pub(crate) acks: mpsc::Receiver<Ack>,
    /// Other joiners' cursors on its page.
    pub(crate) cursors: mpsc::Receiver<ShareMessage>,
}

impl From<Option<EndpointKey>> for Audience {
    fn from(to: Option<EndpointKey>) -> Self {
        to.map_or(Self::Everyone, Self::Key)
    }
}

/// Where a broadcast or an acknowledgement stands among the others: one
/// counter stamps both, so a connection sends them as the hub did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Order(u64);

impl Order {
    fn next(self) -> Self {
        // A share does not send 2^64 messages.
        Self(self.0.saturating_add(1))
    }
}

/// One connection's acknowledgement of its comment.
#[derive(Clone, Debug)]
pub(crate) struct Ack {
    pub(super) order: Order,
    pub(super) message: ShareMessage,
}

/// A message from the hub to the joiners' control streams.
#[derive(Clone, Debug)]
pub(crate) struct Outgoing {
    pub(crate) order: Order,
    pub(crate) audience: Audience,
    pub(crate) message: ShareMessage,
}

/// A message for some joiners, kept to be sent again (recent answers).
#[derive(Clone, Debug)]
struct Addressed {
    audience: Audience,
    message: ShareMessage,
}
