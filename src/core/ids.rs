//! Identifiers: session seeds, comment ids, sequence numbers, GitHub users
//! and share keys.

use std::{fmt, str::FromStr};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::rng::{Rng, random_array};

/// Number of characters in a seed: 16 base32 characters carry 80 bits.
pub(crate) const SEED_LEN: usize = 16;

/// A session seed: 16 characters of lowercase RFC 4648 base32
/// (`a-z`, `2-7`), no padding. Every such string is canonical, since
/// 16 x 5 bits is exactly 80 bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Seed([u8; SEED_LEN]);

fn base32_char(value: u8) -> u8 {
    match value & 31 {
        letter @ 0..=25 => b'a' + letter,
        digit => b'2' + (digit - 26),
    }
}

fn is_base32_char(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'2'..=b'7')
}

impl Seed {
    /// Draws a fresh seed (80 bits) from `rng`.
    pub(crate) fn generate(rng: &dyn Rng) -> Self {
        Self(random_array::<SEED_LEN>(rng).map(base32_char))
    }

    /// The seed's characters as ASCII bytes.
    #[must_use]
    pub(crate) fn as_bytes(&self) -> &[u8; SEED_LEN] {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SeedError;

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a seed is 16 characters of lowercase base32 (a-z, 2-7)")
    }
}

impl std::error::Error for SeedError {}

impl FromStr for Seed {
    type Err = SeedError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let bytes: [u8; SEED_LEN] = raw.as_bytes().try_into().map_err(|_| SeedError)?;
        if bytes.iter().all(|byte| is_base32_char(*byte)) {
            Ok(Self(bytes))
        } else {
            Err(SeedError)
        }
    }
}

impl TryFrom<String> for Seed {
    type Error = SeedError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<Seed> for String {
    fn from(seed: Seed) -> Self {
        seed.to_string()
    }
}

impl fmt::Display for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| fmt::Write::write_char(f, char::from(*byte)))
    }
}

impl fmt::Debug for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Seed({self})")
    }
}

/// A GitHub numeric user id: the only identity jamjam compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct UserId(pub(crate) u64);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A GitHub login: 1 to 39 ASCII letters, digits and hyphens. Logins are
/// display data only (they can be renamed); access uses [`UserId`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct GithubLogin(String);

impl GithubLogin {
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GithubLogin {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = (1..=39).contains(&raw.len())
            && raw
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if valid {
            Ok(Self(raw))
        } else {
            Err("a GitHub login is 1 to 39 letters, digits or hyphens".to_owned())
        }
    }
}

impl From<GithubLogin> for String {
    fn from(login: GithubLogin) -> Self {
        login.0
    }
}

impl fmt::Display for GithubLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A comment's sequence number within its session's queue. Numbers start
/// at 1; [`Seq::ZERO`] is the cursor before the first comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Seq(pub(crate) u64);

impl Seq {
    pub(crate) const ZERO: Self = Self(0);

    /// The following sequence number (saturating; 2^64 comments do not occur).
    #[must_use]
    pub(crate) fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A comment id: a random (version 4) UUID in canonical lowercase,
/// hyphenated form, as komtar's ids. Random rather than sequential so that
/// ids from before a server restart are never reused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct CommentId(Uuid);

impl CommentId {
    pub(crate) fn generate(rng: &dyn Rng) -> Self {
        Self(uuid::Builder::from_random_bytes(random_array(rng)).into_uuid())
    }
}

impl FromStr for CommentId {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let error = || "a comment id is a lowercase hyphenated UUID".to_owned();
        let uuid = Uuid::try_parse(raw).map_err(|_| error())?;
        if uuid.hyphenated().to_string() == raw {
            Ok(Self(uuid))
        } else {
            Err(error())
        }
    }
}

impl TryFrom<String> for CommentId {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<CommentId> for String {
    fn from(id: CommentId) -> Self {
        id.to_string()
    }
}

impl fmt::Display for CommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl fmt::Debug for CommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommentId({self})")
    }
}

/// A share's iroh node id: the 32 bytes of an Ed25519 public key.
/// Serialised as 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct ShareKey(pub(crate) [u8; 32]);

impl FromStr for ShareKey {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let error = || "a share key is 64 lowercase hex characters".to_owned();
        let bytes = HEXLOWER.decode(raw.as_bytes()).map_err(|_| error())?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| error())?;
        Ok(Self(key))
    }
}

impl TryFrom<String> for ShareKey {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<ShareKey> for String {
    fn from(key: ShareKey) -> Self {
        key.to_string()
    }
}

impl fmt::Display for ShareKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for ShareKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShareKey({self})")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::{
        ids::{CommentId, GithubLogin, Seed, Seq, ShareKey},
        rng::SeededRng,
    };

    #[test]
    fn seeds_are_generated_in_the_alphabet() {
        let rng = SeededRng::new(1);
        for _ in 0..200 {
            let seed = Seed::generate(&rng);
            let text = seed.to_string();
            assert_eq!(text.len(), 16);
            assert_eq!(text.parse::<Seed>(), Ok(seed));
        }
        assert_ne!(Seed::generate(&rng), Seed::generate(&rng));
    }

    #[test]
    fn seed_parsing_is_strict() {
        assert!("k3x7q2mfa7wd5brt".parse::<Seed>().is_ok());
        for bad in [
            "",
            "k3x7q2mfa7wd5br",
            "k3x7q2mfa7wd5brtt",
            "K3x9q2mfa7wd5brt",
            "k3x7q2mfa7wd5br1",
            "k3x7q2mfa7wd5br8",
            "k3x7q2mfa7wd5br=",
            "k3x7q2mfa7wd5br.",
            "k3x7q2mfa7wd5bré",
        ] {
            assert!(bad.parse::<Seed>().is_err(), "{bad:?}");
        }
        let json = serde_json::to_string(&"abcdefghijklmn23".parse::<Seed>().unwrap()).unwrap();
        assert_eq!(json, "\"abcdefghijklmn23\"");
        assert!(serde_json::from_str::<Seed>("\"ABCDEFGHIJKLMN23\"").is_err());
    }

    #[test]
    fn comment_ids_are_canonical_uuids() {
        let id = CommentId::generate(&SeededRng::new(3));
        let text = id.to_string();
        assert_eq!(text.parse::<CommentId>(), Ok(id));
        assert_eq!(text.chars().nth(14), Some('4'));
        assert!(text.to_uppercase().parse::<CommentId>().is_err());
        assert!(text.replace('-', "").parse::<CommentId>().is_err());
    }

    #[test]
    fn logins_and_keys() {
        assert!(GithubLogin::try_from("rot256".to_owned()).is_ok());
        assert!(GithubLogin::try_from("a-b".to_owned()).is_ok());
        assert!(GithubLogin::try_from(String::new()).is_err());
        assert!(GithubLogin::try_from("a b".to_owned()).is_err());
        assert!(GithubLogin::try_from("x".repeat(40)).is_err());

        let key = ShareKey([0xab; 32]);
        assert_eq!(key.to_string().parse::<ShareKey>(), Ok(key));
        assert!("AB".repeat(32).parse::<ShareKey>().is_err());
        assert!("ab".repeat(31).parse::<ShareKey>().is_err());
    }

    #[test]
    fn seq_order() {
        assert!(Seq::ZERO < Seq::ZERO.next());
        assert_eq!(Seq(u64::MAX).next(), Seq(u64::MAX));
    }

    proptest! {
        #[test]
        fn seed_parse_accepts_exactly_the_alphabet(raw in "\\PC{0,20}") {
            let valid = raw.len() == 16
                && raw.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'));
            prop_assert_eq!(raw.parse::<Seed>().is_ok(), valid);
        }
    }
}
