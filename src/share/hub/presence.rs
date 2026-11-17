//! Who is connected and where: each admitted connection's seat, its
//! presence updates and cursors, and what everyone is told of them.

use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::Ordering;

use tokio::{
    sync::{mpsc, mpsc::error::TrySendError},
    time,
};

use crate::{
    core::{ratelimit::RateLimiter, settings::Switch},
    protocol::{
        comment::{Author, Draft},
        control::{AckOutcome, CommentRef, ShareMessage},
        presence::{PagePath, Person, Position, Whereabouts},
    },
    share::hub::{Ack, Addressed, Hub, PRESENCE_INTERVAL},
};

/// Who `here` are, as their comments are signed.
fn authors(here: &[Person]) -> Vec<Author> {
    here.iter()
        .map(|person| Author {
            name: person.name.clone(),
            fingerprint: person.fingerprint,
            key: person.key,
        })
        .collect()
}

impl Hub {
    pub(super) fn broadcast_presence(&self) {
        let here = self.here();
        self.broadcast(ShareMessage::Presence { here });
    }

    /// Who is connected, in order of admission, once per key.
    #[must_use]
    pub(crate) fn presence(&self) -> Vec<Author> {
        authors(&self.here())
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
}

/// One admitted connection's number, unique while the share runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionId(pub(super) u64);

impl ConnectionId {
    pub(super) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// One admitted connection.
pub(super) struct Seat {
    pub(super) connection: ConnectionId,
    pub(super) author: Author,
    pub(super) url: Option<PagePath>,
    pub(super) cursor: Switch,
    /// When its whereabouts last changed (see `Inner::changes`).
    pub(super) changed: u64,
    /// Presence updates received (for tests).
    #[cfg(test)]
    pub(super) updates: u64,
    /// Its presence updates, against flooding.
    pub(super) limiter: RateLimiter<()>,
    /// Its cursor messages.
    pub(super) cursor_limiter: RateLimiter<()>,
    /// Other joiners' cursors, for its control stream.
    pub(super) cursors: mpsc::Sender<ShareMessage>,
    /// The acknowledgements of its comments, for its control stream.
    pub(super) acks: mpsc::Sender<Ack>,
}

/// Why a joiner's message changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ignored {
    /// Over its rate limit; dropped.
    RateLimited,
    /// Presence updates past the flood limit (recorded), or comments
    /// faster than the joiner reads their acknowledgements: the joiner is
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
    pub(super) hub: Arc<Hub>,
    pub(super) connection: ConnectionId,
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

    /// Queues a comment from this connection's `author` (see
    /// [`Hub::comment`]) and stamps and queues its acknowledgement for
    /// this connection under the hub's lock: before anything the comment
    /// leads to, its delivery included, is broadcast. [`Ignored::Flood`]
    /// when the connection has as many acknowledgements unread as it may.
    pub(crate) fn comment(
        &self,
        author: &Author,
        reference: CommentRef,
        draft: Draft,
    ) -> Result<AckOutcome, Ignored> {
        let outcome = {
            let mut guard = self.hub.lock();
            let inner = &mut *guard;
            let acks = inner
                .joiners
                .iter()
                .find(|seat| seat.connection == self.connection)
                .map(|seat| seat.acks.clone())
                .ok_or(Ignored::Gone)?;
            let permit = acks.try_reserve().map_err(|error| match error {
                TrySendError::Full(()) => Ignored::Flood,
                TrySendError::Closed(()) => Ignored::Gone,
            })?;
            let (outcome, resolved) =
                self.hub
                    .queue_comment(inner, author, reference.clone(), draft);
            // Under `inner`: the order lock comes second (see [`Hub`]).
            let message = ShareMessage::CommentAck { reference, outcome };
            self.hub
                .stamped(|order| permit.send(Ack { order, message }));
            if let Some(Addressed { audience, message }) = resolved {
                self.hub.send(audience, message);
            }
            outcome
        };
        self.hub.wake_for(outcome);
        Ok(outcome)
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
            ShareMessage::Presence { mut here } => {
                let mine = self.url();
                for person in &mut here {
                    if person.url != mine {
                        person.url = None;
                    }
                }
                ShareMessage::Presence { here }
            }
            other @ (ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. }
            | ShareMessage::CommentAck { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::AnswerResolved { .. }
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
