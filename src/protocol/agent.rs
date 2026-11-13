//! The runtime socket between `jampot recv` / `jampot send` and the running
//! share: newline-delimited JSON, one [`Envelope`] per line, both ways.
//!
//! - recv: the client sends `recv`; when comments are pending the share
//!   sends one `comment` line each and `end`; the client prints them and
//!   answers `printed`. Only then do they count as delivered: if the client
//!   goes away first, the share keeps them for the next `recv`.
//! - send: the client sends `send`; the share answers `sent` or `error`.
//!
//! [`Envelope`]: crate::protocol::control::Envelope

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{CommentId, EndpointKey, Fingerprint},
        invalid::Invalid,
        name::JoinerName,
    },
    protocol::{
        comment::{Author, Comment},
        control::AnswerText,
    },
};

/// Who an answer is for (`jampot send --to`): everyone (`all`), a person
/// by name (`alice`), or by key (`key:<fingerprint>` or `key:<64 hex>`).
/// Names cannot contain `:`, so no name can pass for a key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) enum Addressee {
    Everyone,
    Named(JoinerName),
    Key(KeyRef),
}

/// A key as `--to key:` gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyRef {
    Fingerprint(Fingerprint),
    Full(EndpointKey),
}

impl KeyRef {
    /// Whether `author` has this key.
    #[must_use]
    pub(crate) fn matches(&self, author: &Author) -> bool {
        match self {
            Self::Fingerprint(fingerprint) => author.fingerprint == *fingerprint,
            Self::Full(key) => author.key == *key,
        }
    }
}

/// How `--to` names everyone.
const EVERYONE: &str = "all";
/// How `--to` marks a key.
pub(crate) const KEY_PREFIX: &str = "key:";

impl TryFrom<String> for Addressee {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        if raw == EVERYONE {
            return Ok(Self::Everyone);
        }
        if let Some(key) = raw.strip_prefix(KEY_PREFIX) {
            let invalid = || {
                Invalid::new(
                    "--to key: takes a fingerprint (10 hex characters) or a key (64 hex characters)",
                )
            };
            let key = match key.len() {
                10 => KeyRef::Fingerprint(
                    Fingerprint::try_from(key.to_owned()).map_err(|_| invalid())?,
                ),
                64 => KeyRef::Full(key.parse().map_err(|_| invalid())?),
                _ => return Err(invalid()),
            };
            return Ok(Self::Key(key));
        }
        Ok(Self::Named(JoinerName::try_from(raw)?))
    }
}

impl FromStr for Addressee {
    type Err = Invalid;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::try_from(raw.to_owned())
    }
}

impl From<Addressee> for String {
    fn from(addressee: Addressee) -> Self {
        match addressee {
            Addressee::Everyone => EVERYONE.to_owned(),
            Addressee::Named(name) => name.into(),
            Addressee::Key(KeyRef::Fingerprint(fingerprint)) => {
                format!("{KEY_PREFIX}{fingerprint}")
            }
            Addressee::Key(KeyRef::Full(key)) => format!("{KEY_PREFIX}{key}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AgentRequest {
    /// Wait for pending comments.
    Recv,
    /// The comments of this `recv` are printed.
    Printed,
    /// Answer on the page, next to comment `anchor` if given; only to
    /// `to` (by default the anchor's author, else everyone).
    Send {
        message: AnswerText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<CommentId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Addressee>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AgentReply {
    Comment {
        comment: Comment,
    },
    /// The batch is complete.
    End,
    /// The answer went out.
    Sent,
    Error {
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use crate::{
        protocol::agent::{Addressee, AgentReply, AgentRequest},
        protocol::{control::Envelope, decode_line, encode_line},
    };

    const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn wire_shape() {
        let cases = [
            (AgentRequest::Recv, r#"{"type":"recv"}"#.to_owned()),
            (AgentRequest::Printed, r#"{"type":"printed"}"#.to_owned()),
            (
                AgentRequest::Send {
                    message: "Done.".to_owned().try_into().unwrap(),
                    anchor: Some(ID.parse().unwrap()),
                    to: None,
                },
                format!(r#"{{"type":"send","message":"Done.","anchor":"{ID}"}}"#),
            ),
            (
                AgentRequest::Send {
                    message: "Hi".to_owned().try_into().unwrap(),
                    anchor: None,
                    to: Some("alice".parse().unwrap()),
                },
                r#"{"type":"send","message":"Hi","to":"alice"}"#.to_owned(),
            ),
            (
                AgentRequest::Send {
                    message: "Hi".to_owned().try_into().unwrap(),
                    anchor: None,
                    to: Some(Addressee::Everyone),
                },
                r#"{"type":"send","message":"Hi","to":"all"}"#.to_owned(),
            ),
        ];
        for (request, json) in cases {
            let line = format!(r#"{{"version":1,"message":{json}}}"#);
            assert_eq!(
                *encode_line(&Envelope::from(request.clone())).unwrap(),
                format!("{line}\n").into_bytes()
            );
            let parsed: Envelope<AgentRequest> = decode_line(line.as_bytes()).unwrap();
            assert_eq!(parsed.message, request);
        }
        let reply = encode_line(&Envelope::from(AgentReply::Error {
            error: "no".to_owned(),
        }))
        .unwrap();
        assert_eq!(
            reply.as_slice(),
            b"{\"version\":1,\"message\":{\"type\":\"error\",\"error\":\"no\"}}\n"
        );
        let blank = r#"{"version":1,"message":{"type":"send","message":" "}}"#;
        assert!(decode_line::<Envelope<AgentRequest>>(blank.as_bytes()).is_err());
        for to in ["", " ", "a\nb", &"a".repeat(65), "key:", "key:abc", "a:b"] {
            assert!(to.parse::<Addressee>().is_err(), "{to:?}");
        }
    }
}
