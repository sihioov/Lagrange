//! `risk-gateway` — the persisted gatekeeper every Live order passes through.
//!
//! Design §6.13 (the twelve ordered checks), §16 (fail-closed), §15.2/§15.3
//! (metrics and alert grades); plan Todo 38. Implements the approved decision
//! that "persisted Live gatekeeper state before any KIS submission path ...
//! must survive restart and remain blocking until green".
//!
//! # What this crate guarantees
//!
//! **An order cannot be submitted without an approval, and an approval cannot
//! exist without a committed decision record.** [`RiskApproval`] has no public
//! constructor, so only [`gate::evaluate_and_record`] can mint one, and it
//! does so only after the store confirms the write. It is not `Clone`, and
//! submission consumes it by value, so one approval authorises exactly one
//! submission. This is a type-level property: bypassing the gate is not
//! something the caller is trusted to avoid, it is something that does not
//! compile.
//!
//! **Every uncertain input denies.** The snapshot types have explicit
//! `Unknown` arms rather than `Option`, so "we could not determine this" is a
//! case the check must handle, and every check handles it by denying with
//! [`DenyReason::InputUnavailable`] — kept distinct from the policy reasons so
//! that an outage is never recorded as a rejection.
//!
//! **A decision is reproducible.** Checks are pure functions of
//! `(snapshot, limits)`; nothing reads a clock or a database mid-evaluation.
//! Replaying a persisted snapshot through [`gate::evaluate`] yields the same
//! verdict, which is how the restart clause is tested rather than asserted.
//!
//! # One intent, one decision
//!
//! An intent is evaluated exactly once. A denial terminates it; a retry is a
//! NEW intent with a new correlation id (the state machine of Todo 39). This
//! assumption is what makes the unique index
//! `risk_events_one_gate_decision_per_intent` correct rather than merely
//! strict, and it is why [`store::RiskEventStore`] implementations must reject
//! a second decision instead of overwriting the first.
//!
//! # Scope
//!
//! The KIS submission path itself lands with Todo 39. What is proven here is
//! that a submit-shaped function demanding a [`RiskApproval`] cannot be
//! reached without one; wiring the real adapter and the api-server route to
//! that signature is Todo 39's work.

pub mod checks;
pub mod decision;
pub mod gate;
pub mod limits;
pub mod snapshot;
pub mod store;
pub mod testing;

pub use decision::{
    CHECK_ORDER, Check, CheckOutcome, CheckRecord, Decision, DenyReason, GateOutcome, RiskApproval,
};
pub use gate::{evaluate, evaluate_and_record};
pub use limits::{LimitsError, RiskLimits};
pub use snapshot::RiskSnapshot;
pub use store::{RiskEventStore, StoreError};
