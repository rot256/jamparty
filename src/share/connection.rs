//! One joiner's connection: admission on the control stream, then request
//! streams and the control loop until either side leaves.

use std::{
    fmt,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream, VarInt};
use rootcause::prelude::report;
use tokio::{
    io::{AsyncReadExt, BufReader, Chain},
    sync::{OwnedSemaphorePermit, broadcast},
    task::JoinSet,
    time::{Instant, timeout, timeout_at},
};

use crate::{
    Report,
    adapters::{endpoint, lines},
    core::{
        ids::EndpointKey,
        name::JoinerName,
        text::Untrusted,
        version::{JampartyVersion, PROTOCOL},
    },
    protocol::{
        comment::Author,
        control::{Envelope, Greeting, JoinerMessage, REFUSED_CODE, RefusedReason, ShareMessage},
        decode_line,
        presence::{Support, Whereabouts},
    },
    share::{
        Shared,
        admission::admit,
        http::serve_stream,
        hub::{Inbox, Outbox, Outgoing, Presence, Stopped},
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
            tracing::debug!(error = %Untrusted(&error), "incoming connection failed");
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
            connection.close(VarInt::from_u32(DROPPED_CODE), ended.as_str().as_bytes());
        }
        Ok(Admission::Refused { send, reason }) => {
            tracing::info!(fingerprint = %key.fingerprint(), reason = reason.as_str(), "joiner refused");
            refuse(&connection, send, reason).await;
        }
        Err(error) => tracing::debug!(error = %Untrusted(&error), "joiner left before admission"),
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
    let read = lines::read_hello_with(&mut recv, |line| {
        // Leniently first: a joiner of another protocol is told so, with
        // this share's version.
        let greeting = decode_line::<Greeting>(line).ok();
        Ok((greeting, decode_line::<Envelope<JoinerMessage>>(line)))
    });
    let (first, leftover) = match timeout_at(deadline, read).await {
        Ok(Ok((Some((Some(greeting), _)), _))) if greeting.version != PROTOCOL => {
            shared.versions.incompatible(&greeting);
            return Ok(Admission::Refused {
                send,
                reason: RefusedReason::Incompatible,
            });
        }
        Ok(Ok((first, leftover))) => {
            let first = first.map(|(_, hello)| hello).transpose();
            (first.map_err(Report::from), leftover)
        }
        Ok(Err(error)) => (Err(error), Vec::new()),
        Err(_) => (
            Err(report!("no hello before the admission deadline")),
            Vec::new(),
        ),
    };
    if let Err(error) = &first {
        tracing::debug!(error = %Untrusted(&error), "no usable hello");
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
    // Only a joiner of another protocol hears which jamparty this is, to
    // install it; no other peer needs to know before admission.
    let jamparty = match reason {
        RefusedReason::Incompatible => Some(JampartyVersion::current()),
        RefusedReason::WrongSecret | RefusedReason::BadHello | RefusedReason::Full => None,
    };
    let refused = Envelope::from(ShareMessage::Refused { reason, jamparty });
    if lines::write(&mut send, &refused).await.is_ok() && send.finish().is_ok() {
        // Let the joiner read it and hang up; the close below carries the
        // reason too in case it does not.
        let _ = timeout(REFUSAL_GRACE, connection.closed()).await;
    }
    connection.close(VarInt::from_u32(REFUSED_CODE), reason.as_str().as_bytes());
}

/// Why an admitted joiner's connection ended; sent as the close reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Left {
    ControlStreamClosed,
    ConnectionClosed,
    InvalidControlLine,
    CommentFlood,
    PresenceFlood,
    SecondHello,
    FellBehind,
    ShareStopped,
}

impl Left {
    fn as_str(self) -> &'static str {
        match self {
            Self::ControlStreamClosed => "control stream closed",
            Self::ConnectionClosed => "connection closed",
            Self::InvalidControlLine => "invalid control line",
            Self::CommentFlood => "comment flood",
            Self::PresenceFlood => "presence flood",
            Self::SecondHello => "second hello",
            Self::FellBehind => "fell behind",
            Self::ShareStopped => "share stopped",
        }
    }
}

impl From<Stopped> for Left {
    fn from(stopped: Stopped) -> Self {
        match stopped {
            Stopped::FellBehind => Self::FellBehind,
            Stopped::ShareStopped => Self::ShareStopped,
        }
    }
}

impl fmt::Display for Left {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runs an admitted joiner until it leaves; returns why it ended.
async fn joined(
    connection: &Connection,
    mut send: SendStream,
    mut recv: ControlReader,
    broadcasts: broadcast::Receiver<Outgoing>,
    seated: (Presence, Inbox),
    author: &Author,
    shared: &Arc<Shared>,
) -> Left {
    // Presence is left when this returns.
    let (presence, inbox) = seated;
    let presence = &presence;
    let mut greeting = vec![ShareMessage::Admitted {
        you: author.clone(),
        undelivered: shared.hub.undelivered_by(author),
        presence: Support::Supported,
        site: shared.site.info(),
        jamparty: JampartyVersion::current(),
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
            return Left::ControlStreamClosed;
        }
    }

    let mut requests = JoinSet::new();
    let accept = async {
        while let Ok((send, recv)) = connection.accept_bi().await {
            requests.spawn(serve_stream(send, recv, shared.site.clone()));
            // Reap finished requests so the set does not grow.
            while requests.try_join_next().is_some() {}
        }
        Left::ConnectionClosed
    };

    // Reading and writing run side by side so neither cancels the other
    // mid-line. The writer sends acknowledgements and broadcasts in the
    // hub's order (see [`Outbox`]).
    let reader = async {
        // Renamed by presence updates; comments are signed with the latest.
        let mut author = author.clone();
        loop {
            let message = match lines::read::<Envelope<JoinerMessage>, _>(&mut recv).await {
                Ok(Some(Envelope {
                    version: _,
                    message,
                })) => message,
                Ok(None) => return Left::ControlStreamClosed,
                // A read cut short by the connection closing is the joiner
                // leaving, not a bad line.
                Err(_) if connection.close_reason().is_some() => return Left::ConnectionClosed,
                Err(error) => {
                    tracing::debug!(error = %Untrusted(&error), "invalid control line");
                    return Left::InvalidControlLine;
                }
            };
            match message {
                JoinerMessage::Comment { reference, draft } => {
                    match presence.comment(&author, reference, draft) {
                        Ok(_) => {}
                        Err(ignored) if ignored.disconnects() => return Left::CommentFlood,
                        Err(ignored) => {
                            tracing::debug!(fingerprint = %author.fingerprint, ?ignored, "comment ignored");
                        }
                    }
                }
                JoinerMessage::Presence { name, url, cursor } => {
                    match presence.update(Whereabouts { name, url, cursor }) {
                        Ok(renamed) => author = renamed,
                        Err(ignored) if ignored.disconnects() => return Left::PresenceFlood,
                        Err(ignored) => {
                            tracing::debug!(fingerprint = %author.fingerprint, ?ignored, "presence update ignored");
                        }
                    }
                }
                JoinerMessage::Cursor { position } => {
                    if let Err(ignored) = presence.cursor(position) {
                        tracing::debug!(fingerprint = %author.fingerprint, ?ignored, "cursor ignored");
                    }
                }
                JoinerMessage::Hello(_) => return Left::SecondHello,
            }
        }
    };
    let mut outbox = Outbox::new(presence, author.key, inbox, broadcasts);
    let writer = async {
        loop {
            let message = match outbox.next().await {
                Ok(message) => message,
                Err(stopped) => return Left::from(stopped),
            };
            if lines::write(&mut send, &Envelope::from(message))
                .await
                .is_err()
            {
                return Left::ControlStreamClosed;
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

/// Says once, on the share's log, that a joiner of another protocol came;
/// later ones only at debug level, so joiners cannot flood the log.
#[derive(Default)]
pub(crate) struct VersionLog {
    said: AtomicBool,
}

impl VersionLog {
    fn incompatible(&self, greeting: &Greeting) {
        let version = greeting.jamparty().map_or_else(
            || "another jamparty".to_owned(),
            |jamparty| format!("jamparty {jamparty}"),
        );
        let protocol = greeting.version;
        if self.said.swap(true, Ordering::Relaxed) {
            tracing::debug!(%version, %protocol, "joiner refused: another protocol");
        } else {
            tracing::warn!(
                "a joiner runs {version} (protocol {protocol}); this share runs jamparty {} (protocol {PROTOCOL}) and refused it",
                JampartyVersion::current()
            );
        }
    }
}
