//! Signed cookies for the front door and the session hosts.
//!
//! The value is base64url (no padding) of 66 bytes:
//!
//! | Bytes | Field |
//! |---|---|
//! | 0 | format version, `1` |
//! | 1 | purpose: `1` door, `2` session |
//! | 2..18 | the session's seed (ASCII), or 16 zero bytes for the door |
//! | 18..26 | GitHub user id, big endian |
//! | 26..34 | expiry, Unix seconds, big endian, signed |
//! | 34..66 | HMAC-SHA256 over `DOMAIN` and bytes 0..34 |
//!
//! A session cookie therefore fails on another session and as the door
//! cookie, and the reverse.

use std::fmt;

use data_encoding::BASE64URL_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use jiff::{SignedDuration, Timestamp};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::core::{
    clock::Clock,
    ids::{SEED_LEN, Seed, UserId},
};

/// The front door's cookie.
pub(crate) const DOOR_COOKIE: &str = "__Host-jamjam-door";
/// Every session host's cookie (each host has its own, host-only).
pub(crate) const SESSION_COOKIE: &str = "__Host-jamjam";
/// How long a minted cookie is valid.
pub(crate) const COOKIE_LIFETIME: SignedDuration = SignedDuration::from_hours(7 * 24);
/// Minimum length of the HMAC key.
pub(crate) const MIN_KEY_BYTES: usize = 32;

const DOMAIN: &[u8] = b"jamjam cookie v1\0";
const FORMAT: u8 = 1;
const PURPOSE_DOOR: u8 = 1;
const PURPOSE_SESSION: u8 = 2;
const PAYLOAD_LEN: usize = 34;
const TAG_LEN: usize = 32;
const VALUE_LEN: usize = PAYLOAD_LEN + TAG_LEN;

/// What a cookie is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CookieScope {
    Door,
    Session(Seed),
}

impl CookieScope {
    /// The cookie name used for this scope.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Door => DOOR_COOKIE,
            Self::Session(_) => SESSION_COOKIE,
        }
    }
}

/// The HMAC key (from the cookie key file).
#[derive(Clone)]
pub(crate) struct CookieKey(Vec<u8>);

impl fmt::Debug for CookieKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CookieKey(..)")
    }
}

impl TryFrom<&[u8]> for CookieKey {
    type Error = CookieError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() < MIN_KEY_BYTES {
            return Err(CookieError::ShortKey);
        }
        Ok(Self(bytes.to_vec()))
    }
}

/// A cookie that verified for the scope it was checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedCookie {
    pub(crate) user: UserId,
    pub(crate) scope: CookieScope,
    pub(crate) expires: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CookieError {
    ShortKey,
    ExpiryOutOfRange,
    Malformed,
    BadSignature,
    WrongScope,
    Expired,
}

impl fmt::Display for CookieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ShortKey => "cookie key must be at least 32 bytes",
            Self::ExpiryOutOfRange => "cookie expiry out of range",
            Self::Malformed => "malformed cookie",
            Self::BadSignature => "cookie signature mismatch",
            Self::WrongScope => "cookie is for another host",
            Self::Expired => "cookie expired",
        })
    }
}

impl std::error::Error for CookieError {}

fn tag(key: &CookieKey, payload: &[u8; PAYLOAD_LEN]) -> [u8; TAG_LEN] {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(&key.0).expect("HMAC accepts keys of any length");
    mac.update(DOMAIN);
    mac.update(payload);
    mac.finalize().into_bytes().into()
}

fn encode_payload(scope: CookieScope, user: UserId, expires: Timestamp) -> [u8; PAYLOAD_LEN] {
    let (purpose, seed) = match scope {
        CookieScope::Door => (PURPOSE_DOOR, [0; SEED_LEN]),
        CookieScope::Session(seed) => (PURPOSE_SESSION, *seed.as_bytes()),
    };
    let mut payload = [0; PAYLOAD_LEN];
    let fields = [FORMAT, purpose]
        .into_iter()
        .chain(seed)
        .chain(user.0.to_be_bytes())
        .chain(expires.as_second().to_be_bytes());
    for (out, byte) in payload.iter_mut().zip(fields) {
        *out = byte;
    }
    payload
}

/// Reads fixed-size fields from the front of a byte slice.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], CookieError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(CookieError::Malformed)?;
        self.0 = rest;
        Ok(*head)
    }
}

fn decode_payload(
    payload: &[u8; PAYLOAD_LEN],
) -> Result<(CookieScope, UserId, Timestamp), CookieError> {
    let mut reader = Reader(payload);
    let [format, purpose] = reader.take()?;
    let seed: [u8; SEED_LEN] = reader.take()?;
    let user = UserId(u64::from_be_bytes(reader.take()?));
    let expiry = i64::from_be_bytes(reader.take()?);
    if format != FORMAT {
        return Err(CookieError::Malformed);
    }
    let scope = match purpose {
        PURPOSE_DOOR if seed == [0; SEED_LEN] => CookieScope::Door,
        PURPOSE_SESSION => {
            let text = std::str::from_utf8(&seed).map_err(|_| CookieError::Malformed)?;
            CookieScope::Session(text.parse().map_err(|_| CookieError::Malformed)?)
        }
        _ => return Err(CookieError::Malformed),
    };
    let expires = Timestamp::from_second(expiry).map_err(|_| CookieError::Malformed)?;
    Ok((scope, user, expires))
}

/// Mints a cookie value for `user` in `scope`, valid for
/// [`COOKIE_LIFETIME`] from now.
pub(crate) fn mint(
    key: &CookieKey,
    scope: CookieScope,
    user: UserId,
    clock: &dyn Clock,
) -> Result<String, CookieError> {
    let expires = clock
        .now()
        .checked_add(COOKIE_LIFETIME)
        .map_err(|_| CookieError::ExpiryOutOfRange)?;
    let payload = encode_payload(scope, user, expires);
    let mut value = Vec::with_capacity(VALUE_LEN);
    value.extend_from_slice(&payload);
    value.extend_from_slice(&tag(key, &payload));
    Ok(BASE64URL_NOPAD.encode(&value))
}

/// Verifies a cookie value: signature (constant-time), then scope, then
/// expiry against `clock`.
pub(crate) fn verify(
    key: &CookieKey,
    expected: CookieScope,
    value: &str,
    clock: &dyn Clock,
) -> Result<VerifiedCookie, CookieError> {
    let bytes = BASE64URL_NOPAD
        .decode(value.as_bytes())
        .map_err(|_| CookieError::Malformed)?;
    let raw: [u8; VALUE_LEN] = bytes.try_into().map_err(|_| CookieError::Malformed)?;
    let (payload, presented) = raw
        .split_first_chunk::<PAYLOAD_LEN>()
        .ok_or(CookieError::Malformed)?;
    if !bool::from(tag(key, payload).as_slice().ct_eq(presented)) {
        return Err(CookieError::BadSignature);
    }
    let (scope, user, expires) = decode_payload(payload)?;
    if scope != expected {
        return Err(CookieError::WrongScope);
    }
    if clock.now() >= expires {
        return Err(CookieError::Expired);
    }
    Ok(VerifiedCookie {
        user,
        scope,
        expires,
    })
}

/// The `Set-Cookie` value that stores `value` under `scope`'s name.
#[must_use]
pub(crate) fn set_cookie_header(scope: CookieScope, value: &str) -> String {
    format!(
        "{}={value}; Max-Age={}; Path=/; Secure; HttpOnly; SameSite=Lax",
        scope.name(),
        COOKIE_LIFETIME.as_secs()
    )
}

/// The `Set-Cookie` value that removes `scope`'s cookie.
#[must_use]
pub(crate) fn clear_cookie_header(scope: CookieScope) -> String {
    format!(
        "{}=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Lax",
        scope.name()
    )
}

#[cfg(test)]
#[path = "cookie_tests.rs"]
mod tests;
