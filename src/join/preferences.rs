//! The joiner's settings for this join: the defaults read from the
//! settings file at start, the name for this run, the changes its pages
//! made, and how pages see them. Pages change them for this join only;
//! nothing a page does is written to disk.

use serde::Serialize;

use crate::core::{
    name::JoinerName,
    settings::{Corner, Keys, Settings, SettingsChange, Switch},
};

/// The settings as the page sees them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct PageSettings {
    /// The name this joiner goes by now.
    pub(crate) name: JoinerName,
    /// No name was chosen yet (none in the settings file or the page, no
    /// `--name`): the page asks.
    pub(crate) ask_name: bool,
    pub(crate) keys: Keys,
    pub(crate) comment_mode: Switch,
    pub(crate) show_cursor: Switch,
    pub(crate) corner: Corner,
}

/// The joiner's settings for this join.
pub(crate) struct Preferences {
    /// The settings file's, changed by pages since.
    pub(crate) settings: Settings,
    /// `jampot join --name`: this run's name, until one is chosen in the
    /// page.
    pub(crate) given: Option<JoinerName>,
    /// The name when none is given or chosen (`user@host`).
    pub(crate) default: JoinerName,
}

impl Preferences {
    /// The name this joiner goes by now.
    #[must_use]
    pub(crate) fn name(&self) -> JoinerName {
        self.given
            .clone()
            .or_else(|| self.settings.name.clone())
            .unwrap_or_else(|| self.default.clone())
    }

    #[must_use]
    pub(crate) fn page(&self) -> PageSettings {
        let Settings {
            name,
            keys,
            comment_mode,
            show_cursor,
            corner,
        } = &self.settings;
        PageSettings {
            name: self.name(),
            ask_name: name.is_none() && self.given.is_none(),
            keys: keys.clone(),
            comment_mode: *comment_mode,
            show_cursor: *show_cursor,
            corner: *corner,
        }
    }

    /// Applies a page's change for the rest of this join; a name chosen in
    /// the page replaces `--name`.
    pub(crate) fn change(&mut self, change: SettingsChange) -> PageSettings {
        if change.name.is_some() {
            self.given = None;
        }
        self.settings = self.settings.with(change);
        self.page()
    }
}
