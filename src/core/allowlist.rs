//! The global allowlist: who may sign in, and who may add share keys.

use std::{collections::BTreeSet, fmt};

use crate::core::ids::UserId;

/// `allow` (may view and comment) and `share` (may also add keys and
/// publish); `share` is a subset of `allow`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Allowlist {
    allow: BTreeSet<UserId>,
    share: BTreeSet<UserId>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ShareNotAllowed(pub(crate) UserId);

impl fmt::Display for ShareNotAllowed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "user {} is in share but not in allow", self.0)
    }
}

impl std::error::Error for ShareNotAllowed {}

impl Allowlist {
    pub(crate) fn new(
        allow: impl IntoIterator<Item = UserId>,
        share: impl IntoIterator<Item = UserId>,
    ) -> Result<Self, ShareNotAllowed> {
        let allow: BTreeSet<UserId> = allow.into_iter().collect();
        let share: BTreeSet<UserId> = share.into_iter().collect();
        if let Some(outsider) = share.difference(&allow).next() {
            return Err(ShareNotAllowed(*outsider));
        }
        Ok(Self { allow, share })
    }

    #[must_use]
    pub(crate) fn is_allowed(&self, user: UserId) -> bool {
        self.allow.contains(&user)
    }

    #[must_use]
    pub(crate) fn may_share(&self, user: UserId) -> bool {
        self.share.contains(&user)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::{
        allowlist::{Allowlist, ShareNotAllowed},
        ids::UserId,
    };

    #[test]
    fn share_is_a_subset_of_allow() {
        let list = Allowlist::new([UserId(1), UserId(2)], [UserId(1)]).unwrap();
        assert!(list.is_allowed(UserId(2)));
        assert!(list.may_share(UserId(1)));
        assert!(!list.may_share(UserId(2)));
        assert!(!list.is_allowed(UserId(3)));
        assert_eq!(
            Allowlist::new([UserId(1)], [UserId(3)]),
            Err(ShareNotAllowed(UserId(3)))
        );
    }
}
