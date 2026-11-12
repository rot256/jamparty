//! A token bucket per key (a joiner's endpoint key, or `()` for one shared
//! bucket) on the injected clock, in its GCRA form: each key has a
//! "theoretical arrival time" instead of a token count.
//!
//! With capacity `n` per `period`, a request costs `period / n`; a joiner may
//! run up to `n` requests ahead of the steady rate.

use std::{collections::HashMap, hash::Hash};
#[cfg(test)]
use std::{error::Error, fmt};

use jiff::{SignedDuration, Timestamp};

use crate::core::clock::Clock;

/// Entries are swept when the map grows past this many joiners.
const SWEEP_THRESHOLD: usize = 1024;

/// `capacity` requests per `period`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RateLimit {
    interval: SignedDuration,
    burst: SignedDuration,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvalidRateLimit;

#[cfg(test)]
impl fmt::Display for InvalidRateLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("rate limit needs a positive capacity and period")
    }
}

#[cfg(test)]
impl Error for InvalidRateLimit {}

#[cfg(test)]
impl RateLimit {
    pub(crate) fn new(capacity: u16, period: SignedDuration) -> Result<Self, InvalidRateLimit> {
        let interval = period
            .checked_div(i32::from(capacity))
            .filter(|interval| interval.is_positive())
            .ok_or(InvalidRateLimit)?;
        let burst = period.checked_sub(interval).ok_or(InvalidRateLimit)?;
        Ok(Self { interval, burst })
    }
}

impl RateLimit {
    /// 10 comments per minute: one joiner's limit.
    pub(crate) const PER_JOINER: Self = Self {
        interval: SignedDuration::from_secs(6),
        burst: SignedDuration::from_secs(54),
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
    pub(crate) fn tracked(&self) -> usize {
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
    fn configurable_and_validated() {
        assert_eq!(
            RateLimit::new(10, SignedDuration::from_secs(60)),
            Ok(RateLimit::PER_JOINER)
        );
        assert_eq!(
            RateLimit::new(60, SignedDuration::from_secs(60)),
            Ok(RateLimit::GLOBAL)
        );
        assert!(RateLimit::new(0, SignedDuration::from_secs(60)).is_err());
        assert!(RateLimit::new(1, SignedDuration::ZERO).is_err());
        let clock = clock();
        let mut limiter =
            RateLimiter::new(RateLimit::new(2, SignedDuration::from_secs(10)).unwrap());
        assert!(limiter.check(key(1), &clock).is_ok());
        assert!(limiter.check(key(1), &clock).is_ok());
        assert!(limiter.check(key(1), &clock).is_err());
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
