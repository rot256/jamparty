//! The party token `<iroh public key>:<secret>`: the share's key (how
//! joiners find and authenticate it) and the party secret (what admits
//! them).

use std::{fmt, str::FromStr};

use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::core::{
    ids::EndpointKey,
    rng::{Rng, random_array},
};

const SECRET_BYTES: usize = 16;
/// 128 bits in base32 without padding.
const SECRET_LEN: usize = 26;

/// The party secret: 128 random bits, written as 26 characters of lowercase
/// base32 (`a-z`, `2-7`) without padding. Compared in constant time with
/// [`PartySecret::matches`]; deliberately not `PartialEq`, and `Debug`
/// does not print it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct PartySecret([u8; SECRET_BYTES]);

impl PartySecret {
    pub(crate) fn generate(rng: &dyn Rng) -> Self {
        Self(random_array(rng))
    }

    /// Whether `presented` is this secret, in constant time.
    #[must_use]
    pub(crate) fn matches(&self, presented: &Self) -> bool {
        self.0.ct_eq(&presented.0).into()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SecretError;

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a party secret is 26 characters of lowercase base32 (a-z, 2-7)")
    }
}

impl std::error::Error for SecretError {}

impl FromStr for PartySecret {
    type Err = SecretError;

    /// Accepts only the canonical form: lowercase, no padding, and zero
    /// trailing bits (so each secret has exactly one spelling).
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let canonical_chars = raw
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'2'..=b'7'));
        if raw.len() != SECRET_LEN || !canonical_chars {
            return Err(SecretError);
        }
        let bytes = BASE32_NOPAD
            .decode(raw.to_ascii_uppercase().as_bytes())
            .map_err(|_| SecretError)?;
        Ok(Self(bytes.try_into().map_err(|_| SecretError)?))
    }
}

impl TryFrom<String> for PartySecret {
    type Error = SecretError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<PartySecret> for String {
    fn from(secret: PartySecret) -> Self {
        secret.to_string()
    }
}

impl fmt::Display for PartySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&BASE32_NOPAD.encode(&self.0).to_ascii_lowercase())
    }
}

impl fmt::Debug for PartySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PartySecret(..)")
    }
}

/// `<share key>:<secret>`, as printed by `jamjam share` and taken by
/// `jamjam join`.
#[derive(Clone, Debug)]
pub(crate) struct PartyToken {
    pub(crate) share: EndpointKey,
    pub(crate) secret: PartySecret,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TokenError;

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a party token is <64 hex characters>:<26 base32 characters>")
    }
}

impl std::error::Error for TokenError {}

impl FromStr for PartyToken {
    type Err = TokenError;

    /// Surrounding whitespace (from copy and paste) is ignored.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (share, secret) = raw.trim().split_once(':').ok_or(TokenError)?;
        Ok(Self {
            share: share.parse().map_err(|_| TokenError)?,
            secret: secret.parse().map_err(|_| TokenError)?,
        })
    }
}

impl fmt::Display for PartyToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { share, secret } = self;
        write!(f, "{share}:{secret}")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::{
        ids::EndpointKey,
        rng::SeededRng,
        token::{PartySecret, PartyToken},
    };

    fn token(seed: u64) -> PartyToken {
        PartyToken {
            share: EndpointKey([0x5a; 32]),
            secret: PartySecret::generate(&SeededRng::new(seed)),
        }
    }

    #[test]
    fn token_round_trips() {
        let token = token(1);
        let text = token.to_string();
        let (key, secret) = text.split_once(':').unwrap();
        assert_eq!(key, "5a".repeat(32));
        assert_eq!(secret.len(), 26);
        let parsed: PartyToken = format!("  {text}\n").parse().unwrap();
        assert_eq!(parsed.share, token.share);
        assert!(parsed.secret.matches(&token.secret));
    }

    #[test]
    fn secrets_compare_by_value() {
        let a = PartySecret::generate(&SeededRng::new(1));
        let b = PartySecret::generate(&SeededRng::new(2));
        assert!(a.matches(&a.clone()));
        assert!(!a.matches(&b));
        assert_eq!(format!("{a:?}"), "PartySecret(..)");
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let good = token(3).to_string();
        let (key, secret) = good.split_once(':').unwrap();
        for bad in [
            String::new(),
            key.to_owned(),
            format!("{key}:"),
            format!(":{secret}"),
            format!("{key}:{secret}:x"),
            format!("{key}{secret}"),
            format!("{}:{secret}", key.to_uppercase()),
            format!("{key}:{}", secret.to_uppercase()),
            format!("{key}:{secret}a"),
            format!("{key}:{}", &secret[1..]),
            format!("{key}:{}=", &secret[1..]),
            format!("{key}:{}1", &secret[1..]),
            // Same bits, non-zero trailing bits: not canonical.
            format!("{key}:{}", "a".repeat(25) + "b"),
        ] {
            assert!(bad.parse::<PartyToken>().is_err(), "{bad:?}");
        }
    }

    proptest! {
        #[test]
        fn secret_parse_round_trips(bytes in any::<[u8; 16]>()) {
            let secret = PartySecret(bytes);
            let parsed: PartySecret = secret.to_string().parse().unwrap();
            prop_assert!(parsed.matches(&secret));
        }

        #[test]
        fn secret_parse_never_panics(raw in "\\PC{0,40}") {
            let _ = raw.parse::<PartySecret>();
        }
    }
}
