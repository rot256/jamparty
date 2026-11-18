//! Comments: the draft a joiner sends, and the comment the share queues and
//! delivers (`id`, `author`, `page`, `selector`, `selectedText`,
//! `comment`), with length caps. Text from browsers is data in JSON fields,
//! never formatted into prose.

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{CommentId, EndpointKey, Fingerprint},
        invalid::Invalid,
        name::JoinerName,
        text::{check_len, check_line, check_text},
    },
    protocol::{
        control::MAX_CHOICE,
        resource::{Attachment, MAX_ATTACHMENTS},
    },
};

pub(crate) const MAX_PAGE: usize = 4096;
pub(crate) const MAX_SELECTOR: usize = 2048;
pub(crate) const MAX_SELECTED_TEXT: usize = 2000;
pub(crate) const MAX_COMMENT: usize = 10_000;

/// Who wrote a comment: set by the share from the joiner's admission,
/// never from the comment body. `key` is the joiner's full key, what
/// identifies it; `fingerprint` is its short form, for display only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Author {
    pub(crate) name: JoinerName,
    pub(crate) fingerprint: Fingerprint,
    pub(crate) key: EndpointKey,
}

impl Author {
    #[must_use]
    pub(crate) fn new(name: JoinerName, key: EndpointKey) -> Self {
        Self {
            name,
            fingerprint: key.fingerprint(),
            key,
        }
    }
}

/// The page a comment was made on: its full URL as the browser had it, at
/// most 4096 characters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct CommentPage(String);

impl TryFrom<String> for CommentPage {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        check_len(&raw, "page", MAX_PAGE)?;
        Ok(Self(raw))
    }
}

impl From<CommentPage> for String {
    fn from(page: CommentPage) -> Self {
        page.0
    }
}

impl AsRef<str> for CommentPage {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// What a comment points at: a CSS selector, `jamparty-agent:<id>` for an
/// answer, or empty for the whole page; at most 2048 characters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct CommentSelector(String);

impl TryFrom<String> for CommentSelector {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        check_len(&raw, "selector", MAX_SELECTOR)?;
        Ok(Self(raw))
    }
}

impl From<CommentSelector> for String {
    fn from(selector: CommentSelector) -> Self {
        selector.0
    }
}

impl AsRef<str> for CommentSelector {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The fields of a draft; only reachable through a validated [`Draft`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DraftFields {
    pub(crate) page: CommentPage,
    pub(crate) selector: CommentSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) choice: Option<String>,
    pub(crate) comment: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) attachments: Vec<Attachment>,
}

/// What a joiner sends: a comment without id or author, within the caps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DraftFields", into = "DraftFields")]
pub(crate) struct Draft(DraftFields);

impl TryFrom<DraftFields> for Draft {
    type Error = Invalid;

    fn try_from(fields: DraftFields) -> Result<Self, Self::Error> {
        if let Some(selected) = &fields.selected_text {
            check_len(selected, "selectedText", MAX_SELECTED_TEXT)?;
        }
        if let Some(choice) = &fields.choice {
            check_line(choice, "choice", MAX_CHOICE)?;
        }
        if fields.attachments.len() > MAX_ATTACHMENTS {
            return Err(Invalid::new(format!(
                "a comment can have at most {MAX_ATTACHMENTS} attachments"
            )));
        }
        if fields.attachments.is_empty() || fields.choice.is_some() {
            check_text(&fields.comment, "comment", MAX_COMMENT)?;
        } else {
            check_len(&fields.comment, "comment", MAX_COMMENT)?;
        }
        Ok(Self(fields))
    }
}

impl From<Draft> for DraftFields {
    fn from(draft: Draft) -> Self {
        draft.0
    }
}

/// A queued comment, as delivered to the target (one JSON object each).
/// Built by the share; read back only by `jamparty recv` from its own share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Comment {
    pub(crate) id: CommentId,
    pub(crate) author: Author,
    pub(crate) page: CommentPage,
    pub(crate) selector: CommentSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) choice: Option<String>,
    pub(crate) comment: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) attachments: Vec<Attachment>,
}

impl Comment {
    #[must_use]
    pub(crate) fn new(id: CommentId, author: Author, draft: Draft) -> Self {
        let DraftFields {
            page,
            selector,
            selected_text,
            choice,
            comment,
            attachments,
        } = draft.0;
        Self {
            id,
            author,
            page,
            selector,
            selected_text,
            choice,
            comment,
            attachments,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::{
            ids::{CommentId, EndpointKey},
            rng::SeededRng,
        },
        protocol::{
            comment::{Author, Comment, Draft, DraftFields, MAX_COMMENT},
            resource::{Attachment, MAX_ATTACHMENTS, ResourceId},
        },
    };

    fn fields(comment: &str) -> DraftFields {
        DraftFields {
            page: "http://tzrjdfymwrg5sqai.localhost:4321/"
                .to_owned()
                .try_into()
                .unwrap(),
            selector: "main > h1".to_owned().try_into().unwrap(),
            choice: None,
            selected_text: None,
            comment: comment.to_owned(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn json_shape() {
        let draft = Draft::try_from(DraftFields {
            selected_text: Some("Hello".to_owned()),
            ..fields("Bigger, please")
        })
        .unwrap();
        let author = Author::new(
            "alice@laptop".to_owned().try_into().unwrap(),
            EndpointKey([0xab; 32]),
        );
        let comment = Comment::new(CommentId::generate(&SeededRng::new(1)), author, draft);
        let value = serde_json::to_value(&comment).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "author",
                "comment",
                "id",
                "page",
                "selectedText",
                "selector"
            ]
        );
        assert_eq!(
            value.pointer("/author"),
            Some(&serde_json::json!({
                "name": "alice@laptop",
                "fingerprint": "ababababab",
                "key": "ab".repeat(32),
            }))
        );
        assert_eq!(
            value.pointer("/selectedText"),
            Some(&serde_json::json!("Hello"))
        );
    }

    #[test]
    fn attachments_allow_empty_text_and_have_a_count_cap() {
        let file = Attachment::File {
            file: ResourceId::digest(b"data"),
            name: "report.pdf".to_owned().into(),
        };
        for (count, valid) in [
            (0, false),
            (MAX_ATTACHMENTS, true),
            (MAX_ATTACHMENTS + 1, false),
        ] {
            let draft = DraftFields {
                attachments: vec![file.clone(); count],
                ..fields("")
            };
            assert_eq!(Draft::try_from(draft).is_ok(), valid, "{count} attachments");
        }
    }

    #[test]
    fn caps_are_enforced_when_parsing() {
        assert!(Draft::try_from(fields(&"x".repeat(MAX_COMMENT))).is_ok());
        assert!(Draft::try_from(fields(&"x".repeat(MAX_COMMENT + 1))).is_err());
        assert!(Draft::try_from(fields("   ")).is_err());
        let ok = serde_json::to_value(Draft::try_from(fields("x")).unwrap()).unwrap();
        for (pointer, len) in [
            ("/page", 4097),
            ("/selector", 2049),
            ("/selectedText", 2001),
        ] {
            let mut long = ok.clone();
            long.as_object_mut().unwrap().insert(
                pointer.trim_start_matches('/').to_owned(),
                "s".repeat(len).into(),
            );
            assert!(serde_json::from_value::<Draft>(long).is_err(), "{pointer}");
        }
        // A joiner cannot claim an author or id in its draft.
        for key in ["author", "id", "admin"] {
            let mut extra = ok.clone();
            extra
                .as_object_mut()
                .unwrap()
                .insert(key.to_owned(), "x".into());
            assert!(serde_json::from_value::<Draft>(extra).is_err(), "{key}");
        }
    }
}
