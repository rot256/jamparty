//! The runtime socket between `nabe recv` / `nabe send` and the running
//! share: newline-delimited JSON, one [`Envelope`] per line, both ways.
//!
//! - recv: the client sends `recv`; when comments are pending the share
//!   sends one `comment` line each and `end`; the client prints them and
//!   answers `printed`. Only then do they count as delivered: if the client
//!   goes away first, the share keeps them for the next `recv`.
//! - send: the client sends `send`; the share answers `sent` or `error`.
//!
//! [`Envelope`]: crate::protocol::control::Envelope

use serde::{Deserialize, Serialize};

use crate::{
    core::ids::CommentId,
    protocol::{comment::Comment, control::AnswerText},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AgentRequest {
    /// Wait for pending comments.
    Recv,
    /// The comments of this `recv` are printed.
    Printed,
    /// Answer on the page, next to comment `anchor` if given.
    Send {
        message: AnswerText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<CommentId>,
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
    /// The answer went to every connected joiner.
    Sent,
    Error {
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use crate::{
        protocol::agent::{AgentReply, AgentRequest},
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
                },
                format!(r#"{{"type":"send","message":"Done.","anchor":"{ID}"}}"#),
            ),
        ];
        for (request, json) in cases {
            let line = format!(r#"{{"version":1,"message":{json}}}"#);
            assert_eq!(
                encode_line(&Envelope::from(request.clone())).unwrap(),
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
            reply,
            b"{\"version\":1,\"message\":{\"type\":\"error\",\"error\":\"no\"}}\n"
        );
        let blank = r#"{"version":1,"message":{"type":"send","message":" "}}"#;
        assert!(decode_line::<Envelope<AgentRequest>>(blank.as_bytes()).is_err());
    }
}
