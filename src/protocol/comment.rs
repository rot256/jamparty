//! Comments: the draft a joiner sends, and the comment the share queues and
//! delivers (`id`, `author`, `page`, `selector`, `selectedText`,
//! `comment`), with length caps. Text from browsers is data in JSON fields,
//! never formatted into prose.

use serde::{Deserialize, Serialize};

use crate::core::{
    ids::{CommentId, EndpointKey, Fingerprint},
    invalid::Invalid,
    name::JoinerName,
    text::{check_len, check_text},
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

/// The fields of a draft; only reachable through a validated [`Draft`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DraftFields {
    pub(crate) page: String,
    pub(crate) selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_text: Option<String>,
    pub(crate) comment: String,
}

/// What a joiner sends: a comment without id or author, within the caps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DraftFields", into = "DraftFields")]
pub(crate) struct Draft(DraftFields);

impl TryFrom<DraftFields> for Draft {
    type Error = Invalid;

    fn try_from(fields: DraftFields) -> Result<Self, Self::Error> {
        check_len(&fields.page, "page", MAX_PAGE)?;
        check_len(&fields.selector, "selector", MAX_SELECTOR)?;
        if let Some(selected) = &fields.selected_text {
            check_len(selected, "selectedText", MAX_SELECTED_TEXT)?;
        }
        check_text(&fields.comment, "comment", MAX_COMMENT)?;
        Ok(Self(fields))
    }
}

impl From<Draft> for DraftFields {
    fn from(draft: Draft) -> Self {
        draft.0
    }
}

/// A queued comment, as delivered to the target (one JSON object each).
/// Built by the share; read back only by `jampot recv` from its own share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Comment {
    pub(crate) id: CommentId,
    pub(crate) author: Author,
    pub(crate) page: String,
    pub(crate) selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_text: Option<String>,
    pub(crate) comment: String,
}

impl Comment {
    #[must_use]
    pub(crate) fn new(id: CommentId, author: Author, draft: Draft) -> Self {
        let DraftFields {
            page,
            selector,
            selected_text,
            comment,
        } = draft.0;
        Self {
            id,
            author,
            page,
            selector,
            selected_text,
            comment,
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
        protocol::comment::{Author, Comment, Draft, DraftFields, MAX_COMMENT},
    };

    fn fields(comment: &str) -> DraftFields {
        DraftFields {
            page: "http://tzrjdfymwrg5sqai.localhost:4321/".to_owned(),
            selector: "main > h1".to_owned(),
            selected_text: None,
            comment: comment.to_owned(),
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
