//! The joiner's connection to the share: connect, present the secret, run
//! the control stream, reconnect with backoff.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use iroh::{
    Endpoint,
    endpoint::{ConnectionError, RecvStream, VarInt},
};
use rootcause::prelude::report;
use tokio::{io::BufReader, sync::oneshot, time};

use crate::{
    Report,
    adapters::{
        endpoint, lines,
        path::{self, PathKind},
    },
    core::{
        queue::backoff,
        text::OneLineReport,
        token::PartyToken,
        version::{Incompatible, JampartyVersion, PROTOCOL},
    },
    join::{control::Control, session::Session},
    protocol::{
        control::{
            Envelope, Greeting, Hello, JoinerMessage, REFUSED_CODE, RefusedReason, ShareMessage,
            SiteInfo,
        },
        decode_line,
        presence::Support,
    },
};

/// How long to wait for the share's close reason after admission failed.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// Everything needed to (re)connect.
pub(crate) struct Dial {
    pub(crate) endpoint: Endpoint,
    pub(crate) token: PartyToken,
    /// Direct addresses of the share; empty to use address lookup.
    pub(crate) share_addrs: Vec<SocketAddr>,
    /// How long the share has to answer the hello.
    pub(crate) admission_wait: Duration,
}

/// What the share said when it first admitted this joiner.
#[derive(Clone, Debug)]
pub(crate) struct Welcome {
    /// What the share serves.
    pub(crate) site: SiteInfo,
    /// The share's jamparty.
    pub(crate) jamparty: JampartyVersion,
}

/// Why the joiner gives up on the share for good.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Final {
    /// The share refused it.
    Refused(RefusedReason),
    /// The share speaks another protocol.
    Incompatible(Incompatible),
}

/// How the first admission went.
pub(crate) type First = Result<Welcome, Final>;

/// Who hears of the first admission (taken when told).
type FirstWaiter = Option<oneshot::Sender<First>>;

/// How one connection attempt ended.
enum Ended {
    /// The share refused us, or speaks another protocol; retrying would
    /// not help.
    Final(Final),
    /// Admitted, then lost.
    Lost,
    /// Never admitted.
    Failed(Report),
}

/// Keeps the session connected until the share refuses us; `first` hears
/// how the first admission went.
pub(crate) async fn run(dial: Dial, session: Arc<Session>, first: oneshot::Sender<First>) {
    let mut first = Some(first);
    let mut failures: u32 = 0;
    loop {
        match attempt(&dial, &session, &mut first).await {
            Ended::Final(ended) => {
                tracing::warn!(?ended, "the joiner gives up on the share");
                let reason = match &ended {
                    Final::Refused(reason) => *reason,
                    Final::Incompatible(incompatible) => {
                        session.incompatible(incompatible.clone());
                        RefusedReason::Incompatible
                    }
                };
                session.disconnected(Some(reason));
                if let Some(first) = first.take() {
                    // Nobody waits any more if the start was given up.
                    let _ = first.send(Err(ended));
                }
                return;
            }
            Ended::Lost => {
                tracing::info!("lost the connection to the share; reconnecting");
                failures = 1;
            }
            Ended::Failed(error) => {
                tracing::info!(error = %OneLineReport(&error), "could not reach the share; retrying");
                failures = failures.saturating_add(1);
            }
        }
        session.disconnected(None);
        let wait = Duration::try_from(backoff(failures)).unwrap_or(Duration::from_secs(1));
        time::sleep(wait).await;
    }
}

async fn attempt(dial: &Dial, session: &Session, first: &mut FirstWaiter) -> Ended {
    let connection =
        match endpoint::connect(&dial.endpoint, dial.token.share, &dial.share_addrs).await {
            Ok(connection) => connection,
            Err(error) => return Ended::Failed(error),
        };
    let (mut send, recv) = match connection.open_bi().await {
        Ok(streams) => streams,
        Err(error) => return Ended::Failed(error.into()),
    };
    let mut recv = BufReader::new(recv);
    let hello = Envelope::from(JoinerMessage::Hello(Hello {
        secret: dial.token.secret.clone(),
        name: session.name(),
        jamparty: JampartyVersion::current(),
    }));
    if let Err(error) = lines::write(&mut send, &hello).await {
        return Ended::Failed(error);
    }
    // Leniently first: a share of another protocol says which jamparty it
    // runs even if the rest of its line means nothing here.
    let read = lines::read_with(&mut recv, |line| {
        let greeting = decode_line::<Greeting>(line).ok();
        Ok((greeting, decode_line::<Envelope<ShareMessage>>(line)))
    });
    let Ok(line) = time::timeout(dial.admission_wait, read).await else {
        connection.close(VarInt::from_u32(0), b"no answer");
        return Ended::Failed(report!(
            "the share did not answer the hello within {} s",
            dial.admission_wait.as_secs()
        ));
    };
    let (you, undelivered, welcome) = match line {
        Ok(Some((Some(greeting), _))) if greeting.version != PROTOCOL => {
            connection.close(VarInt::from_u32(0), b"incompatible");
            return Ended::Final(Final::Incompatible(Incompatible {
                share: greeting.jamparty(),
                protocol: greeting.version,
            }));
        }
        Ok(Some((
            _,
            Ok(Envelope {
                version: _,
                message,
            }),
        ))) => match message {
            ShareMessage::Admitted {
                you,
                undelivered,
                presence: Support::Supported,
                site,
                jamparty,
            } => (you, undelivered, Welcome { site, jamparty }),
            ShareMessage::Refused {
                reason: RefusedReason::Incompatible,
                jamparty,
            } => {
                connection.close(VarInt::from_u32(0), b"refused");
                return Ended::Final(Final::Incompatible(Incompatible {
                    share: jamparty,
                    protocol: PROTOCOL,
                }));
            }
            ShareMessage::Refused {
                reason,
                jamparty: _,
            } => {
                connection.close(VarInt::from_u32(0), b"refused");
                return refused(reason);
            }
            ShareMessage::CommentAck { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::Delivered { .. }
            | ShareMessage::Reload { .. }
            | ShareMessage::Status { .. }
            | ShareMessage::Presence { .. }
            | ShareMessage::Cursor { .. } => {
                return Ended::Failed(report!("the share did not start with admission"));
            }
        },
        Ok(Some((_, Err(_)))) | Ok(None) | Err(_) => {
            let closed = time::timeout(CLOSE_WAIT, connection.closed()).await;
            return match refusal(closed.ok()) {
                Some(reason) => refused(reason),
                None => Ended::Failed(report!("the share closed the connection during admission")),
            };
        }
    };
    let control = Arc::new(Control::new(connection, send));
    if session
        .connected(Arc::clone(&control), you, &undelivered)
        .is_err()
    {
        control
            .connection
            .close(VarInt::from_u32(0), b"too many comments");
        return Ended::Failed(report!(
            "the share claims more pending comments than a share keeps"
        ));
    }
    let initial = PathKind::from(&control.connection.paths());
    session.path(initial);
    if let Some(first) = first.take() {
        let _ = first.send(Ok(welcome));
    }
    tracing::info!("connected to the share ({initial})");
    tokio::select! {
        () = receive(&mut recv, &control, session) => {},
        () = path::changes(&control.connection, initial, |path| {
            session.path(path);
            tracing::info!("now {path}");
        }) => {},
    }
    control.connection.close(VarInt::from_u32(0), b"bye");
    Ended::Lost
}

/// A refusal: final, except a full share, which is retried.
fn refused(reason: RefusedReason) -> Ended {
    match reason {
        RefusedReason::Full => Ended::Failed(report!("the share is full")),
        RefusedReason::Incompatible => Ended::Final(Final::Incompatible(Incompatible {
            share: None,
            protocol: PROTOCOL,
        })),
        RefusedReason::WrongSecret | RefusedReason::BadHello => {
            Ended::Final(Final::Refused(reason))
        }
    }
}

/// Applies the share's messages until the stream ends.
async fn receive(recv: &mut BufReader<RecvStream>, control: &Control, session: &Session) {
    loop {
        match lines::read::<Envelope<ShareMessage>, _>(recv).await {
            Ok(Some(Envelope {
                version: _,
                message,
            })) => {
                if session.apply(control, message).is_err() {
                    tracing::warn!("the share claims more pending comments than a share keeps");
                    return;
                }
            }
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(error = %OneLineReport(&error), "control stream ended");
                return;
            }
        }
    }
}

/// The refusal carried by a close, if the share closed with one.
fn refusal(closed: Option<ConnectionError>) -> Option<RefusedReason> {
    match closed {
        Some(ConnectionError::ApplicationClosed(close))
            if close.error_code == VarInt::from_u32(REFUSED_CODE) =>
        {
            RefusedReason::try_from(close.reason.as_ref()).ok()
        }
        Some(_) | None => None,
    }
}
