//! One joiner's connection: admission on the control stream, then request
//! streams and the control loop until either side leaves.

use std::{io::Cursor, sync::Arc, time::Duration};

use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream, VarInt};
use rootcause::prelude::report;
use tokio::{
    io::{AsyncReadExt, BufReader, Chain},
    sync::{
        OwnedSemaphorePermit,
        broadcast::{self, error::RecvError},
        mpsc,
    },
    task::JoinSet,
    time::{Instant, timeout, timeout_at},
};

use crate::{
    Report,
    adapters::{endpoint, lines},
    core::{ids::EndpointKey, name::JoinerName},
    protocol::{
        comment::Author,
        control::{Envelope, JoinerMessage, RefusedReason, ShareMessage},
        presence::{Support, Whereabouts},
    },
    share::{
        Shared,
        admission::{REFUSED_CODE, admit, reason_text},
        http::serve_stream,
        hub::{Outgoing, Presence},
    },
};

/// How long a refused joiner gets to read the refusal before the share
/// closes the connection.
const REFUSAL_GRACE: Duration = Duration::from_secs(2);

/// Close code for a joiner the share drops after admission.
const DROPPED_CODE: u32 = 2;

pub(crate) async fn handle(
    incoming: Incoming,
    unadmitted: OwnedSemaphorePermit,
    shared: Arc<Shared>,
) {
    // One deadline for the whole admission: handshake, control stream and
    // hello.
    let deadline = Instant::now() + shared.admission_timeout;
    let connection = match timeout_at(deadline, incoming).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => {
            tracing::debug!(%error, "incoming connection failed");
            return;
        }
        Err(_) => {
            tracing::debug!("incoming connection timed out");
            return;
        }
    };
    let key = EndpointKey::from(connection.remote_id());
    let admission = admission(&connection, deadline, &shared).await;
    drop(unadmitted);
    match admission {
        Ok(Admission::Admitted { send, recv, name }) => {
            let author = Author::new(name, key);
            // Subscribed before joining, so this joiner hears its own
            // presence too.
            let broadcasts = shared.hub.subscribe();
            let Some(seated) = shared.hub.join(author.clone()) else {
                tracing::info!(name = %author.name, fingerprint = %author.fingerprint, "joiner refused: the share is full");
                refuse(&connection, send, RefusedReason::Full).await;
                return;
            };
            endpoint::admitted(&connection);
            tracing::info!(name = %author.name, fingerprint = %author.fingerprint, "joiner admitted");
            let ended = joined(
                &connection,
                send,
                recv,
                broadcasts,
                seated,
                &author,
                &shared,
            )
            .await;
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

/// The rest of the control stream after the hello: what arrived with it,
/// then the stream.
type ControlReader = BufReader<Chain<Cursor<Vec<u8>>, RecvStream>>;

enum Admission {
    Admitted {
        send: SendStream,
        recv: ControlReader,
        name: JoinerName,
    },
    Refused {
        send: SendStream,
        reason: RefusedReason,
    },
}

/// Waits for the control stream and its hello until `deadline`; no other
/// stream is accepted before this returns.
async fn admission(
    connection: &Connection,
    deadline: Instant,
    shared: &Shared,
) -> Result<Admission, Report> {
    let (send, mut recv) = timeout_at(deadline, connection.accept_bi()).await??;
    // The hello is read without a buffered reader, into a fixed buffer
    // that is wiped (see `lines::read_hello`); what follows it goes on
    // through a buffered reader.
    let (first, leftover) = match timeout_at(deadline, lines::read_hello(&mut recv)).await {
        Ok(Ok((first, leftover))) => (Ok(first), leftover),
        Ok(Err(error)) => (Err(error), Vec::new()),
        Err(_) => (
            Err(report!("no hello before the admission deadline")),
            Vec::new(),
        ),
    };
    if let Err(error) = &first {
        tracing::debug!(%error, "no usable hello");
    }
    Ok(match admit(first, &shared.token.secret) {
        Ok(name) => Admission::Admitted {
            send,
            recv: BufReader::new(Cursor::new(leftover).chain(recv)),
            name,
        },
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
    mut recv: ControlReader,
    mut broadcasts: broadcast::Receiver<Outgoing>,
    seated: (Presence, mpsc::Receiver<ShareMessage>),
    author: &Author,
    shared: &Arc<Shared>,
) -> &'static str {
    // Presence is left when this returns.
    let (presence, mut cursors) = seated;
    let presence = &presence;
    let mut greeting = vec![ShareMessage::Admitted {
        you: author.clone(),
        undelivered: shared.hub.undelivered_by(author),
        presence: Support::Supported,
    }];
    if let Some(reload) = shared.site.reload() {
        greeting.push(ShareMessage::Reload {
            revision: reload.current(),
        });
    }
    greeting.extend(shared.hub.catch_up(author.key));
    for message in greeting {
        if lines::write(&mut send, &Envelope::from(message))
            .await
            .is_err()
        {
            return "control stream closed";
        }
    }

    let mut requests = JoinSet::new();
    let accept = async {
        while let Ok((send, recv)) = connection.accept_bi().await {
            requests.spawn(serve_stream(send, recv, shared.site.clone()));
            // Reap finished requests so the set does not grow.
            while requests.try_join_next().is_some() {}
        }
        "connection closed"
    };

    // Reading and writing run side by side so neither cancels the other
    // mid-line; acknowledgements go to the writer through a channel.
    let (acks, mut ack_rx) = mpsc::channel(16);
    let reader = async {
        // Renamed by presence updates; comments are signed with the latest.
        let mut author = author.clone();
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
                    outcome: shared.hub.comment(&author, reference.clone(), draft),
                    reference,
                },
                JoinerMessage::Presence { name, url, cursor } => {
                    match presence.update(Whereabouts { name, url, cursor }) {
                        Ok(renamed) => author = renamed,
                        Err(ignored) if ignored.disconnects() => return "presence flood",
                        Err(ignored) => {
                            tracing::debug!(fingerprint = %author.fingerprint, ?ignored, "presence update ignored");
                        }
                    }
                    continue;
                }
                JoinerMessage::Cursor { position } => {
                    // Tests read this to tell what reached the share.
                    tracing::debug!(name = %author.name, fingerprint = %author.fingerprint, "cursor message");
                    if let Err(ignored) = presence.cursor(position) {
                        tracing::debug!(fingerprint = %author.fingerprint, ?ignored, "cursor ignored");
                    }
                    continue;
                }
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
                    // Answers and deliveries for someone else are not this
                    // joiner's.
                    Ok(Outgoing { audience, .. }) if !audience.includes(author.key) => continue,
                    Ok(Outgoing { message, .. }) => presence.redact(message),
                    Err(RecvError::Lagged(_)) => return "fell behind",
                    Err(RecvError::Closed) => return "share stopped",
                },
                cursor = cursors.recv() => match cursor {
                    Some(cursor) => cursor,
                    None => return "share stopped",
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
