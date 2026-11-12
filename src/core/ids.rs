//! Identifiers: comment ids, endpoint keys, their fingerprints and the
//! session name `<sid>`.

use std::{fmt, str::FromStr};

use data_encoding::{BASE32_NOPAD, HEXLOWER};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::core::rng::{Rng, random_array};

/// A comment id: a random (version 4) UUID in canonical lowercase,
/// hyphenated form, as komtar's ids.
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

/// An iroh endpoint's public key (Ed25519, 32 bytes), as iroh writes it:
/// 64 lowercase hex characters (iroh's `PublicKey` `Display`). Whether the
/// bytes are a valid curve point is checked where they become an iroh
/// `PublicKey`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct EndpointKey(pub(crate) [u8; 32]);

impl EndpointKey {
    /// The short form shown next to names: the first 5 bytes in hex, as
    /// iroh's `fmt_short`.
    #[must_use]
    pub(crate) fn fingerprint(&self) -> Fingerprint {
        let [a, b, c, d, e, ..] = self.0;
        Fingerprint([a, b, c, d, e])
    }

    /// The session's host label: the first 16 characters of the lowercase
    /// base32 SHA-256 of the key bytes.
    #[must_use]
    pub(crate) fn sid(&self) -> Sid {
        let digest = Sha256::digest(self.0);
        let encoded = BASE32_NOPAD.encode(&digest).to_ascii_lowercase();
        Sid(encoded.chars().take(SID_LEN).collect())
    }
}

impl FromStr for EndpointKey {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let error = || "a key is 64 lowercase hex characters".to_owned();
        let bytes = HEXLOWER.decode(raw.as_bytes()).map_err(|_| error())?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| error())?;
        Ok(Self(key))
    }
}

impl TryFrom<String> for EndpointKey {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<EndpointKey> for String {
    fn from(key: EndpointKey) -> Self {
        key.to_string()
    }
}

impl fmt::Display for EndpointKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for EndpointKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EndpointKey({self})")
    }
}

const FINGERPRINT_BYTES: usize = 5;

/// A key's fingerprint: 10 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Fingerprint([u8; FINGERPRINT_BYTES]);

impl TryFrom<String> for Fingerprint {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let error = || "a fingerprint is 10 lowercase hex characters".to_owned();
        let bytes = HEXLOWER.decode(raw.as_bytes()).map_err(|_| error())?;
        Ok(Self(bytes.try_into().map_err(|_| error())?))
    }
}

impl From<Fingerprint> for String {
    fn from(fingerprint: Fingerprint) -> Self {
        fingerprint.to_string()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

const SID_LEN: usize = 16;

/// A session's host label (`<sid>.localhost`), derived from the share's key
/// with [`EndpointKey::sid`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Sid(String);

impl Sid {
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sid({self})")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::{
        ids::{CommentId, EndpointKey, Fingerprint},
        rng::SeededRng,
    };

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
    fn keys_are_lowercase_hex() {
        let key = EndpointKey([0xab; 32]);
        assert_eq!(key.to_string(), "ab".repeat(32));
        assert_eq!(key.to_string().parse::<EndpointKey>(), Ok(key));
        assert!("AB".repeat(32).parse::<EndpointKey>().is_err());
        assert!("ab".repeat(31).parse::<EndpointKey>().is_err());
        assert!("ab".repeat(33).parse::<EndpointKey>().is_err());
        assert_eq!(
            serde_json::to_string(&key).unwrap(),
            format!("\"{}\"", "ab".repeat(32))
        );
    }

    #[test]
    fn fingerprint_is_the_first_five_bytes() {
        let mut bytes = [0; 32];
        bytes[..6].copy_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab]);
        let fingerprint = EndpointKey(bytes).fingerprint();
        assert_eq!(fingerprint.to_string(), "0123456789");
        let json = serde_json::to_string(&fingerprint).unwrap();
        assert_eq!(
            serde_json::from_str::<Fingerprint>(&json).unwrap(),
            fingerprint
        );
        assert!(serde_json::from_str::<Fingerprint>("\"012345678\"").is_err());
        assert!(serde_json::from_str::<Fingerprint>("\"012345678A\"").is_err());
    }

    #[test]
    fn sid_known_answer() {
        // Computed independently with Python's hashlib and base64.
        assert_eq!(EndpointKey([0; 32]).sid().as_str(), "mzuhvlpymk6xo3ep");
    }

    proptest! {
        #[test]
        fn sid_is_a_hostname_label(bytes in any::<[u8; 32]>()) {
            let sid = EndpointKey(bytes).sid();
            prop_assert_eq!(sid.as_str().len(), 16);
            prop_assert!(sid.as_str().bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')));
        }

        #[test]
        fn key_round_trips(bytes in any::<[u8; 32]>()) {
            let key = EndpointKey(bytes);
            prop_assert_eq!(key.to_string().parse::<EndpointKey>(), Ok(key));
        }
    }
}
