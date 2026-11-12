//! Control-stream messages. Every line is an [`Envelope`]:
//! `{"version":1,"message":{"type":...}}`.
//!
//! Joiner to share: [`JoinerMessage::Hello`] first, then comments. Parsed
//! strictly (unknown fields are refused): it is untrusted input.
//!
//! Share to joiner: admission or refusal, comment acknowledgements,
//! answers, delivered comments, live-reload revisions, the status line and
//! presence. Unknown fields inside these are ignored, so a share may add
//! some.

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::CommentId,
        name::JoinerName,
        text::{check_line, check_text},
        token::PartySecret,
        version::Version1,
    },
    protocol::comment::{Author, Draft},
};

pub(crate) const MAX_STATUS_LINE: usize = 200;
pub(crate) const MAX_ANSWER: usize = 10_000;

/// One control line.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope<T> {
    pub(crate) version: Version1,
    pub(crate) message: T,
}

impl<T> From<T> for Envelope<T> {
    fn from(message: T) -> Self {
        Self {
            version: Version1,
            message,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum JoinerMessage {
    /// The first message: the party secret and the joiner's display name.
    Hello {
        secret: PartySecret,
        name: JoinerName,
    },
    /// A comment; `ref` is the joiner's own number for it, echoed in the
    /// acknowledgement.
    Comment {
        #[serde(rename = "ref")]
        reference: u64,
        draft: Draft,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ShareMessage {
    /// The hello was accepted; `you` is how the joiner's comments are
    /// signed, `undelivered` which of its comments the share still holds
    /// (any others it had queued were delivered meanwhile).
    Admitted {
        you: Author,
        #[serde(default)]
        undelivered: Vec<CommentId>,
    },
    /// The hello was refused; the share closes the connection after this.
    Refused { reason: RefusedReason },
    /// What became of the joiner's comment `ref`.
    CommentAck {
        #[serde(rename = "ref")]
        reference: u64,
        outcome: AckOutcome,
    },
    /// An answer from the agent (`jamjam send`), Markdown; `id` is the
    /// share's, the same for every joiner.
    Answer {
        id: CommentId,
        message: AnswerText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    /// These comments reached the target.
    Delivered { ids: Vec<CommentId> },
    /// The shared content changed; pages reload.
    Reload { revision: u64 },
    /// The status line (`null` clears it).
    Status { line: Option<StatusLine> },
    /// Who is connected now.
    Presence { joiners: Vec<Author> },
}

/// The comment an answer replies to, with where it was made so pages can
/// place the answer next to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Anchor {
    pub(crate) comment: CommentId,
    pub(crate) page: String,
    pub(crate) selector: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RefusedReason {
    /// Wrong secret: the token is wrong or from an earlier run.
    WrongSecret,
    /// The first line was not a valid hello (malformed, too long or
    /// another version).
    BadHello,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub(crate) enum AckOutcome {
    /// Queued for delivery under `id`.
    Queued { id: CommentId },
    /// Over 10 per minute; try again after this many seconds.
    RateLimited { retry_after_secs: u64 },
    /// The share's queue is full.
    QueueFull,
}

/// A short status line shown on the page as is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct StatusLine(String);

impl TryFrom<String> for StatusLine {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        check_line(&raw, "status", MAX_STATUS_LINE)?;
        Ok(Self(raw))
    }
}

impl From<StatusLine> for String {
    fn from(line: StatusLine) -> Self {
        line.0
    }
}

/// An answer's Markdown text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct AnswerText(String);

impl TryFrom<String> for AnswerText {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        check_text(&raw, "message", MAX_ANSWER)?;
        Ok(Self(raw))
    }
}

impl From<AnswerText> for String {
    fn from(text: AnswerText) -> Self {
        text.0
    }
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
