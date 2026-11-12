//! The share's state: the comment queue and its delivery state, the rate
//! limiter, who is connected (and where), the status line and recent
//! answers; broadcasts to every joiner's control stream.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

#[cfg(test)]
use jiff::Timestamp;
use tokio::{
    sync::{Notify, broadcast},
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
        comment::{Author, Comment, Draft},
        control::{AckOutcome, Anchor, AnswerText, CommentRef, ShareMessage, StatusLine},
        presence::{PagePath, Person, Whereabouts},
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
/// Presence changes reach joiners at most this often, together.
const PRESENCE_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct Hub {
    inner: Mutex<Inner>,
    broadcast: broadcast::Sender<ShareMessage>,
    /// Poked when a comment is queued (or by tests after moving the clock).
    wakeup: Notify,
    /// Poked when a joiner's whereabouts changed; see [`flush_presence`].
    presence_changed: Notify,
    clock: Arc<dyn Clock>,
    rng: Arc<dyn Rng>,
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
                limiter: RateLimiter::new(RateLimit::PER_JOINER),
                global: RateLimiter::new(RateLimit::GLOBAL),
                joiners: Vec::new(),
                changes: 0,
                refs: VecDeque::new(),
                next_connection: 0,
                status: None,
                anchors: VecDeque::new(),
                answers: VecDeque::new(),
            }),
            broadcast: broadcast::channel(BROADCAST_CAPACITY).0,
            wakeup: Notify::new(),
            presence_changed: Notify::new(),
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

    /// Adds an admitted joiner to presence until the returned guard drops;
    /// `None` if the share has [`MAX_JOINERS`] connections, or
    /// [`MAX_JOINERS_PER_KEY`] with this key.
    pub(crate) fn join(self: &Arc<Self>, author: Author) -> Option<Presence> {
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
            inner.next_connection = connection.wrapping_add(1);
            inner.changes = inner.changes.wrapping_add(1);
            let changed = inner.changes;
            inner.joiners.push(Seat {
                connection,
                author,
                url: None,
                cursor: Switch::Off,
                changed,
                limiter: RateLimiter::new(RateLimit::PRESENCE),
            });
            connection
        };
        self.broadcast_presence();
        Some(Presence {
            hub: Arc::clone(self),
            connection,
        })
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
    #[cfg(test)]
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
            .filter(|comment| comment.author.key == author.key)
            .map(|comment| comment.id)
            .collect()
    }

    /// Rate-limits and queues a comment from `author`. A `reference` this
    /// joiner sent before gets the comment it queued then.
    pub(crate) fn comment(
        &self,
        author: &Author,
        reference: CommentRef,
        draft: Draft,
    ) -> AckOutcome {
        let mut guard = self.lock();
        let inner = &mut *guard;
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
                inner.anchors.push_back(anchor);
                if inner.refs.len() == MAX_REFS {
                    inner.refs.pop_front();
                }
                inner.refs.push_back((author.key, reference, id));
                drop(guard);
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
                        () = time::sleep(wait.unsigned_abs()) => {}
                    }
                }
                None => notified.await,
            }
        }
    }

    /// Records the in-flight batch's result; `detail` says what went wrong
    /// and goes only to the share's log. Delivered ids go to every joiner;
    /// the status line follows the state.
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
    ) -> Result<(), Invalid> {
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
                        .ok_or_else(|| {
                            Invalid::new(format!("no comment with id {id} on this share"))
                        })?,
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

/// The status lines joiners see; what went wrong is in the share's log.
pub(crate) const RETRYING: &str = "delivery failing, retrying";
pub(crate) const HELD: &str = "agent unavailable; comments are kept until the share restarts";

fn status_line(text: &str) -> Option<StatusLine> {
    StatusLine::try_from(text.to_owned()).ok()
}

/// One admitted connection.
struct Seat {
    connection: u64,
    author: Author,
    url: Option<PagePath>,
    cursor: Switch,
    /// When its whereabouts last changed (see `Inner::changes`).
    changed: u64,
    /// Its presence updates.
    limiter: RateLimiter<()>,
}

/// Why a joiner's message changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ignored {
    /// Over its rate limit; dropped.
    RateLimited,
    /// The connection left presence meanwhile.
    Gone,
}

/// A joiner's place in presence; leaving is announced on drop.
pub(crate) struct Presence {
    hub: Arc<Hub>,
    connection: u64,
}

impl Presence {
    /// Records where the joiner is now and what it is called; everyone
    /// hears of it shortly (see [`flush_presence`]). Returns how its
    /// comments are signed from now on.
    pub(crate) fn update(&self, whereabouts: Whereabouts) -> Result<Author, Ignored> {
        let Whereabouts { name, url, cursor } = whereabouts;
        let author = {
            let mut guard = self.hub.lock();
            let inner = &mut *guard;
            inner.changes = inner.changes.wrapping_add(1);
            let changed = inner.changes;
            let seat = inner
                .joiners
                .iter_mut()
                .find(|seat| seat.connection == self.connection)
                .ok_or(Ignored::Gone)?;
            seat.limiter
                .check((), self.hub.clock.as_ref())
                .map_err(|_| Ignored::RateLimited)?;
            seat.author.name = name;
            seat.url = url;
            seat.cursor = cursor;
            seat.changed = changed;
            seat.author.clone()
        };
        self.hub.presence_changed.notify_one();
        Ok(author)
    }
}

impl Presence {
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
            | ShareMessage::Status { .. }) => other,
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
