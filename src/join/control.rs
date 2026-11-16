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
    time::{self, Instant},
};

use crate::{
    Report,
    adapters::lines,
    core::text::OneLineReport,
    protocol::{
        comment::Draft,
        control::{AckOutcome, CommentRef, Envelope, JoinerMessage},
        presence::{Position, Whereabouts},
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
    lines: mpsc::Sender<Envelope<JoinerMessage>>,
    acks: Mutex<Acks>,
}

impl Control {
    /// Starts the writer on `writer`; it stops when the stream fails or
    /// the control is dropped.
    pub(crate) fn new(connection: Connection, writer: SendStream) -> Self {
        let (lines, queued) = mpsc::channel(WRITE_QUEUE);
        tokio::spawn(write_lines(writer, queued));
        Self {
            connection,
            lines,
            acks: Mutex::new(Acks::default()),
        }
    }

    fn acks(&self) -> MutexGuard<'_, Acks> {
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
        let (waiter, acked) = self.acks().wait(reference.clone());
        let _waiting = Waiting {
            control: self,
            reference: reference.clone(),
            waiter,
        };
        // One deadline for both: a writer that cannot keep up fails the
        // comment as a share that does not answer does.
        let deadline = Instant::now() + ACK_TIMEOUT;
        let line = Envelope::from(JoinerMessage::Comment { reference, draft });
        time::timeout_at(deadline, self.lines.send(line))
            .await
            .map_err(|_| report!("the comment could not be sent in time"))?
            .map_err(|_| report!("the connection to the share was lost"))?;
        time::timeout_at(deadline, acked)
            .await
            .map_err(|_| report!("the share did not acknowledge the comment"))?
            .map_err(|_| report!("the connection to the share was lost"))
    }

    /// Tells the share where this joiner is.
    pub(crate) async fn tell(&self, whereabouts: Whereabouts) -> Result<(), Report> {
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
        self.lines
            .send(Envelope::from(JoinerMessage::Cursor { position }))
            .await
            .map_err(|_| report!("the connection to the share was lost"))
    }

    pub(crate) fn acknowledge(&self, reference: &CommentRef, outcome: AckOutcome) {
        self.acks().acknowledge(reference, outcome);
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

/// One wait for an acknowledgement, among those for the same ref.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WaiterId(u64);

/// Who waits for which ref's acknowledgement. Several may wait for the
/// same ref (two tabs sending the same comment again): the acknowledgement
/// goes to each, and each gives up only its own wait.
#[derive(Debug, Default)]
struct Acks {
    next: u64,
    waiting: HashMap<CommentRef, Vec<(WaiterId, oneshot::Sender<AckOutcome>)>>,
}

impl Acks {
    fn wait(&mut self, reference: CommentRef) -> (WaiterId, oneshot::Receiver<AckOutcome>) {
        let id = WaiterId(self.next);
        self.next = self.next.wrapping_add(1);
        let (ack, acked) = oneshot::channel();
        self.waiting.entry(reference).or_default().push((id, ack));
        (id, acked)
    }

    /// Gives up wait `id` for `reference`, leaving any others.
    fn forget(&mut self, reference: &CommentRef, id: WaiterId) {
        if let Some(waiters) = self.waiting.get_mut(reference) {
            waiters.retain(|(waiter, _)| *waiter != id);
            if waiters.is_empty() {
                self.waiting.remove(reference);
            }
        }
    }

    fn acknowledge(&mut self, reference: &CommentRef, outcome: AckOutcome) {
        for (_, ack) in self.waiting.remove(reference).unwrap_or_default() {
            let _ = ack.send(outcome);
        }
    }

    fn clear(&mut self) {
        self.waiting.clear();
    }

    /// Waits in all.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.waiting.values().map(Vec::len).sum()
    }
}

/// A comment waiting for its acknowledgement; forgotten when dropped.
struct Waiting<'a> {
    control: &'a Control,
    reference: CommentRef,
    waiter: WaiterId,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.control.acks().forget(&self.reference, self.waiter);
    }
}

async fn write_lines(mut writer: SendStream, mut queued: mpsc::Receiver<Envelope<JoinerMessage>>) {
    while let Some(line) = queued.recv().await {
        if let Err(error) = lines::write(&mut writer, &line).await {
            tracing::debug!(error = %OneLineReport(&error), "control stream write failed");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::ids::CommentId,
        join::control::Acks,
        protocol::control::{AckOutcome, CommentRef},
    };

    #[test]
    fn waits_for_the_same_ref_are_each_their_own() {
        let mut acks = Acks::default();
        let reference = CommentRef::try_from("same-ref".to_owned()).unwrap();
        let (first, _first_acked) = acks.wait(reference.clone());
        let (_, mut second_acked) = acks.wait(reference.clone());
        // The first gives up (its tab went away): the second still waits.
        acks.forget(&reference, first);
        assert_eq!(acks.len(), 1);
        let id: CommentId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let outcome = AckOutcome::Queued { id };
        acks.acknowledge(&reference, outcome);
        assert_eq!(second_acked.try_recv(), Ok(outcome));
        assert_eq!(acks.len(), 0);
    }

    #[test]
    fn every_wait_for_a_ref_hears_its_acknowledgement() {
        let mut acks = Acks::default();
        let reference = CommentRef::try_from("same-ref".to_owned()).unwrap();
        let (_, mut first) = acks.wait(reference.clone());
        let (_, mut second) = acks.wait(reference.clone());
        acks.acknowledge(&reference, AckOutcome::QueueFull);
        assert_eq!(first.try_recv(), Ok(AckOutcome::QueueFull));
        assert_eq!(second.try_recv(), Ok(AckOutcome::QueueFull));
    }
}
