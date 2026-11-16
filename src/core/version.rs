//! The iroh protocol's format version, jamparty's own version, and what to
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

/// A protocol version as a first line says it, whatever it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub(crate) struct ProtocolVersion(u32);

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The protocol version this jamparty speaks.
pub(crate) const PROTOCOL: ProtocolVersion = ProtocolVersion(1);

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

/// The share speaks another protocol than this joiner: the joiner cannot
/// go on. Says which versions run where, and what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Incompatible {
    /// The share's jamparty, if it said.
    pub(crate) share: Option<JampartyVersion>,
    /// The share's protocol version.
    pub(crate) protocol: ProtocolVersion,
}

impl fmt::Display for Incompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { share, protocol } = self;
        let ours = JampartyVersion::current();
        match share {
            Some(share) => write!(f, "the share runs jamparty {share} (protocol {protocol})")?,
            None => write!(f, "the share runs another jamparty (protocol {protocol})")?,
        }
        write!(f, "; this is jamparty {ours} (protocol {PROTOCOL}).")?;
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

/// One line for a joiner whose share runs another jamparty of the same
/// protocol: it works, but the versions differ. None when they do not.
#[must_use]
pub(crate) fn mismatch_note(share: &JampartyVersion) -> Option<String> {
    let ours = JampartyVersion::current();
    if *share == ours {
        return None;
    }
    let hint = match share.advice(&ours) {
        Some(Advice::Install(version)) => {
            format!("; consider updating: cargo install --locked jamparty@{version}")
        }
        Some(Advice::AskShare) | None => String::new(),
    };
    Some(format!(
        "the share runs jamparty {share}; this is {ours}{hint}"
    ))
}

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
    use crate::core::version::{
        Incompatible, JampartyVersion, ProtocolVersion, Version1, mismatch_note,
    };

    fn version(text: &str) -> JampartyVersion {
        JampartyVersion::try_from(text.to_owned()).unwrap()
    }

    /// A version far behind and far ahead of this one.
    const BEHIND: &str = "0.0.1";
    const AHEAD: &str = "999.0.0";

    #[test]
    fn an_incompatible_share_says_what_to_do() {
        let ours = JampartyVersion::current();
        // The joiner is behind: install the share's version.
        let behind = Incompatible {
            share: Some(version(AHEAD)),
            protocol: ProtocolVersion(2),
        };
        assert_eq!(
            behind.to_string(),
            format!(
                "the share runs jamparty {AHEAD} (protocol 2); this is jamparty {ours} (protocol 1). \
                 Install the share's version: cargo install --locked jamparty@{AHEAD}"
            )
        );
        // Ahead: the share is to update.
        let ahead = Incompatible {
            share: Some(version(BEHIND)),
            protocol: ProtocolVersion(0),
        };
        assert_eq!(
            ahead.to_string(),
            format!(
                "the share runs jamparty {BEHIND} (protocol 0); this is jamparty {ours} (protocol 1). \
                 Ask the person sharing to update jamparty."
            )
        );
        // Unknown, or not semver: no advice either way.
        let unknown = Incompatible {
            share: None,
            protocol: ProtocolVersion(2),
        };
        assert_eq!(
            unknown.to_string(),
            format!(
                "the share runs another jamparty (protocol 2); this is jamparty {ours} (protocol 1)."
            )
        );
        let odd = Incompatible {
            share: Some(version("nightly")),
            protocol: ProtocolVersion(2),
        };
        assert!(odd.to_string().ends_with("(protocol 1)."), "{odd}");
    }

    #[test]
    fn another_version_of_the_same_protocol_is_a_note() {
        let ours = JampartyVersion::current();
        assert_eq!(mismatch_note(&ours), None);
        assert_eq!(
            mismatch_note(&version(AHEAD)).unwrap(),
            format!(
                "the share runs jamparty {AHEAD}; this is {ours}; consider updating: \
                 cargo install --locked jamparty@{AHEAD}"
            )
        );
        assert_eq!(
            mismatch_note(&version(BEHIND)).unwrap(),
            format!("the share runs jamparty {BEHIND}; this is {ours}")
        );
        assert_eq!(
            mismatch_note(&version("nightly")).unwrap(),
            format!("the share runs jamparty nightly; this is {ours}")
        );
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
