//! The joiner's connection to the share: connect, present the secret, run
//! the control stream, reconnect with backoff.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use iroh::{
    Endpoint,
    endpoint::{ConnectionError, RecvStream, VarInt},
};
use tokio::io::BufReader;

use crate::{
    BoxError,
    adapters::{endpoint, lines},
    core::{name::JoinerName, queue::backoff, token::PartyToken},
    join::session::{Control, Session},
    protocol::control::{Envelope, JoinerMessage, RefusedReason, ShareMessage},
    share::REFUSED_CODE,
};

/// How long to wait for the share's close reason after admission failed.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// Everything needed to (re)connect.
pub(crate) struct Dial {
    pub(crate) endpoint: Endpoint,
    pub(crate) token: PartyToken,
    pub(crate) name: JoinerName,
    /// Direct addresses of the share; empty to use address lookup.
    pub(crate) share_addrs: Vec<SocketAddr>,
}

/// How one connection attempt ended.
enum Ended {
    /// The share refused us; retrying would not help.
    Refused(RefusedReason),
    /// Admitted, then lost.
    Lost,
    /// Never admitted.
    Failed(BoxError),
}

/// Keeps the session connected until the share refuses us.
pub(crate) async fn run(dial: Dial, session: Arc<Session>) {
    let mut failures: u32 = 0;
    loop {
        match attempt(&dial, &session).await {
            Ended::Refused(reason) => {
                tracing::warn!(?reason, "the share refused this joiner");
                session.disconnected(Some(reason));
                return;
            }
            Ended::Lost => {
                tracing::info!("lost the connection to the share; reconnecting");
                failures = 1;
            }
            Ended::Failed(error) => {
                tracing::info!(%error, "could not reach the share; retrying");
                failures = failures.saturating_add(1);
            }
        }
        session.disconnected(None);
        let wait = Duration::try_from(backoff(failures)).unwrap_or(Duration::from_secs(1));
        tokio::time::sleep(wait).await;
    }
}

async fn attempt(dial: &Dial, session: &Session) -> Ended {
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
    let hello = Envelope::from(JoinerMessage::Hello {
        secret: dial.token.secret.clone(),
        name: dial.name.clone(),
    });
    if let Err(error) = lines::write(&mut send, &hello).await {
        return Ended::Failed(error);
    }
    let you = match lines::read::<Envelope<ShareMessage>, _>(&mut recv).await {
        Ok(Some(Envelope {
            version: _,
            message: ShareMessage::Admitted { you },
        })) => you,
        Ok(Some(Envelope {
            version: _,
            message: ShareMessage::Refused { reason },
        })) => {
            connection.close(VarInt::from_u32(0), b"refused");
            return Ended::Refused(reason);
        }
        Ok(_) | Err(_) => {
            let closed = tokio::time::timeout(CLOSE_WAIT, connection.closed()).await;
            return match refusal(closed.ok()) {
                Some(reason) => Ended::Refused(reason),
                None => Ended::Failed("the share closed the connection during admission".into()),
            };
        }
    };
    let control = Arc::new(Control::new(connection, send));
    session.connected(Arc::clone(&control), you);
    receive(&mut recv, &control, session).await;
    control.connection.close(VarInt::from_u32(0), b"bye");
    Ended::Lost
}

/// Applies the share's messages until the stream ends.
async fn receive(recv: &mut BufReader<RecvStream>, control: &Control, session: &Session) {
    loop {
        match lines::read::<Envelope<ShareMessage>, _>(recv).await {
            Ok(Some(Envelope {
                version: _,
                message,
            })) => session.apply(control, message),
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(%error, "control stream ended");
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
