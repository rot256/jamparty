//! The joiner's view of its session: the live control connection (if any),
//! what the share last said, and the event stream for pages.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use iroh::endpoint::{Connection, SendStream};
use rootcause::prelude::report;
use serde::Serialize;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time,
};

use crate::{
    Report,
    adapters::lines,
    core::{ids::CommentId, request::LocalOrigin},
    join::markdown,
    protocol::{
        comment::{Author, Draft},
        control::{
            AckOutcome, CommentRef, Envelope, JoinerMessage, RefusedReason, ShareMessage,
            StatusLine,
        },
    },
};

const EVENT_CAPACITY: usize = 256;
/// Answers kept for pages that load later.
const MAX_ANSWERS: usize = 100;

/// What `/_jampot/api/status` reports.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct View {
    /// Connected to the share and admitted.
    pub(crate) connected: bool,
    /// Set when the share refused this joiner; it does not retry then.
    pub(crate) refused: Option<RefusedReason>,
    /// How this joiner's comments are signed.
    pub(crate) you: Option<Author>,
    /// Who is connected, this joiner included.
    pub(crate) joiners: Vec<Author>,
    /// The live-reload revision (shared directories only).
    pub(crate) revision: Option<u64>,
    pub(crate) status: Option<StatusLine>,
    /// This joiner's comments the share holds undelivered.
    pub(crate) pending: usize,
    /// Their ids.
    pub(crate) undelivered: Vec<CommentId>,
    /// Recent answers, oldest first.
    pub(crate) answers: Vec<Answer>,
}

/// An answer as pages show it: rendered Markdown, and where it belongs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Answer {
    pub(crate) id: CommentId,
    pub(crate) html: String,
    /// The selector of the comment it answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) anchor: Option<String>,
    /// The page that comment was made on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page: Option<String>,
}

/// One event on `/_jampot/api/events`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Event {
    /// The connection to the share came up or went down.
    Share {
        connected: bool,
    },
    Reload {
        revision: u64,
    },
    Presence {
        joiners: Vec<Author>,
    },
    Status {
        line: Option<StatusLine>,
    },
    Delivered {
        ids: Vec<CommentId>,
    },
    Answer(Answer),
}

/// How long to wait for the share to acknowledge a comment.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Lines waiting for the control stream's writer.
const WRITE_QUEUE: usize = 16;

/// The admitted control stream: one task writes comments, in whole lines;
/// acknowledgements are matched by the page's ref.
pub(crate) struct Control {
    pub(crate) connection: Connection,
    lines: mpsc::Sender<Envelope<JoinerMessage>>,
    acks: Mutex<HashMap<CommentRef, oneshot::Sender<AckOutcome>>>,
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

    fn acknowledge(&self, reference: &CommentRef, outcome: AckOutcome) {
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

pub(crate) struct Session {
    pub(crate) origin: LocalOrigin,
    view: watch::Sender<View>,
    control: Mutex<Option<Arc<Control>>>,
    events: broadcast::Sender<Event>,
    /// Ids of this joiner's comments the share has not delivered yet.
    mine: Mutex<HashSet<CommentId>>,
}

impl Session {
    pub(crate) fn new(origin: LocalOrigin) -> Self {
        Self {
            origin,
            view: watch::channel(View::default()).0,
            control: Mutex::new(None),
            events: broadcast::channel(EVENT_CAPACITY).0,
            mine: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn view(&self) -> View {
        self.view.borrow().clone()
    }

    pub(crate) fn watch(&self) -> watch::Receiver<View> {
        self.view.subscribe()
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, event: Event) {
        // No receivers means no open pages.
        let _ = self.events.send(event);
    }

    pub(crate) fn control(&self) -> Option<Arc<Control>> {
        self.control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The share admitted us on `control`; of our comments it still holds
    /// only `undelivered`, the others were delivered while we were away.
    pub(crate) fn connected(&self, control: Arc<Control>, you: Author, undelivered: &[CommentId]) {
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) = Some(control);
        let mut delivered = Vec::new();
        self.update_mine(|mine| {
            mine.retain(|id| {
                let held = undelivered.contains(id);
                if !held {
                    delivered.push(*id);
                }
                held
            });
        });
        self.view.send_modify(|view| {
            view.connected = true;
            view.you = Some(you);
        });
        self.emit(Event::Share { connected: true });
        if !delivered.is_empty() {
            self.emit(Event::Delivered { ids: delivered });
        }
    }

    /// The connection is gone (or never came up).
    pub(crate) fn disconnected(&self, refused: Option<RefusedReason>) {
        let control = self
            .control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(control) = control {
            control.abandon();
        }
        let was_connected = self.view.borrow().connected;
        self.view.send_modify(|view| {
            view.connected = false;
            view.refused = refused;
            view.joiners.clear();
        });
        if was_connected {
            self.emit(Event::Share { connected: false });
        }
    }

    /// One of our comments, sent again, was delivered already as `id`;
    /// returns how many are pending.
    pub(crate) fn already_delivered(&self, id: CommentId) -> usize {
        self.emit(Event::Delivered { ids: vec![id] });
        self.update_mine(|mine| {
            mine.remove(&id);
        })
    }

    /// One of our comments was queued under `id`; returns how many are
    /// pending.
    pub(crate) fn queued(&self, id: CommentId) -> usize {
        self.update_mine(|mine| {
            mine.insert(id);
        })
    }

    /// Changes the set of our undelivered comments; returns its size.
    fn update_mine(&self, change: impl FnOnce(&mut HashSet<CommentId>)) -> usize {
        let undelivered: Vec<CommentId> = {
            let mut mine = self.mine.lock().unwrap_or_else(PoisonError::into_inner);
            change(&mut mine);
            mine.iter().copied().collect()
        };
        let pending = undelivered.len();
        self.view.send_modify(|view| {
            view.pending = pending;
            view.undelivered = undelivered;
        });
        pending
    }

    /// Applies a message from the share after admission.
    pub(crate) fn apply(&self, control: &Control, message: ShareMessage) {
        match message {
            ShareMessage::CommentAck { reference, outcome } => {
                control.acknowledge(&reference, outcome);
            }
            ShareMessage::Reload { revision } => {
                self.view.send_modify(|view| view.revision = Some(revision));
                self.emit(Event::Reload { revision });
            }
            ShareMessage::Presence { joiners } => {
                self.view
                    .send_modify(|view| view.joiners.clone_from(&joiners));
                self.emit(Event::Presence { joiners });
            }
            ShareMessage::Status { line } => {
                self.view.send_modify(|view| view.status.clone_from(&line));
                self.emit(Event::Status { line });
            }
            ShareMessage::Delivered { ids } => {
                self.update_mine(|mine| {
                    for id in &ids {
                        mine.remove(id);
                    }
                });
                self.emit(Event::Delivered { ids });
            }
            ShareMessage::Answer {
                id,
                message,
                anchor,
            } => {
                if self
                    .view
                    .borrow()
                    .answers
                    .iter()
                    .any(|answer| answer.id == id)
                {
                    return;
                }
                let (anchor, page) = match anchor {
                    Some(anchor) => (Some(anchor.selector), Some(anchor.page)),
                    None => (None, None),
                };
                let answer = Answer {
                    id,
                    html: markdown::render(&String::from(message)),
                    anchor,
                    page,
                };
                self.view.send_modify(|view| {
                    if view.answers.len() == MAX_ANSWERS {
                        view.answers.remove(0);
                    }
                    view.answers.push(answer.clone());
                });
                self.emit(Event::Answer(answer));
            }
            ShareMessage::Admitted { .. } | ShareMessage::Refused { .. } => {
                tracing::debug!("unexpected admission message after admission");
            }
        }
    }
}
