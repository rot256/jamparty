//! Content addresses and metadata for comment attachments.

use std::{fmt, str::FromStr};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};

use crate::core::{invalid::Invalid, text::escaped};

pub(crate) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_ATTACHMENTS: usize = 4;
const MAX_NAME_BYTES: usize = 160;
pub(crate) const UPLOAD_PATH: &str = "/_jamparty/api/attachments";
pub(crate) const MAX_UPLOAD_REPLY_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct ResourceId([u8; 32]);

impl ResourceId {
    #[must_use]
    pub(crate) fn digest(bytes: &[u8]) -> Self {
        Self(Sha3_256::digest(bytes).into())
    }
}

impl FromStr for ResourceId {
    type Err = Invalid;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let invalid =
            || Invalid::new("attachment id must be sha3: followed by 64 lowercase hex digits");
        let hex = raw.strip_prefix("sha3:").ok_or_else(invalid)?;
        if hex.len() != 64 {
            return Err(invalid());
        }
        let mut digest = [0; 32];
        HEXLOWER
            .decode_mut(hex.as_bytes(), &mut digest)
            .map_err(|_| invalid())?;
        Ok(Self(digest))
    }
}

impl TryFrom<String> for ResourceId {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<ResourceId> for String {
    fn from(id: ResourceId) -> Self {
        id.to_string()
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha3:{}", HEXLOWER.encode(&self.0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct AttachmentName(String);

impl From<String> for AttachmentName {
    fn from(raw: String) -> Self {
        let mut name = String::new();
        for character in raw
            .chars()
            .filter(|c| !escaped(*c) && !matches!(c, '/' | '\\'))
        {
            if name.len() + character.len_utf8() > MAX_NAME_BYTES {
                break;
            }
            name.push(character);
        }
        let name = name.trim();
        Self(if name.is_empty() {
            "attachment".to_owned()
        } else {
            name.to_owned()
        })
    }
}

impl From<AttachmentName> for String {
    fn from(name: AttachmentName) -> Self {
        name.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum Attachment {
    File {
        file: ResourceId,
        name: AttachmentName,
    },
}

impl Attachment {
    #[must_use]
    pub(crate) fn id(&self) -> ResourceId {
        match self {
            Self::File { file, name: _ } => *file,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Uploaded {
    pub(crate) id: ResourceId,
}

#[cfg(test)]
mod tests {
    use crate::protocol::resource::{AttachmentName, MAX_NAME_BYTES, ResourceId};

    #[test]
    fn resource_ids_are_strict_sha3_256() {
        let known = "sha3:a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a";
        let id = ResourceId::digest(b"");
        assert_eq!(id.to_string(), known);
        assert_eq!(known.parse::<ResourceId>().unwrap(), id);
        for invalid in [
            known.to_uppercase(),
            known.replacen("sha3:", "sha256:", 1),
            known.replacen("a7", "A7", 1),
            known.replacen("a7", "g7", 1),
            format!("{known}0"),
            known.trim_end_matches('a').to_owned(),
        ] {
            assert!(invalid.parse::<ResourceId>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn filenames_are_bounded_display_metadata() {
        let name = AttachmentName::from(format!("/a\\b\n{}", "é".repeat(MAX_NAME_BYTES)));
        let name = String::from(name);
        assert!(name.starts_with("abé"));
        assert!(name.len() <= MAX_NAME_BYTES);
    }
}
