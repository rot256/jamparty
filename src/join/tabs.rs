//! The joiner's open pages (browser tabs): each names itself with a random
//! id when it opens the event stream, and says which URL it is on. The
//! joiner is where its most recently active tab is.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{core::invalid::Invalid, protocol::presence::PagePath};

pub(crate) const MAX_TAB_ID: usize = 64;
/// Tabs one joiner keeps track of; a page opening more is refused.
pub(crate) const MAX_TABS: usize = 64;

/// A tab's id: 1 to 64 characters of `[A-Za-z0-9-]` (the page uses a
/// random UUID).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct TabId(String);

impl TryFrom<String> for TabId {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = (1..=MAX_TAB_ID).contains(&raw.len())
            && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if valid {
            Ok(Self(raw))
        } else {
            Err(Invalid::new(format!(
                "a tab id is 1 to {MAX_TAB_ID} characters of A-Z, a-z, 0-9 and -"
            )))
        }
    }
}

impl From<TabId> for String {
    fn from(tab: TabId) -> Self {
        tab.0
    }
}

struct Tab {
    url: PagePath,
    /// Its open event streams (a reconnecting page may briefly have two).
    streams: usize,
    /// When it was last active (see `Tabs::clock`).
    active: u64,
}

/// Why a tab's report was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabError {
    /// No open event stream has this id.
    Unknown,
    /// Too many tabs.
    Full,
}

#[derive(Default)]
pub(crate) struct Tabs {
    tabs: HashMap<TabId, Tab>,
    clock: u64,
}

impl Tabs {
    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    /// A tab opened an event stream on `url`; it becomes the active one.
    pub(crate) fn open(&mut self, tab: TabId, url: PagePath) -> Result<(), TabError> {
        let active = self.tick();
        if let Some(known) = self.tabs.get_mut(&tab) {
            known.streams = known.streams.saturating_add(1);
            known.url = url;
            known.active = active;
            return Ok(());
        }
        if self.tabs.len() >= MAX_TABS {
            return Err(TabError::Full);
        }
        self.tabs.insert(
            tab,
            Tab {
                url,
                streams: 1,
                active,
            },
        );
        Ok(())
    }

    /// One of a tab's event streams closed; the tab is gone with its last.
    pub(crate) fn close(&mut self, tab: &TabId) {
        if let Some(known) = self.tabs.get_mut(tab) {
            known.streams = known.streams.saturating_sub(1);
            if known.streams == 0 {
                self.tabs.remove(tab);
            }
        }
    }

    /// The tab is on `url` now and active.
    pub(crate) fn visit(&mut self, tab: &TabId, url: PagePath) -> Result<(), TabError> {
        let active = self.tick();
        let known = self.tabs.get_mut(tab).ok_or(TabError::Unknown)?;
        known.url = url;
        known.active = active;
        Ok(())
    }

    /// Where the joiner is: the URL of its most recently active tab.
    #[must_use]
    pub(crate) fn current(&self) -> Option<PagePath> {
        self.tabs
            .values()
            .max_by_key(|known| known.active)
            .map(|known| known.url.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        join::tabs::{MAX_TABS, TabError, TabId, Tabs},
        protocol::presence::PagePath,
    };

    fn tab(id: &str) -> TabId {
        id.to_owned().try_into().unwrap()
    }

    fn url(path: &str) -> PagePath {
        path.to_owned().try_into().unwrap()
    }

    #[test]
    fn the_most_recently_active_tab_wins() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.current(), None);
        tabs.open(tab("a"), url("/a")).unwrap();
        tabs.open(tab("b"), url("/b")).unwrap();
        assert_eq!(tabs.current(), Some(url("/b")));
        tabs.visit(&tab("a"), url("/a#2")).unwrap();
        assert_eq!(tabs.current(), Some(url("/a#2")));
        assert_eq!(tabs.visit(&tab("c"), url("/c")), Err(TabError::Unknown));

        // A tab reconnecting has two streams for a moment.
        tabs.open(tab("a"), url("/a#2")).unwrap();
        tabs.close(&tab("a"));
        assert_eq!(tabs.current(), Some(url("/a#2")));
        tabs.close(&tab("a"));
        assert_eq!(tabs.current(), Some(url("/b")));
        tabs.close(&tab("b"));
        assert_eq!(tabs.current(), None);
    }

    #[test]
    fn tabs_are_bounded_and_ids_checked() {
        let mut tabs = Tabs::default();
        for index in 0..MAX_TABS {
            tabs.open(tab(&format!("t{index}")), url("/")).unwrap();
        }
        assert_eq!(tabs.open(tab("one-more"), url("/")), Err(TabError::Full));
        for bad in ["", "a b", "a/b", &"a".repeat(65)] {
            assert!(TabId::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
    }
}
