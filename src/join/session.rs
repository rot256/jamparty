//! The joiner's view of its session: the live control connection (if any),
//! what the share last said, and the event stream for pages.

use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use serde::Serialize;
use tokio::{
    sync::{Mutex as AsyncMutex, Notify, broadcast, watch},
    time,
};

use crate::{
    Report,
    adapters::settings,
    core::{
        ids::{CommentId, EndpointKey},
        name::JoinerName,
        rng::{Rng, random_array},
        settings::{Corner, Keys, Settings, SettingsChange, Switch},
    },
    join::{
        control::Control,
        markdown,
        tabs::{TabError, TabId, Tabs},
    },
    protocol::{
        comment::{Author, CommentPage, CommentSelector},
        control::{RefusedReason, Revision, ShareMessage, StatusLine},
        presence::{PagePath, Person, Position, Whereabouts},
    },
};

const EVENT_CAPACITY: usize = 256;
/// Answers kept for pages that load later.
const MAX_ANSWERS: usize = 100;

/// What `/_jampot/api/status` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct View {
    /// Connected to the share and admitted.
    pub(crate) connected: bool,
    /// Set when the share refused this joiner; it does not retry then.
    pub(crate) refused: Option<RefusedReason>,
    /// How this joiner's comments are signed.
    pub(crate) you: Option<Author>,
    /// Who is connected, this joiner included.
    pub(crate) joiners: Vec<Author>,
    /// The same people with their colour, page and cursor switch.
    pub(crate) here: Vec<Person>,
    /// The live-reload revision (shared directories only).
    pub(crate) revision: Option<Revision>,
    pub(crate) status: Option<StatusLine>,
    /// This joiner's comments the share holds undelivered.
    pub(crate) pending: usize,
    /// Their ids.
    pub(crate) undelivered: Vec<CommentId>,
    /// Recent answers, oldest first.
    pub(crate) answers: Vec<Answer>,
    pub(crate) settings: PageSettings,
    /// Counts the changes to this view and the events sent to pages (each
    /// event carries the count after it): a page applies a status only if
    /// it is no older than the newest event it has applied.
    pub(crate) rev: u64,
    /// This joiner process: revisions count within it (a joiner restarted
    /// on the same port starts counting again).
    pub(crate) epoch: Epoch,
}

/// A joiner process's random id, which revisions belong to (64 bits, as 16
/// lowercase hex characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct Epoch(u64);

impl Epoch {
    #[must_use]
    pub(crate) fn generate(rng: &dyn Rng) -> Self {
        Self(u64::from_le_bytes(random_array(rng)))
    }
}

impl From<Epoch> for String {
    fn from(epoch: Epoch) -> Self {
        format!("{:016x}", epoch.0)
    }
}

impl View {
    fn new(settings: PageSettings, epoch: Epoch) -> Self {
        Self {
            connected: false,
            refused: None,
            you: None,
            joiners: Vec::new(),
            here: Vec::new(),
            revision: None,
            status: None,
            pending: 0,
            undelivered: Vec::new(),
            answers: Vec::new(),
            settings,
            rev: 0,
            epoch,
        }
    }
}

/// The settings as the page sees them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct PageSettings {
    /// The name this joiner goes by now.
    pub(crate) name: JoinerName,
    /// No name was chosen yet (none saved, no `--name`): the page asks.
    pub(crate) ask_name: bool,
    pub(crate) keys: Keys,
    pub(crate) comment_mode: Switch,
    pub(crate) show_cursor: Switch,
    pub(crate) corner: Corner,
}

/// The joiner's settings: the saved ones, and the name for this run.
pub(crate) struct Preferences {
    /// The settings file.
    pub(crate) path: PathBuf,
    pub(crate) saved: Settings,
    /// `jampot join --name`: this run's name, until one is chosen in the
    /// page.
    pub(crate) given: Option<JoinerName>,
    /// The name when none is given or saved (`user@host`).
    pub(crate) default: JoinerName,
}

impl Preferences {
    fn name(&self) -> JoinerName {
        self.given
            .clone()
            .or_else(|| self.saved.name.clone())
            .unwrap_or_else(|| self.default.clone())
    }

    fn page(&self) -> PageSettings {
        let Settings {
            name,
            keys,
            comment_mode,
            show_cursor,
            corner,
        } = &self.saved;
        PageSettings {
            name: self.name(),
            ask_name: name.is_none() && self.given.is_none(),
            keys: keys.clone(),
            comment_mode: *comment_mode,
            show_cursor: *show_cursor,
            corner: *corner,
        }
    }
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

/// One event on `/_jampot/api/events`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum Event {
    /// The connection to the share came up or went down.
    Share {
        connected: bool,
    },
    Reload {
        revision: Revision,
    },
    Presence {
        joiners: Vec<Author>,
        here: Vec<Person>,
    },
    Status {
        line: Option<StatusLine>,
    },
    Delivered {
        ids: Vec<CommentId>,
    },
    Answer(Answer),
    /// The settings changed (in this page or another).
    Settings(PageSettings),
    /// Someone's pointer on page `url` (sent only to tabs on it), or
    /// (none) they left it.
    Cursor {
        key: EndpointKey,
        url: PagePath,
        position: Option<Position>,
    },
}

/// An event as pages get it: with the view's revision after it, in this
/// joiner's epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Stamped {
    pub(crate) epoch: Epoch,
    pub(crate) rev: u64,
    #[serde(flatten)]
    #[cfg_attr(test, ts(flatten))]
    pub(crate) event: Event,
}

/// The least time between two presence reports: at most 4 per second.
/// Every report goes through the reporter task (see [`report_changes`]),
/// so nothing sends presence faster; the share disconnects a joiner past
/// 10 per second sustained (`RateLimit::PRESENCE_FLOOD`), well above.
pub(crate) const REPORT_GAP: Duration = Duration::from_millis(250);

/// What the share was told on the current connection.
#[derive(Default)]
struct Told {
    whereabouts: Option<Whereabouts>,
    /// The latest cursor on a page the share has not been told yet; sent
    /// right after the presence that tells it.
    held: Option<(PagePath, Option<Position>)>,
}

/// Sends the joiner's whereabouts to the share as they change, coalesced:
/// the latest state after each [`REPORT_GAP`], then any cursor that waited
/// for it.
pub(crate) async fn report_changes(session: Arc<Session>) {
    loop {
        session.changed.notified().await;
        session.send_report().await;
        time::sleep(REPORT_GAP).await;
    }
}

/// Delivered comments the share told of before this joiner heard they
/// were queued, at most this many (the share's delivery can overtake its
/// own acknowledgement).
const MAX_EARLY: usize = 100;

/// This joiner's comments the share holds undelivered.
#[derive(Default)]
struct Mine {
    undelivered: HashSet<CommentId>,
    /// Ids the share delivered before their acknowledgement arrived.
    early: VecDeque<CommentId>,
}

impl Mine {
    fn queued(&mut self, id: CommentId) {
        match self.early.iter().position(|early| *early == id) {
            Some(index) => {
                self.early.remove(index);
            }
            None => {
                self.undelivered.insert(id);
            }
        }
    }

    fn delivered(&mut self, ids: &[CommentId]) {
        for id in ids {
            if !self.undelivered.remove(id) {
                if self.early.len() == MAX_EARLY {
                    self.early.pop_front();
                }
                self.early.push_back(*id);
            }
        }
    }
}

/// Why a cursor was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CursorError {
    /// The cursor switch is off: nothing is sent.
    Off,
    /// The joiner has another page (or none) for the tab.
    Elsewhere,
    Tab(TabError),
    /// Not connected to the share.
    Unreachable,
}

pub(crate) struct Session {
    preferences: Mutex<Preferences>,
    view: watch::Sender<View>,
    control: Mutex<Option<Arc<Control>>>,
    events: broadcast::Sender<Stamped>,
    /// Ids of this joiner's comments the share has not delivered yet.
    mine: Mutex<Mine>,
    tabs: Mutex<Tabs>,
    /// What the share was last told on the current connection, and a cursor
    /// waiting for its page to be told first; locked while telling, so
    /// presence and cursors go out in order.
    told: AsyncMutex<Told>,
    /// Poked when the whereabouts may have changed.
    changed: Notify,
}

impl Session {
    pub(crate) fn new(preferences: Preferences, epoch: Epoch) -> Self {
        let view = View::new(preferences.page(), epoch);
        Self {
            preferences: Mutex::new(preferences),
            view: watch::channel(view).0,
            control: Mutex::new(None),
            events: broadcast::channel(EVENT_CAPACITY).0,
            mine: Mutex::new(Mine::default()),
            tabs: Mutex::new(Tabs::default()),
            told: AsyncMutex::new(Told::default()),
            changed: Notify::new(),
        }
    }

    fn tabs(&self) -> MutexGuard<'_, Tabs> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A page opened its event stream as `tab`, on `url`.
    pub(crate) fn open_tab(
        &self,
        tab: TabId,
        url: Option<PagePath>,
        seq: u64,
    ) -> Result<(), TabError> {
        self.tabs().open(tab, url, seq)
    }

    /// One of `tab`'s event streams closed.
    pub(crate) fn close_tab(&self, tab: &TabId) {
        self.tabs().close(tab);
    }

    /// `tab` moved to `url` (or became active there).
    pub(crate) fn visit(
        &self,
        tab: &TabId,
        url: Option<PagePath>,
        seq: u64,
    ) -> Result<(), TabError> {
        self.tabs().visit(tab, url, seq)
    }

    /// The URL `tab` is on.
    pub(crate) fn tab_url(&self, tab: &TabId) -> Option<PagePath> {
        self.tabs().url(tab).cloned()
    }

    /// Sends the pointer in `tab` on page `url` to the share, only while
    /// the cursor switch is on and only if `url` is the page the joiner has
    /// for that tab (a stale registration never routes a cursor). The tab
    /// becomes the active one first, so the share forwards it to the
    /// people on its page.
    pub(crate) async fn cursor(
        &self,
        tab: &TabId,
        seq: u64,
        url: &PagePath,
        position: Option<Position>,
    ) -> Result<(), CursorError> {
        match self.preferences().saved.show_cursor {
            Switch::On => {}
            Switch::Off => return Err(CursorError::Off),
        }
        let (known, latest) = {
            let tabs = self.tabs();
            let Some(latest) = tabs.seq(tab) else {
                return Err(CursorError::Tab(TabError::Unknown));
            };
            (tabs.url(tab).cloned(), latest)
        };
        // From before the tab's latest report, or for another page: stale.
        if seq < latest || known.as_ref() != Some(url) {
            return Err(CursorError::Elsewhere);
        }
        if self.tabs().current().as_ref() != Some(url) {
            self.visit(tab, Some(url.clone()), seq)
                .map_err(CursorError::Tab)?;
        }
        let control = self.control().ok_or(CursorError::Unreachable)?;
        let mut told = self.told.lock().await;
        if told.whereabouts.as_ref() == Some(&self.whereabouts()) {
            told.held = None;
            return control
                .cursor(position)
                .await
                .map_err(|_| CursorError::Unreachable);
        }
        // The share must learn the page first: the cursor (the latest only)
        // waits for the next report, which keeps to the presence budget.
        told.held = Some((url.clone(), position));
        drop(told);
        self.report();
        Ok(())
    }

    fn whereabouts(&self) -> Whereabouts {
        let (name, cursor) = {
            let preferences = self.preferences();
            (preferences.name(), preferences.saved.show_cursor)
        };
        Whereabouts {
            name,
            url: self.tabs().current(),
            cursor,
        }
    }

    /// Asks for the share to be told where this joiner is: the reporter
    /// task (see [`report_changes`]) sends the latest state, at most every
    /// [`REPORT_GAP`], so a burst of changes stays within the share's
    /// budget and ends on the last one.
    pub(crate) fn report(&self) {
        self.changed.notify_one();
    }

    /// Tells the share where this joiner is now, if that changed since it
    /// was last told (on this connection), then sends a cursor that waited
    /// for that page. Only the reporter task and a new connection call it.
    async fn send_report(&self) {
        let mut told = self.told.lock().await;
        let Some(control) = self.control() else {
            return;
        };
        let now = self.whereabouts();
        if told.whereabouts.as_ref() != Some(&now) {
            match control.tell(now.clone()).await {
                Ok(()) => told.whereabouts = Some(now),
                Err(error) => {
                    tracing::debug!(%error, "could not report presence");
                    return;
                }
            }
        }
        if let Some((url, position)) = told.held.take() {
            let page = told.whereabouts.as_ref().and_then(|told| told.url.as_ref());
            // Only for the page the share now has; a later one is moot.
            if page == Some(&url)
                && let Err(error) = control.cursor(position).await
            {
                tracing::debug!(%error, "could not send a held cursor");
            }
        }
    }

    fn preferences(&self) -> MutexGuard<'_, Preferences> {
        self.preferences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The name this joiner goes by now (in its hello, and its comments).
    pub(crate) fn name(&self) -> JoinerName {
        self.preferences().name()
    }

    /// Saves a change from the page and tells every page. A name chosen in
    /// the page replaces `--name` for the rest of the run.
    /// Returns the settings now and the revision of the event that says so.
    pub(crate) fn change_settings(
        &self,
        change: SettingsChange,
    ) -> Result<(PageSettings, u64), Report> {
        let page = {
            let mut preferences = self.preferences();
            let named = change.name.is_some();
            // Read, changed and written under a lock: another joiner of
            // this home may have changed other settings meanwhile.
            let saved = settings::update(&preferences.path, change)?;
            preferences.saved = saved;
            if named {
                preferences.given = None;
            }
            preferences.page()
        };
        self.modify(|view| view.settings.clone_from(&page));
        let rev = self.emit(Event::Settings(page.clone()));
        Ok((page, rev))
    }

    pub(crate) fn view(&self) -> View {
        self.view.borrow().clone()
    }

    pub(crate) fn watch(&self) -> watch::Receiver<View> {
        self.view.subscribe()
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Stamped> {
        self.events.subscribe()
    }

    /// Changes the view; returns its new revision.
    fn modify(&self, change: impl FnOnce(&mut View)) -> u64 {
        let mut rev = 0;
        self.view.send_modify(|view| {
            change(view);
            view.rev = view.rev.wrapping_add(1);
            rev = view.rev;
        });
        rev
    }

    /// Sends `event` to every page; returns its revision.
    fn emit(&self, event: Event) -> u64 {
        let rev = self.modify(|_| {});
        let epoch = self.view.borrow().epoch;
        // No receivers means no open pages.
        let _ = self.events.send(Stamped { epoch, rev, event });
        rev
    }

    /// This joiner process's epoch.
    pub(crate) fn epoch(&self) -> Epoch {
        self.view.borrow().epoch
    }

    pub(crate) fn control(&self) -> Option<Arc<Control>> {
        self.control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The share admitted us on `control`; of our comments it still holds
    /// only `undelivered`, the others were delivered while we were away.
    pub(crate) async fn connected(
        &self,
        control: Arc<Control>,
        you: Author,
        undelivered: &[CommentId],
    ) {
        {
            // A new connection has been told nothing yet.
            let mut told = self.told.lock().await;
            *told = Told::default();
            *self.control.lock().unwrap_or_else(PoisonError::into_inner) = Some(control);
        }
        let mut delivered = Vec::new();
        self.update_mine(
            |Mine {
                 undelivered: mine, ..
             }| {
                mine.retain(|id| {
                    let held = undelivered.contains(id);
                    if !held {
                        delivered.push(*id);
                    }
                    held
                });
            },
        );
        self.modify(|view| {
            view.connected = true;
            view.you = Some(you);
        });
        self.emit(Event::Share { connected: true });
        if !delivered.is_empty() {
            self.emit(Event::Delivered { ids: delivered });
        }
        self.send_report().await;
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
        self.modify(|view| {
            view.connected = false;
            view.refused = refused;
            view.joiners.clear();
            view.here.clear();
        });
        if was_connected {
            self.emit(Event::Share { connected: false });
        }
    }

    /// One of our comments, sent again, was delivered already as `id`;
    /// returns how many are pending, and the view's revision.
    pub(crate) fn already_delivered(&self, id: CommentId) -> (usize, u64) {
        self.emit(Event::Delivered { ids: vec![id] });
        self.update_mine(|mine| {
            mine.undelivered.remove(&id);
        })
    }

    /// One of our comments was queued under `id`; returns how many are
    /// pending, and the view's revision that has it.
    pub(crate) fn queued(&self, id: CommentId) -> (usize, u64) {
        self.update_mine(|mine| mine.queued(id))
    }

    /// The share delivered these comments.
    fn delivered(&self, ids: &[CommentId]) {
        self.update_mine(|mine| mine.delivered(ids));
    }

    /// Changes the set of our undelivered comments; returns its size and
    /// the view's revision.
    fn update_mine(&self, change: impl FnOnce(&mut Mine)) -> (usize, u64) {
        let undelivered: Vec<CommentId> = {
            let mut mine = self.mine.lock().unwrap_or_else(PoisonError::into_inner);
            change(&mut mine);
            mine.undelivered.iter().copied().collect()
        };
        let pending = undelivered.len();
        let rev = self.modify(|view| {
            view.pending = pending;
            view.undelivered = undelivered;
        });
        (pending, rev)
    }

    /// Applies a message from the share after admission.
    pub(crate) fn apply(&self, control: &Control, message: ShareMessage) {
        match message {
            ShareMessage::CommentAck { reference, outcome } => {
                control.acknowledge(&reference, outcome);
            }
            ShareMessage::Reload { revision } => {
                self.modify(|view| view.revision = Some(revision));
                self.emit(Event::Reload { revision });
            }
            ShareMessage::Presence { joiners, here } => {
                // An older share says only who is here.
                let here = if here.is_empty() {
                    joiners
                        .iter()
                        .map(|author| Person::new(author.clone(), None, Switch::Off))
                        .collect()
                } else {
                    here
                };
                self.modify(|view| {
                    view.joiners.clone_from(&joiners);
                    view.here.clone_from(&here);
                });
                self.emit(Event::Presence { joiners, here });
            }
            ShareMessage::Status { line } => {
                self.modify(|view| view.status.clone_from(&line));
                self.emit(Event::Status { line });
            }
            ShareMessage::Delivered { ids } => {
                self.delivered(&ids);
                self.emit(Event::Delivered { ids });
            }
            ShareMessage::Answer {
                id,
                message,
                anchor,
                to,
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
                let you = self.view.borrow().you.as_ref().map(|you| you.key);
                let answer = Answer {
                    id,
                    html: markdown::render(&String::from(message)),
                    anchor,
                    page,
                    to_you: to.is_some() && to == you,
                };
                self.modify(|view| {
                    if view.answers.len() == MAX_ANSWERS {
                        view.answers.remove(0);
                    }
                    view.answers.push(answer.clone());
                });
                self.emit(Event::Answer(answer));
            }
            ShareMessage::Cursor { key, url, position } => {
                self.emit(Event::Cursor { key, url, position });
            }
            ShareMessage::Admitted { .. } | ShareMessage::Refused { .. } => {
                tracing::debug!("unexpected admission message after admission");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::{ids::CommentId, rng::SeededRng},
        join::session::{MAX_EARLY, Mine},
    };

    #[test]
    fn a_delivery_that_overtakes_its_acknowledgement_is_kept() {
        let rng = SeededRng::new(3);
        let (early, late) = (CommentId::generate(&rng), CommentId::generate(&rng));
        let mut mine = Mine::default();
        // Delivered before the joiner heard it was queued: never pending.
        mine.delivered(&[early]);
        mine.queued(early);
        assert!(mine.undelivered.is_empty());
        assert!(mine.early.is_empty());
        // The usual order.
        mine.queued(late);
        assert!(mine.undelivered.contains(&late));
        mine.delivered(&[late]);
        assert!(mine.undelivered.is_empty() && mine.early.is_empty());
        // Bounded.
        let others: Vec<CommentId> = (0..MAX_EARLY + 5)
            .map(|_| CommentId::generate(&rng))
            .collect();
        mine.delivered(&others);
        assert_eq!(mine.early.len(), MAX_EARLY);
    }
}
