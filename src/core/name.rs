//! Joiners' display names: self-chosen and unverified, shown next to the
//! joiner's key fingerprint.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::core::invalid::Invalid;

pub(crate) const MAX_NAME: usize = 40;

/// Used when the user and host names leave nothing usable.
const FALLBACK_NAME: &str = "joiner";

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '+' | '-')
}

/// 1 to 40 characters of `[A-Za-z0-9._@+-]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct JoinerName(String);

impl JoinerName {
    /// The default name, `<user>@<host>`, with characters outside the
    /// alphabet dropped and cut to 40 characters.
    #[must_use]
    pub(crate) fn default_for(user: &str, host: &str) -> Self {
        let name: String = format!("{user}@{host}")
            .chars()
            .filter(|c| is_name_char(*c))
            .take(MAX_NAME)
            .collect();
        match name.as_str() {
            "" | "@" => Self(FALLBACK_NAME.to_owned()),
            _ => Self(name),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for JoinerName {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = !raw.is_empty() && raw.len() <= MAX_NAME && raw.chars().all(is_name_char);
        if valid {
            Ok(Self(raw))
        } else {
            Err(Invalid::new(format!(
                "a name is 1 to {MAX_NAME} characters of letters, digits and ._@+-"
            )))
        }
    }
}

impl FromStr for JoinerName {
    type Err = Invalid;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::try_from(raw.to_owned())
    }
}

impl From<JoinerName> for String {
    fn from(name: JoinerName) -> Self {
        name.0
    }
}

impl fmt::Display for JoinerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::{
        invalid::Invalid,
        name::{JoinerName, MAX_NAME},
    };

    fn name(raw: &str) -> Result<JoinerName, Invalid> {
        JoinerName::try_from(raw.to_owned())
    }

    #[test]
    fn validation() {
        for good in ["a", "alice@laptop", "A.b_c+d-e@f", &"x".repeat(MAX_NAME)] {
            assert!(name(good).is_ok(), "{good:?}");
        }
        for bad in [
            "",
            " ",
            "alice bob",
            "a\nb",
            "é",
            "<script>",
            "a:b",
            &"x".repeat(MAX_NAME + 1),
        ] {
            assert!(name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn defaults() {
        assert_eq!(
            JoinerName::default_for("alice", "laptop").as_str(),
            "alice@laptop"
        );
        assert_eq!(
            JoinerName::default_for("Ålice B", "my host.local").as_str(),
            "liceB@myhost.local"
        );
        assert_eq!(JoinerName::default_for("", "").as_str(), "joiner");
        assert_eq!(JoinerName::default_for("ü", "ö").as_str(), "joiner");
        assert_eq!(
            JoinerName::default_for(&"u".repeat(30), &"h".repeat(30)).as_str(),
            format!("{}@{}", "u".repeat(30), "h".repeat(9))
        );
    }

    proptest! {
        #[test]
        fn defaults_are_valid(user in "\\PC{0,50}", host in "\\PC{0,50}") {
            let default = JoinerName::default_for(&user, &host);
            prop_assert!(name(default.as_str()).is_ok());
        }
    }
}
