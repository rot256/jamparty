//! What the joiner knows, as its pages see it, and each change as the
//! messages that tell pages of it. Pure: no I/O, no locks, no clock.
//!
//! Every message but `answer` and `cursor` replaces one slice of the
//! [`Snapshot`] whole, so a page that applies a snapshot and then the
//! messages after it, in order, holds the joiner's state.

use std::collections::{BTreeSet, VecDeque};

use serde::Serialize;

use crate::{
    core::{ids::CommentId, settings::Switch},
    join::{api::ToPage, markdown, preferences::PageSettings},
    protocol::{
        comment::{Author, CommentPage, CommentSelector},
        control::{AckOutcome, RefusedReason, Revision, ShareMessage, StatusLine},
        presence::Person,
    },
};

/// Answers kept for pages that load later.
pub(crate) const MAX_ANSWERS: usize = 100;
/// Delivered comments the share told of before this joiner heard they
/// were queued, at most this many (an older share's delivery can overtake
/// its own acknowledgement).
const MAX_EARLY: usize = 100;

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
}

/// An answer as pages show it: rendered Markdown, and where it belongs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Answer {
    pub(crate) id: CommentId,
    pub(crate) html: String,
    /// The selector of the comment it answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(crate) anchor: Option<CommentSelector>,
    /// The page that comment was made on.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub(crate) page: Option<CommentPage>,
    /// Addressed to this joiner alone (`jampot send --to`, or an answer to
    /// its comment); the page says so.
    #[serde(rename = "toYou")]
    pub(crate) to_you: bool,
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
    /// share first said (a joiner that restarted does not know yet).
    pub(crate) mine: Option<Vec<CommentId>>,
    pub(crate) settings: PageSettings,
    /// Recent answers, oldest first.
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

/// This joiner's comments the share holds undelivered.
#[derive(Debug, Default)]
pub(crate) struct Mine {
    /// None until the share first admitted this joiner.
    undelivered: Option<BTreeSet<CommentId>>,
    /// Ids the share delivered before their acknowledgement arrived.
    early: VecDeque<CommentId>,
}

impl Mine {
    #[must_use]
    pub(crate) fn view(&self) -> Option<Vec<CommentId>> {
        self.undelivered
            .as_ref()
            .map(|undelivered| undelivered.iter().copied().collect())
    }

    /// The share admitted this joiner holding `undelivered` of its
    /// comments; any others were delivered meanwhile.
    fn admitted(&mut self, undelivered: &[CommentId]) {
        self.undelivered = Some(undelivered.iter().copied().collect());
        self.early.clear();
    }

    fn queued(&mut self, id: CommentId) {
        match self.early.iter().position(|early| *early == id) {
            Some(index) => {
                self.early.remove(index);
            }
            None => {
                self.undelivered.get_or_insert_default().insert(id);
            }
        }
    }

    fn delivered(&mut self, ids: &[CommentId]) {
        for id in ids {
            let known = self
                .undelivered
                .as_mut()
                .is_some_and(|undelivered| undelivered.remove(id));
            if !known {
                if self.early.len() == MAX_EARLY {
                    self.early.pop_front();
                }
                self.early.push_back(*id);
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
    /// its comments.
    pub(crate) fn admitted(&mut self, you: Author, undelivered: &[CommentId]) -> Vec<ToPage> {
        self.link.connected = true;
        self.link.refused = None;
        self.link.you = Some(you);
        self.mine.admitted(undelivered);
        vec![ToPage::Link(self.link.clone()), self.mine_message()]
    }

    /// The connection is gone (or never came up); `refused` when for good.
    pub(crate) fn disconnected(&mut self, refused: Option<RefusedReason>) -> Vec<ToPage> {
        let link = Link {
            connected: false,
            refused,
            you: self.link.you.clone(),
            status: self.link.status.clone(),
        };
        let mut messages = Vec::new();
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
    pub(crate) fn acknowledged(&mut self, outcome: AckOutcome) -> Vec<ToPage> {
        match outcome {
            AckOutcome::Queued { id } => self.mine.queued(id),
            AckOutcome::Delivered { id } => self.mine.delivered(&[id]),
            AckOutcome::RateLimited { .. } | AckOutcome::QueueFull => return Vec::new(),
        }
        vec![self.mine_message()]
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
    /// go to [`State::acknowledged`] through the comment's request).
    pub(crate) fn apply(&mut self, message: ShareMessage) -> Vec<ToPage> {
        match message {
            ShareMessage::Reload { revision } => {
                self.revision = Some(revision);
                vec![ToPage::Reload { revision }]
            }
            ShareMessage::Presence { joiners, here } => {
                // An older share says only who is here.
                let here = if here.is_empty() {
                    joiners
                        .into_iter()
                        .map(|author| Person::new(author, None, Switch::Off))
                        .collect()
                } else {
                    here
                };
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
                anchor,
                to,
            } => {
                if self.answers.iter().any(|answer| answer.id == id) {
                    return Vec::new();
                }
                let (anchor, page) = match anchor {
                    Some(anchor) => (Some(anchor.selector), Some(anchor.page)),
                    None => (None, None),
                };
                let you = self.link.you.as_ref().map(|you| you.key);
                let answer = Answer {
                    id,
                    html: markdown::render(&String::from(message)),
                    anchor,
                    page,
                    to_you: to.is_some() && to == you,
                };
                if self.answers.len() == MAX_ANSWERS {
                    self.answers.pop_front();
                }
                self.answers.push_back(answer.clone());
                vec![ToPage::Answer(answer)]
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
                if snapshot.answers.len() == MAX_ANSWERS {
                    snapshot.answers.remove(0);
                }
                snapshot.answers.push(answer.clone());
            }
        }
        ToPage::Hello(hello) => *snapshot = hello.snapshot.clone(),
        ToPage::Cursor { .. } | ToPage::Reply { .. } | ToPage::Ping => {}
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
