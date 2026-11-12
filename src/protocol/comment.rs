//! A comment as handed to clients: `id`, `author`, `page`, `selector`,
//! `selectedText`, `comment`, with length caps. Text from browsers is data
//! in JSON fields, never formatted into prose.

use serde::{Deserialize, Serialize};

use crate::core::{
    ids::{CommentId, GithubLogin, UserId},
    queue::Identified,
    text::{check_len, check_text},
};

pub(crate) const MAX_PAGE: usize = 4096;
pub(crate) const MAX_SELECTOR: usize = 2048;
pub(crate) const MAX_SELECTED_TEXT: usize = 2000;
pub(crate) const MAX_COMMENT: usize = 10_000;

/// Who wrote a comment, set by the server from the viewer's cookie.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Author {
    pub(crate) id: UserId,
    pub(crate) login: GithubLogin,
}

/// The fields of a comment; only reachable through a validated [`Comment`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommentFields {
    pub(crate) id: CommentId,
    pub(crate) author: Author,
    pub(crate) page: String,
    pub(crate) selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_text: Option<String>,
    pub(crate) comment: String,
}

/// A comment whose fields are within the caps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CommentFields", into = "CommentFields")]
pub(crate) struct Comment(CommentFields);

impl Comment {
    #[must_use]
    pub(crate) fn fields(&self) -> &CommentFields {
        &self.0
    }
}

impl TryFrom<CommentFields> for Comment {
    type Error = String;

    fn try_from(fields: CommentFields) -> Result<Self, Self::Error> {
        check_len(&fields.page, "page", MAX_PAGE)?;
        check_len(&fields.selector, "selector", MAX_SELECTOR)?;
        if let Some(selected) = &fields.selected_text {
            check_len(selected, "selectedText", MAX_SELECTED_TEXT)?;
        }
        check_text(&fields.comment, "comment", MAX_COMMENT)?;
        Ok(Self(fields))
    }
}

impl From<Comment> for CommentFields {
    fn from(comment: Comment) -> Self {
        comment.0
    }
}

impl Identified for Comment {
    fn id(&self) -> CommentId {
        self.0.id
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::{ids::CommentId, rng::SeededRng},
        protocol::comment::{Author, Comment, CommentFields, MAX_COMMENT},
    };

    pub(crate) fn fields(comment: &str) -> CommentFields {
        CommentFields {
            id: CommentId::generate(&SeededRng::new(1)),
            author: Author {
                id: crate::core::ids::UserId(42),
                login: "octocat".to_owned().try_into().unwrap(),
            },
            page: "https://aaaaaaaaaaaaaaaa.jam.example/".to_owned(),
            selector: "main > h1".to_owned(),
            selected_text: None,
            comment: comment.to_owned(),
        }
    }

    #[test]
    fn json_shape() {
        let comment = Comment::try_from(CommentFields {
            selected_text: Some("Hello".to_owned()),
            ..fields("Bigger, please")
        })
        .unwrap();
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
            value.pointer("/author/login"),
            Some(&serde_json::json!("octocat"))
        );
        assert_eq!(value.pointer("/author/id"), Some(&serde_json::json!(42)));
        let back: Comment = serde_json::from_value(value).unwrap();
        assert_eq!(back, comment);
    }

    #[test]
    fn caps_are_enforced_when_parsing() {
        assert!(Comment::try_from(fields(&"x".repeat(MAX_COMMENT))).is_ok());
        assert!(Comment::try_from(fields(&"x".repeat(MAX_COMMENT + 1))).is_err());
        assert!(Comment::try_from(fields("   ")).is_err());
        let mut long = serde_json::to_value(Comment::try_from(fields("x")).unwrap()).unwrap();
        *long.pointer_mut("/selector").unwrap() = serde_json::Value::String("s".repeat(2049));
        assert!(serde_json::from_value::<Comment>(long).is_err());
        let mut extra = serde_json::to_value(Comment::try_from(fields("x")).unwrap()).unwrap();
        extra
            .pointer_mut("/author")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("admin".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<Comment>(extra).is_err());
    }
}
