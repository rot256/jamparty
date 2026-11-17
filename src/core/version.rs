//! Release-line compatibility and the independent NDJSON format version.

use std::{cmp::Ordering, fmt};

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

const MAX_JAMPARTY_VERSION: usize = 64;

/// A jamparty release's version, as Cargo has it (`0.1.0`, `0.2.0-rc.1`): 1
/// to 64 characters of `[0-9A-Za-z.+-]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct JampartyVersion(String);

impl JampartyVersion {
    /// This jamparty's version.
    #[must_use]
    pub(crate) fn current() -> Self {
        Self(env!("CARGO_PKG_VERSION").to_owned())
    }
}

impl JampartyVersion {
    /// Before 1.0, compatibility follows the minor (0.x); from 1.0 on,
    /// it follows the major. Prerelease and build suffixes do not change it.
    pub(crate) fn compatible(&self, other: &Self) -> bool {
        let line = |version: &Self| {
            let parsed = semver::Version::parse(&version.0).ok()?;
            Some((
                parsed.major,
                if parsed.major == 0 { parsed.minor } else { 0 },
            ))
        };
        matches!((line(self), line(other)), (Some(a), Some(b)) if a == b)
    }

    /// How this version compares with `other` as semver; none when either
    /// is not semver.
    fn compare(&self, other: &Self) -> Option<Ordering> {
        let mine = semver::Version::parse(&self.0).ok()?;
        let theirs = semver::Version::parse(&other.0).ok()?;
        Some(mine.cmp(&theirs))
    }

    /// What to do about a share that runs `self`, said to a joiner that
    /// runs `ours`: install the share's version when ours is older, ask
    /// for an update when it is newer, nothing when equal or unknown.
    fn advice(&self, ours: &Self) -> Option<Advice<'_>> {
        match self.compare(ours)? {
            Ordering::Greater => Some(Advice::Install(self)),
            Ordering::Less => Some(Advice::AskShare),
            Ordering::Equal => None,
        }
    }
}

/// What a joiner can do about another version at the share.
enum Advice<'a> {
    /// This joiner is behind: install the share's version.
    Install(&'a JampartyVersion),
    /// This joiner is ahead: the share is to update.
    AskShare,
}

/// The share speaks another release line than this joiner: the joiner cannot
/// go on. Says which versions run where, and what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Incompatible {
    /// The share's jamparty, if it said.
    pub(crate) share: Option<JampartyVersion>,
}

impl fmt::Display for Incompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { share } = self;
        let ours = JampartyVersion::current();
        match share {
            Some(share) => write!(f, "the share runs jamparty {share}")?,
            None => write!(f, "the share runs another jamparty")?,
        }
        write!(f, "; this is jamparty {ours}.")?;
        match share.as_ref().and_then(|share| share.advice(&ours)) {
            Some(Advice::Install(version)) => write!(
                f,
                " Install the share's version: cargo install --locked jamparty@{version}"
            ),
            Some(Advice::AskShare) => f.write_str(" Ask the person sharing to update jamparty."),
            None => Ok(()),
        }
    }
}

impl std::error::Error for Incompatible {}

impl TryFrom<String> for JampartyVersion {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = (1..=MAX_JAMPARTY_VERSION).contains(&raw.len())
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
        if valid {
            Ok(Self(raw))
        } else {
            Err(Invalid::new(format!(
                "a jamparty version is 1 to {MAX_JAMPARTY_VERSION} characters of 0-9, A-Z, a-z, ., + and -"
            )))
        }
    }
}

impl From<JampartyVersion> for String {
    fn from(version: JampartyVersion) -> Self {
        version.0
    }
}

impl fmt::Display for JampartyVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::version::{Incompatible, JampartyVersion, Version1};

    fn version(text: &str) -> JampartyVersion {
        JampartyVersion::try_from(text.to_owned()).unwrap()
    }

    #[test]
    fn release_lines() {
        for (left, right, compatible) in [
            ("0.2.0", "0.2.99", true),
            ("0.2.0-rc.1", "0.2.0", true),
            ("0.2.0-rc.1+build", "0.2.9", true),
            ("0.1.9", "0.2.0", false),
            ("1.0.0", "1.99.0", true),
            ("1.9.0", "2.0.0", false),
            ("0.9.0", "1.0.0", false),
            ("nightly", "nightly", false),
        ] {
            assert_eq!(version(left).compatible(&version(right)), compatible);
            assert_eq!(version(right).compatible(&version(left)), compatible);
        }
    }

    #[test]
    fn an_incompatible_share_says_what_to_do() {
        let ours = JampartyVersion::current();
        let behind = Incompatible {
            share: Some(version("999.0.0")),
        };
        assert_eq!(
            behind.to_string(),
            format!(
                "the share runs jamparty 999.0.0; this is jamparty {ours}. Install the share's version: cargo install --locked jamparty@999.0.0"
            )
        );
        let ahead = Incompatible {
            share: Some(version("0.0.1")),
        };
        assert_eq!(
            ahead.to_string(),
            format!(
                "the share runs jamparty 0.0.1; this is jamparty {ours}. Ask the person sharing to update jamparty."
            )
        );
        for share in [None, Some(version("nightly"))] {
            let message = Incompatible { share }.to_string();
            assert!(!message.contains("cargo install"), "{message}");
        }
    }

    #[test]
    fn jamparty_versions() {
        assert!(JampartyVersion::try_from(JampartyVersion::current().to_string()).is_ok());
        assert!(JampartyVersion::try_from("0.2.0-rc.1+build".to_owned()).is_ok());
        for bad in ["", "1 0", "1/0", "v\n", &"1".repeat(65)] {
            assert!(
                JampartyVersion::try_from(bad.to_owned()).is_err(),
                "{bad:?}"
            );
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
