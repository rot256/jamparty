//! What the joiner knows, as its pages see it, and each change as the
//! messages that tell pages of it. Pure: no I/O, no locks, no clock.
//!
//! Every message but `answer` and `cursor` replaces one slice of the
//! [`Snapshot`] whole, so a page that applies a snapshot and then the
//! messages after it, in order, holds the joiner's state.

use std::collections::{BTreeSet, VecDeque};

use serde::Serialize;

use crate::{
    adapters::path::PathKind,
    core::{ids::CommentId, settings::Switch},
    join::{
        api::ToPage,
        markdown::{self, AnswerHtml},
        preferences::PageSettings,
    },
    protocol::{
        answer::{AnswerAnchor, PlaceAnchor},
        comment::{Author, CommentPage, CommentSelector},
        control::{
            AckOutcome, Choices, MAX_QUEUED_PER_KEY, RefusedReason, Revision, ShareMessage,
            StatusLine,
        },
        line_size,
        presence::Person,
    },
};

/// Answers kept for pages that load later.
pub(crate) const MAX_ANSWERS: usize = 100;
/// The encoded size of answers kept for pages that load later, including
/// their text, locations and choices. Older answers go first.
pub(crate) const MAX_ANSWERS_BYTES: usize = 256 * 1024;

/// Whether answers of `count` and `bytes` have room for one more
/// of `incoming` bytes.
fn room(count: usize, bytes: usize, incoming: usize) -> bool {
    count < MAX_ANSWERS && bytes.saturating_add(incoming) <= MAX_ANSWERS_BYTES
}

/// The joiner's link to the share.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Link {
    /// Connected to the share and admitted.
    pub(crate) connected: bool,
    /// Set when the share refused this joiner; it does not retry then.
    pub(crate) refused: Option<RefusedReason>,
    /// How this joiner's comments are signed.
    pub(crate) you: Option<Author>,
    /// The share's status line.
    pub(crate) status: Option<StatusLine>,
    /// Locally observed selected transport, never supplied by the share.
    pub(crate) path: Option<PathKind>,
}

/// An answer as pages show it: rendered Markdown, and where it belongs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Answer {
    pub(crate) id: CommentId,
    #[cfg_attr(test, ts(type = "string"))]
    pub(crate) html: AnswerHtml,
    /// The ID of the comment it answers.
    #[serde(rename = "replyTo", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(crate) reply_to: Option<CommentId>,
    /// The selector of the comment it answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(crate) anchor: Option<CommentSelector>,
    /// The page that comment was made on.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(crate) page: Option<CommentPage>,
    /// Addressed to this joiner alone (`jamparty send --to`, or an answer to
    /// its comment); the page says so.
    #[serde(rename = "toYou")]
    pub(crate) to_you: bool,
    pub(crate) choices: Choices,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(crate) place: Option<PlaceAnchor>,
    /// Arrived after the share's admission replay.
    pub(crate) live: bool,
}

impl Answer {
    fn encoded_size(&self) -> usize {
        line_size(self).unwrap_or(MAX_ANSWERS_BYTES)
    }
}

/// Everything a page is told when it connects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Snapshot {
    pub(crate) link: Link,
    /// Who is here, this joiner included, with their colour, page and
    /// cursor switch.
    pub(crate) people: Vec<Person>,
    /// This joiner's comments the share holds undelivered; none until the
    /// share first said.
    pub(crate) mine: Option<Vec<CommentId>>,
    pub(crate) settings: PageSettings,
    /// Recent answers, oldest first: at most [`MAX_ANSWERS`], encoding to at most
    /// [`MAX_ANSWERS_BYTES`] together.
    pub(crate) answers: Vec<Answer>,
    /// The live-reload revision (shared directories only).
    pub(crate) revision: Option<Revision>,
}

#[cfg(test)]
impl Snapshot {
    /// How many of this joiner's comments the share holds (0 while
    /// unknown).
    #[must_use]
    pub(crate) fn pending(&self) -> usize {
        self.mine.as_ref().map_or(0, Vec::len)
    }
}

/// This joiner's comments the share holds undelivered. The share sends
/// acknowledgements and deliveries in the order it made them, a comment's
/// acknowledgement before its delivery, and the joiner applies them in
/// that order: its view never holds more than the share's queue for its
/// key, [`MAX_QUEUED_PER_KEY`].
#[derive(Debug, Default)]
pub(crate) struct Mine {
    /// None until the share first admitted this joiner.
    undelivered: Option<BTreeSet<CommentId>>,
}

/// The share claims more undelivered comments of this joiner than it can
/// hold: it is not one to trust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TooMany;

impl Mine {
    #[must_use]
    pub(crate) fn view(&self) -> Option<Vec<CommentId>> {
        self.undelivered
            .as_ref()
            .map(|undelivered| undelivered.iter().copied().collect())
    }

    /// The share admitted this joiner holding `undelivered` of its
    /// comments; any others were delivered meanwhile.
    fn admitted(&mut self, undelivered: &[CommentId]) -> Result<(), TooMany> {
        let undelivered: BTreeSet<CommentId> = undelivered.iter().copied().collect();
        if undelivered.len() > MAX_QUEUED_PER_KEY {
            return Err(TooMany);
        }
        self.undelivered = Some(undelivered);
        Ok(())
    }

    fn queued(&mut self, id: CommentId) -> Result<(), TooMany> {
        let undelivered = self.undelivered.get_or_insert_default();
        if undelivered.len() >= MAX_QUEUED_PER_KEY && !undelivered.contains(&id) {
            return Err(TooMany);
        }
        undelivered.insert(id);
        Ok(())
    }

    fn delivered(&mut self, ids: &[CommentId]) {
        if let Some(undelivered) = &mut self.undelivered {
            for id in ids {
                undelivered.remove(id);
            }
        }
    }
}

/// The joiner's state, as pages see it.
#[derive(Debug)]
pub(crate) struct State {
    link: Link,
    people: Vec<Person>,
    mine: Mine,
    settings: PageSettings,
    answers: VecDeque<Answer>,
    catching_up: bool,
    /// The encoded size of `answers`, in bytes.
    answers_bytes: usize,
    revision: Option<Revision>,
}

impl State {
    #[must_use]
    pub(crate) fn new(settings: PageSettings) -> Self {
        Self {
            link: Link::default(),
            people: Vec::new(),
            mine: Mine::default(),
            settings,
            answers: VecDeque::new(),
            catching_up: true,
            answers_bytes: 0,
            revision: None,
        }
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            link: self.link.clone(),
            people: self.people.clone(),
            mine: self.mine.view(),
            settings: self.settings.clone(),
            answers: self.answers.iter().cloned().collect(),
            revision: self.revision,
        }
    }

    /// Whether this joiner shows its cursor.
    #[must_use]
    pub(crate) fn cursor(&self) -> Switch {
        self.settings.show_cursor
    }

    fn mine_message(&self) -> ToPage {
        ToPage::Mine {
            undelivered: self.mine.view(),
        }
    }

    /// The share admitted this joiner as `you`, holding `undelivered` of
    /// its comments. A status line from before is dropped: the share
    /// sends its own after admission, if it has one.
    pub(crate) fn admitted(
        &mut self,
        you: Author,
        undelivered: &[CommentId],
    ) -> Result<Vec<ToPage>, TooMany> {
        self.mine.admitted(undelivered)?;
        self.catching_up = true;
        self.link = Link {
            connected: true,
            path: None,
            refused: None,
            you: Some(you),
            status: None,
        };
        Ok(vec![ToPage::Link(self.link.clone()), self.mine_message()])
    }

    pub(crate) fn path(&mut self, path: PathKind) -> Vec<ToPage> {
        self.link.path = Some(path);
        vec![ToPage::Link(self.link.clone())]
    }

    /// The connection is gone (or never came up); `refused` when for good.
    pub(crate) fn disconnected(&mut self, refused: Option<RefusedReason>) -> Vec<ToPage> {
        let link = Link {
            connected: false,
            path: None,
            refused,
            you: self.link.you.clone(),
            status: self.link.status.clone(),
        };
        // Open questions must be replayed after admission: a resolution may
        // have happened while disconnected, and catch-up omits it.
        let mut messages: Vec<_> = self
            .answers
            .iter()
            .filter(|answer| !answer.choices.is_empty())
            .map(|answer| ToPage::AnswerResolved { id: answer.id })
            .collect();
        self.answers.retain(|answer| answer.choices.is_empty());
        self.answers_bytes = self.answers.iter().map(Answer::encoded_size).sum();
        if link != self.link {
            self.link = link;
            messages.push(ToPage::Link(self.link.clone()));
        }
        if !self.people.is_empty() {
            self.people.clear();
            messages.push(ToPage::People { here: Vec::new() });
        }
        messages
    }

    /// The share's answer to one of this joiner's comments.
    pub(crate) fn acknowledged(&mut self, outcome: AckOutcome) -> Result<Vec<ToPage>, TooMany> {
        match outcome {
            AckOutcome::Queued { id } => self.mine.queued(id)?,
            AckOutcome::Delivered { id } => self.mine.delivered(&[id]),
            AckOutcome::RateLimited { .. }
            | AckOutcome::ChoiceRefused
            | AckOutcome::QueueFull
            | AckOutcome::AttachmentUnavailable => {
                return Ok(Vec::new());
            }
        }
        Ok(vec![self.mine_message()])
    }

    /// The settings changed.
    pub(crate) fn set_settings(&mut self, settings: PageSettings) -> Vec<ToPage> {
        if settings == self.settings {
            return Vec::new();
        }
        self.settings = settings;
        vec![ToPage::Settings(self.settings.clone())]
    }

    /// Applies a message from the share after admission (acknowledgements
    /// go to [`State::acknowledged`]).
    pub(crate) fn apply(&mut self, message: ShareMessage) -> Vec<ToPage> {
        match message {
            ShareMessage::Reload { revision } => {
                self.revision = Some(revision);
                vec![ToPage::Reload { revision }]
            }
            ShareMessage::Presence { here } => {
                // Admission sends the cached answers first, then starts the
                // broadcast queue, which includes this joiner's arrival.
                self.catching_up = false;
                self.people.clone_from(&here);
                vec![ToPage::People { here }]
            }
            ShareMessage::Status { line } => {
                self.link.status = line;
                vec![ToPage::Link(self.link.clone())]
            }
            ShareMessage::Delivered { ids } => {
                self.mine.delivered(&ids);
                vec![self.mine_message()]
            }
            ShareMessage::Answer {
                id,
                message,
                choices,
                anchor,
                to,
            } => {
                if self.answers.iter().any(|answer| answer.id == id) {
                    return Vec::new();
                }
                let (reply_to, anchor, page, place) = match anchor {
                    Some(AnswerAnchor::Comment { comment }) => (
                        Some(comment.comment),
                        (!comment.selector.as_ref().trim().is_empty()).then_some(comment.selector),
                        Some(comment.page),
                        None,
                    ),
                    Some(AnswerAnchor::Place { place }) => (None, None, None, Some(place)),
                    None => (None, None, None, None),
                };
                let you = self.link.you.as_ref().map(|you| you.key);
                let answer = Answer {
                    id,
                    choices,
                    html: markdown::render(&String::from(message)),
                    reply_to,
                    anchor,
                    page,
                    place,
                    to_you: to.is_some() && to == you,
                    live: !self.catching_up,
                };
                let incoming = answer.encoded_size();
                while !room(self.answers.len(), self.answers_bytes, incoming) {
                    let Some(oldest) = self.answers.pop_front() else {
                        break;
                    };
                    self.answers_bytes = self.answers_bytes.saturating_sub(oldest.encoded_size());
                }
                self.answers_bytes = self.answers_bytes.saturating_add(incoming);
                self.answers.push_back(answer.clone());
                vec![ToPage::Answer(answer)]
            }
            ShareMessage::AnswerResolved { id } => {
                self.answers.retain(|answer| answer.id != id);
                self.answers_bytes = self.answers.iter().map(Answer::encoded_size).sum();
                vec![ToPage::AnswerResolved { id }]
            }
            ShareMessage::Cursor { key, url, position } => {
                vec![ToPage::Cursor { key, url, position }]
            }
            ShareMessage::CommentAck { .. }
            | ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. } => {
                tracing::debug!("unexpected control message after admission");
                Vec::new()
            }
        }
    }
}

/// How a page holds the joiner's state: a snapshot, changed by each
/// message in turn (tests check the joiner against it).
#[cfg(test)]
pub(crate) fn follow(snapshot: &mut Snapshot, message: &ToPage) {
    match message {
        ToPage::Link(link) => snapshot.link = link.clone(),
        ToPage::People { here } => snapshot.people.clone_from(here),
        ToPage::Mine { undelivered } => snapshot.mine.clone_from(undelivered),
        ToPage::Settings(settings) => snapshot.settings = settings.clone(),
        ToPage::Reload { revision } => snapshot.revision = Some(*revision),
        ToPage::Answer(answer) => {
            if !snapshot.answers.iter().any(|known| known.id == answer.id) {
                let incoming = answer.encoded_size();
                let mut bytes: usize = snapshot.answers.iter().map(Answer::encoded_size).sum();
                while !snapshot.answers.is_empty() && !room(snapshot.answers.len(), bytes, incoming)
                {
                    bytes = bytes.saturating_sub(snapshot.answers.remove(0).encoded_size());
                }
                snapshot.answers.push(answer.clone());
            }
        }
        ToPage::AnswerResolved { id } => snapshot.answers.retain(|answer| answer.id != *id),
        ToPage::Hello(hello) => hello.clone_into(snapshot),
        ToPage::Cursor { .. }
        | ToPage::CommentReply { .. }
        | ToPage::SettingsReply { .. }
        | ToPage::Ping => {}
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
