//! Lifecycle and state enums shared across the platform.
//!
//! All states serialize lowercase on the wire, implement `Display`/`FromStr`,
//! and are exhaustively matched by downstream todos — a sixth `JobStatus` or
//! an extra `AttemptOutcome` is a compile error, not a data surprise.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Defines a lifecycle enum with lowercase wire strings, `Display`, and
/// `FromStr`.
macro_rules! lifecycle_enum {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// The wire (lowercase) string of this state.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            /// Every public state of this lifecycle.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(DomainError::InvalidId {
                        kind: stringify!($name).to_owned(),
                        value: s.to_owned(),
                    }),
                }
            }
        }
    };
}

lifecycle_enum! {
    /// Public job states (design NFR-REL-002: exactly these five). `ORPHANED`
    /// is an ATTEMPT-level outcome only, never a public job state.
    JobStatus {
        Queued => "queued",
        Running => "running",
        Succeeded => "succeeded",
        Failed => "failed",
        Canceled => "canceled",
    }
}

impl JobStatus {
    /// Whether the job has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }
}

lifecycle_enum! {
    /// Outcome of a single job attempt (todo 19). `Orphaned` marks a lease
    /// expired by a dead worker and permits at most one requeue.
    AttemptOutcome {
        Succeeded => "succeeded",
        Failed => "failed",
        Canceled => "canceled",
        Orphaned => "orphaned",
    }
}

lifecycle_enum! {
    /// Dataset quality state (todo 11): `READY | WARNING | BLOCKED`.
    DataState {
        Ready => "ready",
        Warning => "warning",
        Blocked => "blocked",
    }
}

lifecycle_enum! {
    /// Data entitlement lifecycle (todo 5): only `Active` permits Member uses.
    EntitlementState {
        Pending => "pending",
        Active => "active",
        Expired => "expired",
        Revoked => "revoked",
    }
}

lifecycle_enum! {
    /// Instrument listing status (todo 9). Delisted instruments are kept, not
    /// removed, for point-in-time correctness.
    InstrumentStatus {
        Listed => "listed",
        Delisted => "delisted",
        Suspended => "suspended",
    }
}

lifecycle_enum! {
    /// Strategy package promotion state (todo 17):
    /// `Draft | Validated | Paper | LiveCandidate | Retired`.
    StrategyState {
        Draft => "draft",
        Validated => "validated",
        Paper => "paper",
        LiveCandidate => "live_candidate",
        Retired => "retired",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_status_exactly_five_and_terminal() {
        assert_eq!(JobStatus::ALL.len(), 5);
        assert!(JobStatus::Succeeded.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert_eq!(JobStatus::Queued.as_str(), "queued");
        assert_eq!("succeeded".parse::<JobStatus>(), Ok(JobStatus::Succeeded));
        assert!(matches!(
            "orphaned".parse::<JobStatus>(),
            Err(DomainError::InvalidId { .. })
        ));
    }

    #[test]
    fn attempt_outcome_has_orphaned() {
        assert!(AttemptOutcome::ALL.contains(&AttemptOutcome::Orphaned));
        assert_eq!(AttemptOutcome::Orphaned.as_str(), "orphaned");
    }

    #[test]
    fn serde_round_trip() {
        for state in JobStatus::ALL {
            let json = serde_json::to_string(state).unwrap();
            let back: JobStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, *state);
        }
        let json = serde_json::to_string(&StrategyState::LiveCandidate).unwrap();
        assert_eq!(json, "\"live_candidate\"");
        assert!(serde_json::from_str::<JobStatus>("\"blocked\"").is_err());
    }
}
