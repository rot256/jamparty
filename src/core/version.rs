//! The iroh protocol's format version.

use serde::{Deserialize, Serialize};

use crate::core::invalid::Invalid;

/// Version 1, the only one understood. Serialises as the number `1` and
/// rejects anything else when parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub(crate) struct Version1;

impl TryFrom<u32> for Version1 {
    type Error = Invalid;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self),
            other => Err(Invalid::new(format!(
                "unsupported version {other}, expected 1"
            ))),
        }
    }
}

impl From<Version1> for u32 {
    fn from(_: Version1) -> Self {
        1
    }
}

#[cfg(test)]
mod tests {
    use crate::core::version::Version1;

    #[test]
    fn only_one_is_accepted() {
        assert_eq!(serde_json::to_string(&Version1).unwrap(), "1");
        assert!(serde_json::from_str::<Version1>("1").is_ok());
        assert!(serde_json::from_str::<Version1>("2").is_err());
        assert!(serde_json::from_str::<Version1>("0").is_err());
        assert!(serde_json::from_str::<Version1>("\"1\"").is_err());
    }
}
