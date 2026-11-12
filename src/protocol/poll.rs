//! Poll streams: "comments for session S after cursor C, wait up to W".

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{Seed, Seq},
        version::Version1,
    },
    protocol::{MAX_POLL_WAIT, ProtocolError, comment::Comment},
};

/// A poll wait in milliseconds, at most [`MAX_POLL_WAIT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub(crate) struct WaitMillis(u32);

impl WaitMillis {
    #[must_use]
    pub(crate) fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for WaitMillis {
    type Error = String;

    fn try_from(millis: u32) -> Result<Self, Self::Error> {
        if i128::from(millis) > MAX_POLL_WAIT.as_millis() {
            return Err(format!(
                "wait_ms must be at most {}",
                MAX_POLL_WAIT.as_millis()
            ));
        }
        Ok(Self(millis))
    }
}

impl From<WaitMillis> for u32 {
    fn from(wait: WaitMillis) -> Self {
        wait.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PollRequest {
    pub(crate) version: Version1,
    pub(crate) seed: Seed,
    /// Return comments with a sequence number above this.
    pub(crate) after: Seq,
    pub(crate) wait_ms: WaitMillis,
}

/// One delivered comment and its place in the queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolledComment {
    pub(crate) seq: Seq,
    pub(crate) comment: Comment,
}

/// Responses accept unknown fields (they come from the server), so a newer
/// server may add some.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PollResponse {
    pub(crate) version: Version1,
    #[serde(flatten)]
    pub(crate) outcome: PollOutcome,
}

/// `{"comments": [...]}` (empty at the deadline) or `{"error": {...}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PollOutcome {
    Comments(Vec<PolledComment>),
    Error(ProtocolError),
}

#[cfg(test)]
mod tests {
    use crate::{
        core::{ids::Seq, version::Version1},
        protocol::poll::{PollOutcome, PollRequest, PollResponse},
    };

    #[test]
    fn request_round_trip_and_limits() {
        let line = r#"{"version":1,"seed":"aaaaaaaaaaaaaaaa","after":7,"wait_ms":60000}"#;
        let request: PollRequest = serde_json::from_str(line).unwrap();
        assert_eq!(request.after, Seq(7));
        assert_eq!(serde_json::to_string(&request).unwrap(), line);
        for bad in [
            r#"{"version":1,"seed":"aaaaaaaaaaaaaaaa","after":7,"wait_ms":60001}"#,
            r#"{"version":2,"seed":"aaaaaaaaaaaaaaaa","after":7,"wait_ms":1}"#,
            r#"{"version":1,"seed":"AAAAAAAAAAAAAAAA","after":7,"wait_ms":1}"#,
            r#"{"version":1,"seed":"aaaaaaaaaaaaaaaa","after":-1,"wait_ms":1}"#,
            r#"{"version":1,"seed":"aaaaaaaaaaaaaaaa","after":7,"wait_ms":1,"x":1}"#,
        ] {
            assert!(serde_json::from_str::<PollRequest>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn response_shapes() {
        let empty = PollResponse {
            version: Version1,
            outcome: PollOutcome::Comments(vec![]),
        };
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            r#"{"version":1,"comments":[]}"#
        );
        let parsed: PollResponse = serde_json::from_str(
            r#"{"version":1,"error":{"code":"not_found","message":"no such session"}}"#,
        )
        .unwrap();
        assert!(matches!(parsed.outcome, PollOutcome::Error(_)));
    }
}
