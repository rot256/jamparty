//! Locations used by answers, validated when either socket decodes them.

use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, de};
use url::Url;

use crate::{
    core::{invalid::Invalid, request::is_pdf_path, text::check_text},
    protocol::comment::{MAX_PAGE, MAX_SELECTOR, PdfAnchor},
};

/// The socket request names a comment by id. The share fills in its location
/// before sending it on to the joiners.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AnswerAnchor<C> {
    Comment { comment: C },
    Place { place: PlaceAnchor },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum PlaceAnchor {
    Html {
        path: AnswerPath,
        selector: AnswerSelector,
    },
    Pdf {
        #[serde(deserialize_with = "deserialize_pdf_path")]
        path: AnswerPath,
        #[cfg_attr(test, ts(type = "PdfAnchorFields"))]
        pdf: PdfAnchor,
    },
}

/// An absolute path within the share, in URL form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct AnswerPath(String);

impl TryFrom<String> for AnswerPath {
    type Error = Invalid;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        let invalid = || {
            Invalid::new(
                "answer path must be a share URL path of at most 4096 characters (no query or fragment)",
            )
        };
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.len() > MAX_PAGE
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\\' | b'"' | b'?' | b'#'))
        {
            return Err(invalid());
        }
        let url = Url::parse(&format!("http://localhost{path}")).map_err(|_| invalid())?;
        if url.path() != path {
            return Err(invalid());
        }
        if path.as_bytes().iter().enumerate().any(|(index, byte)| {
            *byte == b'%'
                && !path
                    .as_bytes()
                    .get(index + 1..index + 3)
                    .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
        }) {
            return Err(invalid());
        }
        let decoded = percent_encoding::percent_decode_str(&path)
            .decode_utf8()
            .map_err(|_| invalid())?;
        if decoded.starts_with("//")
            || decoded.contains('\\')
            || decoded.contains(char::is_control)
            || decoded.split('/').any(|part| matches!(part, "." | ".."))
            || decoded.split('/').nth(1) == Some("_jamparty")
        {
            return Err(invalid());
        }
        Ok(Self(path))
    }
}

impl FromStr for AnswerPath {
    type Err = Invalid;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        path.to_owned().try_into()
    }
}

impl From<AnswerPath> for String {
    fn from(path: AnswerPath) -> Self {
        path.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct AnswerSelector(String);

impl TryFrom<String> for AnswerSelector {
    type Error = Invalid;

    fn try_from(selector: String) -> Result<Self, Self::Error> {
        check_text(&selector, "selector", MAX_SELECTOR)?;
        Ok(Self(selector))
    }
}

impl FromStr for AnswerSelector {
    type Err = Invalid;

    fn from_str(selector: &str) -> Result<Self, Self::Err> {
        selector.to_owned().try_into()
    }
}

impl From<AnswerSelector> for String {
    fn from(selector: AnswerSelector) -> Self {
        selector.0
    }
}

fn deserialize_pdf_path<'de, D: Deserializer<'de>>(input: D) -> Result<AnswerPath, D::Error> {
    let path = AnswerPath::deserialize(input)?;
    if is_pdf_path(&path.0) {
        Ok(path)
    } else {
        Err(de::Error::custom("expected a .pdf path"))
    }
}

/// Clap parser for PATH#page=N&box=x0,y0,x1,y1.
pub(crate) fn pdf_place(raw: &str) -> Result<PlaceAnchor, Invalid> {
    let invalid = || Invalid::new("expected PATH#page=N&box=x0,y0,x1,y1");
    let (path, fragment) = raw.split_once('#').ok_or_else(invalid)?;
    if !is_pdf_path(path) {
        return Err(Invalid::new("expected a .pdf path"));
    }
    let path = if path.starts_with('/') {
        path.parse()?
    } else {
        format!("/{path}").parse()?
    };
    let (page, bounds) = fragment
        .strip_prefix("page=")
        .and_then(|fragment| fragment.split_once("&box="))
        .ok_or_else(invalid)?;
    let page = page.parse().map_err(|_| invalid())?;
    let bounds = bounds
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<f64>, _>>()
        .map_err(|_| invalid())?;
    let [x0, y0, x1, y1] = bounds.as_slice() else {
        return Err(invalid());
    };
    let pdf = PdfAnchor::new(page, [*x0, *y0, *x1, *y1])?;
    Ok(PlaceAnchor::Pdf { path, pdf })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::protocol::{agent::AgentRequest, control::ShareMessage, decode_line, encode_line};

    #[test]
    fn place_anchors_round_trip_on_both_sockets() {
        for place in [
            json!({"type":"html","path":"/pricing.html","selector":"#intro"}),
            json!({"type":"pdf","path":"/paper.pdf","pdf":{"page":2,
                "box":{"x0":72.0,"y0":140.0,"x1":540.0,"y1":180.0}}}),
        ] {
            let anchor = json!({"type":"place","place":place});
            let send = json!({"type":"send","message":"Hi","anchor":anchor});
            let request = decode_line::<AgentRequest>(send.to_string().as_bytes()).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), send);
            let answer = json!({"type":"answer","id":"0f8fad5b-d9cb-469f-a165-70867728950e",
                "message":"Hi","anchor":anchor});
            let response = decode_line::<ShareMessage>(answer.to_string().as_bytes()).unwrap();
            let bytes = encode_line(&response).unwrap();
            assert_eq!(decode_line::<ShareMessage>(&bytes).unwrap(), response);
        }
    }

    #[test]
    fn hostile_places_are_refused_at_the_peer_boundary() {
        for place in [
            json!({"type":"html","path":"//other.example/","selector":"p"}),
            json!({"type":"html","path":"/%2e%2e/x","selector":"p"}),
            json!({"type":"html","path":"/","selector":"x".repeat(2049)}),
            json!({"type":"html","path":format!("/{}", "x".repeat(4096)),"selector":"p"}),
            json!({"type":"html","path":"/","selector":"p","extra":true}),
            json!({"type":"pdf","path":"/paper.pdf","pdf":{"page":1,
                "box":{"x0":2,"y0":1,"x1":1,"y1":3}}}),
        ] {
            let answer = json!({"type":"answer","id":"0f8fad5b-d9cb-469f-a165-70867728950e",
                "message":"Hi","anchor":{"type":"place","place":place}});
            assert!(decode_line::<ShareMessage>(answer.to_string().as_bytes()).is_err());
        }
    }
}
