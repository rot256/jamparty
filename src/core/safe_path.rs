//! Validated `next` targets for redirects after sign-in and entry.

use std::{fmt, str::FromStr};

/// Maximum length of a `next` target.
pub(crate) const MAX_NEXT_LEN: usize = 2048;

/// A path (with optional query) on the same host: starts with exactly one
/// `/`, has no backslash, no ASCII whitespace or control characters
/// (browsers drop tabs and newlines, which could turn `/\t/x` into `//x`),
/// and no fragment. It cannot name another host or a scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SafePath(String);

impl SafePath {
    /// The root path `/`.
    #[must_use]
    pub(crate) fn root() -> Self {
        Self("/".to_owned())
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnsafePath;

impl fmt::Display for UnsafePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("next must be a path on this host")
    }
}

impl std::error::Error for UnsafePath {}

impl FromStr for SafePath {
    type Err = UnsafePath;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut bytes = raw.bytes();
        let valid = raw.len() <= MAX_NEXT_LEN
            && bytes.next() == Some(b'/')
            && bytes.next() != Some(b'/')
            && !raw
                .chars()
                .any(|c| c == '\\' || c == '#' || c.is_control() || c.is_whitespace());
        if valid {
            Ok(Self(raw.to_owned()))
        } else {
            Err(UnsafePath)
        }
    }
}

impl fmt::Display for SafePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::core::safe_path::SafePath;

    #[test]
    fn accepts_paths_on_this_host() {
        for good in [
            "/",
            "/a",
            "/a/b?c=d",
            "/a//b",
            "/%2F%2Fevil",
            "/a:b",
            "/?next=//x",
            "/é",
        ] {
            assert!(str::parse::<SafePath>(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn rejects_everything_that_can_leave_the_host() {
        for bad in [
            "",
            "a",
            "//evil.example",
            "///evil",
            "/\\evil.example",
            "\\\\evil",
            "https://evil.example",
            "javascript:alert(1)",
            "/\t/evil",
            "/\n/evil",
            "/ x",
            "/a#b",
            &format!("/{}", "a".repeat(2048)),
        ] {
            assert!(str::parse::<SafePath>(bad).is_err(), "{bad:?}");
        }
    }

    proptest! {
        #[test]
        fn accepted_paths_start_with_one_slash(raw in "\\PC{0,40}") {
            if let Ok(path) = str::parse::<SafePath>(raw.as_str()) {
                prop_assert!(path.as_str().starts_with('/'));
                prop_assert!(!path.as_str().starts_with("//"));
                prop_assert!(!path.as_str().contains('\\'));
            }
        }
    }
}
