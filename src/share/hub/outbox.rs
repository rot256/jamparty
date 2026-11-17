//! What one connection's control stream is sent next: its
//! acknowledgements and the hub's broadcasts in the order the hub stamped
//! them, then other joiners' cursors.

use tokio::sync::{
    broadcast::{
        self,
        error::{RecvError, TryRecvError},
    },
    mpsc::error::TryRecvError as AckTryRecvError,
};

use crate::{
    core::ids::EndpointKey,
    protocol::control::ShareMessage,
    share::hub::{Ack, Inbox, Outgoing, Presence},
};

/// Why a connection's outbox has nothing more to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stopped {
    /// It fell more than the broadcast channel holds behind.
    FellBehind,
    /// The share is stopping.
    ShareStopped,
}

/// One connection's messages, in order. An acknowledgement is stamped
/// before anything its comment leads to is broadcast, so it goes out
/// before its comment's delivery; and new acknowledgements cannot hold
/// back an older broadcast (such as the delivery of an earlier batch),
/// which strict priority for acknowledgements would. Cursors go out when
/// nothing else is waiting, in no particular order with the rest.
#[must_use = "an outbox sends nothing until asked for its next message"]
pub(crate) struct Outbox<'a> {
    presence: &'a Presence,
    key: EndpointKey,
    inbox: Inbox,
    broadcasts: broadcast::Receiver<Outgoing>,
    /// At most one of each taken but not sent yet.
    ack: Option<Ack>,
    broadcast: Option<Outgoing>,
}

impl<'a> Outbox<'a> {
    /// The outbox of `presence`'s connection, whose joiner has `key`;
    /// `broadcasts` subscribed before it joined.
    pub(crate) fn new(
        presence: &'a Presence,
        key: EndpointKey,
        inbox: Inbox,
        broadcasts: broadcast::Receiver<Outgoing>,
    ) -> Self {
        Self {
            presence,
            key,
            inbox,
            broadcasts,
            ack: None,
            broadcast: None,
        }
    }

    /// The next message to send. Cancel-safe: what it took is kept for
    /// the next call.
    ///
    /// With one of an acknowledgement and a broadcast held, the other
    /// channel is checked without waiting: one stamped earlier is in it
    /// already, as stamping and sending happen together. The earlier of
    /// the two goes first; with the other channel empty, the held one.
    pub(crate) async fn next(&mut self) -> Result<ShareMessage, Stopped> {
        loop {
            match (self.ack.take(), self.broadcast.take()) {
                (None, None) => {
                    tokio::select! {
                        biased;
                        ack = self.inbox.acks.recv() => {
                            self.ack = Some(ack.ok_or(Stopped::ShareStopped)?);
                        }
                        outgoing = self.broadcasts.recv() => {
                            self.broadcast = Some(outgoing.map_err(Stopped::from)?);
                        }
                        cursor = self.inbox.cursors.recv() => {
                            return cursor.ok_or(Stopped::ShareStopped);
                        }
                    }
                }
                (Some(ack), None) => match self.broadcasts.try_recv() {
                    Ok(outgoing) => {
                        self.ack = Some(ack);
                        self.broadcast = Some(outgoing);
                    }
                    Err(TryRecvError::Empty) => return Ok(ack.message),
                    Err(TryRecvError::Lagged(_)) => return Err(Stopped::FellBehind),
                    Err(TryRecvError::Closed) => return Err(Stopped::ShareStopped),
                },
                (None, Some(outgoing)) => match self.inbox.acks.try_recv() {
                    Ok(ack) => {
                        self.ack = Some(ack);
                        self.broadcast = Some(outgoing);
                    }
                    Err(AckTryRecvError::Empty) => {
                        if let Some(message) = self.mine(outgoing) {
                            return Ok(message);
                        }
                    }
                    Err(AckTryRecvError::Disconnected) => return Err(Stopped::ShareStopped),
                },
                (Some(ack), Some(outgoing)) => {
                    if ack.order < outgoing.order {
                        self.broadcast = Some(outgoing);
                        return Ok(ack.message);
                    }
                    self.ack = Some(ack);
                    if let Some(message) = self.mine(outgoing) {
                        return Ok(message);
                    }
                }
            }
        }
    }

    /// A broadcast as this joiner gets it; none if for someone else
    /// (answers and deliveries for another joiner).
    fn mine(&self, outgoing: Outgoing) -> Option<ShareMessage> {
        let Outgoing {
            order: _,
            audience,
            message,
        } = outgoing;
        audience
            .includes(self.key)
            .then(|| self.presence.redact(message))
    }
}

impl From<RecvError> for Stopped {
    fn from(error: RecvError) -> Self {
        match error {
            RecvError::Lagged(_) => Self::FellBehind,
            RecvError::Closed => Self::ShareStopped,
        }
    }
}
