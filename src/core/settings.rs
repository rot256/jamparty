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

/// The overlay colour scheme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum Theme {
    #[default]
    Auto,
    Dark,
    Light,
}

/// The overlay's text and control size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum TextSize {
    Small,
    #[default]
    Default,
    Large,
    Larger,
}

const MAX_KEY: usize = 20;
/// Largest supported answer timeout, in seconds.
pub(crate) const MAX_ANSWER_TIMEOUT: u16 = 3600;
/// Answers dismiss themselves after this long unless set otherwise.
pub(crate) const DEFAULT_ANSWER_TIMEOUT: u16 = 30;
/// Default key for dismissing all answers.
pub(crate) const DEFAULT_CLEAR_ANSWERS_KEY: &str = "d";
/// Keys jamparty uses itself: `/` opens a page comment, Escape cancels.
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
/// overlay's rebinding refuses: not a key jamparty uses itself, a modifier,
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
            return Err(Invalid::new(format!("{raw} is used by jamparty")));
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

/// Seconds before an answer is dismissed: zero disables the timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "number"))]
pub(crate) struct AnswerTimeout(u16);

impl Default for AnswerTimeout {
    fn default() -> Self {
        Self(DEFAULT_ANSWER_TIMEOUT)
    }
}

impl TryFrom<u16> for AnswerTimeout {
    type Error = Invalid;

    fn try_from(seconds: u16) -> Result<Self, Self::Error> {
        if seconds > MAX_ANSWER_TIMEOUT {
            return Err(out_of_range());
        }
        Ok(Self(seconds))
    }
}

fn out_of_range() -> Invalid {
    Invalid::new(format!(
        "answer timeout must be 0 (off) or 1–{MAX_ANSWER_TIMEOUT} seconds"
    ))
}

impl From<AnswerTimeout> for u16 {
    fn from(timeout: AnswerTimeout) -> Self {
        timeout.0
    }
}

impl std::str::FromStr for AnswerTimeout {
    type Err = Invalid;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.parse::<u16>().map_err(|_| out_of_range())?.try_into()
    }
}

impl std::str::FromStr for KeyBinding {
    type Err = Invalid;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.to_owned().try_into()
    }
}

fn clear_answers_default() -> KeyBinding {
    KeyBinding(DEFAULT_CLEAR_ANSWERS_KEY.to_owned())
}

/// The fields of [`Keys`]; only reachable through a validated one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct KeysFields {
    pub(crate) comment_mode: KeyBinding,
    pub(crate) show_cursor: KeyBinding,
    #[serde(default = "clear_answers_default")]
    pub(crate) clear_answers: KeyBinding,
}

/// The keys for overlay actions; no two the same.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "KeysFields", into = "KeysFields")]
#[cfg_attr(test, derive(ts_rs::TS), ts(as = "KeysFields"))]
pub(crate) struct Keys(KeysFields);

impl TryFrom<KeysFields> for Keys {
    type Error = Invalid;

    fn try_from(fields: KeysFields) -> Result<Self, Self::Error> {
        let keys = [
            &fields.comment_mode,
            &fields.show_cursor,
            &fields.clear_answers,
        ];
        for (index, key) in keys.iter().enumerate() {
            if keys.iter().skip(index + 1).any(|other| key.same_key(other)) {
                return Err(Invalid::new(format!("{} is bound twice", key.0)));
            }
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
            clear_answers: clear_answers_default(),
        })
    }
}

/// What the settings file holds. Fields a newer jamparty adds are ignored.
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
    #[serde(default)]
    pub(crate) theme: Theme,
    #[serde(default)]
    pub(crate) text_size: TextSize,
    #[serde(default)]
    pub(crate) answer_timeout: AnswerTimeout,
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
            theme: Theme::default(),
            text_size: TextSize::default(),
            answer_timeout: AnswerTimeout::default(),
        }
    }
}

/// Overrides supplied for this join, independent of the CLI parser.
#[derive(Clone, Debug, Default)]
pub(crate) struct SettingsOverride {
    pub(crate) answer_timeout: Option<AnswerTimeout>,
    pub(crate) clear_answers_key: Option<KeyBinding>,
    pub(crate) comment_mode_key: Option<KeyBinding>,
    pub(crate) show_cursor_key: Option<KeyBinding>,
}

impl SettingsOverride {
    pub(crate) fn apply(self, mut settings: Settings) -> Result<Settings, Invalid> {
        let Self {
            answer_timeout,
            clear_answers_key,
            comment_mode_key,
            show_cursor_key,
        } = self;
        let mut keys = KeysFields::from(settings.keys);
        if let Some(key) = clear_answers_key {
            keys.clear_answers = key;
        }
        if let Some(key) = comment_mode_key {
            keys.comment_mode = key;
        }
        if let Some(key) = show_cursor_key {
            keys.show_cursor = key;
        }
        settings.keys = keys.try_into()?;
        if let Some(timeout) = answer_timeout {
            settings.answer_timeout = timeout;
        }
        Ok(settings)
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
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) theme: Option<Theme>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) text_size: Option<TextSize>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub(crate) answer_timeout: Option<AnswerTimeout>,
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
            theme,
            text_size,
            answer_timeout,
        } = change;
        Self {
            name: name.or_else(|| self.name.clone()),
            keys: keys.unwrap_or_else(|| self.keys.clone()),
            comment_mode: comment_mode.unwrap_or(self.comment_mode),
            show_cursor: show_cursor.unwrap_or(self.show_cursor),
            corner: corner.unwrap_or(self.corner),
            theme: theme.unwrap_or(self.theme),
            text_size: text_size.unwrap_or(self.text_size),
            answer_timeout: answer_timeout.unwrap_or(self.answer_timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::settings::{Corner, Keys, Settings, SettingsChange, Switch, TextSize, Theme};

    #[test]
    fn defaults_and_parsing() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, Settings::default());
        assert_eq!(defaults.comment_mode, Switch::On);
        assert_eq!(defaults.show_cursor, Switch::Off);
        assert_eq!(defaults.corner, Corner::BottomRight);
        assert_eq!(
            serde_json::to_string(&defaults).unwrap(),
            r#"{"keys":{"commentMode":"c","showCursor":"x","clearAnswers":"d"},"commentMode":"on","showCursor":"off","corner":"bottom-right","theme":"auto","textSize":"default","answerTimeout":30}"#
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
        // A newer jamparty's fields are ignored.
        let newer: Settings = serde_json::from_str(r#"{"name":"alice","future":true}"#).unwrap();
        assert_eq!(newer.name.unwrap().as_str(), "alice");
    }

    #[test]
    fn theme_is_validated_in_both_settings_formats() {
        assert_eq!(Settings::default().theme, Theme::Auto);
        for (name, theme) in [
            ("auto", Theme::Auto),
            ("light", Theme::Light),
            ("dark", Theme::Dark),
        ] {
            let json = format!(r#"{{"theme":"{name}"}}"#);
            let settings: Settings = serde_json::from_str(&json).unwrap();
            let change: SettingsChange = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.theme, theme);
            assert_eq!(Settings::default().with(change), settings);
        }
        assert!(serde_json::from_str::<Settings>(r#"{"theme":"sepia"}"#).is_err());
        assert!(serde_json::from_str::<SettingsChange>(r#"{"theme":"sepia"}"#).is_err());
    }

    #[test]
    fn text_size_is_validated_and_round_trips() {
        assert_eq!(Settings::default().text_size, TextSize::Default);
        for (name, size) in [
            ("small", TextSize::Small),
            ("default", TextSize::Default),
            ("large", TextSize::Large),
            ("larger", TextSize::Larger),
        ] {
            let json = format!(r#"{{"textSize":"{name}"}}"#);
            let saved: Settings = serde_json::from_str(&json).unwrap();
            let change: SettingsChange = serde_json::from_str(&json).unwrap();
            assert_eq!(saved.text_size, size);
            assert_eq!(Settings::default().with(change), saved);
            assert_eq!(saved.with(SettingsChange::default()), saved);
            let encoded = serde_json::to_string(&saved).unwrap();
            assert_eq!(serde_json::from_str::<Settings>(&encoded).unwrap(), saved);
        }
        for json in [
            r#"{"textSize":"Larger"}"#,
            r#"{"textSize":1.5}"#,
            r#"{"textSize":"huge"}"#,
        ] {
            assert!(serde_json::from_str::<Settings>(json).is_err());
            assert!(serde_json::from_str::<SettingsChange>(json).is_err());
        }
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
    fn timeout_bounds_defaults_and_changes() {
        assert_eq!(u16::from(Settings::default().answer_timeout), 30);
        for seconds in [0, 1, 3600] {
            let text = format!(r#"{{"answerTimeout":{seconds}}}"#);
            let settings: Settings = serde_json::from_str(&text).unwrap();
            let change: SettingsChange = serde_json::from_str(&text).unwrap();
            assert_eq!(u16::from(settings.answer_timeout), seconds);
            assert_eq!(Settings::default().with(change), settings);
        }
        for raw in ["-1", "3601", "65536", "1.5", "true", r#""10""#] {
            let text = format!(r#"{{"answerTimeout":{raw}}}"#);
            assert!(serde_json::from_str::<Settings>(&text).is_err(), "{text}");
            assert!(
                serde_json::from_str::<SettingsChange>(&text).is_err(),
                "{text}"
            );
        }
        let on = Settings::default().with(serde_json::from_str(r#"{"answerTimeout":10}"#).unwrap());
        assert_eq!(
            u16::from(
                on.with(serde_json::from_str(r#"{"answerTimeout":0}"#).unwrap())
                    .answer_timeout
            ),
            0
        );
    }

    #[test]
    fn clear_answers_defaults_and_conflicts() {
        for (comment, cursor, clear, valid) in [
            ("c", "x", "d", true),
            ("k", "F2", "q", true),
            ("d", "x", "D", false),
            ("c", "D", "d", false),
            ("c", "x", "Enter", false),
            ("c", "x", "/", false),
        ] {
            let text = format!(
                r#"{{"commentMode":{comment:?},"showCursor":{cursor:?},"clearAnswers":{clear:?}}}"#
            );
            assert_eq!(serde_json::from_str::<Keys>(&text).is_ok(), valid, "{text}");
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
