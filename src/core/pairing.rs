//! Pairing codes: `jamjam1-<key>-<checksum>-<label>`.
//!
//! - `key`: the share's Ed25519 public key (iroh node id), 64 lowercase hex
//!   characters.
//! - `checksum`: 8 lowercase hex characters, the first 4 bytes of
//!   SHA-256(`DOMAIN` || key || label). It covers key and label, so any
//!   typo is caught except with probability 2^-32.
//! - `label`: the suggested device label, see [`PairingLabel`]. It may
//!   contain `-`; the code is split on the first three hyphens.
//!
//! Hex is case-sensitive (lowercase only); surrounding whitespace from
//! pasting is ignored. Whether the key is a valid Ed25519 point is the
//! transport's concern.

use std::{fmt, str::FromStr};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::ids::ShareKey;

const PREFIX: &str = "jamjam1";
const DOMAIN: &[u8] = b"jamjam pairing v1\0";
/// Maximum label length in characters.
pub(crate) const MAX_LABEL_LEN: usize = 40;

/// A device label: 1 to 40 characters of ASCII letters, digits and
/// `.`, `_`, `@`, `+`, `-`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct PairingLabel(String);

impl PairingLabel {
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_label_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'@' | b'+' | b'-')
}

impl TryFrom<String> for PairingLabel {
    type Error = PairingError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        if (1..=MAX_LABEL_LEN).contains(&raw.len()) && raw.bytes().all(is_label_byte) {
            Ok(Self(raw))
        } else {
            Err(PairingError::Label)
        }
    }
}

impl From<PairingLabel> for String {
    fn from(label: PairingLabel) -> Self {
        label.0
    }
}

impl fmt::Display for PairingLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PairingError {
    Format,
    Key,
    Label,
    Checksum,
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Format => "not a jamjam pairing code (jamjam1-<key>-<checksum>-<label>)",
            Self::Key => "the key in the pairing code is not 64 lowercase hex characters",
            Self::Label => "the label must be 1 to 40 letters, digits or . _ @ + -",
            Self::Checksum => "the pairing code's checksum does not match; check for typos",
        })
    }
}

impl std::error::Error for PairingError {}

/// A parsed pairing code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PairingCode {
    pub(crate) key: ShareKey,
    pub(crate) label: PairingLabel,
}

fn checksum(key: &ShareKey, label: &PairingLabel) -> String {
    let digest = Sha256::new()
        .chain_update(DOMAIN)
        .chain_update(key.0)
        .chain_update(label.as_str())
        .finalize();
    HEXLOWER.encode(digest.get(..4).unwrap_or_default())
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { key, label } = self;
        write!(f, "{PREFIX}-{key}-{}-{label}", checksum(key, label))
    }
}

impl FromStr for PairingCode {
    type Err = PairingError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut parts = raw.trim().splitn(4, '-');
        let (Some(PREFIX), Some(key), Some(sum), Some(label)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(PairingError::Format);
        };
        let key: ShareKey = key.parse().map_err(|_| PairingError::Key)?;
        let label = PairingLabel::try_from(label.to_owned())?;
        if checksum(&key, &label) != sum {
            return Err(PairingError::Checksum);
        }
        Ok(Self { key, label })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::{
        ids::ShareKey,
        pairing::{PairingCode, PairingError, PairingLabel},
    };

    fn code(label: &str) -> PairingCode {
        PairingCode {
            key: ShareKey([0x4f; 32]),
            label: PairingLabel::try_from(label.to_owned()).unwrap(),
        }
    }

    #[test]
    fn format_and_parse() {
        let text = code("t3@mtrx").to_string();
        assert!(text.starts_with(&format!("jamjam1-{}-", "4f".repeat(32))));
        assert!(text.ends_with("-t3@mtrx"));
        assert_eq!(text.len(), 8 + 64 + 1 + 8 + 1 + 7);
        assert_eq!(text.parse::<PairingCode>(), Ok(code("t3@mtrx")));
        assert_eq!(
            format!("  {text}\n").parse::<PairingCode>(),
            Ok(code("t3@mtrx"))
        );
        let dashed = code("ci-runner-1").to_string();
        assert_eq!(dashed.parse::<PairingCode>(), Ok(code("ci-runner-1")));
    }

    #[test]
    fn rejects_bad_codes() {
        let text = code("box").to_string();
        assert_eq!("".parse::<PairingCode>(), Err(PairingError::Format));
        assert_eq!(
            text.replacen("jamjam1", "jamjam2", 1)
                .parse::<PairingCode>(),
            Err(PairingError::Format)
        );
        assert_eq!(
            text.to_uppercase().parse::<PairingCode>(),
            Err(PairingError::Format)
        );
        assert_eq!(
            text.replacen("4f", "4F", 1).parse::<PairingCode>(),
            Err(PairingError::Key)
        );
        assert_eq!(
            text.replace("-box", "-b x").parse::<PairingCode>(),
            Err(PairingError::Label)
        );
        assert_eq!(
            text.replace("-box", "-bot").parse::<PairingCode>(),
            Err(PairingError::Checksum)
        );
        assert_eq!(
            text.replace("-box", "-").parse::<PairingCode>(),
            Err(PairingError::Label)
        );
        assert!(PairingLabel::try_from("x".repeat(41)).is_err());
    }

    proptest! {
        #[test]
        fn round_trip(key in any::<[u8; 32]>(), label in "[A-Za-z0-9._@+\\-]{1,40}") {
            let code = PairingCode {
                key: ShareKey(key),
                label: PairingLabel::try_from(label).unwrap(),
            };
            prop_assert_eq!(code.to_string().parse::<PairingCode>(), Ok(code));
        }

        #[test]
        fn single_character_typos_are_detected(
            key in any::<[u8; 32]>(),
            label in "[a-z0-9@.]{1,20}",
            position in 0usize..100,
            replacement in "[ -~]",
        ) {
            let text = PairingCode {
                key: ShareKey(key),
                label: PairingLabel::try_from(label).unwrap(),
            }
            .to_string();
            let position = position % text.len();
            let typo: String = text
                .chars()
                .enumerate()
                .map(|(index, c)| if index == position { replacement.chars().next().unwrap() } else { c })
                .collect();
            prop_assume!(typo != text);
            prop_assert!(typo.trim() == text || typo.parse::<PairingCode>().is_err());
        }

        #[test]
        fn deleted_characters_are_detected(key in any::<[u8; 32]>(), position in 0usize..100) {
            let text = PairingCode { key: ShareKey(key), label: PairingLabel::try_from("dev".to_owned()).unwrap() }
                .to_string();
            let position = position % text.len();
            let typo: String = text
                .chars()
                .enumerate()
                .filter_map(|(index, c)| (index != position).then_some(c))
                .collect();
            prop_assert!(typo.parse::<PairingCode>().is_err());
        }
    }
}
