//! The session model and its validation.

use std::fmt;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::core::{
    allowlist::Allowlist,
    ids::{Seed, ShareKey, UserId},
    text::check_line,
};

/// Maximum session label length in characters.
pub(crate) const MAX_SESSION_LABEL: usize = 100;
/// Maximum serialised size of a session's `client` blob.
pub(crate) const MAX_CLIENT_BYTES: usize = 4096;

/// A session label: 1 to 100 characters, not blank, no control characters.
/// Surrounding whitespace is trimmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct SessionLabel(String);

impl SessionLabel {
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionLabel {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let trimmed = raw.trim();
        check_line(trimmed, "label", MAX_SESSION_LABEL)?;
        Ok(Self(trimmed.to_owned()))
    }
}

impl From<SessionLabel> for String {
    fn from(label: SessionLabel) -> Self {
        label.0
    }
}

impl fmt::Display for SessionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque JSON owned by the client, at most [`MAX_CLIENT_BYTES`] when
/// serialised. The server stores and returns it and never reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value", into = "serde_json::Value")]
pub(crate) struct ClientBlob(serde_json::Value);

impl TryFrom<serde_json::Value> for ClientBlob {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        let size = serde_json::to_vec(&value)
            .map_err(|error| error.to_string())?
            .len();
        if size > MAX_CLIENT_BYTES {
            return Err(format!(
                "client must be at most {MAX_CLIENT_BYTES} bytes of JSON"
            ));
        }
        Ok(Self(value))
    }
}

impl From<ClientBlob> for serde_json::Value {
    fn from(blob: ClientBlob) -> Self {
        blob.0
    }
}

/// Session expiry from the config: default and maximum lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionLimits {
    default_expiry: SignedDuration,
    max_expiry: SignedDuration,
}

impl SessionLimits {
    pub(crate) fn new(
        default_expiry: SignedDuration,
        max_expiry: SignedDuration,
    ) -> Result<Self, SessionError> {
        if !default_expiry.is_positive() || default_expiry > max_expiry {
            return Err(SessionError::Limits);
        }
        Ok(Self {
            default_expiry,
            max_expiry,
        })
    }
}

impl Default for SessionLimits {
    /// 7 days by default, at most 30.
    fn default() -> Self {
        Self {
            default_expiry: SignedDuration::from_hours(7 * 24),
            max_expiry: SignedDuration::from_hours(30 * 24),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionError {
    Limits,
    Expiry,
    EmptyAccess,
    NotAllowlisted(UserId),
    NotSharer(UserId),
    Record(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limits => f.write_str("default expiry must be positive and at most the maximum"),
            Self::Expiry => {
                f.write_str("expiry must be positive and at most the configured maximum")
            }
            Self::EmptyAccess => f.write_str("access must name at least one person, or be absent"),
            Self::NotAllowlisted(user) => write!(f, "user {user} is not on the allowlist"),
            Self::NotSharer(user) => write!(f, "user {user} may not publish sessions"),
            Self::Record(reason) => write!(f, "invalid session record: {reason}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Validates a requested access list: absent means everyone on the
/// allowlist; present must be non-empty and within the allowlist. The
/// result is sorted and deduplicated.
pub(crate) fn validate_access(
    access: Option<Vec<UserId>>,
    allowlist: &Allowlist,
) -> Result<Option<Vec<UserId>>, SessionError> {
    let Some(mut access) = access else {
        return Ok(None);
    };
    if access.is_empty() {
        return Err(SessionError::EmptyAccess);
    }
    if let Some(outsider) = access.iter().find(|user| !allowlist.is_allowed(**user)) {
        return Err(SessionError::NotAllowlisted(*outsider));
    }
    access.sort_unstable();
    access.dedup();
    Ok(Some(access))
}

/// What a share asks for when it creates a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionRequest {
    pub(crate) label: SessionLabel,
    pub(crate) access: Option<Vec<UserId>>,
    /// Lifetime; the configured default when absent.
    pub(crate) expires_in: Option<SignedDuration>,
    pub(crate) client: ClientBlob,
}

/// Who creates a session: the share key and the person who added it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Creator {
    pub(crate) owner: UserId,
    pub(crate) key: ShareKey,
}

/// One shared site, as stored in `sessions.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SessionRecord", into = "SessionRecord")]
pub(crate) struct Session {
    record: SessionRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRecord {
    pub(crate) seed: Seed,
    pub(crate) label: SessionLabel,
    pub(crate) owner: UserId,
    pub(crate) key: ShareKey,
    pub(crate) created: Timestamp,
    pub(crate) expires: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) access: Option<Vec<UserId>>,
    #[serde(default)]
    pub(crate) client: ClientBlob,
}

impl TryFrom<SessionRecord> for Session {
    type Error = SessionError;

    /// Checks what can be checked without the allowlist (which may have
    /// changed since the record was written): expiry after creation, and a
    /// non-empty, sorted, duplicate-free access list.
    fn try_from(record: SessionRecord) -> Result<Self, Self::Error> {
        if record.expires <= record.created {
            return Err(SessionError::Record(
                "expires must be after created".to_owned(),
            ));
        }
        if let Some(access) = &record.access {
            let strictly_sorted = access
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a < b));
            if access.is_empty() || !strictly_sorted {
                return Err(SessionError::Record(
                    "access must be non-empty, sorted and without duplicates".to_owned(),
                ));
            }
        }
        Ok(Self { record })
    }
}

impl From<Session> for SessionRecord {
    fn from(session: Session) -> Self {
        session.record
    }
}

impl Session {
    /// Validates `request` and creates a session for `creator`, who must
    /// be allowed to share.
    pub(crate) fn create(
        request: SessionRequest,
        seed: Seed,
        creator: Creator,
        now: Timestamp,
        limits: SessionLimits,
        allowlist: &Allowlist,
    ) -> Result<Self, SessionError> {
        let SessionRequest {
            label,
            access,
            expires_in,
            client,
        } = request;
        if !allowlist.may_share(creator.owner) {
            return Err(SessionError::NotSharer(creator.owner));
        }
        let lifetime = expires_in.unwrap_or(limits.default_expiry);
        if !lifetime.is_positive() || lifetime > limits.max_expiry {
            return Err(SessionError::Expiry);
        }
        let expires = now
            .checked_add(lifetime)
            .map_err(|_| SessionError::Expiry)?;
        Self::try_from(SessionRecord {
            seed,
            label,
            owner: creator.owner,
            key: creator.key,
            created: now,
            expires,
            access: validate_access(access, allowlist)?,
            client,
        })
    }

    #[must_use]
    pub(crate) fn record(&self) -> &SessionRecord {
        &self.record
    }

    #[must_use]
    pub(crate) fn seed(&self) -> Seed {
        self.record.seed
    }

    #[must_use]
    pub(crate) fn is_expired(&self, now: Timestamp) -> bool {
        now >= self.record.expires
    }

    /// Whether `user` may open this session: on the allowlist, and the
    /// owner or (when an access list is set) on it.
    #[must_use]
    pub(crate) fn may_view(&self, user: UserId, allowlist: &Allowlist) -> bool {
        let listed = match &self.record.access {
            None => true,
            Some(access) => access.binary_search(&user).is_ok(),
        };
        allowlist.is_allowed(user) && (user == self.record.owner || listed)
    }

    /// Replaces the access list, with the same rules as at creation.
    pub(crate) fn set_access(
        &mut self,
        access: Option<Vec<UserId>>,
        allowlist: &Allowlist,
    ) -> Result<(), SessionError> {
        self.record.access = validate_access(access, allowlist)?;
        Ok(())
    }

    pub(crate) fn set_label(&mut self, label: SessionLabel) {
        self.record.label = label;
    }

    pub(crate) fn set_client(&mut self, client: ClientBlob) {
        self.record.client = client;
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
