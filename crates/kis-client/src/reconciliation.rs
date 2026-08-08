//! Reconciling our record against the broker's (plan Todo 40).
//!
//! Design §6.12 and §7.3; requirement FR-LIVE-004: "시작 시 주문·포지션·잔고를
//! 증권사 상태와 대사해야 한다. 불일치가 해결되지 않으면 전략 시작과 신규
//! 주문이 차단된다."
//!
//! Todo 36 deliberately deferred the typed account, position and open-order
//! snapshots to this module, "which consumes them". They live here.
//!
//! # The rule
//!
//! **The broker is the truth about the broker.** We never overwrite an
//! unexplained difference in our favour: a position we cannot account for is a
//! position that really exists in a real account, and quietly adopting our own
//! number would hide it. Every unexplained difference BLOCKS, and only an
//! audited Owner action can clear it (§16 "내부·브로커 포지션 불일치 → Live
//! 전략 일시정지, 관리자 승인 필요").
//!
//! # One definition of green
//!
//! [`ReconciliationOutcome::is_green`] is the ONLY definition. The Risk
//! Gateway's check 5 reads a `Reconciliation` snapshot field derived from it,
//! and `reconciliation_runs` records it. Two readings of "green" would mean
//! the gate and the reconciler could disagree about whether trading is
//! allowed, which is the kind of disagreement that is only discovered by
//! trading when you should not have.

use crate::order_state::OrderIntentState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A position as one side reports it. Quantities are whole units; cash and
/// values are scale-4 decimal strings, exactly as they cross the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionSnapshot {
    pub instrument_id: String,
    pub quantity: i64,
}

/// The broker's view of an account at a moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerAccountSnapshot {
    pub account_no_masked: String,
    /// Settled cash, as an exact decimal string.
    pub cash: String,
    pub positions: Vec<PositionSnapshot>,
    /// Orders the broker still considers working.
    pub open_orders: Vec<BrokerOpenOrder>,
    /// Fills the broker reports for today, by execution id.
    pub day_fills: Vec<BrokerFill>,
    /// When the broker says this snapshot was taken (unix seconds).
    pub as_of_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerOpenOrder {
    pub broker_order_no: String,
    pub instrument_id: String,
    pub side: String,
    pub quantity: i64,
    pub filled_quantity: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerFill {
    pub execution_id: String,
    pub broker_order_no: String,
    pub quantity: i64,
}

/// Our own view, assembled from the ledger and the intent store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalAccountSnapshot {
    pub cash: String,
    pub positions: Vec<PositionSnapshot>,
    /// Intents we believe are still working, with the broker order number we
    /// have bound to each (`None` while `SUBMITTING`/`SUBMITTED`/`UNKNOWN`).
    pub working_intents: Vec<LocalIntent>,
    pub known_execution_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalIntent {
    pub intent_ref: String,
    pub state: OrderIntentState,
}

/// One way the two views differ.
///
/// Every variant blocks. There is no "warning" tier: a difference we cannot
/// explain is a difference we cannot trade through, and a tier that did not
/// block would exist only to be ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mismatch")]
pub enum Mismatch {
    /// Quantities differ for an instrument.
    Position {
        instrument_id: String,
        ours: i64,
        brokers: i64,
    },
    /// Settled cash differs.
    Cash { ours: String, brokers: String },
    /// The broker is working an order we have no intent for. The most serious
    /// kind: a real order nobody is managing.
    UnmappedBrokerOrder {
        broker_order_no: String,
        instrument_id: String,
    },
    /// We think an order is working; the broker has never heard of it.
    UnknownToBroker {
        intent_ref: String,
        broker_order_no: String,
    },
    /// An intent stuck in `UNKNOWN`: we do not know whether an order exists.
    UnresolvedIntent { intent_ref: String },
    /// A fill the broker reports that we have not applied.
    MissingFill {
        execution_id: String,
        broker_order_no: String,
    },
    /// The snapshot is too old to reconcile against.
    StaleSnapshot { age_secs: i64, max_age_secs: i64 },
}

impl Mismatch {
    /// Whether an automated pass may resolve this without an Owner.
    ///
    /// Only one kind qualifies: a fill the broker reports and we have not
    /// applied is not a disagreement about the world, it is a message we
    /// missed, and applying it is exactly what the ledger's idempotent
    /// `fill_id` handling is for. Everything else is a genuine difference in
    /// belief about a real account, and adopting our own number would be
    /// overwriting broker truth we cannot explain.
    pub const fn is_auto_resolvable(&self) -> bool {
        matches!(self, Mismatch::MissingFill { .. })
    }

    pub const fn kind(&self) -> &'static str {
        match self {
            Mismatch::Position { .. } => "POSITION",
            Mismatch::Cash { .. } => "CASH",
            Mismatch::UnmappedBrokerOrder { .. } => "UNMAPPED_BROKER_ORDER",
            Mismatch::UnknownToBroker { .. } => "UNKNOWN_TO_BROKER",
            Mismatch::UnresolvedIntent { .. } => "UNRESOLVED_INTENT",
            Mismatch::MissingFill { .. } => "MISSING_FILL",
            Mismatch::StaleSnapshot { .. } => "STALE_SNAPSHOT",
        }
    }
}

/// The result of one reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationOutcome {
    pub mismatches: Vec<Mismatch>,
    /// Fills the pass would apply to close a `MissingFill`.
    pub fills_to_apply: Vec<BrokerFill>,
    /// Intents needing a broker status lookup before anything else can happen.
    pub lookups_required: Vec<String>,
}

impl ReconciliationOutcome {
    /// THE definition of green. Trading is permitted only when this is true.
    ///
    /// Green means every difference is resolved, not "resolvable". A pass that
    /// found fills to apply is NOT green until they have actually been
    /// applied and a later pass finds nothing — otherwise "green" would mean
    /// "green after some work nobody has done yet", and the Risk Gateway would
    /// let orders through on the strength of it.
    pub fn is_green(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// Differences that no automated pass may clear.
    pub fn blocking(&self) -> Vec<&Mismatch> {
        self.mismatches
            .iter()
            .filter(|m| !m.is_auto_resolvable())
            .collect()
    }

    /// Whether an Owner must intervene before trading can resume.
    pub fn requires_owner(&self) -> bool {
        !self.blocking().is_empty()
    }
}

/// Compares the two views.
///
/// Pure: no clock, no network. `now_secs` is passed so a reconciliation can be
/// replayed from stored snapshots and reach the same verdict, the same
/// property the gate and the order machine rely on.
///
/// # Caller contract: sweep in-flight intents to `Unknown` FIRST
///
/// An intent sitting in `Submitting` or `Submitted` is epistemically
/// identical to one in `Unknown` — the broker may or may not hold the order —
/// but only `Unknown` is flagged here, and deliberately so. Flagging every
/// in-flight intent unconditionally would make a runtime pass catch the
/// ordinary 200ms submission window and flap readiness between green and
/// blocked for no reason.
///
/// The consequence is a real hazard the caller must close: after a crash
/// mid-submit, the intent stays `Submitting`, positions and cash may agree
/// (the order may never have landed), and this function returns GREEN while
/// `OrderIntentRepo::unresolved()` still lists it — two "may we trade?"
/// signals disagreeing, which is the exact failure mode this module's single
/// definition of green exists to prevent.
///
/// So a STARTUP pass must sweep first: take `unresolved()`, apply
/// [`crate::order_state::Event::SubmissionTimedOut`] to every in-flight
/// intent (a legal transition to `Unknown`), and only then reconcile. The
/// swept intents are then flagged as [`Mismatch::UnresolvedIntent`] and
/// demand the broker lookup that can actually settle them. A RUNTIME pass
/// skips the sweep, because there an in-flight intent is simply young.
/// `reconciliation_green_despite_an_in_flight_intent_is_the_callers_hazard`
/// pins this.
pub fn reconcile(
    local: &LocalAccountSnapshot,
    broker: &BrokerAccountSnapshot,
    now_secs: i64,
    max_age_secs: i64,
) -> ReconciliationOutcome {
    let mut mismatches = Vec::new();
    let mut fills_to_apply = Vec::new();
    let mut lookups_required = Vec::new();

    // A snapshot too old to trust is not evidence of agreement. Checked first:
    // every comparison below is only as good as the snapshot's currency.
    let age = now_secs.saturating_sub(broker.as_of_secs);
    if age > max_age_secs || age < 0 {
        mismatches.push(Mismatch::StaleSnapshot {
            age_secs: age,
            max_age_secs,
        });
    }

    // Positions, over the UNION of both sides. Iterating only our own would
    // miss an instrument the broker holds and we do not — which is precisely
    // the case that matters most.
    let ours: BTreeMap<&str, i64> = local
        .positions
        .iter()
        .map(|p| (p.instrument_id.as_str(), p.quantity))
        .collect();
    let theirs: BTreeMap<&str, i64> = broker
        .positions
        .iter()
        .map(|p| (p.instrument_id.as_str(), p.quantity))
        .collect();
    let instruments: BTreeSet<&str> = ours.keys().chain(theirs.keys()).copied().collect();
    for instrument in instruments {
        let a = ours.get(instrument).copied().unwrap_or(0);
        let b = theirs.get(instrument).copied().unwrap_or(0);
        if a != b {
            mismatches.push(Mismatch::Position {
                instrument_id: instrument.to_string(),
                ours: a,
                brokers: b,
            });
        }
    }

    // Cash, compared as exact decimal text after normalising trailing zeros.
    if !decimal_eq(&local.cash, &broker.cash) {
        mismatches.push(Mismatch::Cash {
            ours: local.cash.clone(),
            brokers: broker.cash.clone(),
        });
    }

    // Orders, in both directions.
    let broker_orders: BTreeMap<&str, &BrokerOpenOrder> = broker
        .open_orders
        .iter()
        .map(|o| (o.broker_order_no.as_str(), o))
        .collect();
    let mut ours_bound: BTreeSet<&str> = BTreeSet::new();

    for intent in &local.working_intents {
        match intent.state.broker_order_no() {
            Some(no) => {
                ours_bound.insert(no);
                if !broker_orders.contains_key(no) && !is_terminal_locally(&intent.state) {
                    // We think it is working; the broker does not have it.
                    mismatches.push(Mismatch::UnknownToBroker {
                        intent_ref: intent.intent_ref.clone(),
                        broker_order_no: no.to_string(),
                    });
                }
            }
            None => {
                // No broker order bound yet. If it is UNKNOWN we do not even
                // know whether one exists, and only a lookup can say.
                if intent.state == OrderIntentState::Unknown {
                    mismatches.push(Mismatch::UnresolvedIntent {
                        intent_ref: intent.intent_ref.clone(),
                    });
                    lookups_required.push(intent.intent_ref.clone());
                }
            }
        }
    }

    for (no, order) in &broker_orders {
        if !ours_bound.contains(no) {
            mismatches.push(Mismatch::UnmappedBrokerOrder {
                broker_order_no: (*no).to_string(),
                instrument_id: order.instrument_id.clone(),
            });
        }
    }

    // Fills the broker reports that we have never applied.
    let known: BTreeSet<&str> = local
        .known_execution_ids
        .iter()
        .map(String::as_str)
        .collect();
    for fill in &broker.day_fills {
        if !known.contains(fill.execution_id.as_str()) {
            mismatches.push(Mismatch::MissingFill {
                execution_id: fill.execution_id.clone(),
                broker_order_no: fill.broker_order_no.clone(),
            });
            fills_to_apply.push(fill.clone());
        }
    }

    ReconciliationOutcome {
        mismatches,
        fills_to_apply,
        lookups_required,
    }
}

/// Whether a state means the order is finished from our side.
fn is_terminal_locally(state: &OrderIntentState) -> bool {
    state.is_terminal()
}

/// Exact decimal comparison that ignores trailing-zero formatting.
///
/// `"1000"` and `"1000.0000"` are the same amount; PostgreSQL renders
/// `numeric(18,4)` with the scale and a broker may not. Comparing the strings
/// directly would report a cash mismatch on every single reconciliation, and
/// the resulting alert fatigue would bury a real one.
fn decimal_eq(a: &str, b: &str) -> bool {
    fn normalise(s: &str) -> (bool, String, String) {
        let s = s.trim();
        let (neg, rest) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (int, frac) = rest.split_once('.').unwrap_or((rest, ""));
        let int = int.trim_start_matches('0');
        let frac = frac.trim_end_matches('0');
        (
            neg && !(int.is_empty() && frac.is_empty()),
            int.to_string(),
            frac.to_string(),
        )
    }
    normalise(a) == normalise(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_comparison_ignores_formatting_but_not_value() {
        assert!(decimal_eq("1000", "1000.0000"));
        assert!(decimal_eq("0", "0.0000"));
        assert!(decimal_eq("0", "-0.0000"));
        assert!(decimal_eq("007.50", "7.5"));
        assert!(!decimal_eq("1000", "1000.0001"));
        assert!(!decimal_eq("100", "-100"));
    }
}
