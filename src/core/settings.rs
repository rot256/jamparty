//! The joiner's settings (docs/ui.md): its name, the overlay's key
//! bindings, its two switches and the corner it sits in. Each join starts
//! from the settings file (see `adapters::settings`); pages change them
//! for that join.

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

/// The screen corner the overlay's pill, gear and panels sit in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum Corner {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

const MAX_KEY: usize = 20;
/// Keys jampot uses itself: `/` opens a page comment, Escape cancels.
pub(crate) const RESERVED_KEYS: [&str; 2] = ["/", "Escape"];
/// Keys pressed only with others: the overlay's rebinding waits past them.
pub(crate) const MODIFIER_KEYS: [&str; 13] = [
    "Alt",
    "AltGraph",
    "CapsLock",
    "Control",
    "Fn",
    "FnLock",
    "Hyper",
    "Meta",
    "NumLock",
    "ScrollLock",
    "Shift",
    "Super",
    "Symbol",
];
/// Keys a page needs for itself: forms, focus and buttons.
pub(crate) const PAGE_KEYS: [&str; 3] = ["Enter", "Tab", " "];

/// A key as the browser names it (`KeyboardEvent.key`): `c`, `F2`,
/// `ArrowUp`; at most 20 characters, no control characters, and none the
/// overlay's rebinding refuses: not a key jampot uses itself, a modifier,
/// or Enter, Tab or Space. Checked here too, since any page script can
/// send a change.
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
        let named = |keys: &[&str]| keys.iter().any(|key| key.eq_ignore_ascii_case(&raw));
        if named(&RESERVED_KEYS) {
            return Err(Invalid::new(format!("{raw} is used by jampot")));
        }
        if named(&MODIFIER_KEYS) || named(&PAGE_KEYS) {
            return Err(Invalid::new(format!("{raw} cannot be bound")));
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
    #[serde(default)]
    pub(crate) corner: Corner,
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
            corner: Corner::default(),
        }
    }
}

/// A change from the page: the fields given replace the current ones.
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
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) corner: Option<Corner>,
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
            corner,
        } = change;
        Self {
            name: name.or_else(|| self.name.clone()),
            keys: keys.unwrap_or_else(|| self.keys.clone()),
            comment_mode: comment_mode.unwrap_or(self.comment_mode),
            show_cursor: show_cursor.unwrap_or(self.show_cursor),
            corner: corner.unwrap_or(self.corner),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::settings::{Corner, Keys, Settings, SettingsChange, Switch};

    #[test]
    fn defaults_and_parsing() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, Settings::default());
        assert_eq!(defaults.comment_mode, Switch::On);
        assert_eq!(defaults.show_cursor, Switch::Off);
        assert_eq!(defaults.corner, Corner::BottomRight);
        assert_eq!(
            serde_json::to_string(&defaults).unwrap(),
            r#"{"keys":{"commentMode":"c","showCursor":"x"},"commentMode":"on","showCursor":"off","corner":"bottom-right"}"#
        );
        // Each corner, and only those.
        for (text, corner) in [
            ("bottom-right", Corner::BottomRight),
            ("bottom-left", Corner::BottomLeft),
            ("top-right", Corner::TopRight),
            ("top-left", Corner::TopLeft),
        ] {
            let change: SettingsChange =
                serde_json::from_str(&format!(r#"{{"corner":"{text}"}}"#)).unwrap();
            assert_eq!(Settings::default().with(change).corner, corner);
        }
        assert!(serde_json::from_str::<SettingsChange>(r#"{"corner":"middle"}"#).is_err());
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
            ("c", "escape"),
            ("Shift", "x"),
            ("c", "Control"),
            ("Alt", "x"),
            ("c", "Meta"),
            ("CapsLock", "x"),
            ("c", "Enter"),
            ("Tab", "x"),
            (" ", "x"),
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
