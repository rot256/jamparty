//! The party token: the share's iroh public key (how joiners find and
//! authenticate it) and the party secret (what admits them), written as
//! one base58 string.

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

/// The share's key and the party secret, written as the base58 (Bitcoin
/// alphabet: no `0`, `O`, `I` or `l`) of their 48 bytes, the 32-byte key
/// then the 16-byte secret, as `jampot serve` prints it and `jampot join`
/// takes it. That is at most 66 characters, usually 64 to 66 (the all-zero
/// token is 48 `1`s). Base58 gives each byte string exactly one spelling.
#[derive(Clone, Debug)]
pub(crate) struct PartyToken {
    pub(crate) share: EndpointKey,
    pub(crate) secret: PartySecret,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TokenError;

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "a party token is one base58 string of about 66 characters, as jampot serve prints it",
        )
    }
}

const KEY_BYTES: usize = 32;
/// The key and the secret, as the token encodes them.
const TOKEN_BYTES: usize = KEY_BYTES + SECRET_BYTES;
/// Room for the base58 text of [`TOKEN_BYTES`] bytes (at most 66
/// characters), which bs58 also uses as its working space.
const MAX_TOKEN_TEXT: usize = 70;

impl Error for TokenError {}

impl FromStr for PartyToken {
    type Err = TokenError;

    /// Surrounding whitespace (from copy and paste) is ignored. The text
    /// must decode to exactly 48 bytes; an extra leading `1` is an extra
    /// zero byte and is rejected.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > MAX_TOKEN_TEXT {
            return Err(TokenError);
        }
        let mut bytes = Zeroizing::new([0; TOKEN_BYTES]);
        let decoded = bs58::decode(raw)
            .onto(bytes.as_mut_slice())
            .map_err(|_| TokenError)?;
        if decoded != TOKEN_BYTES {
            return Err(TokenError);
        }
        let (key, rest) = bytes.split_at(KEY_BYTES);
        let mut token = Self {
            share: EndpointKey(key.try_into().map_err(|_| TokenError)?),
            secret: PartySecret([0; SECRET_BYTES]),
        };
        token.secret.0.copy_from_slice(rest);
        Ok(token)
    }
}

impl fmt::Display for PartyToken {
    /// The bytes and the text only ever sit in fixed buffers wiped on drop
    /// (the text carries the secret).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { share, secret } = self;
        let mut bytes = Zeroizing::new([0; TOKEN_BYTES]);
        let (key, rest) = bytes.split_at_mut(KEY_BYTES);
        key.copy_from_slice(&share.0);
        rest.copy_from_slice(&secret.0);
        let mut text = Zeroizing::new([0; MAX_TOKEN_TEXT]);
        let written = bs58::encode(bytes.as_slice())
            .onto(text.as_mut_slice())
            .map_err(|_| fmt::Error)?;
        let text = text.get(..written).ok_or(fmt::Error)?;
        f.write_str(str::from_utf8(text).map_err(|_| fmt::Error)?)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use data_encoding::BASE32_NOPAD;

    use crate::core::{
        ids::EndpointKey,
        rng::SeededRng,
        token::{MAX_TOKEN_TEXT, PartySecret, PartyToken, SECRET_BYTES, SECRET_LEN},
    };

    fn token(seed: u64) -> PartyToken {
        PartyToken {
            share: EndpointKey([0x5a; 32]),
            secret: PartySecret::generate(&SeededRng::new(seed)),
        }
    }

    const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

    #[test]
    fn token_round_trips() {
        let token = token(1);
        let text = token.to_string();
        assert!((64..=66).contains(&text.len()), "{text}");
        assert!(text.chars().all(|c| BASE58.contains(c)), "{text}");
        let parsed: PartyToken = format!("  {text}\n").parse().unwrap();
        assert_eq!(parsed.share, token.share);
        assert!(parsed.secret.matches(&token.secret));
        // Known answer: all-zero key and secret.
        let zeros = PartyToken {
            share: EndpointKey([0; 32]),
            secret: PartySecret([0; SECRET_BYTES]),
        };
        assert_eq!(zeros.to_string(), "1".repeat(48));
        assert!(zeros.to_string().parse::<PartyToken>().is_ok());
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
        let token = token(3);
        let good = token.to_string();
        let old_format = format!("{}:{}", token.share, token.secret);
        let mut chars = good.chars();
        let (first, rest) = (chars.next().unwrap(), chars.as_str());
        let swap = |c: char| if c == 'z' { 'y' } else { 'z' };
        for bad in [
            String::new(),
            old_format,
            format!("{good}:"),
            // Characters outside base58.
            format!("0{rest}"),
            format!("O{rest}"),
            format!("I{rest}"),
            format!("l{rest}"),
            format!("{first}{rest}="),
            // Too long for 48 bytes. (A shortened token can still be another
            // valid one: base58 carries no checksum.)
            format!("{good}{}", swap(first)),
            "z".repeat(MAX_TOKEN_TEXT + 1),
            // A leading `1` is one more (zero) byte: not 48 bytes.
            format!("1{good}"),
            "1".repeat(47),
        ] {
            assert!(bad.parse::<PartyToken>().is_err(), "{bad:?}");
        }
    }

    proptest! {
        #[test]
        fn token_parse_round_trips(key in any::<[u8; 32]>(), secret in any::<[u8; 16]>()) {
            let token = PartyToken { share: EndpointKey(key), secret: PartySecret(secret) };
            let text = token.to_string();
            prop_assert!(text.len() <= 66);
            let parsed: PartyToken = text.parse().unwrap();
            prop_assert_eq!(parsed.share, token.share);
            prop_assert!(parsed.secret.matches(&token.secret));
        }

        #[test]
        fn token_parse_never_panics(raw in "\\PC{0,80}") {
            let _ = raw.parse::<PartyToken>();
        }

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
