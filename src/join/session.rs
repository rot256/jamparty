//! The joiner's session: its state and its pages under one lock. Every
//! change goes through [`Session::commit`], which changes the state and
//! queues the messages that say so to every page before it unlocks: pages
//! hear of changes in the order they were made, after the `hello` they
//! attached with. Settings are written to their file outside that lock,
//! one change at a time.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rootcause::prelude::report;
use tokio::{sync, sync::watch, task};

use crate::{
    core::{ids::CommentId, name::JoinerName, settings::Switch, version::Incompatible},
    describe,
    join::{
        api::{Change, Outcome, RequestNumber, Submission, ToPage},
        control::Control,
        pages::{Attached, Full, PageId, Pages},
        preferences::{self, Preferences},
        state::{Snapshot, State},
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
}

pub(crate) struct Session {
    inner: Mutex<Inner>,
    control: Mutex<Option<Arc<Control>>>,
    /// The state after each commit, for the local server and tests.
    snapshot: watch::Sender<Snapshot>,
    /// Set when the share turned out to speak another protocol.
    incompatible: Mutex<Option<Incompatible>>,
    uplink: Uplink,
    /// Held from writing a settings change to taking it, so changes are
    /// written and taken in the order they came.
    settings_writes: sync::Mutex<()>,
}

/// A reply to one page's request, queued after a commit's messages.
struct Reply {
    page: PageId,
    to: RequestNumber,
    outcome: Outcome,
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
            }),
            control: Mutex::new(None),
            incompatible: Mutex::new(None),
            uplink: Uplink::default(),
            settings_writes: sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes the state with `change` and, under the same lock, queues
    /// the messages it returns to every page, then `reply` to its page.
    fn commit(&self, change: impl FnOnce(&mut Inner) -> Vec<ToPage>, reply: Option<Reply>) {
        let mut inner = self.lock();
        let messages = change(&mut inner);
        let changed = messages.iter().any(|message| match message {
            ToPage::Cursor { .. } | ToPage::Reply { .. } | ToPage::Ping => false,
            ToPage::Hello(_)
            | ToPage::Link(_)
            | ToPage::People { .. }
            | ToPage::Mine { .. }
            | ToPage::Settings(_)
            | ToPage::Reload { .. }
            | ToPage::Answer(_) => true,
        });
        for message in &messages {
            inner.pages.publish(message);
        }
        if let Some(Reply { page, to, outcome }) = reply {
            inner.pages.send(page, &ToPage::Reply { to, outcome });
        }
        if changed {
            self.snapshot.send_replace(inner.state.snapshot());
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

    pub(crate) fn control(&self) -> Option<Arc<Control>> {
        self.control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        let reply = |outcome| {
            Some(Reply {
                page,
                to: n,
                outcome,
            })
        };
        let draft = match Draft::try_from(DraftFields {
            page: on,
            selector,
            selected_text,
            comment,
        }) {
            Ok(draft) => draft,
            Err(error) => {
                let outcome = Outcome::Refused {
                    error: error.to_string(),
                    retry_after: None,
                };
                self.commit(|_| Vec::new(), reply(outcome));
                return;
            }
        };
        let unsent = || Outcome::Unsent {
            error: NOT_SENT.to_owned(),
        };
        let Some(control) = self.control() else {
            self.commit(|_| Vec::new(), reply(unsent()));
            return;
        };
        let acked = match control.comment(reference, draft).await {
            Ok(outcome) => outcome,
            Err(_) => {
                self.commit(|_| Vec::new(), reply(unsent()));
                return;
            }
        };
        let outcome = match acked {
            AckOutcome::Queued { id } | AckOutcome::Delivered { id } => Outcome::Commented { id },
            AckOutcome::RateLimited { retry_after } => Outcome::Refused {
                error: "too many comments; try again shortly".to_owned(),
                retry_after: Some(retry_after),
            },
            // Kept in the page and sent again later, as when unreachable.
            AckOutcome::QueueFull => Outcome::Unsent {
                error: "the share's queue is full".to_owned(),
            },
        };
        self.commit(|inner| inner.state.acknowledged(acked), reply(outcome));
    }

    /// Saves a change from a page and tells every page. The file is
    /// written on a blocking thread, outside the session's lock.
    pub(crate) async fn change_settings(&self, page: PageId, change: Change) {
        let Change { n, change } = change;
        let _turn = self.settings_writes.lock().await;
        let path = self.lock().preferences.path.clone();
        let saved = task::spawn_blocking(move || preferences::save(&path, change))
            .await
            .unwrap_or_else(|error| Err(report!("the settings write failed: {error}")));
        match saved {
            Ok(saved) => self.commit(
                |inner| {
                    let settings = inner.preferences.take(saved);
                    inner.state.set_settings(settings)
                },
                Some(Reply {
                    page,
                    to: n,
                    outcome: Outcome::Saved,
                }),
            ),
            Err(error) => {
                tracing::warn!(error = %describe(&error), "could not save the settings");
                let outcome = Outcome::Refused {
                    error: format!("could not save the settings: {}", describe(&error)),
                    retry_after: None,
                };
                self.reply(page, n, outcome);
            }
        }
        // A new name or cursor switch is news for the share.
        self.uplink.changed();
    }

    /// Answers request `to` of `page` without changing anything.
    pub(crate) fn reply(&self, page: PageId, to: RequestNumber, outcome: Outcome) {
        self.commit(|_| Vec::new(), Some(Reply { page, to, outcome }));
    }

    /// The share admitted us on `control`; of our comments it still holds
    /// `undelivered`.
    pub(crate) fn connected(&self, control: Arc<Control>, you: Author, undelivered: &[CommentId]) {
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) = Some(control);
        self.commit(|inner| inner.state.admitted(you, undelivered), None);
        self.uplink.changed();
    }

    /// The share speaks another protocol: the joiner gives up.
    pub(crate) fn incompatible(&self, incompatible: Incompatible) {
        *self
            .incompatible
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(incompatible);
    }

    /// Why the share's protocol is not this joiner's, if it is not.
    pub(crate) fn incompatibility(&self) -> Option<Incompatible> {
        self.incompatible
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        self.commit(|inner| inner.state.disconnected(refused), None);
    }

    /// Applies a message from the share after admission.
    pub(crate) fn apply(&self, control: &Control, message: ShareMessage) {
        match message {
            ShareMessage::CommentAck { reference, outcome } => {
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
                self.commit(|inner| inner.state.apply(other), None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        core::settings::{Settings, Switch},
        join::{preferences::Preferences, session::Session},
        protocol::presence::{CursorSelector, Fraction, PagePath, Position},
    };

    fn session() -> Session {
        let Settings {
            name,
            keys,
            comment_mode,
            show_cursor: _,
            corner,
        } = Settings::default();
        Session::new(Preferences {
            path: PathBuf::from("/nonexistent/settings.json"),
            saved: Settings {
                name,
                keys,
                comment_mode,
                show_cursor: Switch::On,
                corner,
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
}
