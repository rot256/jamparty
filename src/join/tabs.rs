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
    /// None when the page cannot say (its URL is not representable).
    url: Option<PagePath>,
    /// The page's number for its latest report (see `Tabs::visit`).
    seq: u64,
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

    /// A tab opened an event stream on `url` (none: it cannot say), as of
    /// its report `seq`; it becomes the active one. A stream that reopens
    /// by itself names the page it first opened on: older than what the
    /// tab reported since, which then stands.
    pub(crate) fn open(
        &mut self,
        tab: TabId,
        url: Option<PagePath>,
        seq: u64,
    ) -> Result<(), TabError> {
        let active = self.tick();
        if let Some(known) = self.tabs.get_mut(&tab) {
            known.streams = known.streams.saturating_add(1);
            known.active = active;
            if seq >= known.seq {
                known.url = url;
                known.seq = seq;
            }
            return Ok(());
        }
        if self.tabs.len() >= MAX_TABS {
            return Err(TabError::Full);
        }
        self.tabs.insert(
            tab,
            Tab {
                url,
                seq,
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

    /// The tab is on `url` now (none: it cannot say) and active, as its
    /// report `seq` says; a report older than one already taken changes
    /// nothing (requests can overtake each other).
    pub(crate) fn visit(
        &mut self,
        tab: &TabId,
        url: Option<PagePath>,
        seq: u64,
    ) -> Result<(), TabError> {
        let active = self.tick();
        let known = self.tabs.get_mut(tab).ok_or(TabError::Unknown)?;
        if seq < known.seq {
            return Ok(());
        }
        known.url = url;
        known.seq = seq;
        known.active = active;
        Ok(())
    }

    /// The tab's latest report number.
    #[must_use]
    pub(crate) fn seq(&self, tab: &TabId) -> Option<u64> {
        self.tabs.get(tab).map(|known| known.seq)
    }

    /// The URL the tab is on.
    #[must_use]
    pub(crate) fn url(&self, tab: &TabId) -> Option<&PagePath> {
        self.tabs.get(tab).and_then(|known| known.url.as_ref())
    }

    /// Where the joiner is: the URL of its most recently active tab.
    #[must_use]
    pub(crate) fn current(&self) -> Option<PagePath> {
        self.tabs
            .values()
            .max_by_key(|known| known.active)
            .and_then(|known| known.url.clone())
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

    fn url(path: &str) -> Option<PagePath> {
        Some(path.to_owned().try_into().unwrap())
    }

    #[test]
    fn the_most_recently_active_tab_wins() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.current(), None);
        tabs.open(tab("a"), url("/a"), 0).unwrap();
        tabs.open(tab("b"), url("/b"), 0).unwrap();
        assert_eq!(tabs.current(), url("/b"));
        tabs.visit(&tab("a"), url("/a#2"), 1).unwrap();
        assert_eq!(tabs.current(), url("/a#2"));
        assert_eq!(tabs.visit(&tab("c"), url("/c"), 1), Err(TabError::Unknown));

        // A tab reconnecting has two streams for a moment.
        // ... naming its first page: the later report stands.
        tabs.open(tab("a"), url("/a"), 0).unwrap();
        assert_eq!(tabs.url(&tab("a")), url("/a#2").as_ref());
        // An older report changes nothing.
        tabs.visit(&tab("a"), url("/old"), 0).unwrap();
        assert_eq!(tabs.url(&tab("a")), url("/a#2").as_ref());
        tabs.close(&tab("a"));
        assert_eq!(tabs.current(), url("/a#2"));
        tabs.close(&tab("a"));
        assert_eq!(tabs.current(), url("/b"));
        tabs.close(&tab("b"));
        assert_eq!(tabs.current(), None);
        // A tab that cannot say where it is makes the joiner nowhere.
        tabs.open(tab("c"), url("/c"), 0).unwrap();
        tabs.visit(&tab("c"), None, 1).unwrap();
        assert_eq!(tabs.current(), None);
        assert_eq!(tabs.url(&tab("c")), None);
    }

    #[test]
    fn tabs_are_bounded_and_ids_checked() {
        let mut tabs = Tabs::default();
        for index in 0..MAX_TABS {
            tabs.open(tab(&format!("t{index}")), url("/"), 0).unwrap();
        }
        assert_eq!(tabs.open(tab("one-more"), url("/"), 0), Err(TabError::Full));
        for bad in ["", "a b", "a/b", &"a".repeat(65)] {
            assert!(TabId::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
    }
}
