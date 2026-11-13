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
use tokio::{
    sync::{Notify, broadcast, mpsc},
    time,
};

use crate::{
    core::{
        clock::Clock,
        ids::{CommentId, EndpointKey},
        invalid::Invalid,
        queue::{DeliveryQueue, DeliveryResult, DeliveryState, QueueError},
        ratelimit::{RateLimit, RateLimiter},
        rng::Rng,
        settings::Switch,
    },
    protocol::{
        agent::{Addressee, KEY_PREFIX},
        comment::{Author, Comment, Draft},
        control::{AckOutcome, Anchor, AnswerText, CommentRef, ShareMessage, StatusLine},
        presence::{PagePath, Person, Position, Whereabouts},
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

    fn broadcast_presence(&self) {
        let here = self.here();
        let joiners = here
            .iter()
            .map(|person| Author {
                name: person.name.clone(),
                fingerprint: person.fingerprint,
                key: person.key,
            })
            .collect();
        self.broadcast(ShareMessage::Presence { joiners, here });
    }

    /// Who is connected, in order of admission, once per key.
    #[must_use]
    pub(crate) fn presence(&self) -> Vec<Author> {
        self.here()
            .into_iter()
            .map(|person| Author {
                name: person.name,
                fingerprint: person.fingerprint,
                key: person.key,
            })
            .collect()
    }

    /// Who is connected and where, once per key (in order of the key's
    /// first admission, as its latest connection says).
    #[must_use]
    pub(crate) fn here(&self) -> Vec<Person> {
        let inner = self.lock();
        let mut latest: Vec<&Seat> = Vec::new();
        for seat in &inner.joiners {
            match latest
                .iter_mut()
                .find(|known| known.author.key == seat.author.key)
            {
                Some(known) => {
                    if seat.changed > known.changed {
                        *known = seat;
                    }
                }
                None => latest.push(seat),
            }
        }
        latest
            .into_iter()
            .map(|seat| Person::new(seat.author.clone(), seat.url.clone(), seat.cursor))
            .collect()
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
    fn wake_for(&self, outcome: AckOutcome) {
        match outcome {
            AckOutcome::Queued { .. } => self.wakeup.notify_one(),
            AckOutcome::Delivered { .. }
            | AckOutcome::RateLimited { .. }
            | AckOutcome::QueueFull => {}
        }
    }

    fn queue_comment(
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
            let retry = limited.retry_after;
            let secs = retry.as_secs() + i64::from(retry.subsec_nanos() > 0);
            return AckOutcome::RateLimited {
                retry_after_secs: u64::try_from(secs).unwrap_or(0),
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

    /// Records the in-flight batch's result; `detail` says what went wrong
    /// and goes only to the share's log. Delivered ids go to each comment's
    /// author only; the status line follows the state.
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
                DeliveryState::Retrying { .. } => status_line(RETRYING),
                DeliveryState::Held => status_line(HELD),
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
        // Each author hears of its own comments only.
        let mut authors: Vec<(EndpointKey, Vec<CommentId>)> = Vec::new();
        for comment in &delivered {
            let key = comment.author.key;
            match authors.iter_mut().find(|(author, _)| *author == key) {
                Some((_, ids)) => ids.push(comment.id),
                None => authors.push((key, vec![comment.id])),
            }
        }
        for (key, ids) in authors {
            self.send(Audience::Key(key), ShareMessage::Delivered { ids });
        }
        if let Some(line) = status {
            self.broadcast(ShareMessage::Status { line });
        }
    }

    /// Sends an answer, to everyone or to one person: `to` (resolved among
    /// the people here) or, without `to`, the author of the `anchor`
    /// comment. `anchor` must be a comment this share has seen recently.
    /// Returns who it is for (none: everyone).
    pub(crate) fn answer(
        &self,
        message: AnswerText,
        anchor: Option<CommentId>,
        to: Option<Addressee>,
    ) -> Result<Option<Author>, Invalid> {
        let named = match &to {
            Some(person @ (Addressee::Named(_) | Addressee::Key(_))) => {
                Some(self.addressee(person)?)
            }
            Some(Addressee::Everyone) | None => None,
        };
        let (message, recipient) = {
            let mut inner = self.lock();
            let anchor = match anchor {
                None => None,
                Some(id) => Some(
                    inner
                        .anchors
                        .iter()
                        .find(|(anchor, _)| anchor.comment == id)
                        .cloned()
                        .ok_or_else(|| {
                            Invalid::new(format!("no comment with id {id} on this share"))
                        })?,
                ),
            };
            let recipient = match (to, anchor.as_ref()) {
                (Some(Addressee::Named(_) | Addressee::Key(_)), _) => named,
                (Some(Addressee::Everyone), _) | (None, None) => None,
                (None, Some((_, author))) => Some(author.clone()),
            };
            let message = ShareMessage::Answer {
                id: CommentId::generate(self.rng.as_ref()),
                message,
                anchor: anchor.map(|(anchor, _)| anchor),
                to: recipient.as_ref().map(|author| author.key),
            };
            if inner.answers.len() == MAX_ANSWERS {
                inner.answers.pop_front();
            }
            let audience = Audience::from(recipient.as_ref().map(|author| author.key));
            inner.answers.push_back(Outgoing {
                audience,
                message: message.clone(),
            });
            (Outgoing { audience, message }, recipient)
        };
        match &recipient {
            Some(author) => {
                tracing::info!(to = %author.name, fingerprint = %author.fingerprint, "answer sent");
            }
            None => tracing::info!("answer sent to everyone"),
        }
        let Outgoing { audience, message } = message;
        self.send(audience, message);
        Ok(recipient)
    }

    /// The one person here with that name (names only), or that key.
    fn addressee(&self, to: &Addressee) -> Result<Author, Invalid> {
        let here = self.presence();
        let listed = |people: &[&Author]| {
            people
                .iter()
                .map(|author| format!("{} ({KEY_PREFIX}{})", author.name, author.fingerprint))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (found, wanted): (Vec<&Author>, String) = match to {
            Addressee::Named(name) => (
                here.iter().filter(|author| author.name == *name).collect(),
                format!("called {name}"),
            ),
            Addressee::Key(key) => (
                here.iter().filter(|author| key.matches(author)).collect(),
                format!("with {}", String::from(to.clone())),
            ),
            Addressee::Everyone => return Err(Invalid::new("everyone is not one person")),
        };
        match found.as_slice() {
            [author] => Ok((*author).clone()),
            [] => {
                let everyone: Vec<&Author> = here.iter().collect();
                Err(Invalid::new(if everyone.is_empty() {
                    format!("nobody {wanted} is here; nobody is connected")
                } else {
                    format!("nobody {wanted} is here; here: {}", listed(&everyone))
                }))
            }
            several => Err(Invalid::new(format!(
                "several people here are {wanted}: {}; pass --to {KEY_PREFIX}<fingerprint>",
                listed(several)
            ))),
        }
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

/// The status lines joiners see; what went wrong is in the share's log.
pub(crate) const RETRYING: &str = "delivery failing, retrying";
pub(crate) const HELD: &str = "agent unavailable; comments are kept until the share restarts";

fn status_line(text: &str) -> Option<StatusLine> {
    StatusLine::try_from(text.to_owned()).ok()
}

/// One admitted connection's number, unique while the share runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionId(u64);

impl ConnectionId {
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// One admitted connection.
struct Seat {
    connection: ConnectionId,
    author: Author,
    url: Option<PagePath>,
    cursor: Switch,
    /// When its whereabouts last changed (see `Inner::changes`).
    changed: u64,
    /// Presence updates received (for tests).
    #[cfg(test)]
    updates: u64,
    /// Its presence updates, against flooding.
    limiter: RateLimiter<()>,
    /// Its cursor messages.
    cursor_limiter: RateLimiter<()>,
    /// Other joiners' cursors, for its control stream.
    cursors: mpsc::Sender<ShareMessage>,
}

/// Why a joiner's message changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ignored {
    /// Over its rate limit; dropped.
    RateLimited,
    /// Presence updates past the flood limit: recorded, but the joiner is
    /// disconnected.
    Flood,
    /// A cursor while the joiner's cursor switch is off.
    CursorOff,
    /// A cursor before the joiner said which page it is on.
    Nowhere,
    /// The connection left presence meanwhile.
    Gone,
}

impl Ignored {
    /// Whether the joiner is to be disconnected for it.
    #[must_use]
    pub(crate) fn disconnects(self) -> bool {
        match self {
            Self::Flood => true,
            Self::RateLimited | Self::CursorOff | Self::Nowhere | Self::Gone => false,
        }
    }
}

/// A joiner's place in presence; leaving is announced on drop.
pub(crate) struct Presence {
    hub: Arc<Hub>,
    connection: ConnectionId,
}

impl Presence {
    /// Records where the joiner is now and what it is called, always (the
    /// state is small; the last update wins); everyone hears of it shortly
    /// (see [`flush_presence`]). Returns how its comments are signed from
    /// now on, or [`Ignored::Flood`] when the joiner sends far more than it
    /// should (it is then to be disconnected).
    pub(crate) fn update(&self, whereabouts: Whereabouts) -> Result<Author, Ignored> {
        let Whereabouts { name, url, cursor } = whereabouts;
        let (author, flooding) = {
            let mut guard = self.hub.lock();
            let inner = &mut *guard;
            inner.changes = inner.changes.wrapping_add(1);
            let changed = inner.changes;
            let seat = inner
                .joiners
                .iter_mut()
                .find(|seat| seat.connection == self.connection)
                .ok_or(Ignored::Gone)?;
            seat.author.name = name;
            seat.url = url;
            seat.cursor = cursor;
            seat.changed = changed;
            #[cfg(test)]
            {
                seat.updates = seat.updates.saturating_add(1);
            }
            let flooding = seat.limiter.check((), self.hub.clock.as_ref()).is_err();
            (seat.author.clone(), flooding)
        };
        self.hub.presence_changed.notify_one();
        if flooding {
            return Err(Ignored::Flood);
        }
        Ok(author)
    }
}

impl Presence {
    /// Queues a comment from this connection's `author` (see
    /// [`Hub::comment`]) and sends its acknowledgement to this connection
    /// under the hub's lock: before anything the comment leads to, its
    /// delivery included, is sent.
    pub(crate) fn comment(
        &self,
        author: &Author,
        reference: CommentRef,
        draft: Draft,
    ) -> AckOutcome {
        let outcome = {
            let mut inner = self.hub.lock();
            let outcome = self
                .hub
                .queue_comment(&mut inner, author, reference.clone(), draft);
            self.hub.send(
                Audience::Connection(self.connection),
                ShareMessage::CommentAck { reference, outcome },
            );
            outcome
        };
        self.hub.wake_for(outcome);
        outcome
    }

    /// This connection's number.
    #[must_use]
    pub(crate) fn connection(&self) -> ConnectionId {
        self.connection
    }

    /// Forwards this joiner's cursor to the other people on its page (not
    /// to its own key's other connections); returns to how many
    /// connections.
    pub(crate) fn cursor(&self, position: Option<Position>) -> Result<usize, Ignored> {
        #[cfg(test)]
        self.hub.cursors_received.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.hub.lock();
        let inner = &mut *guard;
        let seat = inner
            .joiners
            .iter_mut()
            .find(|seat| seat.connection == self.connection)
            .ok_or(Ignored::Gone)?;
        match seat.cursor {
            Switch::On => {}
            Switch::Off => return Err(Ignored::CursorOff),
        }
        let url = seat.url.clone().ok_or(Ignored::Nowhere)?;
        seat.cursor_limiter
            .check((), self.hub.clock.as_ref())
            .map_err(|_| Ignored::RateLimited)?;
        let key = seat.author.key;
        let message = ShareMessage::Cursor {
            key,
            url: url.clone(),
            position,
        };
        let mut sent = 0;
        for other in &inner.joiners {
            if other.author.key != key && other.url.as_ref() == Some(&url) {
                // A full queue drops this one; the next movement follows.
                if other.cursors.try_send(message.clone()).is_ok() {
                    sent += 1;
                }
            }
        }
        Ok(sent)
    }

    /// The page this joiner is on, as it last said.
    #[must_use]
    pub(crate) fn url(&self) -> Option<PagePath> {
        self.hub
            .lock()
            .joiners
            .iter()
            .find(|seat| seat.connection == self.connection)
            .and_then(|seat| seat.url.clone())
    }

    /// `message` as this joiner gets it: in presence, only the pages of
    /// people on its own page (everyone sees who is here, nobody where
    /// the others are).
    #[must_use]
    pub(crate) fn redact(&self, message: ShareMessage) -> ShareMessage {
        match message {
            ShareMessage::Presence { joiners, mut here } => {
                let mine = self.url();
                for person in &mut here {
                    if person.url != mine {
                        person.url = None;
                    }
                }
                ShareMessage::Presence { joiners, here }
            }
            other @ (ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. }
            | ShareMessage::CommentAck { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::Delivered { .. }
            | ShareMessage::Reload { .. }
            | ShareMessage::Status { .. }
            | ShareMessage::Cursor { .. }) => other,
        }
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.hub
            .lock()
            .joiners
            .retain(|seat| seat.connection != self.connection);
        self.hub.broadcast_presence();
    }
}

/// Tells everyone about changed whereabouts, at most every
/// [`PRESENCE_INTERVAL`]: changes in between go out together.
pub(crate) async fn flush_presence(hub: Arc<Hub>) {
    loop {
        hub.presence_changed.notified().await;
        hub.broadcast_presence();
        time::sleep(PRESENCE_INTERVAL).await;
    }
}

#[cfg(test)]
#[path = "hub_tests.rs"]
mod tests;
