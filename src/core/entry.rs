//! One-time entry tokens: the front door hands a browser to a session host.
//!
//! A token is random, single use, valid for [`ENTRY_TOKEN_LIFETIME`], and
//! bound to the user id, the seed, a nonce held by the browser, and the
//! `next` path it will redirect to. Tokens live in memory only; the store
//! keeps SHA-256 digests of tokens and nonces, never the values.

use std::{collections::HashMap, fmt};

use data_encoding::BASE64URL_NOPAD;
use jiff::{SignedDuration, Timestamp};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::core::{
    clock::Clock,
    ids::{Seed, UserId},
    rng::{Rng, random_array},
    safe_path::SafePath,
};

/// How long an entry token can be redeemed.
pub(crate) const ENTRY_TOKEN_LIFETIME: SignedDuration = SignedDuration::from_secs(60);
/// Upper bound on outstanding tokens (expired ones are purged first).
pub(crate) const MAX_PENDING_ENTRIES: usize = 10_000;

type Digest32 = [u8; 32];

fn digest(value: &str) -> Digest32 {
    Sha256::digest(value.as_bytes()).into()
}

fn random_token(rng: &dyn Rng) -> String {
    BASE64URL_NOPAD.encode(&random_array::<32>(rng))
}

/// A one-time entry token (32 random bytes, base64url).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct EntryToken(String);

impl EntryToken {
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EntryToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EntryToken(..)")
    }
}

/// The browser-held nonce a token is bound to (32 random bytes, base64url).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct EntryNonce(String);

impl EntryNonce {
    pub(crate) fn generate(rng: &dyn Rng) -> Self {
        Self(random_token(rng))
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EntryNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EntryNonce(..)")
    }
}

#[derive(Debug)]
struct Pending {
    user: UserId,
    seed: Seed,
    nonce: Digest32,
    next: SafePath,
    expires: Timestamp,
}

/// What a redeemed token grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Redeemed {
    pub(crate) user: UserId,
    pub(crate) next: SafePath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryError {
    /// Too many outstanding tokens.
    Full,
    /// Unknown, already used, expired, or bound to another seed or nonce.
    /// One variant for all of them, so callers answer identically.
    Invalid,
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "too many pending entry tokens",
            Self::Invalid => "invalid or expired entry token",
        })
    }
}

impl std::error::Error for EntryError {}

/// Outstanding entry tokens.
#[derive(Debug, Default)]
pub(crate) struct EntryStore {
    pending: HashMap<Digest32, Pending>,
}

impl EntryStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Number of outstanding (possibly expired, not yet purged) tokens.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn purge(&mut self, now: Timestamp) {
        self.pending.retain(|_, pending| pending.expires > now);
    }

    /// Issues a token for `user` to enter `seed`, bound to `nonce` and `next`.
    pub(crate) fn issue(
        &mut self,
        rng: &dyn Rng,
        clock: &dyn Clock,
        user: UserId,
        seed: Seed,
        nonce: &EntryNonce,
        next: SafePath,
    ) -> Result<EntryToken, EntryError> {
        let now = clock.now();
        self.purge(now);
        if self.pending.len() >= MAX_PENDING_ENTRIES {
            return Err(EntryError::Full);
        }
        let expires = now
            .checked_add(ENTRY_TOKEN_LIFETIME)
            .map_err(|_| EntryError::Full)?;
        let token = random_token(rng);
        self.pending.insert(
            digest(&token),
            Pending {
                user,
                seed,
                nonce: digest(nonce.as_str()),
                next,
                expires,
            },
        );
        Ok(EntryToken(token))
    }

    /// Redeems `token` on `seed`'s host with the browser's `nonce`. The token
    /// is consumed by any attempt, successful or not.
    pub(crate) fn redeem(
        &mut self,
        clock: &dyn Clock,
        token: &str,
        seed: &Seed,
        nonce: &str,
    ) -> Result<Redeemed, EntryError> {
        let now = clock.now();
        let pending = self.pending.remove(&digest(token));
        self.purge(now);
        let pending = pending.ok_or(EntryError::Invalid)?;
        let nonce_matches = bool::from(pending.nonce.ct_eq(&digest(nonce)));
        if pending.expires <= now || pending.seed != *seed || !nonce_matches {
            return Err(EntryError::Invalid);
        }
        Ok(Redeemed {
            user: pending.user,
            next: pending.next,
        })
    }
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use crate::core::{
        clock::ManualClock,
        entry::{ENTRY_TOKEN_LIFETIME, EntryError, EntryNonce, EntryStore, MAX_PENDING_ENTRIES},
        ids::{Seed, UserId},
        rng::SeededRng,
        safe_path::SafePath,
    };

    struct Fixture {
        rng: SeededRng,
        clock: ManualClock,
        store: EntryStore,
        seed: Seed,
        nonce: EntryNonce,
    }

    fn fixture() -> Fixture {
        let rng = SeededRng::new(9);
        let nonce = EntryNonce::generate(&rng);
        Fixture {
            rng,
            clock: ManualClock::new(Timestamp::from_second(1_800_000_000).unwrap()),
            store: EntryStore::new(),
            seed: "aaaaaaaaaaaaaaaa".parse().unwrap(),
            nonce,
        }
    }

    impl Fixture {
        fn issue(&mut self) -> String {
            let next = str::parse::<SafePath>("/page?x=1").unwrap();
            self.store
                .issue(
                    &self.rng,
                    &self.clock,
                    UserId(7),
                    self.seed,
                    &self.nonce,
                    next,
                )
                .unwrap()
                .as_str()
                .to_owned()
        }
    }

    #[test]
    fn redeem_once() {
        let mut f = fixture();
        let token = f.issue();
        let redeemed = f
            .store
            .redeem(&f.clock, &token, &f.seed, f.nonce.as_str())
            .unwrap();
        assert_eq!(redeemed.user, UserId(7));
        assert_eq!(redeemed.next.as_str(), "/page?x=1");
        assert_eq!(
            f.store.redeem(&f.clock, &token, &f.seed, f.nonce.as_str()),
            Err(EntryError::Invalid)
        );
        assert!(f.store.is_empty());
    }

    #[test]
    fn expires_after_sixty_seconds() {
        let mut f = fixture();
        let token = f.issue();
        f.clock
            .advance(ENTRY_TOKEN_LIFETIME - SignedDuration::from_secs(1));
        assert!(
            f.store
                .redeem(&f.clock, &token, &f.seed, f.nonce.as_str())
                .is_ok()
        );

        let token = f.issue();
        f.clock.advance(ENTRY_TOKEN_LIFETIME);
        assert_eq!(
            f.store.redeem(&f.clock, &token, &f.seed, f.nonce.as_str()),
            Err(EntryError::Invalid)
        );
    }

    #[test]
    fn bound_to_seed_and_nonce_and_consumed_on_failure() {
        let mut f = fixture();
        let other_seed: Seed = "bbbbbbbbbbbbbbbb".parse().unwrap();
        let other_nonce = EntryNonce::generate(&f.rng);

        let token = f.issue();
        assert_eq!(
            f.store
                .redeem(&f.clock, &token, &other_seed, f.nonce.as_str()),
            Err(EntryError::Invalid)
        );
        assert_eq!(
            f.store.redeem(&f.clock, &token, &f.seed, f.nonce.as_str()),
            Err(EntryError::Invalid),
            "a failed attempt consumes the token"
        );

        let token = f.issue();
        assert_eq!(
            f.store
                .redeem(&f.clock, &token, &f.seed, other_nonce.as_str()),
            Err(EntryError::Invalid)
        );
        assert_eq!(
            f.store
                .redeem(&f.clock, "nonsense", &f.seed, f.nonce.as_str()),
            Err(EntryError::Invalid)
        );
    }

    #[test]
    fn tokens_are_distinct_and_expired_ones_are_purged() {
        let mut f = fixture();
        let a = f.issue();
        let b = f.issue();
        assert_ne!(a, b);
        assert_eq!(f.store.len(), 2);
        f.clock.advance(ENTRY_TOKEN_LIFETIME);
        let _ = f.issue();
        assert_eq!(f.store.len(), 1);
    }

    #[test]
    fn store_is_bounded() {
        let mut f = fixture();
        for _ in 0..MAX_PENDING_ENTRIES {
            let _ = f.issue();
        }
        let next = SafePath::root();
        assert_eq!(
            f.store
                .issue(&f.rng, &f.clock, UserId(1), f.seed, &f.nonce, next.clone()),
            Err(EntryError::Full)
        );
        f.clock.advance(ENTRY_TOKEN_LIFETIME);
        assert!(
            f.store
                .issue(&f.rng, &f.clock, UserId(1), f.seed, &f.nonce, next)
                .is_ok()
        );
    }
}
