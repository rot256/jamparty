//! The error of a value that failed validation.

use std::{error::Error, fmt};

/// A value that failed validation, and why (said for people, naming the
/// field and the rule).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Invalid(String);

impl Invalid {
    pub(crate) fn new(why: impl Into<String>) -> Self {
        Self(why.into())
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for Invalid {}
