//! The iroh protocol's format version, jampot's own version, and what to
//! say when a share and a joiner run different ones.

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

/// The protocol version this jampot speaks.
pub(crate) const PROTOCOL: u32 = 1;

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

impl JampotVersion {
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
    Install(&'a JampotVersion),
    /// This joiner is ahead: the share is to update.
    AskShare,
}

/// The share speaks another protocol than this joiner: the joiner cannot
/// go on. Says which versions run where, and what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Incompatible {
    /// The share's jampot, if it said.
    pub(crate) share: Option<JampotVersion>,
    /// The share's protocol version.
    pub(crate) protocol: u32,
}

impl fmt::Display for Incompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { share, protocol } = self;
        let ours = JampotVersion::current();
        match share {
            Some(share) => write!(f, "the share runs jampot {share} (protocol {protocol})")?,
            None => write!(f, "the share runs another jampot (protocol {protocol})")?,
        }
        write!(f, "; this is jampot {ours} (protocol {PROTOCOL}).")?;
        match share.as_ref().and_then(|share| share.advice(&ours)) {
            Some(Advice::Install(version)) => write!(
                f,
                " Install the share's version: cargo install --locked jampot@{version}"
            ),
            Some(Advice::AskShare) => f.write_str(" Ask the person sharing to update jampot."),
            None => Ok(()),
        }
    }
}

impl std::error::Error for Incompatible {}

/// One line for a joiner whose share runs another jampot of the same
/// protocol: it works, but the versions differ. None when they do not.
#[must_use]
pub(crate) fn mismatch_note(share: &JampotVersion) -> Option<String> {
    let ours = JampotVersion::current();
    if *share == ours {
        return None;
    }
    let hint = match share.advice(&ours) {
        Some(Advice::Install(version)) => {
            format!("; consider updating: cargo install --locked jampot@{version}")
        }
        Some(Advice::AskShare) | None => String::new(),
    };
    Some(format!(
        "the share runs jampot {share}; this is {ours}{hint}"
    ))
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
    use crate::core::version::{Incompatible, JampotVersion, Version1, mismatch_note};

    fn version(text: &str) -> JampotVersion {
        JampotVersion::try_from(text.to_owned()).unwrap()
    }

    /// A version far behind and far ahead of this one.
    const BEHIND: &str = "0.0.1";
    const AHEAD: &str = "999.0.0";

    #[test]
    fn an_incompatible_share_says_what_to_do() {
        let ours = JampotVersion::current();
        // The joiner is behind: install the share's version.
        let behind = Incompatible {
            share: Some(version(AHEAD)),
            protocol: 2,
        };
        assert_eq!(
            behind.to_string(),
            format!(
                "the share runs jampot {AHEAD} (protocol 2); this is jampot {ours} (protocol 1). \
                 Install the share's version: cargo install --locked jampot@{AHEAD}"
            )
        );
        // Ahead: the share is to update.
        let ahead = Incompatible {
            share: Some(version(BEHIND)),
            protocol: 0,
        };
        assert_eq!(
            ahead.to_string(),
            format!(
                "the share runs jampot {BEHIND} (protocol 0); this is jampot {ours} (protocol 1). \
                 Ask the person sharing to update jampot."
            )
        );
        // Unknown, or not semver: no advice either way.
        let unknown = Incompatible {
            share: None,
            protocol: 2,
        };
        assert_eq!(
            unknown.to_string(),
            format!(
                "the share runs another jampot (protocol 2); this is jampot {ours} (protocol 1)."
            )
        );
        let odd = Incompatible {
            share: Some(version("nightly")),
            protocol: 2,
        };
        assert!(odd.to_string().ends_with("(protocol 1)."), "{odd}");
    }

    #[test]
    fn another_version_of_the_same_protocol_is_a_note() {
        let ours = JampotVersion::current();
        assert_eq!(mismatch_note(&ours), None);
        assert_eq!(
            mismatch_note(&version(AHEAD)).unwrap(),
            format!(
                "the share runs jampot {AHEAD}; this is {ours}; consider updating: \
                 cargo install --locked jampot@{AHEAD}"
            )
        );
        assert_eq!(
            mismatch_note(&version(BEHIND)).unwrap(),
            format!("the share runs jampot {BEHIND}; this is {ours}")
        );
        assert_eq!(
            mismatch_note(&version("nightly")).unwrap(),
            format!("the share runs jampot nightly; this is {ours}")
        );
    }

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
