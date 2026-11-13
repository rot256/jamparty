//! A [`Duration`] as whole seconds on the wire, rounded up, for serde's
//! `with` (a wait of 1.2 s is said as 2 s, never as less than it is).

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serializer};

/// The whole seconds `duration` takes, rounded up.
#[must_use]
pub(crate) fn whole(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

pub(crate) fn serialize<S: Serializer>(
    duration: &Duration,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(whole(*duration))
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Duration, D::Error> {
    u64::deserialize(deserializer).map(Duration::from_secs)
}

/// An optional [`Duration`] as whole seconds, or absent.
pub(crate) mod optional {
    use std::time::Duration;

    use serde::Serializer;

    use crate::core::secs::whole;

    pub(crate) fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(duration) => serializer.serialize_u64(whole(*duration)),
            None => serializer.serialize_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::core::secs::whole;

    #[test]
    fn rounds_up() {
        assert_eq!(whole(Duration::ZERO), 0);
        assert_eq!(whole(Duration::from_secs(6)), 6);
        assert_eq!(whole(Duration::from_millis(5001)), 6);
    }
}
