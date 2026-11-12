//! A per-joiner token bucket (keyed by the joiner's endpoint key) on the injected clock, in its GCRA form: each
//! joiner has a "theoretical arrival time" instead of a token count.
//!
//! With capacity `n` per `period`, a request costs `period / n`; a joiner may
//! run up to `n` requests ahead of the steady rate.

use std::collections::HashMap;
#[cfg(test)]
use std::fmt;

use jiff::{SignedDuration, Timestamp};

use crate::core::{clock::Clock, ids::EndpointKey};

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
impl std::error::Error for InvalidRateLimit {}

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

impl Default for RateLimit {
    /// 10 comments per minute.
    fn default() -> Self {
        Self {
            interval: SignedDuration::from_secs(6),
            burst: SignedDuration::from_secs(54),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RateLimited {
    pub(crate) retry_after: SignedDuration,
}

#[derive(Debug)]
pub(crate) struct RateLimiter {
    limit: RateLimit,
    arrival: HashMap<EndpointKey, Timestamp>,
}

impl RateLimiter {
    #[must_use]
    pub(crate) fn new(limit: RateLimit) -> Self {
        Self {
            limit,
            arrival: HashMap::new(),
        }
    }

    /// Takes one token for `joiner`, or says how long to wait.
    pub(crate) fn check(
        &mut self,
        joiner: EndpointKey,
        clock: &dyn Clock,
    ) -> Result<(), RateLimited> {
        let now = clock.now();
        if self.arrival.len() > SWEEP_THRESHOLD {
            self.arrival.retain(|_, arrival| *arrival > now);
        }
        let arrival = self
            .arrival
            .get(&joiner)
            .copied()
            .map_or(now, |arrival| arrival.max(now));
        let ahead = arrival.duration_since(now);
        if ahead > self.limit.burst {
            return Err(RateLimited {
                retry_after: ahead - self.limit.burst,
            });
        }
        let next = arrival.checked_add(self.limit.interval).unwrap_or(arrival);
        self.arrival.insert(joiner, next);
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
    use proptest::prelude::*;

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
        let mut limiter = RateLimiter::new(RateLimit::default());
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
            Ok(RateLimit::default())
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
        let mut limiter = RateLimiter::new(RateLimit::default());
        for joiner in 0..2000 {
            limiter.check(key(joiner), &clock).unwrap();
        }
        clock.advance(SignedDuration::from_secs(60));
        limiter.check(key(0), &clock).unwrap();
        assert_eq!(limiter.tracked(), 1);
    }

    proptest! {
        #[test]
        fn never_more_than_capacity_plus_elapsed(steps in proptest::collection::vec(0i64..20_000, 1..300)) {
            let clock = clock();
            let start = clock.now();
            let mut limiter = RateLimiter::new(RateLimit::default());
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
