//! Actors, roles, and branded IDs used by the entitlement gate.
//!
//! These are **provisional** domain identifiers. Todo 2 (`crates/domain`) defines the
//! canonical branded ID/primitives contracts; this crate intentionally duplicates the
//! minimal subset so the gate stays self-contained and does not depend on an
//! in-flight sibling crate.

use std::fmt;

/// A user id. Never derive rights from this string - only from an explicit
/// entitlement record's `covered_users`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(pub String);

/// A dataset id, e.g. `krx_eod_bars`, `krx_instruments`, `krx_calendar`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DatasetId(pub String);

/// An entitlement record id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntitlementId(pub String);

impl UserId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl DatasetId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Canonical KRX end-of-day bars dataset.
    pub fn krx_eod_bars() -> Self {
        Self::new("krx_eod_bars")
    }
}

impl EntitlementId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for DatasetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for EntitlementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The market-data provider that the entitlement covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataProvider {
    /// Korea Exchange (KRX) licensed end-of-day / reference / calendar data.
    Krx,
}

impl DataProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Krx => "krx",
        }
    }
}

/// Role of an actor. Owner-only development paths bypass entitlement; Member-visible
/// surfaces are gated for **both** roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Member,
    Owner,
}

/// An authenticated actor making an access request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Actor {
    pub user_id: UserId,
    pub role: Role,
}

impl Actor {
    pub fn new(user_id: impl Into<String>, role: Role) -> Self {
        Self {
            user_id: UserId::new(user_id),
            role,
        }
    }

    pub fn member(user_id: impl Into<String>) -> Self {
        Self::new(user_id, Role::Member)
    }

    pub fn owner(user_id: impl Into<String>) -> Self {
        Self::new(user_id, Role::Owner)
    }

    pub fn is_owner(&self) -> bool {
        self.role == Role::Owner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_roles() {
        assert!(Actor::owner("own_1").is_owner());
        assert!(!Actor::member("usr_a").is_owner());
        assert_eq!(Actor::member("usr_a").user_id, UserId::new("usr_a"));
    }

    #[test]
    fn canonical_dataset() {
        assert_eq!(DatasetId::krx_eod_bars(), DatasetId::new("krx_eod_bars"));
    }
}
