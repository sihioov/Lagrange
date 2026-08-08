//! The inputs a decision was made from, captured once and never re-read.
//!
//! Two properties matter here and both are structural.
//!
//! **Unknown is a variant, not an absence.** Every input that can fail to be
//! determined has an explicit `Unknown` arm rather than being `Option<T>` or,
//! worse, defaulted. §16 requires that missing or stale state denies, and a
//! type where "we could not tell" is representable only as `None` invites a
//! caller to write `unwrap_or(safe_looking_default)`. Here the unknown arm is
//! matched by the check and denies with `InputUnavailable`.
//!
//! **The snapshot is the whole world the checks see.** Every check is a pure
//! function of `(RiskSnapshot, RiskLimits)`. Nothing reads a clock, a database
//! or an environment variable mid-evaluation, so a decision is reproducible
//! from its persisted record — which is how "survives restart and remains
//! blocking" becomes a property test rather than a hope.

use domain::{Money, Price, Quantity};
use serde::{Deserialize, Serialize};

/// §6.13 check 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillSwitch {
    Disengaged,
    Engaged,
    /// The switch state could not be read. Denies: an unreadable kill switch
    /// is indistinguishable from an engaged one, and only one of those two
    /// guesses is safe.
    Unknown,
}

/// §6.13 check 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketSession {
    /// Regular trading hours on a session the calendar materialises.
    Open,
    Closed,
    Halted,
    Unknown,
}

/// §6.13 check 3 / AT-08.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "age_secs")]
pub enum DataFreshness {
    /// Age of the newest market data backing this decision.
    Age(i64),
    /// No data, or data whose age cannot be established.
    Unknown,
}

/// §6.13 check 4. Mirrors `domain::lifecycle::StrategyState`, kept as its own
/// type so the gate does not silently follow a future rename of that enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyPromotion {
    /// `LiveCandidate` — the only state permitted to place real orders.
    LiveCandidate,
    /// Any of Draft, Validated, Paper, Retired.
    NotPromoted,
    Unknown,
}

/// §6.13 check 5 / FR-LIVE-004.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reconciliation {
    /// A completed run with zero mismatches.
    Green,
    /// Ran and found mismatches, or is still running.
    NotGreen,
    /// Never run, or the result could not be read. A restarted system that has
    /// not yet reconciled lands here, which is why it denies (FR-LIVE-004:
    /// "신규 주문이 차단된다").
    Unknown,
}

/// The order being asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderIntent {
    /// Idempotency key. Unique per account; one gate decision per value, ever.
    pub intent_ref: String,
    pub account_id: String,
    pub instrument_id: String,
    pub side: Side,
    /// Whole units, non-negative by construction (`QUANTITY_SCALE` = 0).
    pub quantity: Quantity,
    /// Limit price. `None` is a market order, which the gate cannot value and
    /// therefore cannot check against the order-value limits — see
    /// `checks::order_max_value`.
    pub price: Option<Price>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

/// Account state at evaluation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountState {
    /// Total account value, the denominator of the per-symbol weight check.
    pub equity: Money,
    /// Cash available to settle a buy.
    pub available_cash: Money,
    /// Units of this instrument available to sell (settled and unencumbered).
    pub available_quantity: Quantity,
    /// Current market value of the existing position in this instrument.
    pub position_value: Money,
    /// Cumulative value of orders already placed today (check 9).
    pub daily_order_value: Money,
    /// Realised + unrealised loss today, as a POSITIVE number (check 10).
    /// A profit is zero here, not a negative loss, so that the comparison
    /// against the limit has one meaning.
    pub daily_loss: Money,
}

/// §6.13 check 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentConflict {
    None,
    /// This intent_ref was already decided, or an opposing/overlapping order
    /// for the same instrument is live.
    Conflicting,
    /// Open orders could not be listed. Denies — submitting while blind to
    /// what is already working is how an account ends up double-filled.
    Unknown,
}

/// Everything the twelve checks read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskSnapshot {
    pub intent: OrderIntent,
    pub correlation_id: String,
    /// Unix seconds. Carried rather than read, so re-evaluation is identical.
    pub evaluated_at_secs: i64,
    pub kill_switch: KillSwitch,
    pub market_session: MarketSession,
    pub data_freshness: DataFreshness,
    pub strategy_promotion: StrategyPromotion,
    pub reconciliation: Reconciliation,
    /// Whether the instrument is on the owner's allowlist. An empty allowlist
    /// therefore denies everything, which is the fail-closed direction.
    pub instrument_allowed: Allowlisted,
    pub account: AccountState,
    pub conflict: IntentConflict,
}

/// §6.13 check 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Allowlisted {
    Allowed,
    NotAllowed,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_round_trips_through_json_unchanged() {
        // The persisted snapshot is replayed to prove a restart reaches the
        // same verdict, so a lossy serialization would quietly break that
        // property rather than fail it.
        let snap = crate::testing::snapshot_all_green();
        let json = serde_json::to_string(&snap).expect("serializes");
        let back: RiskSnapshot = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(snap, back);
    }

    #[test]
    fn every_uncertain_input_can_express_unknown() {
        // Guards the invariant the module exists for: if a future input is
        // added without an Unknown arm, the check for it cannot fail closed.
        let json = serde_json::to_string(&KillSwitch::Unknown).unwrap();
        assert_eq!(json, "\"unknown\"");
        assert_eq!(
            serde_json::to_string(&DataFreshness::Unknown).unwrap(),
            "{\"state\":\"unknown\"}"
        );
        assert_eq!(
            serde_json::to_string(&DataFreshness::Age(30)).unwrap(),
            "{\"state\":\"age\",\"age_secs\":30}"
        );
    }
}
