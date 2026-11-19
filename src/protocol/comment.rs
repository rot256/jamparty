//! Comments: the draft a joiner sends, and the comment the share queues and
//! delivers, with length caps. Attachment bytes travel separately. Text
//! from browsers is kept in JSON fields.

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

/// A page-relative rectangle in PDF points, measured from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct PdfBox {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct PdfAnchorFields {
    page: u32,
    #[serde(rename = "box")]
    bounds: PdfBox,
}

/// Validated at both the browser-facing socket and the peer boundary.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PdfAnchorFields", into = "PdfAnchorFields")]
#[cfg_attr(test, derive(ts_rs::TS), ts(as = "PdfAnchorFields"))]
pub(crate) struct PdfAnchor(PdfAnchorFields);

// Validation excludes NaN, so equality is reflexive.
impl Eq for PdfAnchor {}

impl TryFrom<PdfAnchorFields> for PdfAnchor {
    type Error = Invalid;

    fn try_from(fields: PdfAnchorFields) -> Result<Self, Self::Error> {
        let PdfAnchorFields {
            page,
            bounds: PdfBox { x0, y0, x1, y1 },
        } = fields;
        const MAX_POINTS: f64 = 14_400.0;
        if page == 0 {
            return Err(Invalid::new("PDF page must be at least 1"));
        }
        if ![x0, y0, x1, y1]
            .into_iter()
            .all(|n| (0.0..=MAX_POINTS).contains(&n))
            || x0 >= x1
            || y0 >= y1
        {
            return Err(Invalid::new(
                "PDF box must be ordered and within 0–14400 points",
            ));
        }
        Ok(Self(fields))
    }
}

impl From<PdfAnchor> for PdfAnchorFields {
    fn from(anchor: PdfAnchor) -> Self {
        anchor.0
    }
}

/// The fields of a draft; only reachable through a validated [`Draft`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DraftFields {
    pub(crate) page: CommentPage,
    pub(crate) selector: CommentSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pdf: Option<PdfAnchor>,
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
    pub(crate) pdf: Option<PdfAnchor>,
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
            pdf,
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
            pdf,
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
            comment::{
                Author, Comment, Draft, DraftFields, MAX_COMMENT, PdfAnchor, PdfAnchorFields,
                PdfBox,
            },
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
            pdf: None,
            selected_text: None,
            comment: comment.to_owned(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn json_shape() {
        let draft = Draft::try_from(DraftFields {
            pdf: Some(
                serde_json::from_value(serde_json::json!({
                    "page": 2, "box": {"x0": 10.5, "y0": 20, "x1": 30, "y1": 40}
                }))
                .unwrap(),
            ),
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
                "pdf",
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
            value.pointer("/pdf"),
            Some(&serde_json::json!({
                "page": 2, "box": {"x0": 10.5, "y0": 20.0, "x1": 30.0, "y1": 40.0}
            }))
        );
        assert_eq!(
            value.pointer("/selectedText"),
            Some(&serde_json::json!("Hello"))
        );
    }

    #[test]
    fn pdf_coordinates_are_validated_at_deserialization() {
        let bounds = PdfBox {
            x0: 0.0,
            y0: 1.0,
            x1: 2.0,
            y1: 3.0,
        };
        assert!(PdfAnchor::try_from(PdfAnchorFields { page: 1, bounds }).is_ok());
        for invalid in [
            PdfBox {
                x0: f64::NAN,
                ..bounds
            },
            PdfBox {
                x1: f64::INFINITY,
                ..bounds
            },
            PdfBox {
                y0: -0.01,
                ..bounds
            },
            PdfBox {
                x1: 14_401.0,
                ..bounds
            },
            PdfBox { x0: 2.0, ..bounds },
            PdfBox { y0: 4.0, ..bounds },
        ] {
            assert!(
                PdfAnchor::try_from(PdfAnchorFields {
                    page: 1,
                    bounds: invalid
                })
                .is_err()
            );
        }
        for invalid in [
            serde_json::json!({"page": 0, "box": bounds}),
            serde_json::json!({"page": 1, "box": bounds, "rotate": 90}),
            serde_json::json!({"page": 1, "box": {"x0": 0, "y0": 1, "x1": 2, "y1": 3, "unit": "pt"}}),
        ] {
            let mut draft = serde_json::to_value(fields("PDF comment")).unwrap();
            draft.as_object_mut().unwrap().insert("pdf".into(), invalid);
            assert!(serde_json::from_value::<Draft>(draft).is_err());
        }
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
