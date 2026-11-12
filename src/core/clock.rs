//! Injected time source.

use std::sync::{Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};

/// The current time. Production uses [`SystemClock`]; tests use [`ManualClock`].
pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Wall-clock time from the operating system.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock that only moves when told to.
#[derive(Debug)]
pub(crate) struct ManualClock {
    now: Mutex<Timestamp>,
}

impl ManualClock {
    #[must_use]
    pub(crate) fn new(start: Timestamp) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Moves the clock by `step` (which may be negative).
    ///
    /// # Panics
    ///
    /// Panics if the result leaves the range of [`Timestamp`].
    pub(crate) fn advance(&self, step: SignedDuration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now
            .checked_add(step)
            .expect("manual clock advanced out of range");
    }

    pub(crate) fn set(&self, to: Timestamp) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = to;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use crate::core::clock::{Clock, ManualClock, SystemClock};

    #[test]
    fn manual_clock_moves_only_when_told() {
        let clock = ManualClock::new(Timestamp::UNIX_EPOCH);
        assert_eq!(clock.now(), Timestamp::UNIX_EPOCH);
        clock.advance(SignedDuration::from_secs(90));
        assert_eq!(clock.now().as_second(), 90);
        clock.advance(SignedDuration::from_secs(-30));
        assert_eq!(clock.now().as_second(), 60);
        clock.set(Timestamp::UNIX_EPOCH);
        assert_eq!(clock.now(), Timestamp::UNIX_EPOCH);
    }

    #[test]
    fn system_clock_is_after_2025() {
        assert!(SystemClock.now().as_second() > 1_735_689_600);
    }
}
