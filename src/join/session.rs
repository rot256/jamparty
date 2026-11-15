//! The joiner's session: its state, its pages, its settings and its
//! control stream to the share, under one lock. Every change goes through
//! [`Session::commit`], which changes the state and queues the messages
//! that say so to every page, and a reply to the page that asked, before
//! it unlocks: pages hear of changes in the order they were made, after
//! the `hello` they attached with. Settings a page changes hold for this
//! join only: nothing a page does is written to disk.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

use crate::{
    core::{ids::CommentId, name::JoinerName, settings::Switch, version::Incompatible},
    join::{
        api::{Change, CommentOutcome, RequestNumber, SettingsOutcome, Submission, ToPage},
        control::Control,
        pages::{Attached, Dropped, Full, PageId, Pages},
        preferences::Preferences,
        state::{Snapshot, State, TooMany},
        uplink::Uplink,
    },
    protocol::{
        comment::{Author, Draft, DraftFields},
        control::{AckOutcome, RefusedReason, ShareMessage},
        presence::{PagePath, Position, Whereabouts},
    },
};

const NOT_SENT: &str = "not sent: the share is not reachable";

struct Inner {
    state: State,
    pages: Pages,
    preferences: Preferences,
    /// The admitted control stream, and which admission it came with.
    control: Option<(Admission, Arc<Control>)>,
    /// The latest admission.
    admissions: Admission,
    /// Set when the share turned out to speak another protocol.
    incompatible: Option<Incompatible>,
}

/// One admission to the share, counted from the first: tells one control
/// stream from the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Admission(u64);

impl Admission {
    #[must_use]
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

pub(crate) struct Session {
    inner: Mutex<Inner>,
    /// The state after each commit, for the local server and tests.
    snapshot: watch::Sender<Snapshot>,
    uplink: Uplink,
}

/// A reply to one page's request, queued after a commit's messages.
struct Reply {
    page: PageId,
    message: ToPage,
}

/// What a commit queues: messages for every page, then a reply for one.
struct Told {
    messages: Vec<ToPage>,
    reply: Option<Reply>,
}

impl From<Vec<ToPage>> for Told {
    fn from(messages: Vec<ToPage>) -> Self {
        Self {
            messages,
            reply: None,
        }
    }
}

impl Session {
    pub(crate) fn new(preferences: Preferences) -> Self {
        let state = State::new(preferences.page());
        Self {
            snapshot: watch::channel(state.snapshot()).0,
            inner: Mutex::new(Inner {
                state,
                pages: Pages::default(),
                preferences,
                control: None,
                admissions: Admission::default(),
                incompatible: None,
            }),
            uplink: Uplink::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes the state with `change` and, under the same lock, queues
    /// the messages it returns to every page, then its reply to its page.
    fn commit(&self, change: impl FnOnce(&mut Inner) -> Told) {
        let mut inner = self.lock();
        let Told { messages, reply } = change(&mut inner);
        let changed = messages.iter().any(|message| match message {
            ToPage::Cursor { .. }
            | ToPage::CommentReply { .. }
            | ToPage::SettingsReply { .. }
            | ToPage::Ping => false,
            ToPage::Hello(_)
            | ToPage::Link(_)
            | ToPage::People { .. }
            | ToPage::Mine { .. }
            | ToPage::Settings(_)
            | ToPage::Reload { .. }
            | ToPage::Answer(_) => true,
        });
        let mut dropped = Dropped::None;
        for message in &messages {
            if let Dropped::Some = inner.pages.publish(message) {
                dropped = Dropped::Some;
            }
        }
        if let Some(Reply { page, message }) = reply
            && let Dropped::Some = inner.pages.send(page, &message)
        {
            dropped = Dropped::Some;
        }
        if changed {
            self.snapshot.send_replace(inner.state.snapshot());
        }
        drop(inner);
        match dropped {
            // The page in front may be gone: the joiner is elsewhere.
            Dropped::Some => self.uplink.changed(),
            Dropped::None => {}
        }
    }

    pub(crate) fn uplink(&self) -> &Uplink {
        &self.uplink
    }

    /// The state as of the latest commit.
    pub(crate) fn watch(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.subscribe()
    }

    /// The name this joiner goes by now (in its hello, and its comments).
    pub(crate) fn name(&self) -> JoinerName {
        self.lock().preferences.name()
    }

    /// Where this joiner is, as the share is to be told.
    pub(crate) fn whereabouts(&self) -> Whereabouts {
        let inner = self.lock();
        Whereabouts {
            name: inner.preferences.name(),
            url: inner.pages.current(),
            cursor: inner.state.cursor(),
        }
    }

    /// The admitted control stream, if any.
    pub(crate) fn control(&self) -> Option<Arc<Control>> {
        self.admitted().map(|(_, control)| control)
    }

    /// The admitted control stream and its admission, if any.
    pub(crate) fn admitted(&self) -> Option<(Admission, Arc<Control>)> {
        self.lock().control.clone()
    }

    /// A page attached: `hello` is the first thing in its queue.
    pub(crate) fn attach(&self) -> Result<Attached, Full> {
        let mut inner = self.lock();
        let hello = ToPage::Hello(inner.state.snapshot());
        inner.pages.attach(&hello)
    }

    /// A page's socket closed.
    pub(crate) fn detach(&self, page: PageId) {
        self.lock().pages.detach(page);
        self.uplink.changed();
    }

    /// The page is on `url` (none: it cannot say), and in front.
    pub(crate) fn visit(&self, page: PageId, url: Option<PagePath>) {
        self.lock().pages.visit(page, url);
        self.uplink.changed();
    }

    /// The pointer on the page moved: sent on to the share only while the
    /// cursor switch is on, and only once the page said where it is. A
    /// page the pointer moves on comes in front first, so the share
    /// forwards it to the people on its page; a pointer leaving a page
    /// that is not in front is no news.
    pub(crate) fn cursor(&self, page: PageId, position: Option<Position>) {
        let url = {
            let mut inner = self.lock();
            match inner.state.cursor() {
                Switch::On => {}
                Switch::Off => return,
            }
            let Some(url) = inner.pages.url(page).cloned() else {
                return;
            };
            if !inner.pages.is_current(page) {
                match position {
                    Some(_) => inner.pages.activate(page),
                    None => return,
                }
            }
            url
        };
        self.uplink.cursor(url, position);
    }

    /// Hands a page's comment to the share and replies with what became
    /// of it, after the `mine` message that has it.
    pub(crate) async fn comment(&self, page: PageId, submission: Submission) {
        let Submission {
            n,
            reference,
            page: on,
            selector,
            selected_text,
            comment,
        } = submission;
        let draft = match Draft::try_from(DraftFields {
            page: on,
            selector,
            selected_text,
            comment,
        }) {
            Ok(draft) => draft,
            Err(error) => {
                let outcome = CommentOutcome::Refused {
                    error: error.to_string(),
                    retry_after: None,
                };
                self.reply_comment(page, n, outcome);
                return;
            }
        };
        let unsent = || CommentOutcome::Unsent {
            error: NOT_SENT.to_owned(),
        };
        let Some(control) = self.control() else {
            self.reply_comment(page, n, unsent());
            return;
        };
        let Ok(acked) = control.comment(reference, draft).await else {
            self.reply_comment(page, n, unsent());
            return;
        };
        let outcome = match acked {
            AckOutcome::Queued { id } | AckOutcome::Delivered { id } => {
                CommentOutcome::Commented { id }
            }
            AckOutcome::RateLimited { retry_after } => CommentOutcome::Refused {
                error: "too many comments; try again shortly".to_owned(),
                retry_after: Some(retry_after),
            },
            // Kept in the page and sent again later, as when unreachable.
            AckOutcome::QueueFull => CommentOutcome::Unsent {
                error: "the share's queue is full".to_owned(),
            },
        };
        // The acknowledgement changed the state when it arrived.
        self.reply_comment(page, n, outcome);
    }

    /// Applies a change from a page, for this join only, and tells every
    /// page.
    pub(crate) fn change_settings(&self, page: PageId, change: Change) {
        let Change { n, change } = change;
        self.commit(|inner| {
            let settings = inner.preferences.change(change);
            Told {
                messages: inner.state.set_settings(settings),
                reply: Some(Reply {
                    page,
                    message: ToPage::SettingsReply {
                        to: n,
                        outcome: SettingsOutcome::Saved,
                    },
                }),
            }
        });
        // A new name or cursor switch is news for the share.
        self.uplink.changed();
    }

    /// Answers comment `to` of `page` without changing anything.
    pub(crate) fn reply_comment(&self, page: PageId, to: RequestNumber, outcome: CommentOutcome) {
        self.reply(page, ToPage::CommentReply { to, outcome });
    }

    /// Answers settings change `to` of `page` without changing anything.
    pub(crate) fn reply_settings(&self, page: PageId, to: RequestNumber, outcome: SettingsOutcome) {
        self.reply(page, ToPage::SettingsReply { to, outcome });
    }

    fn reply(&self, page: PageId, message: ToPage) {
        self.commit(|_| Told {
            messages: Vec::new(),
            reply: Some(Reply { page, message }),
        });
    }

    /// The share admitted us on `control`; of our comments it still holds
    /// `undelivered`.
    /// Fails, changing nothing, for a share that claims more undelivered
    /// comments than a share holds.
    pub(crate) fn connected(
        &self,
        control: Arc<Control>,
        you: Author,
        undelivered: &[CommentId],
    ) -> Result<(), TooMany> {
        let mut result = Ok(());
        self.commit(|inner| match inner.state.admitted(you, undelivered) {
            Ok(messages) => {
                inner.admissions = inner.admissions.next();
                inner.control = Some((inner.admissions, control));
                messages.into()
            }
            Err(too_many) => {
                result = Err(too_many);
                Vec::new().into()
            }
        });
        if result.is_ok() {
            self.uplink.changed();
        }
        result
    }

    /// The share speaks another protocol: the joiner gives up.
    pub(crate) fn incompatible(&self, incompatible: Incompatible) {
        self.lock().incompatible = Some(incompatible);
    }

    /// Why the share's protocol is not this joiner's, if it is not.
    pub(crate) fn incompatibility(&self) -> Option<Incompatible> {
        self.lock().incompatible.clone()
    }

    /// The connection is gone (or never came up).
    pub(crate) fn disconnected(&self, refused: Option<RefusedReason>) {
        self.commit(|inner| {
            if let Some((_, control)) = inner.control.take() {
                control.abandon();
            }
            inner.state.disconnected(refused).into()
        });
    }

    /// Applies a message from the share after admission; fails, changing
    /// nothing, on an acknowledgement that would have this joiner hold
    /// more undelivered comments than a share keeps.
    pub(crate) fn apply(&self, control: &Control, message: ShareMessage) -> Result<(), TooMany> {
        match message {
            ShareMessage::CommentAck { reference, outcome } => {
                // Applied here, in the share's order (an acknowledgement
                // before its comment's delivery), before the comment's
                // reply.
                let mut result = Ok(());
                self.commit(|inner| match inner.state.acknowledged(outcome) {
                    Ok(messages) => messages.into(),
                    Err(too_many) => {
                        result = Err(too_many);
                        Vec::new().into()
                    }
                });
                result?;
                control.acknowledge(&reference, outcome);
            }
            other @ (ShareMessage::Admitted { .. }
            | ShareMessage::Refused { .. }
            | ShareMessage::Answer { .. }
            | ShareMessage::Delivered { .. }
            | ShareMessage::Reload { .. }
            | ShareMessage::Status { .. }
            | ShareMessage::Presence { .. }
            | ShareMessage::Cursor { .. }) => {
                self.commit(|inner| inner.state.apply(other).into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::{
        core::settings::{Settings, Switch},
        join::{api::ToPage, pages::PAGE_QUEUE, preferences::Preferences, session::Session},
        protocol::presence::{CursorSelector, Fraction, PagePath, Position},
    };

    /// A session with its cursor switch on.
    pub(crate) fn session() -> Session {
        let Settings {
            name,
            keys,
            comment_mode,
            show_cursor: _,
            corner,
            answer_timeout,
        } = Settings::default();
        Session::new(Preferences {
            settings: Settings {
                name,
                keys,
                comment_mode,
                show_cursor: Switch::On,
                corner,
                answer_timeout,
            },
            given: None,
            default: "alice".parse().unwrap(),
        })
    }

    fn url(path: &str) -> Option<PagePath> {
        Some(path.to_owned().try_into().unwrap())
    }

    #[test]
    fn a_pointer_leaving_a_background_tab_does_not_bring_it_in_front() {
        let session = session();
        let back = session.attach().unwrap();
        let front = session.attach().unwrap();
        session.visit(back.id, url("/back"));
        session.visit(front.id, url("/front"));
        session.cursor(back.id, None);
        assert_eq!(session.whereabouts().url, url("/front"));
        // A pointer that moves there does.
        let position = Position {
            selector: CursorSelector::try_from("h1".to_owned()).unwrap(),
            x: Fraction::try_from(0).unwrap(),
            y: Fraction::try_from(0).unwrap(),
        };
        session.cursor(back.id, Some(position));
        assert_eq!(session.whereabouts().url, url("/back"));
    }

    #[test]
    fn a_page_dropped_for_falling_behind_wakes_the_uplink() {
        let session = session();
        let mut quick = session.attach().unwrap();
        let slow = session.attach().unwrap();
        session.visit(quick.id, url("/quick"));
        session.visit(slow.id, url("/slow"));
        assert!(session.uplink().woken());
        assert!(!session.uplink().woken());
        for _ in 0..PAGE_QUEUE {
            session.commit(|_| vec![ToPage::Ping].into());
            while quick.queue.try_recv().is_ok() {}
        }
        // The slow page, in front, is gone: the share is to hear the
        // joiner is on the quick one's page now.
        assert!(session.uplink().woken());
        assert_eq!(session.whereabouts().url, url("/quick"));
    }
}
