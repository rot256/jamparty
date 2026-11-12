//! The joiner's settings (docs/ui.md): its saved name, the overlay's key
//! bindings and its two switches. Kept by the joiner process on disk (see
//! `adapters::settings`), changed from the page.

use serde::{Deserialize, Serialize};

use crate::core::{invalid::Invalid, name::JoinerName, text::check_line};

/// A switch in the control panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum Switch {
    On,
    Off,
}

pub(crate) const MAX_KEY: usize = 20;
/// Keys jampot uses itself: `/` opens a page comment, Escape cancels.
pub(crate) const RESERVED_KEYS: [&str; 2] = ["/", "Escape"];

/// A key as the browser names it (`KeyboardEvent.key`): `c`, `F2`,
/// `ArrowUp`; at most 20 characters, no control characters, not a key
/// jampot uses itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct KeyBinding(String);

impl KeyBinding {
    fn same_key(&self, other: &Self) -> bool {
        self.0.to_lowercase() == other.0.to_lowercase()
    }
}

impl TryFrom<String> for KeyBinding {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        check_line(&raw, "a key", MAX_KEY)?;
        if RESERVED_KEYS.contains(&raw.as_str()) {
            return Err(Invalid::new(format!("{raw} is used by jampot")));
        }
        Ok(Self(raw))
    }
}

impl From<KeyBinding> for String {
    fn from(key: KeyBinding) -> Self {
        key.0
    }
}

/// The fields of [`Keys`]; only reachable through a validated one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct KeysFields {
    pub(crate) comment_mode: KeyBinding,
    pub(crate) show_cursor: KeyBinding,
}

/// The keys that flip the switches; no two the same.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "KeysFields", into = "KeysFields")]
#[cfg_attr(test, derive(ts_rs::TS), ts(as = "KeysFields"))]
pub(crate) struct Keys(KeysFields);

impl TryFrom<KeysFields> for Keys {
    type Error = Invalid;

    fn try_from(fields: KeysFields) -> Result<Self, Self::Error> {
        if fields.comment_mode.same_key(&fields.show_cursor) {
            return Err(Invalid::new(format!(
                "{} is bound twice",
                fields.comment_mode.0
            )));
        }
        Ok(Self(fields))
    }
}

impl From<Keys> for KeysFields {
    fn from(keys: Keys) -> Self {
        keys.0
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self(KeysFields {
            comment_mode: KeyBinding("c".to_owned()),
            show_cursor: KeyBinding("x".to_owned()),
        })
    }
}

/// What the settings file holds. Fields a newer jampot adds are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settings {
    /// The name chosen in the page; none until the person picks one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<JoinerName>,
    #[serde(default)]
    pub(crate) keys: Keys,
    #[serde(default = "comment_mode_default")]
    pub(crate) comment_mode: Switch,
    #[serde(default = "show_cursor_default")]
    pub(crate) show_cursor: Switch,
}

/// Comment mode starts on (docs/ui.md, open question: kept on).
fn comment_mode_default() -> Switch {
    Switch::On
}

/// Nobody's pointer is shared until they turn it on.
fn show_cursor_default() -> Switch {
    Switch::Off
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: None,
            keys: Keys::default(),
            comment_mode: comment_mode_default(),
            show_cursor: show_cursor_default(),
        }
    }
}

/// A change from the page: the fields given replace the saved ones.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct SettingsChange {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) name: Option<JoinerName>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) keys: Option<Keys>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) comment_mode: Option<Switch>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) show_cursor: Option<Switch>,
}

impl Settings {
    /// These settings with `change` applied.
    #[must_use]
    pub(crate) fn with(&self, change: SettingsChange) -> Self {
        let SettingsChange {
            name,
            keys,
            comment_mode,
            show_cursor,
        } = change;
        Self {
            name: name.or_else(|| self.name.clone()),
            keys: keys.unwrap_or_else(|| self.keys.clone()),
            comment_mode: comment_mode.unwrap_or(self.comment_mode),
            show_cursor: show_cursor.unwrap_or(self.show_cursor),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::settings::{Keys, Settings, SettingsChange, Switch};

    #[test]
    fn defaults_and_parsing() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, Settings::default());
        assert_eq!(defaults.comment_mode, Switch::On);
        assert_eq!(defaults.show_cursor, Switch::Off);
        assert_eq!(
            serde_json::to_string(&defaults).unwrap(),
            r#"{"keys":{"commentMode":"c","showCursor":"x"},"commentMode":"on","showCursor":"off"}"#
        );
        // A newer jampot's fields are ignored.
        let newer: Settings = serde_json::from_str(r#"{"name":"alice","theme":"dark"}"#).unwrap();
        assert_eq!(newer.name.unwrap().as_str(), "alice");
    }

    #[test]
    fn keys_are_checked() {
        let keys = |comment: &str, cursor: &str| {
            serde_json::from_str::<Keys>(&format!(
                r#"{{"commentMode":{comment:?},"showCursor":{cursor:?}}}"#
            ))
        };
        assert!(keys("k", "F2").is_ok());
        for (comment, cursor) in [
            ("k", "k"),
            ("k", "K"),
            ("/", "x"),
            ("c", "Escape"),
            ("", "x"),
            (" ", "x"),
            ("\n", "x"),
            (&"a".repeat(21), "x"),
        ] {
            assert!(keys(comment, cursor).is_err(), "{comment:?} {cursor:?}");
        }
    }

    #[test]
    fn changes_replace_only_what_they_name() {
        let change: SettingsChange =
            serde_json::from_str(r#"{"name":"bob","showCursor":"on"}"#).unwrap();
        let changed = Settings::default().with(change);
        assert_eq!(changed.name.unwrap().as_str(), "bob");
        assert_eq!(changed.show_cursor, Switch::On);
        assert_eq!(changed.comment_mode, Switch::On);
        assert!(serde_json::from_str::<SettingsChange>(r#"{"admin":true}"#).is_err());
        assert!(serde_json::from_str::<SettingsChange>(r#"{"name":"a b"}"#).is_err());
    }
}
