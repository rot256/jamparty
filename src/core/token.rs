//! The party token `<iroh public key>:<secret>`: the share's key (how
//! joiners find and authenticate it) and the party secret (what admits
//! them).

use std::{error::Error, fmt, str::FromStr};

use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

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
///
/// Its bytes are zeroed when it is dropped, and so is every copy jampot
/// makes on the way to and from text (formatting, parsing, serde): those
/// go through fixed buffers wiped on drop, never through a `String` or
/// `Vec` that could reallocate and leave a copy behind. Copies outside
/// jampot's hands (argv, stdout's buffer, the transport's buffers) are not
/// covered.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
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

    /// Writes the text form to `out` (lowercase base32, in a buffer wiped
    /// on drop).
    fn write_text(&self, out: impl FnOnce(&str) -> fmt::Result) -> fmt::Result {
        let mut text = Zeroizing::new([0; SECRET_LEN]);
        BASE32_NOPAD.encode_mut(&self.0, text.as_mut_slice());
        text.make_ascii_lowercase();
        out(str::from_utf8(text.as_slice()).map_err(|_| fmt::Error)?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SecretError;

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a party secret is 26 characters of lowercase base32 (a-z, 2-7)")
    }
}

impl Error for SecretError {}

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
        let mut upper = Zeroizing::new([0; SECRET_LEN]);
        upper.copy_from_slice(raw.as_bytes());
        upper.make_ascii_uppercase();
        let mut secret = Self([0; SECRET_BYTES]);
        // The decoder checks that the trailing bits are zero.
        BASE32_NOPAD
            .decode_mut(upper.as_slice(), &mut secret.0)
            .map_err(|_| SecretError)?;
        Ok(secret)
    }
}

impl fmt::Display for PartySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_text(|text| f.write_str(text))
    }
}

impl Serialize for PartySecret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut result = Err(ser::Error::custom(
            "the party secret could not be formatted",
        ));
        // The only failure is the one `result` already holds.
        let _ = self.write_text(|text| {
            result = serializer.serialize_str(text);
            Ok(())
        });
        result
    }
}

impl<'de> Deserialize<'de> for PartySecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(SecretVisitor)
    }
}

/// Parses the secret from a borrowed string where the format allows (no
/// copy of ours), and wipes an owned one after parsing.
struct SecretVisitor;

impl de::Visitor<'_> for SecretVisitor {
    type Value = PartySecret;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a party secret")
    }

    fn visit_str<E: de::Error>(self, raw: &str) -> Result<Self::Value, E> {
        raw.parse().map_err(E::custom)
    }

    fn visit_string<E: de::Error>(self, raw: String) -> Result<Self::Value, E> {
        let raw = Zeroizing::new(raw);
        raw.parse().map_err(E::custom)
    }
}

impl fmt::Debug for PartySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PartySecret(..)")
    }
}

/// `<share key>:<secret>`, as printed by `jampot share` and taken by
/// `jampot join`.
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

impl Error for TokenError {}

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

    use data_encoding::BASE32_NOPAD;

    use crate::core::{
        ids::EndpointKey,
        rng::SeededRng,
        token::{PartySecret, PartyToken, SECRET_BYTES, SECRET_LEN},
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
    fn secrets_round_trip_through_json() {
        let secret = PartySecret::generate(&SeededRng::new(4));
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, format!("\"{secret}\""));
        let parsed: PartySecret = serde_json::from_str(&json).unwrap();
        assert!(parsed.matches(&secret));
        // An escape makes serde_json hand over an owned String.
        let text = secret.to_string();
        let mut chars = text.chars();
        let first = chars.next().unwrap();
        let escaped = format!("\"\\u{:04x}{}\"", u32::from(first), chars.as_str());
        let parsed: PartySecret = serde_json::from_str(&escaped).unwrap();
        assert!(parsed.matches(&secret));
        assert!(serde_json::from_str::<PartySecret>("\"nope\"").is_err());
        assert!(serde_json::from_str::<PartySecret>("7").is_err());
    }

    #[test]
    fn text_buffers_fit_exactly() {
        // encode_mut and decode_mut need exactly these lengths.
        assert_eq!(BASE32_NOPAD.encode_len(SECRET_BYTES), SECRET_LEN);
        assert_eq!(BASE32_NOPAD.decode_len(SECRET_LEN), Ok(SECRET_BYTES));
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
