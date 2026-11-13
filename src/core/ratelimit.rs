//! A token bucket per key (a joiner's endpoint key, or `()` for one shared
//! bucket) on the injected clock, in its GCRA form: each key has a
//! "theoretical arrival time" instead of a token count.
//!
//! With capacity `n` per `period`, a request costs `period / n`; a joiner may
//! run up to `n` requests ahead of the steady rate.

use std::{collections::HashMap, hash::Hash};

use jiff::{SignedDuration, Timestamp};

use crate::core::clock::Clock;

/// Entries are swept when the map grows past this many joiners.
const SWEEP_THRESHOLD: usize = 1024;

/// `n` requests per `period`, as one per `interval` (`period / n`) with
/// up to `burst` (`period - interval`) taken ahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RateLimit {
    interval: SignedDuration,
    burst: SignedDuration,
}

#[cfg(test)]
impl RateLimit {
    /// The steady-state time between two requests.
    #[must_use]
    pub(crate) fn interval(self) -> SignedDuration {
        self.interval
    }
}

impl RateLimit {
    /// 10 comments per minute: one joiner's limit.
    pub(crate) const PER_JOINER: Self = Self {
        interval: SignedDuration::from_secs(6),
        burst: SignedDuration::from_secs(54),
    };
    /// 100 presence updates per 10 s from one connection: past that it
    /// floods (a joiner coalesces its own to at most 4 per second plus
    /// what its cursor needs) and is disconnected.
    pub(crate) const PRESENCE_FLOOD: Self = Self {
        interval: SignedDuration::from_millis(100),
        burst: SignedDuration::from_millis(9900),
    };
    /// 25 cursor messages per second from one connection (pages send at
    /// most about 15).
    pub(crate) const CURSOR: Self = Self {
        interval: SignedDuration::from_millis(40),
        burst: SignedDuration::from_millis(960),
    };
    /// 60 comments per minute: everyone's together.
    pub(crate) const GLOBAL: Self = Self {
        interval: SignedDuration::from_secs(1),
        burst: SignedDuration::from_secs(59),
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RateLimited {
    pub(crate) retry_after: SignedDuration,
}

#[derive(Debug)]
pub(crate) struct RateLimiter<K> {
    limit: RateLimit,
    arrival: HashMap<K, Timestamp>,
}

impl<K: Hash + Eq> RateLimiter<K> {
    #[must_use]
    pub(crate) fn new(limit: RateLimit) -> Self {
        Self {
            limit,
            arrival: HashMap::new(),
        }
    }

    /// `key`'s next arrival time if it may take a token now.
    fn next_arrival(&self, key: &K, now: Timestamp) -> Result<Timestamp, RateLimited> {
        let arrival = self
            .arrival
            .get(key)
            .copied()
            .map_or(now, |arrival| arrival.max(now));
        let ahead = arrival.duration_since(now);
        if ahead > self.limit.burst {
            return Err(RateLimited {
                retry_after: ahead - self.limit.burst,
            });
        }
        Ok(arrival.checked_add(self.limit.interval).unwrap_or(arrival))
    }

    /// Whether `key` may take a token now, without taking it.
    pub(crate) fn peek(&self, key: &K, clock: &dyn Clock) -> Result<(), RateLimited> {
        self.next_arrival(key, clock.now()).map(|_| ())
    }

    /// Takes one token for `key`, or says how long to wait.
    pub(crate) fn check(&mut self, key: K, clock: &dyn Clock) -> Result<(), RateLimited> {
        let now = clock.now();
        if self.arrival.len() > SWEEP_THRESHOLD {
            self.arrival.retain(|_, arrival| *arrival > now);
        }
        let next = self.next_arrival(&key, now)?;
        self.arrival.insert(key, next);
        Ok(())
    }

    /// Number of joiners currently tracked.
    #[cfg(test)]
    #[must_use]
    fn tracked(&self) -> usize {
        self.arrival.len()
    }
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};
    use proptest::{collection, prelude::*};

    use crate::core::{
        clock::{Clock, ManualClock},
        ids::EndpointKey,
        ratelimit::{RateLimit, RateLimited, RateLimiter},
    };

    fn key(n: u16) -> EndpointKey {
        let mut bytes = [0; 32];
        let [first, second, ..] = &mut bytes;
        [*first, *second] = n.to_le_bytes();
        EndpointKey(bytes)
    }

    fn clock() -> ManualClock {
        ManualClock::new(Timestamp::from_second(1_800_000_000).unwrap())
    }

    #[test]
    fn ten_per_minute_by_default() {
        let clock = clock();
        let mut limiter = RateLimiter::new(RateLimit::PER_JOINER);
        for _ in 0..10 {
            assert_eq!(limiter.check(key(1), &clock), Ok(()));
        }
        assert_eq!(
            limiter.check(key(1), &clock),
            Err(RateLimited {
                retry_after: SignedDuration::from_secs(6)
            })
        );
        assert_eq!(limiter.check(key(2), &clock), Ok(()), "per joiner");
        clock.advance(SignedDuration::from_secs(5));
        assert!(limiter.check(key(1), &clock).is_err());
        clock.advance(SignedDuration::from_secs(1));
        assert_eq!(limiter.check(key(1), &clock), Ok(()));
        assert!(limiter.check(key(1), &clock).is_err());
        clock.advance(SignedDuration::from_secs(60));
        for _ in 0..10 {
            assert_eq!(limiter.check(key(1), &clock), Ok(()));
        }
    }

    #[test]
    fn idle_joiners_are_swept() {
        let clock = clock();
        let mut limiter = RateLimiter::new(RateLimit::PER_JOINER);
        for joiner in 0..2000 {
            limiter.check(key(joiner), &clock).unwrap();
        }
        clock.advance(SignedDuration::from_secs(60));
        limiter.check(key(0), &clock).unwrap();
        assert_eq!(limiter.tracked(), 1);
    }

    proptest! {
        #[test]
        fn never_more_than_capacity_plus_elapsed(steps in collection::vec(0i64..20_000, 1..300)) {
            let clock = clock();
            let start = clock.now();
            let mut limiter = RateLimiter::new(RateLimit::PER_JOINER);
            let mut accepted = 0i64;
            for step in steps {
                clock.advance(SignedDuration::from_millis(step));
                if limiter.check(key(1), &clock).is_ok() {
                    accepted += 1;
                }
                let elapsed = clock.now().duration_since(start).as_millis();
                // At most 10 up front plus one per 6 s since.
                prop_assert!(i128::from(accepted) <= 10 + elapsed / 6_000);
            }
        }
    }
}
