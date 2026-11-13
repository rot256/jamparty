//! The iroh protocol's format version, and jampot's own version.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::core::invalid::Invalid;

/// Version 1, the only one understood. Serialises as the number `1` and
/// rejects anything else when parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub(crate) struct Version1;

impl TryFrom<u32> for Version1 {
    type Error = Invalid;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self),
            other => Err(Invalid::new(format!(
                "unsupported version {other}, expected 1"
            ))),
        }
    }
}

impl From<Version1> for u32 {
    fn from(_: Version1) -> Self {
        1
    }
}

pub(crate) const MAX_JAMPOT_VERSION: usize = 64;

/// A jampot release's version, as Cargo has it (`0.1.0`, `0.2.0-rc.1`): 1
/// to 64 characters of `[0-9A-Za-z.+-]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct JampotVersion(String);

impl JampotVersion {
    /// This jampot's version.
    #[must_use]
    pub(crate) fn current() -> Self {
        Self(env!("CARGO_PKG_VERSION").to_owned())
    }
}

impl TryFrom<String> for JampotVersion {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = (1..=MAX_JAMPOT_VERSION).contains(&raw.len())
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
        if valid {
            Ok(Self(raw))
        } else {
            Err(Invalid::new(format!(
                "a jampot version is 1 to {MAX_JAMPOT_VERSION} characters of 0-9, A-Z, a-z, ., + and -"
            )))
        }
    }
}

impl From<JampotVersion> for String {
    fn from(version: JampotVersion) -> Self {
        version.0
    }
}

impl fmt::Display for JampotVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::version::{JampotVersion, Version1};

    #[test]
    fn jampot_versions() {
        assert!(JampotVersion::try_from(JampotVersion::current().to_string()).is_ok());
        assert!(JampotVersion::try_from("0.2.0-rc.1+build".to_owned()).is_ok());
        for bad in ["", "1 0", "1/0", "v\n", &"1".repeat(65)] {
            assert!(JampotVersion::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_one_is_accepted() {
        assert_eq!(serde_json::to_string(&Version1).unwrap(), "1");
        assert!(serde_json::from_str::<Version1>("1").is_ok());
        assert!(serde_json::from_str::<Version1>("2").is_err());
        assert!(serde_json::from_str::<Version1>("0").is_err());
        assert!(serde_json::from_str::<Version1>("\"1\"").is_err());
    }
}
