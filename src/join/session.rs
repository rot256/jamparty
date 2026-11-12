//! The joiner's view of its session: the live control connection (if any),
//! what the share last said, and the event stream for pages.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use iroh::endpoint::{Connection, SendStream};
use serde::Serialize;
use tokio::sync::{broadcast, oneshot, watch};

use crate::{
    BoxError,
    adapters::lines,
    core::{ids::CommentId, request::LocalOrigin},
    join::markdown,
    protocol::{
        comment::{Author, Draft},
        control::{AckOutcome, Envelope, JoinerMessage, RefusedReason, ShareMessage, StatusLine},
    },
};

const EVENT_CAPACITY: usize = 256;
/// Answers kept for pages that load later.
const MAX_ANSWERS: usize = 100;

/// What `/_jamjam/api/status` reports.
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

/// One event on `/_jamjam/api/events`.
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

/// The admitted control stream: writes comments, matches their
/// acknowledgements.
pub(crate) struct Control {
    pub(crate) connection: Connection,
    writer: tokio::sync::Mutex<SendStream>,
    acks: Mutex<HashMap<u64, oneshot::Sender<AckOutcome>>>,
    next_ref: AtomicU64,
}

impl Control {
    pub(crate) fn new(connection: Connection, writer: SendStream) -> Self {
        Self {
            connection,
            writer: tokio::sync::Mutex::new(writer),
            acks: Mutex::new(HashMap::new()),
            next_ref: AtomicU64::new(0),
        }
    }

    fn acks(&self) -> MutexGuard<'_, HashMap<u64, oneshot::Sender<AckOutcome>>> {
        self.acks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends a comment and waits for its acknowledgement; fails if the
    /// connection goes away first.
    pub(crate) async fn comment(&self, draft: Draft) -> Result<AckOutcome, BoxError> {
        let reference = self.next_ref.fetch_add(1, Ordering::Relaxed);
        let (ack, acked) = oneshot::channel();
        self.acks().insert(reference, ack);
        let line = Envelope::from(JoinerMessage::Comment { reference, draft });
        let written = lines::write(&mut *self.writer.lock().await, &line).await;
        if let Err(error) = written {
            self.acks().remove(&reference);
            return Err(error);
        }
        Ok(acked
            .await
            .map_err(|_| "the connection to the share was lost")?)
    }

    fn acknowledge(&self, reference: u64, outcome: AckOutcome) {
        if let Some(ack) = self.acks().remove(&reference) {
            let _ = ack.send(outcome);
        }
    }

    /// Fails every comment still waiting for its acknowledgement.
    pub(crate) fn abandon(&self) {
        self.acks().clear();
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

    /// The share admitted us on `control`.
    pub(crate) fn connected(&self, control: Arc<Control>, you: Author) {
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) = Some(control);
        self.view.send_modify(|view| {
            view.connected = true;
            view.you = Some(you);
        });
        self.emit(Event::Share { connected: true });
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
                control.acknowledge(reference, outcome);
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
