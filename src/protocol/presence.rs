//! Who is here and where (docs/ui.md): the joiner tells the share its
//! name, the page it is on and whether its cursor is shown; the share
//! tells everyone, with a colour per person, and forwards cursors between
//! joiners on the same page.

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{EndpointKey, Fingerprint},
        invalid::Invalid,
        name::JoinerName,
        settings::Switch,
        text::check_line,
    },
    protocol::comment::Author,
};

pub(crate) const MAX_PAGE_PATH: usize = 512;

/// Where a joiner is: the path, query and fragment of the page's URL (the
/// host differs per joiner), as the browser serialises them: 1 to 512
/// printable ASCII characters, starting with `/`, without `"` or `\`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct PagePath(String);

impl TryFrom<String> for PagePath {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = raw.starts_with('/')
            && raw.len() <= MAX_PAGE_PATH
            && raw
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\');
        if valid {
            Ok(Self(raw))
        } else {
            Err(Invalid::new(format!(
                "a page is 1 to {MAX_PAGE_PATH} printable ASCII characters starting with /, without \" or \\"
            )))
        }
    }
}

impl From<PagePath> for String {
    fn from(path: PagePath) -> Self {
        path.0
    }
}

/// The colours people are shown in (avatars, cursors).
const PALETTE_SIZE: u8 = 8;

/// A person's colour, an index into the page's palette; derived from their
/// key, so it is the same in every page and every session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "number"))]
pub(crate) struct Colour(u8);

impl From<EndpointKey> for Colour {
    fn from(key: EndpointKey) -> Self {
        let EndpointKey([first, ..]) = key;
        Self(first % PALETTE_SIZE)
    }
}

impl TryFrom<u8> for Colour {
    type Error = Invalid;

    fn try_from(index: u8) -> Result<Self, Self::Error> {
        if index < PALETTE_SIZE {
            Ok(Self(index))
        } else {
            Err(Invalid::new(format!(
                "a colour is 0 to {}",
                PALETTE_SIZE - 1
            )))
        }
    }
}

impl From<Colour> for u8 {
    fn from(colour: Colour) -> Self {
        colour.0
    }
}

/// One person in presence: who, in which colour, where, and whether their
/// cursor is shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Person {
    pub(crate) name: JoinerName,
    pub(crate) fingerprint: Fingerprint,
    pub(crate) key: EndpointKey,
    pub(crate) colour: Colour,
    /// The page they are on, once their page said.
    #[serde(default)]
    pub(crate) url: Option<PagePath>,
    pub(crate) cursor: Switch,
}

impl Person {
    #[must_use]
    pub(crate) fn new(author: Author, url: Option<PagePath>, cursor: Switch) -> Self {
        let Author {
            name,
            fingerprint,
            key,
        } = author;
        Self {
            name,
            fingerprint,
            colour: Colour::from(key),
            key,
            url,
            cursor,
        }
    }
}

/// What a joiner tells the share about itself (all of it, each time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Whereabouts {
    pub(crate) name: JoinerName,
    pub(crate) url: Option<PagePath>,
    pub(crate) cursor: Switch,
}

pub(crate) const MAX_CURSOR_SELECTOR: usize = 512;

/// The CSS selector of the element under a pointer: 1 to 512 characters,
/// no control characters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct CursorSelector(String);

impl TryFrom<String> for CursorSelector {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        check_line(&raw, "a cursor selector", MAX_CURSOR_SELECTOR)?;
        Ok(Self(raw))
    }
}

impl From<CursorSelector> for String {
    fn from(selector: CursorSelector) -> Self {
        selector.0
    }
}

/// A fraction of an element's width or height, in ten-thousandths (0 to
/// 10000).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "number"))]
pub(crate) struct Fraction(u16);

pub(crate) const FRACTION_ONE: u16 = 10_000;

impl TryFrom<u16> for Fraction {
    type Error = Invalid;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value <= FRACTION_ONE {
            Ok(Self(value))
        } else {
            Err(Invalid::new(format!("a fraction is 0 to {FRACTION_ONE}")))
        }
    }
}

impl From<Fraction> for u16 {
    fn from(fraction: Fraction) -> Self {
        fraction.0
    }
}

/// Where a pointer is: an element and the offset in its box. Never raw
/// viewport coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Position {
    pub(crate) selector: CursorSelector,
    pub(crate) x: Fraction,
    pub(crate) y: Fraction,
}

#[cfg(test)]
mod tests {
    use crate::{
        core::ids::EndpointKey,
        protocol::presence::{
            Colour, CursorSelector, Fraction, MAX_CURSOR_SELECTOR, MAX_PAGE_PATH, PALETTE_SIZE,
            PagePath,
        },
    };

    #[test]
    fn page_paths() {
        for good in [
            "/",
            "/a?b=c#d",
            "/%22x%5C",
            &format!("/{}", "a".repeat(MAX_PAGE_PATH - 1)),
        ] {
            assert!(PagePath::try_from(good.to_owned()).is_ok(), "{good}");
        }
        for bad in [
            "",
            "a/",
            "http://x/",
            "/a b",
            "/\"",
            "/\\",
            "/é",
            "/\n",
            &format!("/{}", "a".repeat(MAX_PAGE_PATH)),
        ] {
            assert!(PagePath::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn colours() {
        assert!(Colour::try_from(PALETTE_SIZE - 1).is_ok());
        assert!(Colour::try_from(PALETTE_SIZE).is_err());
        let colour = u8::from(Colour::from(EndpointKey([0xff; 32])));
        assert_eq!(colour, 0xff % PALETTE_SIZE);
    }

    #[test]
    fn cursor_positions() {
        assert!(CursorSelector::try_from("main > h1".to_owned()).is_ok());
        assert!(CursorSelector::try_from(String::new()).is_err());
        assert!(CursorSelector::try_from("a".repeat(MAX_CURSOR_SELECTOR + 1)).is_err());
        assert!(CursorSelector::try_from("a\nb".to_owned()).is_err());
        assert!(Fraction::try_from(10_000).is_ok());
        assert!(Fraction::try_from(10_001).is_err());
    }
}
