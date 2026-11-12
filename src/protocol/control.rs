//! The control stream: newline-delimited JSON requests from the share and
//! one response per request, matched by `id`.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{CommentId, GithubLogin, Seed, Seq},
        session::{ClientBlob, SessionLabel},
        text::{check_line, check_text},
        version::Version1,
    },
    protocol::ProtocolError,
};

pub(crate) const MAX_STATUS_LINE: usize = 200;
pub(crate) const MAX_ANSWER: usize = 10_000;

/// One request line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControlRequest {
    pub(crate) version: Version1,
    /// Chosen by the share, echoed in the response.
    pub(crate) id: u64,
    pub(crate) request: Request,
}

/// Session access in requests: GitHub logins, which the server maps to
/// user ids through its resolved allowlist (a login not on it is refused).
pub(crate) type Logins = Vec<GithubLogin>;

/// A change of a session's access list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AccessUpdate {
    /// Everyone on the allowlist.
    Everyone,
    /// Only these people (and the owner).
    Only(Logins),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Create {
        label: SessionLabel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<Logins>,
        /// Lifetime in seconds; the server's default when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_in_secs: Option<u64>,
        #[serde(default)]
        client: ClientBlob,
    },
    /// Reattach to a session this key created.
    Resume {
        seed: Seed,
    },
    Update {
        seed: Seed,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<SessionLabel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<AccessUpdate>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client: Option<ClientBlob>,
    },
    End {
        seed: Seed,
    },
    /// Set (or with `null`, clear) the session's status line.
    SetStatus {
        seed: Seed,
        status: Option<StatusLine>,
    },
    Answer {
        seed: Seed,
        message: AnswerText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<CommentId>,
    },
    Status {
        seed: Seed,
    },
    /// This key's sessions.
    List,
    /// Acknowledge delivery of everything up to `up_to`.
    Ack {
        seed: Seed,
        up_to: Seq,
    },
}

/// A short, opaque status line shown on the page as is.
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

/// One response line. Unknown fields are accepted (a newer server may add
/// some).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ControlResponse {
    pub(crate) version: Version1,
    pub(crate) id: u64,
    #[serde(flatten)]
    pub(crate) outcome: Outcome,
}

/// `{"ok": {...}}` or `{"error": {...}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Ok(Reply),
    Error(ProtocolError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Reply {
    /// Answer to create, resume and update.
    Session(SessionInfo),
    /// Answer to end, set_status and answer.
    Done,
    Status(StatusInfo),
    List {
        sessions: Vec<SessionInfo>,
    },
    /// Answer to ack: how many comments were dropped (0 for a stale ack).
    Acked {
        removed: u64,
    },
}

/// What a client sees of one of its sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionInfo {
    pub(crate) seed: Seed,
    pub(crate) label: SessionLabel,
    /// The entry link, `https://<domain>/s/<seed>`.
    pub(crate) url: String,
    pub(crate) created: Timestamp,
    pub(crate) expires: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) access: Option<Logins>,
    pub(crate) client: ClientBlob,
}

/// Whether a share process is attached to the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Presence {
    Online,
    Offline,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StatusInfo {
    pub(crate) session: SessionInfo,
    pub(crate) share: Presence,
    /// Comments waiting for acknowledgement.
    pub(crate) queued: u64,
    /// Last assigned and last acknowledged sequence numbers.
    pub(crate) last_seq: Seq,
    pub(crate) acked_through: Seq,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<StatusLine>,
    /// Number of open viewer streams.
    pub(crate) viewers: u64,
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
