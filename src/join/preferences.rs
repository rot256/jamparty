//! The joiner's settings: the saved ones, the name for this run, and how
//! pages see them.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    Report,
    adapters::settings,
    core::{
        name::JoinerName,
        settings::{Corner, Keys, Settings, SettingsChange, Switch},
    },
};

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
    /// The name this joiner goes by now.
    #[must_use]
    pub(crate) fn name(&self) -> JoinerName {
        self.given
            .clone()
            .or_else(|| self.saved.name.clone())
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

    /// Takes settings [`save`] wrote; a name chosen in the page replaces
    /// `--name` for the rest of the run.
    pub(crate) fn take(&mut self, saved: Saved) -> PageSettings {
        let Saved {
            settings,
            chose_name,
        } = saved;
        self.saved = settings;
        match chose_name {
            ChoseName::Yes => self.given = None,
            ChoseName::No => {}
        }
        self.page()
    }
}

/// Whether a change chose a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChoseName {
    Yes,
    No,
}

/// Settings written to the file, for [`Preferences::take`].
#[derive(Debug)]
#[must_use = "saved settings are to be taken"]
pub(crate) struct Saved {
    settings: Settings,
    chose_name: ChoseName,
}

/// Saves `change` to the settings file at `path`: read, changed and
/// written under its lock, since another joiner of this home may have
/// changed other settings meanwhile. Blocks on the file.
pub(crate) fn save(path: &Path, change: SettingsChange) -> Result<Saved, Report> {
    let chose_name = if change.name.is_some() {
        ChoseName::Yes
    } else {
        ChoseName::No
    };
    Ok(Saved {
        settings: settings::update(path, change)?,
        chose_name,
    })
}
