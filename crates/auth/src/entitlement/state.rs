//! `PENDING | ACTIVE | EXPIRED | REVOKED` entitlement lifecycle with typed transitions.

/// Lifecycle of a `data_entitlements` record.
///
/// - `PENDING` - contract recorded, awaiting activation; never grants Member access.
/// - `ACTIVE`  - written rights in effect for the effective window.
/// - `EXPIRED` - the effective window has lapsed; terminal.
/// - `REVOKED` - rights withdrawn; terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EntitlementState {
    #[default]
    Pending,
    Active,
    Expired,
    Revoked,
}

impl EntitlementState {
    /// Stable machine-readable tag (mirrors the `data_entitlements.lifecycle` column).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Active => "ACTIVE",
            Self::Expired => "EXPIRED",
            Self::Revoked => "REVOKED",
        }
    }

    /// **Only `ACTIVE` permits Member-visible access.** Every other state fails closed.
    pub const fn allows_member_access(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Typed transition table. `EXPIRED` and `REVOKED` are terminal; `Pending → Active`
/// additionally requires the transition date to fall inside the effective window
/// (checked by [`crate::entitlement::entitlement::Entitlement::transition`]).
pub(crate) const fn is_allowed_transition(from: EntitlementState, to: EntitlementState) -> bool {
    matches!(
        (from, to),
        (EntitlementState::Pending, EntitlementState::Active)
            | (EntitlementState::Pending, EntitlementState::Expired)
            | (EntitlementState::Pending, EntitlementState::Revoked)
            | (EntitlementState::Active, EntitlementState::Expired)
            | (EntitlementState::Active, EntitlementState::Revoked)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::error::TransitionError;

    #[test]
    fn stable_tags() {
        assert_eq!(EntitlementState::Pending.as_str(), "PENDING");
        assert_eq!(EntitlementState::Active.as_str(), "ACTIVE");
        assert_eq!(EntitlementState::Expired.as_str(), "EXPIRED");
        assert_eq!(EntitlementState::Revoked.as_str(), "REVOKED");
    }

    #[test]
    fn only_active_allows_member_access() {
        assert!(EntitlementState::Active.allows_member_access());
        for s in [EntitlementState::Pending, EntitlementState::Expired, EntitlementState::Revoked] {
            assert!(!s.allows_member_access(), "{:?} must fail closed", s);
        }
    }

    #[test]
    fn allowed_transition_table() {
        use EntitlementState as S;
        // Legal:
        assert!(is_allowed_transition(S::Pending, S::Active));
        assert!(is_allowed_transition(S::Pending, S::Expired));
        assert!(is_allowed_transition(S::Pending, S::Revoked));
        assert!(is_allowed_transition(S::Active, S::Expired));
        assert!(is_allowed_transition(S::Active, S::Revoked));
        // Illegal:
        assert!(!is_allowed_transition(S::Active, S::Pending));
        assert!(!is_allowed_transition(S::Active, S::Active));
        assert!(!is_allowed_transition(S::Pending, S::Pending));
        assert!(!is_allowed_transition(S::Expired, S::Active));
        assert!(!is_allowed_transition(S::Expired, S::Pending));
        assert!(!is_allowed_transition(S::Expired, S::Revoked));
        assert!(!is_allowed_transition(S::Revoked, S::Active));
        assert!(!is_allowed_transition(S::Revoked, S::Pending));
        assert!(!is_allowed_transition(S::Revoked, S::Expired));
        assert!(!is_allowed_transition(S::Revoked, S::Revoked));
    }

    #[test]
    fn transition_error_is_typed() {
        let err = TransitionError::InvalidTransition {
            from: EntitlementState::Revoked,
            to: EntitlementState::Active,
        };
        assert!(err.to_string().contains("REVOKED"));
        assert!(err.to_string().contains("ACTIVE"));
    }
}
