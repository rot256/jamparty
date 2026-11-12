//! The share <-> server protocol over iroh (ALPN `jamjam/1`).
//!
//! - Control: one long-lived bidirectional stream opened by the share;
//!   newline-delimited JSON, one [`control::ControlRequest`] per line,
//!   answered by one [`control::ControlResponse`] per line with the same
//!   `id`. Only the share sends requests.
//! - Polls: one stream per poll; one [`poll::PollRequest`] line, one
//!   [`poll::PollResponse`] line.
//! - Requests: HTTP/1.1 on server-opened streams (not modelled here).
//!
//! Every message carries `"version": 1`.

pub(crate) mod comment;
pub(crate) mod control;
pub(crate) mod poll;

use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

/// The iroh ALPN.
pub(crate) const ALPN: &[u8] = b"jamjam/1";
/// Maximum length of one control line (request or response), newline
/// included. A `list` answer is bounded by the sessions of one key.
pub(crate) const MAX_CONTROL_LINE_BYTES: usize = 1024 * 1024;
/// Maximum length of a poll request line.
pub(crate) const MAX_POLL_REQUEST_BYTES: usize = 1024;
/// Maximum number of comments in one poll response.
pub(crate) const MAX_POLL_BATCH: usize = 100;
/// Maximum length of a poll response line: a full batch of maximal comments.
pub(crate) const MAX_POLL_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Longest a poll may wait for comments.
pub(crate) const MAX_POLL_WAIT: SignedDuration = SignedDuration::from_secs(60);

/// Why a request failed. `code` is for programs, `message` for people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtocolError {
    pub(crate) code: ErrorCode,
    pub(crate) message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorCode {
    /// Malformed or invalid request.
    BadRequest,
    /// No such session for this key (also used for sessions of other keys).
    NotFound,
    /// The key's owner may not do this (e.g. no longer in `share`).
    Forbidden,
    /// Queue full or a similar temporary condition.
    Unavailable,
    Internal,
}
