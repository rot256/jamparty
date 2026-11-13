//! The admitted control stream to the share: comments out (matched with
//! their acknowledgements by the page's ref), presence and cursors.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use iroh::endpoint::{Connection, SendStream};
use rootcause::prelude::report;
use tokio::{
    sync::{mpsc, oneshot},
    time,
};

use crate::{
    Report,
    adapters::lines,
    protocol::{
        comment::Draft,
        control::{AckOutcome, CommentRef, Envelope, JoinerMessage},
        presence::{Position, Support, Whereabouts},
    },
};

/// How long to wait for the share to acknowledge a comment.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Lines waiting for the control stream's writer.
const WRITE_QUEUE: usize = 16;

/// The admitted control stream: one task writes comments, in whole lines;
/// acknowledgements are matched by the page's ref.
pub(crate) struct Control {
    pub(crate) connection: Connection,
    /// Whether the share takes presence (and cursor) messages.
    presence: Support,
    lines: mpsc::Sender<Envelope<JoinerMessage>>,
    acks: Mutex<HashMap<CommentRef, oneshot::Sender<AckOutcome>>>,
}

impl Control {
    /// Starts the writer on `writer`; it stops when the stream fails or
    /// the control is dropped.
    pub(crate) fn new(connection: Connection, writer: SendStream, presence: Support) -> Self {
        let (lines, queued) = mpsc::channel(WRITE_QUEUE);
        tokio::spawn(write_lines(writer, queued));
        Self {
            connection,
            presence,
            lines,
            acks: Mutex::new(HashMap::new()),
        }
    }

    fn acks(&self) -> MutexGuard<'_, HashMap<CommentRef, oneshot::Sender<AckOutcome>>> {
        self.acks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends a comment and waits for its acknowledgement; fails if the
    /// connection goes away or the share does not answer in time. Safe to
    /// cancel: the line is written whole or not at all, and the wait is
    /// forgotten.
    pub(crate) async fn comment(
        &self,
        reference: CommentRef,
        draft: Draft,
    ) -> Result<AckOutcome, Report> {
        let (ack, acked) = oneshot::channel();
        self.acks().insert(reference.clone(), ack);
        let _waiting = Waiting {
            control: self,
            reference: reference.clone(),
        };
        let line = Envelope::from(JoinerMessage::Comment { reference, draft });
        self.lines
            .send(line)
            .await
            .map_err(|_| report!("the connection to the share was lost"))?;
        time::timeout(ACK_TIMEOUT, acked)
            .await
            .map_err(|_| report!("the share did not acknowledge the comment"))?
            .map_err(|_| report!("the connection to the share was lost"))
    }

    /// Tells the share where this joiner is, if it takes that.
    pub(crate) async fn tell(&self, whereabouts: Whereabouts) -> Result<(), Report> {
        match self.presence {
            Support::Supported => {}
            Support::Unsupported => return Ok(()),
        }
        let Whereabouts { name, url, cursor } = whereabouts;
        self.lines
            .send(Envelope::from(JoinerMessage::Presence {
                name,
                url,
                cursor,
            }))
            .await
            .map_err(|_| report!("the connection to the share was lost"))
    }

    /// Sends the joiner's pointer (none: it left the page).
    pub(crate) async fn cursor(&self, position: Option<Position>) -> Result<(), Report> {
        match self.presence {
            Support::Supported => {}
            Support::Unsupported => return Ok(()),
        }
        self.lines
            .send(Envelope::from(JoinerMessage::Cursor { position }))
            .await
            .map_err(|_| report!("the connection to the share was lost"))
    }

    pub(crate) fn acknowledge(&self, reference: &CommentRef, outcome: AckOutcome) {
        if let Some(ack) = self.acks().remove(reference) {
            let _ = ack.send(outcome);
        }
    }

    /// Fails every comment still waiting for its acknowledgement.
    pub(crate) fn abandon(&self) {
        self.acks().clear();
    }

    /// Comments waiting for their acknowledgement.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn waiting(&self) -> usize {
        self.acks().len()
    }
}

/// A comment waiting for its acknowledgement; forgotten when dropped.
struct Waiting<'a> {
    control: &'a Control,
    reference: CommentRef,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.control.acks().remove(&self.reference);
    }
}

async fn write_lines(mut writer: SendStream, mut queued: mpsc::Receiver<Envelope<JoinerMessage>>) {
    while let Some(line) = queued.recv().await {
        if let Err(error) = lines::write(&mut writer, &line).await {
            tracing::debug!(%error, "control stream write failed");
            return;
        }
    }
}
