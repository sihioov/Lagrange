//! Fixtures and store doubles.
//!
//! Public (not `#[cfg(test)]`) so integration tests in `tests/` can build the
//! same all-green baseline and vary one field at a time. That pattern is what
//! makes the table tests readable: each case says exactly which of the twelve
//! inputs it broke, and everything else is known good.

use crate::decision::Decision;
use crate::limits::RiskLimits;
use crate::snapshot::{
    AccountState, Allowlisted, DataFreshness, IntentConflict, KillSwitch, MarketSession,
    OrderIntent, Reconciliation, RiskSnapshot, Side, StrategyPromotion,
};
use crate::store::{RiskEventStore, StoreError};
use domain::{Currency, Money, Price, Quantity};
use std::collections::BTreeSet;
use std::sync::Mutex;

/// KRW money from a decimal string.
pub fn krw(value: &str) -> Money {
    Money::parse(value, Currency::KRW).expect("valid KRW amount")
}

/// The limit set the fixtures are sized against: 30% per symbol, 1,000,000
/// per order, 5,000,000 per day, 500,000 daily loss, 300s data age.
pub fn limits() -> RiskLimits {
    RiskLimits::new(
        "risk-limits-v1",
        3_000,
        krw("1000000"),
        krw("5000000"),
        krw("500000"),
        300,
    )
    .expect("valid limit set")
}

/// A snapshot that passes all twelve checks.
///
/// A 10-unit buy at 7,250 is 72,500: comfortably inside the per-order and
/// daily limits, and 7.25% of a 1,000,000 account, inside the 30% weight cap.
pub fn snapshot_all_green() -> RiskSnapshot {
    RiskSnapshot {
        intent: OrderIntent {
            intent_ref: "intent-1".into(),
            account_id: "account-1".into(),
            instrument_id: "069500.KRX".into(),
            side: Side::Buy,
            quantity: Quantity::parse("10").expect("valid quantity"),
            price: Some(Price::parse("7250").expect("valid price")),
        },
        correlation_id: "correlation-1".into(),
        evaluated_at_secs: 1_800_000_000,
        kill_switch: KillSwitch::Disengaged,
        market_session: MarketSession::Open,
        data_freshness: DataFreshness::Age(30),
        strategy_promotion: StrategyPromotion::LiveCandidate,
        reconciliation: Reconciliation::Green,
        instrument_allowed: Allowlisted::Allowed,
        account: AccountState {
            equity: krw("1000000"),
            available_cash: krw("500000"),
            available_quantity: Quantity::parse("100").expect("valid quantity"),
            position_value: krw("0"),
            daily_order_value: krw("0"),
            daily_loss: krw("0"),
        },
        conflict: IntentConflict::None,
    }
}

/// A store that accepts the first decision per intent and remembers it.
///
/// Models the real store's unique index: a second decision for an
/// `intent_ref` that already has one is an error, never an overwrite.
///
/// State lives behind a `Mutex` rather than a `RefCell` so the double is
/// `Send + Sync`: the gate's future must stay `Send` to be usable from an
/// axum handler, and a test double that quietly made it `!Send` would compile
/// here and fail only once the real caller was written.
#[derive(Default)]
pub struct RecordingStore {
    rows: Mutex<Vec<(Decision, RiskSnapshot)>>,
    intents: Mutex<BTreeSet<String>>,
}

impl RecordingStore {
    /// Every recorded decision with the snapshot it was made from.
    pub fn records(&self) -> Vec<(Decision, RiskSnapshot)> {
        self.rows
            .lock()
            .expect("recording store is not poisoned")
            .clone()
    }
}

impl RiskEventStore for RecordingStore {
    async fn record(
        &self,
        decision: &Decision,
        snapshot: &RiskSnapshot,
    ) -> Result<String, StoreError> {
        if !self
            .intents
            .lock()
            .expect("recording store is not poisoned")
            .insert(decision.intent_ref.clone())
        {
            return Err(StoreError::new(format!(
                "intent {} already has a gate decision",
                decision.intent_ref
            )));
        }
        let mut rows = self.rows.lock().expect("recording store is not poisoned");
        rows.push((decision.clone(), snapshot.clone()));
        Ok(format!("risk-event-{}", rows.len()))
    }
}

/// A store whose write always fails, for the §16 durability path.
pub struct FailingStore {
    detail: String,
}

impl FailingStore {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl RiskEventStore for FailingStore {
    async fn record(
        &self,
        _decision: &Decision,
        _snapshot: &RiskSnapshot,
    ) -> Result<String, StoreError> {
        Err(StoreError::new(self.detail.clone()))
    }
}
