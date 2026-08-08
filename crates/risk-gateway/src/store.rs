//! Where a decision is durably recorded.
//!
//! The trait exists so the gate can be proven without a database, but the
//! contract it states is a database contract: the write must be committed
//! before `record` returns, and the returned id must be the row's. An
//! implementation that buffered, batched, or wrote asynchronously would let
//! the gate mint an approval for a decision that no longer exists after a
//! crash — the exact failure §16 blocks Live orders to prevent.

use crate::decision::Decision;
use crate::snapshot::RiskSnapshot;

/// Why a decision could not be recorded.
///
/// Deliberately not an enum of causes: the gate's response to every one of
/// them is identical (deny), and a richer type would invite a caller to treat
/// some failures as recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    pub detail: String,
}

impl StoreError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "risk decision was not recorded: {}", self.detail)
    }
}

impl std::error::Error for StoreError {}

/// Persists gate decisions.
///
/// Implementors must guarantee:
///
/// 1. **Committed on return.** `Ok(id)` means the row survives a crash.
/// 2. **One decision per intent.** A second `record` for an `intent_ref` that
///    already has one must return `Err`, not overwrite. The unique index
///    `risk_events_one_gate_decision_per_intent` provides this for the real
///    store; a double-decision must never become a second approval.
/// 3. **Append-only.** Nothing may edit a recorded decision afterwards
///    (migration 0018 revokes UPDATE/DELETE and installs a reject trigger).
///
/// The method is async because the real implementation talks to PostgreSQL,
/// and the write has to happen INSIDE the gate. Lifting persistence out to the
/// caller — the shape a synchronous trait would have forced — would dissolve
/// the guarantee entirely: the approval must not exist until the row does.
pub trait RiskEventStore {
    /// Records the decision and its full input snapshot, returning the
    /// `risk_events.id` of the row.
    fn record(
        &self,
        decision: &Decision,
        snapshot: &RiskSnapshot,
    ) -> impl std::future::Future<Output = Result<String, StoreError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_error_names_what_failed_without_pretending_to_be_recoverable() {
        let e = StoreError::new("connection reset");
        assert_eq!(
            e.to_string(),
            "risk decision was not recorded: connection reset"
        );
    }
}
