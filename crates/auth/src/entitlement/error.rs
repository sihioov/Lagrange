//! Typed denial and transition errors for the entitlement gate.

use std::fmt;

use crate::entitlement::date::CalendarDate;
use crate::entitlement::identity::DatasetId;
use crate::entitlement::state::EntitlementState;
use crate::entitlement::use_registry::KrUse;

/// Stable denial code surfaced by API/report/artifact layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenialCode {
    /// The KR-derived Member-visible use requires an ACTIVE data entitlement.
    DataEntitlementRequired,
    /// The requested path is Owner-only development, never available to Members.
    OwnerOnlyDevelopmentPath,
}

impl DenialCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DataEntitlementRequired => "DATA_ENTITLEMENT_REQUIRED",
            Self::OwnerOnlyDevelopmentPath => "OWNER_ONLY_DEVELOPMENT_PATH",
        }
    }
}

impl fmt::Display for DenialCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a request was denied (richer than the code, for audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenialReason {
    /// No entitlement record covers the requested dataset at all.
    NoEntitlementRecord,
    /// An entitlement exists but its lifecycle/date status is not `ACTIVE`.
    EntitlementNotActive(EntitlementState),
    /// The actor's user id is not among the contract's covered users.
    UserNotCovered,
    /// The requested use is not covered by the entitlement.
    UseNotCovered,
    /// The requested use is an Owner-only development path.
    OwnerOnlyDevelopmentPath,
}

/// The typed failure returned by the authorization service (fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementDenied {
    pub code: DenialCode,
    pub dataset: DatasetId,
    pub use_kind: KrUse,
    /// Effective status of the governing entitlement, when one exists.
    pub state: Option<EntitlementState>,
    pub reason: DenialReason,
}

impl fmt::Display for EntitlementDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (dataset={}, use={}, state={:?}, reason={:?})",
            self.code, self.dataset, self.use_kind, self.state, self.reason
        )
    }
}

impl std::error::Error for EntitlementDenied {}

/// Typed lifecycle transition error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The transition is not in the allowed table.
    InvalidTransition {
        from: EntitlementState,
        to: EntitlementState,
    },
    /// `Pending -> Active` requires the transition date inside the effective window.
    OutsideEffectiveWindow {
        on: CalendarDate,
        effective_from: CalendarDate,
        effective_until: CalendarDate,
    },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid entitlement transition {} -> {}", from.as_str(), to.as_str())
            }
            Self::OutsideEffectiveWindow { on, effective_from, effective_until } => {
                write!(
                    f,
                    "activation date {on} outside effective window [{effective_from}, {effective_until}]"
                )
            }
        }
    }
}

impl std::error::Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlement::identity::{Actor, DatasetId};
    use crate::entitlement::use_registry::KrUse;

    #[test]
    fn denial_code_stable_strings() {
        assert_eq!(DenialCode::DataEntitlementRequired.as_str(), "DATA_ENTITLEMENT_REQUIRED");
        assert_eq!(DenialCode::OwnerOnlyDevelopmentPath.as_str(), "OWNER_ONLY_DEVELOPMENT_PATH");
    }

    #[test]
    fn denied_display_carries_code() {
        let d = EntitlementDenied {
            code: DenialCode::DataEntitlementRequired,
            dataset: DatasetId::krx_eod_bars(),
            use_kind: KrUse::Recommendation,
            state: Some(EntitlementState::Expired),
            reason: DenialReason::EntitlementNotActive(EntitlementState::Expired),
        };
        let s = d.to_string();
        assert!(s.contains("DATA_ENTITLEMENT_REQUIRED"));
        assert!(s.contains("krx_eod_bars"));
    }

    #[test]
    fn actor_not_required_in_error_type() {
        // Guard against accidental coupling: denial is about (dataset, use, state).
        let _ = Actor::member("x");
    }
}
