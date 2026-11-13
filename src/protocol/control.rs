//! Control-stream messages. Every line is an [`Envelope`]:
//! `{"version":1,"message":{"type":...}}`.
//!
//! Joiner to share: [`JoinerMessage::Hello`] first, then comments and,
//! when the share says it takes them, presence. Parsed strictly (unknown
//! fields are refused): it is untrusted input.
//!
//! Share to joiner: admission or refusal, comment acknowledgements,
//! answers, delivered comments, live-reload revisions, the status line and
//! presence. Unknown fields inside these are ignored, so a share may add
//! some.

use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{CommentId, EndpointKey},
        invalid::Invalid,
        name::JoinerName,
        port::Port,
        settings::Switch,
        text::{check_line, check_text},
        token::PartySecret,
        version::Version1,
    },
    protocol::{
        comment::{Author, CommentPage, CommentSelector, Draft},
        presence::{PagePath, Person, Position, Support},
    },
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
    /// A comment; `ref` is the page's own id for it, echoed in the
    /// acknowledgement. The share queues a `ref` once per joiner: sending
    /// it again ("send again" after a lost acknowledgement) gets the same
    /// comment id back.
    Comment {
        #[serde(rename = "ref")]
        reference: CommentRef,
        draft: Draft,
    },
    /// Where the joiner is and how it is called now: sent after admission
    /// and on every change, only to a share that supports it (see
    /// [`ShareMessage::Admitted`]).
    Presence {
        name: JoinerName,
        /// The page the joiner's most recently active tab is on.
        #[serde(default)]
        url: Option<PagePath>,
        cursor: Switch,
    },
    /// The joiner's pointer moved to `position`, or (none) left the page.
    /// Sent only while its cursor switch is on, and only on movement.
    Cursor {
        #[serde(default)]
        position: Option<Position>,
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
        /// Whether the share takes `presence` (and `cursor`) messages.
        #[serde(default)]
        presence: Support,
        /// What the share serves (none from an older share).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<SiteInfo>,
    },
    /// The hello was refused; the share closes the connection after this.
    Refused { reason: RefusedReason },
    /// What became of the joiner's comment `ref`.
    CommentAck {
        #[serde(rename = "ref")]
        reference: CommentRef,
        outcome: AckOutcome,
    },
    /// An answer from the agent (`jampot send`), Markdown; `id` is the
    /// share's, the same for every joiner. With `to`, it is for that key
    /// alone and sent only to its connections.
    Answer {
        id: CommentId,
        message: AnswerText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<EndpointKey>,
    },
    /// These comments reached the target.
    Delivered { ids: Vec<CommentId> },
    /// The shared content changed; pages reload.
    Reload { revision: Revision },
    /// The status line (`null` clears it).
    Status { line: Option<StatusLine> },
    /// Who is connected now: `joiners` once per key (what older joiners
    /// read), `here` the same people with their colour, page and cursor
    /// switch.
    Presence {
        joiners: Vec<Author>,
        #[serde(default)]
        here: Vec<Person>,
    },
    /// Joiner `key`'s pointer on page `url` (the recipient's page), or
    /// (none) it left. Sent only to joiners on that page that announced
    /// it.
    Cursor {
        key: EndpointKey,
        url: PagePath,
        #[serde(default)]
        position: Option<Position>,
    },
}

/// The live-reload revision of a shared directory: starts at 0 and counts
/// content changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "number"))]
pub(crate) struct Revision(u64);

impl Revision {
    /// The revision after this one.
    #[must_use]
    pub(crate) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl From<u64> for Revision {
    fn from(count: u64) -> Self {
        Self(count)
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What the share serves, as far as joiners care.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SiteInfo {
    /// A dev server (`jampot proxy`) on `port`, its URL's effective port.
    /// Joiners serve the site on that port when it is free on their
    /// machine, so the dev server's own scripts that dial that port (live
    /// reload) reach it through them.
    Proxy { port: Port },
    /// A directory (`jampot serve`).
    Files,
    /// A kind from a newer share.
    #[serde(other)]
    Other,
}

/// The comment an answer replies to, with where it was made so pages can
/// place the answer next to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Anchor {
    pub(crate) comment: CommentId,
    pub(crate) page: CommentPage,
    pub(crate) selector: CommentSelector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum RefusedReason {
    /// Wrong secret: the token is wrong or from an earlier run.
    WrongSecret,
    /// The first line was not a valid hello (malformed, too long or
    /// another version).
    BadHello,
    /// The share has as many joiners as it takes (in all, or with this
    /// key); the joiner retries later.
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub(crate) enum AckOutcome {
    /// Queued for delivery under `id`.
    Queued { id: CommentId },
    /// Sent before (same `ref`) as `id`, which is delivered already.
    Delivered { id: CommentId },
    /// Over 10 per minute; try again after this long (whole seconds on
    /// the wire).
    RateLimited {
        #[serde(rename = "retry_after_secs", with = "crate::core::secs")]
        retry_after: Duration,
    },
    /// The share's queue is full, or holds as many of this joiner's
    /// comments as it takes.
    QueueFull,
}

pub(crate) const MAX_COMMENT_REF: usize = 64;

/// The page's id for a comment: 1 to 64 characters of `[A-Za-z0-9-]`
/// (the page uses a random UUID).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct CommentRef(String);

impl TryFrom<String> for CommentRef {
    type Error = Invalid;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let valid = (1..=MAX_COMMENT_REF).contains(&raw.len())
            && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if valid {
            Ok(Self(raw))
        } else {
            Err(Invalid::new(format!(
                "a comment ref is 1 to {MAX_COMMENT_REF} characters of A-Z, a-z, 0-9 and -"
            )))
        }
    }
}

impl From<CommentRef> for String {
    fn from(reference: CommentRef) -> Self {
        reference.0
    }
}

/// A short status line shown on the page as is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "string"))]
pub(crate) struct StatusLine(String);

impl TryFrom<String> for StatusLine {
    type Error = Invalid;

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
    type Error = Invalid;

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
