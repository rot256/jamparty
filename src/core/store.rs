//! The server-written JSON files: `sessions.json` and `keys.json`.
//!
//! Both are versioned containers, `{"version": 1, ...}`. Loading rejects
//! unknown fields and duplicate seeds or keys, so a damaged file stops
//! startup instead of being half-read.

use std::collections::BTreeSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::core::{
    ids::{ShareKey, UserId},
    pairing::PairingLabel,
    session::Session,
    version::Version1,
};

/// A paired share key, as stored in `keys.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyRecord {
    pub(crate) key: ShareKey,
    pub(crate) user_id: UserId,
    pub(crate) label: PairingLabel,
    pub(crate) added: Timestamp,
    #[serde(default)]
    pub(crate) last_seen: Option<Timestamp>,
}

/// `sessions.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "RawSessions")]
pub(crate) struct SessionsFile {
    version: Version1,
    sessions: Vec<Session>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSessions {
    version: Version1,
    sessions: Vec<Session>,
}

impl TryFrom<RawSessions> for SessionsFile {
    type Error = String;

    fn try_from(raw: RawSessions) -> Result<Self, Self::Error> {
        Self::new(raw.sessions)
    }
}

impl SessionsFile {
    pub(crate) fn new(sessions: Vec<Session>) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        if let Some(duplicate) = sessions.iter().find(|session| !seen.insert(session.seed())) {
            return Err(format!("duplicate session {}", duplicate.seed()));
        }
        Ok(Self {
            version: Version1,
            sessions,
        })
    }

    #[must_use]
    pub(crate) fn sessions(&self) -> &[Session] {
        &self.sessions
    }
}

/// `keys.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "RawKeys")]
pub(crate) struct KeysFile {
    version: Version1,
    keys: Vec<KeyRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    version: Version1,
    keys: Vec<KeyRecord>,
}

impl TryFrom<RawKeys> for KeysFile {
    type Error = String;

    fn try_from(raw: RawKeys) -> Result<Self, Self::Error> {
        Self::new(raw.keys)
    }
}

impl KeysFile {
    pub(crate) fn new(keys: Vec<KeyRecord>) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        if let Some(duplicate) = keys.iter().find(|record| !seen.insert(record.key)) {
            return Err(format!("duplicate key {}", duplicate.key));
        }
        Ok(Self {
            version: Version1,
            keys,
        })
    }

    #[must_use]
    pub(crate) fn keys(&self) -> &[KeyRecord] {
        &self.keys
    }
}

#[cfg(test)]
mod tests {
    use crate::core::store::{KeysFile, SessionsFile};

    const SESSION: &str = r#"{
        "seed": "k3x7q2mfa7wd5brt",
        "label": "landing page review",
        "owner": 1234567,
        "key": "4f1ca90e77b2000000000000000000000000000000000000000000000000beef",
        "created": "2026-11-12T16:00:00Z",
        "expires": "2026-11-19T16:00:00Z",
        "access": [2345678, 3456789],
        "client": { "kind": "t3", "thread": "1b3f246a", "title": "Landing page redesign" }
    }"#;

    const KEY: &str = r#"{
        "key": "4f1ca90e77b2000000000000000000000000000000000000000000000000beef",
        "user_id": 1234567,
        "label": "t3@mtrx",
        "added": "2026-11-12T16:00:00Z",
        "last_seen": null
    }"#;

    #[test]
    fn sessions_file_round_trip() {
        let text = format!(r#"{{"version": 1, "sessions": [{SESSION}]}}"#);
        let file: SessionsFile = serde_json::from_str(&text).unwrap();
        assert_eq!(file.sessions().len(), 1);
        let again: SessionsFile =
            serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
        assert_eq!(again, file);
        let value: serde_json::Value = serde_json::to_value(&file).unwrap();
        assert_eq!(value.pointer("/version"), Some(&serde_json::json!(1)));
        assert_eq!(
            value.pointer("/sessions/0/client/kind"),
            Some(&serde_json::json!("t3"))
        );
    }

    #[test]
    fn sessions_file_rejects_damage() {
        for bad in [
            format!(r#"{{"version": 2, "sessions": [{SESSION}]}}"#),
            format!(r#"{{"sessions": [{SESSION}]}}"#),
            format!(r#"{{"version": 1, "sessions": [{SESSION}, {SESSION}]}}"#),
            format!(r#"{{"version": 1, "sessions": [{SESSION}], "extra": 1}}"#),
            r#"{"version": 1, "sessions": [{"seed": "x"}]}"#.to_owned(),
            format!(
                r#"{{"version": 1, "sessions": [{}]}}"#,
                SESSION.replace("[2345678, 3456789]", "[]")
            ),
            format!(
                r#"{{"version": 1, "sessions": [{}]}}"#,
                SESSION.replace("[2345678, 3456789]", "[3456789, 2345678]")
            ),
            format!(
                r#"{{"version": 1, "sessions": [{}]}}"#,
                SESSION.replace("2026-11-19", "2026-11-11")
            ),
            format!(
                r#"{{"version": 1, "sessions": [{}]}}"#,
                SESSION.replace("\"owner\"", "\"other\": 1, \"owner\"")
            ),
            r#"{"version": 1, "sessions": ["#.to_owned(),
        ] {
            assert!(serde_json::from_str::<SessionsFile>(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keys_file_round_trip_and_duplicates() {
        let text = format!(r#"{{"version": 1, "keys": [{KEY}]}}"#);
        let file: KeysFile = serde_json::from_str(&text).unwrap();
        assert_eq!(file.keys().len(), 1);
        let again: KeysFile = serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
        assert_eq!(again, file);
        let duplicate = format!(r#"{{"version": 1, "keys": [{KEY}, {KEY}]}}"#);
        assert!(serde_json::from_str::<KeysFile>(&duplicate).is_err());
        let bad_label = text.replace("t3@mtrx", "t3 mtrx");
        assert!(serde_json::from_str::<KeysFile>(&bad_label).is_err());
    }
}
