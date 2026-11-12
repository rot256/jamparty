//! One joiner's connection: admission on the control stream, then request
//! streams and the control loop until either side leaves.

use std::{sync::Arc, time::Duration};

use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream, VarInt};
use tokio::{
    io::BufReader,
    sync::{broadcast::error::RecvError, mpsc},
    task::JoinSet,
    time::timeout,
};

use crate::{
    BoxError,
    adapters::lines,
    core::{ids::EndpointKey, name::JoinerName},
    protocol::{
        comment::Author,
        control::{Envelope, JoinerMessage, RefusedReason, ShareMessage},
    },
    share::{
        Shared,
        admission::{REFUSED_CODE, admit, reason_text},
        http::serve_stream,
    },
};

/// How long a refused joiner gets to read the refusal before the share
/// closes the connection.
const REFUSAL_GRACE: Duration = Duration::from_secs(2);

/// Close code for a joiner the share drops after admission.
const DROPPED_CODE: u32 = 2;

pub(crate) async fn handle(incoming: Incoming, shared: Arc<Shared>) {
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::debug!(%error, "incoming connection failed");
            return;
        }
    };
    let key = EndpointKey::from(connection.remote_id());
    match admission(&connection, &shared).await {
        Ok(Admission::Admitted { send, recv, name }) => {
            let author = Author {
                name,
                fingerprint: key.fingerprint(),
            };
            tracing::info!(name = %author.name, fingerprint = %author.fingerprint, "joiner admitted");
            let ended = joined(&connection, send, recv, key, &author, &shared).await;
            tracing::info!(name = %author.name, fingerprint = %author.fingerprint, reason = %ended, "joiner left");
            connection.close(VarInt::from_u32(DROPPED_CODE), ended.as_bytes());
        }
        Ok(Admission::Refused { send, reason }) => {
            tracing::info!(fingerprint = %key.fingerprint(), reason = reason_text(reason), "joiner refused");
            refuse(&connection, send, reason).await;
        }
        Err(error) => tracing::debug!(%error, "joiner left before admission"),
    }
}

enum Admission {
    Admitted {
        send: SendStream,
        recv: BufReader<RecvStream>,
        name: JoinerName,
    },
    Refused {
        send: SendStream,
        reason: RefusedReason,
    },
}

/// Waits for the control stream and its hello; no other stream is accepted
/// before this returns.
async fn admission(connection: &Connection, shared: &Shared) -> Result<Admission, BoxError> {
    let (send, recv) = timeout(shared.hello_timeout, connection.accept_bi()).await??;
    let mut recv = BufReader::new(recv);
    let first = timeout(shared.hello_timeout, lines::read(&mut recv))
        .await
        .map_err(|_| ())
        .and_then(|line| line.map_err(|_| ()));
    Ok(match admit(first, &shared.token.secret) {
        Ok(name) => Admission::Admitted { send, recv, name },
        Err(reason) => Admission::Refused { send, reason },
    })
}

async fn refuse(connection: &Connection, mut send: SendStream, reason: RefusedReason) {
    let refused = Envelope::from(ShareMessage::Refused { reason });
    if lines::write(&mut send, &refused).await.is_ok() && send.finish().is_ok() {
        // Let the joiner read it and hang up; the close below carries the
        // reason too in case it does not.
        let _ = timeout(REFUSAL_GRACE, connection.closed()).await;
    }
    connection.close(
        VarInt::from_u32(REFUSED_CODE),
        reason_text(reason).as_bytes(),
    );
}

/// Runs an admitted joiner until it leaves; returns why it ended.
async fn joined(
    connection: &Connection,
    mut send: SendStream,
    mut recv: BufReader<RecvStream>,
    key: EndpointKey,
    author: &Author,
    shared: &Arc<Shared>,
) -> &'static str {
    let mut broadcasts = shared.hub.subscribe();
    let mut greeting = vec![ShareMessage::Admitted {
        you: author.clone(),
        undelivered: shared.hub.undelivered_by(author),
    }];
    if let Some(reload) = shared.site.reload() {
        greeting.push(ShareMessage::Reload {
            revision: reload.current(),
        });
    }
    greeting.extend(shared.hub.catch_up());
    for message in greeting {
        if lines::write(&mut send, &Envelope::from(message))
            .await
            .is_err()
        {
            return "control stream closed";
        }
    }
    let _presence = shared.hub.join(author.clone());

    let mut requests = JoinSet::new();
    let accept = async {
        while let Ok((send, recv)) = connection.accept_bi().await {
            requests.spawn(serve_stream(send, recv, shared.site.clone()));
        }
        "connection closed"
    };

    // Reading and writing run side by side so neither cancels the other
    // mid-line; acknowledgements go to the writer through a channel.
    let (acks, mut ack_rx) = mpsc::channel(16);
    let reader = async {
        loop {
            let message = match lines::read::<Envelope<JoinerMessage>, _>(&mut recv).await {
                Ok(Some(Envelope {
                    version: _,
                    message,
                })) => message,
                Ok(None) => return "control stream closed",
                Err(error) => {
                    tracing::debug!(%error, "invalid control line");
                    return "invalid control line";
                }
            };
            let ack = match message {
                JoinerMessage::Comment { reference, draft } => ShareMessage::CommentAck {
                    reference,
                    outcome: shared.hub.comment(key, author, draft),
                },
                JoinerMessage::Hello { .. } => return "second hello",
            };
            if acks.send(ack).await.is_err() {
                return "control stream closed";
            }
        }
    };
    let writer = async {
        loop {
            let message = tokio::select! {
                ack = ack_rx.recv() => match ack {
                    Some(ack) => ack,
                    None => return "control stream closed",
                },
                message = broadcasts.recv() => match message {
                    Ok(message) => message,
                    Err(RecvError::Lagged(_)) => return "fell behind",
                    Err(RecvError::Closed) => return "share stopped",
                },
            };
            if lines::write(&mut send, &Envelope::from(message))
                .await
                .is_err()
            {
                return "control stream closed";
            }
        }
    };
    tokio::select! {
        ended = accept => ended,
        ended = reader => ended,
        ended = writer => ended,
    }
}

/// Forwards live-reload revisions to every joiner.
pub(crate) async fn forward_reloads(shared: Arc<Shared>) {
    let Some(reload) = shared.site.reload() else {
        return;
    };
    let mut revisions = reload.subscribe();
    while revisions.changed().await.is_ok() {
        let revision = *revisions.borrow_and_update();
        shared.hub.broadcast(ShareMessage::Reload { revision });
    }
}
