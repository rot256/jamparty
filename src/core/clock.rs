//! Injected time source.

#[cfg(test)]
use std::sync::{Mutex, PoisonError};

#[cfg(test)]
use jiff::SignedDuration;
use jiff::Timestamp;

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
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ManualClock {
    now: Mutex<Timestamp>,
}

#[cfg(test)]
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

#[cfg(test)]
impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use crate::core::clock::{Clock, ManualClock};

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
}
