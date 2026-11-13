//! A TCP port someone can connect to: 1 to 65535.

use std::{fmt, num::NonZeroU16, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::core::invalid::Invalid;

/// A TCP port other than 0 (which only means "any" when binding).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub(crate) struct Port(NonZeroU16);

impl Port {
    /// HTTP's default port, which origins leave out.
    pub(crate) const HTTP: Self = Self(NonZeroU16::new(80).expect("80 is not 0"));
}

impl TryFrom<u16> for Port {
    type Error = Invalid;

    fn try_from(raw: u16) -> Result<Self, Self::Error> {
        NonZeroU16::new(raw)
            .map(Self)
            .ok_or_else(|| Invalid::new("a port is 1 to 65535"))
    }
}

impl From<Port> for u16 {
    fn from(port: Port) -> Self {
        port.0.get()
    }
}

impl FromStr for Port {
    type Err = Invalid;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        raw.parse::<u16>()
            .map_err(|_| Invalid::new("a port is 1 to 65535"))?
            .try_into()
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::port::Port;

    #[test]
    fn ports_are_1_to_65535() {
        assert_eq!("1313".parse::<Port>().map(u16::from), Ok(1313));
        assert_eq!("65535".parse::<Port>().map(u16::from), Ok(65535));
        for bad in ["0", "65536", "-1", "", " 80", "http"] {
            assert!(bad.parse::<Port>().is_err(), "{bad}");
        }
        assert!(Port::try_from(0).is_err());
        assert_eq!(
            serde_json::to_string(&Port::try_from(80).unwrap()).unwrap(),
            "80"
        );
        assert!(serde_json::from_str::<Port>("0").is_err());
    }
}
