//! The deterministic Paper session flow (plan Todo 31; design §9.2
//! processing order, §10.2; requirements UC-04, FR-PAPER-002/003, AT-07).
//!
//! The documented order is structural:
//!
//! ```text
//! DailyBarClosedEvent(T)   -> compute targets
//! PendingTarget(T+1)       -> persist (unique per account+date)
//! SessionOpenEvent(T+1)    -> orders + fills at the RAW OPEN (+/- slippage)
//! DailyBarClosedEvent(T+1) -> mark to market
//! ```
//!
//! [`plan_session_open`] refuses to execute a target at any session other
//! than its `effective_date`, which is what makes "시가 시점에 당일
//! 고가·저가·종가를 참조하는 오류" impossible by construction rather than by
//! convention.
//!
//! This module is PURE: it owns no clock, no calendar, and no database. The
//! caller resolves the next trading session (Todo 9's calendar) and supplies
//! the session's raw open prices; entitlement gating lives at the API layer.
//! Everything here is a deterministic function of `(state, target, prices)`,
//! which is what makes a crashed session re-plan to byte-identical events.

use std::collections::BTreeMap;

use domain::{FillId, InstrumentId, OrderId, Price, TradingDate};
use uuid::Uuid;

use crate::error::PortfolioError;
use crate::ledger::{LedgerEvent, LedgerState};
use crate::side::Side;
use crate::sizing::{OrderRequest, SizingInput, SizingReport, TargetAllocation, plan_rebalance};

/// Namespace of the deterministic Paper order/fill ids ("LAGRANGEPAPER00").
pub const PAPER_NAMESPACE: Uuid = Uuid::from_u128(0x4c414752_414e4745_50415045_52303000);

/// A target computed at close(T) and effective at the next session's open.
///
/// Persisted uniquely per `(account_id, effective_date)`: recomputing the
/// same close never produces a second target for the same session.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingTarget {
    /// The Paper account this target belongs to. Every derived id includes
    /// it, so two accounts running the same strategy on the same date can
    /// never collide (AT-07).
    pub account_id: Uuid,
    /// The session at whose OPEN this target executes (T+1).
    pub effective_date: TradingDate,
    /// The target weights from the strategy's `TargetPortfolio`.
    pub targets: Vec<TargetAllocation>,
}

impl PendingTarget {
    /// The deterministic target id: `uuid5(ns, account|effective_date)`.
    /// Re-persisting the same close is a no-op, never a duplicate target.
    pub fn id(&self) -> Uuid {
        Uuid::new_v5(
            &PAPER_NAMESPACE,
            format!("target|{}|{}", self.account_id, self.effective_date).as_bytes(),
        )
    }

    /// The deterministic order id for one instrument+side of this target.
    pub fn order_id_for(&self, instrument_id: &InstrumentId, side: Side) -> OrderId {
        OrderId::from_uuid(Uuid::new_v5(
            &PAPER_NAMESPACE,
            format!(
                "order|{}|{}|{}|{}",
                self.account_id,
                self.effective_date,
                instrument_id,
                side_code(side)
            )
            .as_bytes(),
        ))
    }

    /// The deterministic fill id for one instrument+side of this target.
    ///
    /// Paper fills the whole order at the modeled open in one execution, so
    /// there is exactly one fill per order; a future partial-fill model
    /// would extend this key with the fill index.
    pub fn fill_id_for(&self, instrument_id: &InstrumentId, side: Side) -> FillId {
        FillId::from_uuid(Uuid::new_v5(
            &PAPER_NAMESPACE,
            format!(
                "fill|{}|{}|{}|{}",
                self.account_id,
                self.effective_date,
                instrument_id,
                side_code(side)
            )
            .as_bytes(),
        ))
    }
}

fn side_code(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

/// One session's deterministic event stream plus the sizing evidence behind
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionOpenPlan {
    /// `OrderPlaced` + `Fill` per planned order, sells before buys, in
    /// strictly increasing sequence starting at `state.last_seq + 1`.
    pub events: Vec<LedgerEvent>,
    /// The full sizing report (explainability: target vs current value,
    /// skip reasons, estimated fees).
    pub report: SizingReport,
}

/// Plans one Paper session open.
///
/// Fails closed and produces NOTHING when:
/// - `target.effective_date != session_date` ([`PortfolioError::TargetNotEffective`]);
/// - any targeted instrument has no open price ([`PortfolioError::MissingPrice`]).
///
/// Fill prices are the cost profile's execution price over the RAW open
/// (`open x (1 +/- slippage)`), so the ledger never re-applies slippage and
/// a Paper fill is arithmetically identical to a backtest fill at the same
/// open (design §9.3-9.4, the Todo 18 shared-ledger contract).
pub fn plan_session_open(
    state: &LedgerState,
    target: &PendingTarget,
    session_date: &TradingDate,
    open_prices: &BTreeMap<InstrumentId, Price>,
    lot_sizes: &BTreeMap<InstrumentId, u64>,
) -> Result<SessionOpenPlan, PortfolioError> {
    if target.effective_date != *session_date {
        return Err(PortfolioError::TargetNotEffective {
            effective_date: target.effective_date.to_string(),
            session_date: session_date.to_string(),
        });
    }

    let report = plan_rebalance(&SizingInput {
        cash: state.cash,
        positions: state.positions.clone(),
        open_prices: open_prices.clone(),
        targets: target.targets.clone(),
        lot_sizes: lot_sizes.clone(),
        profile: state.cost_profile.clone(),
    })?;

    let mut events = Vec::with_capacity(report.orders.len() * 2);
    let mut seq = state.last_seq;
    for order in &report.orders {
        let execution_price = execution_price_for(state, order, open_prices)?;
        let order_id = target.order_id_for(&order.instrument_id, order.side);

        seq += 1;
        events.push(LedgerEvent::OrderPlaced {
            seq,
            order_id,
            instrument_id: order.instrument_id.clone(),
            side: order.side,
            quantity: order.quantity,
        });

        seq += 1;
        events.push(LedgerEvent::Fill {
            seq,
            fill_id: target.fill_id_for(&order.instrument_id, order.side),
            order_id,
            instrument_id: order.instrument_id.clone(),
            side: order.side,
            quantity: order.quantity,
            price: execution_price.amount(),
        });
    }

    Ok(SessionOpenPlan { events, report })
}

fn execution_price_for(
    state: &LedgerState,
    order: &OrderRequest,
    open_prices: &BTreeMap<InstrumentId, Price>,
) -> Result<Price, PortfolioError> {
    let raw =
        open_prices
            .get(&order.instrument_id)
            .ok_or_else(|| PortfolioError::MissingPrice {
                instrument_id: order.instrument_id.clone(),
            })?;
    state.cost_profile.execution_price(raw, order.side)
}

/// The session's close valuation: marks every held position at its close
/// price and appends the day's equity.
///
/// Fails closed with [`PortfolioError::MissingMark`] when a held position
/// has no close price — a Paper account is never valued on partial data.
pub fn close_valuation_event(
    state: &LedgerState,
    date: TradingDate,
    close_prices: &BTreeMap<InstrumentId, Price>,
) -> Result<LedgerEvent, PortfolioError> {
    for instrument_id in state.positions.keys() {
        if !close_prices.contains_key(instrument_id) {
            return Err(PortfolioError::MissingMark {
                instrument_id: instrument_id.clone(),
            });
        }
    }
    Ok(LedgerEvent::MarkToMarket {
        seq: state.last_seq + 1,
        date,
        prices: close_prices.clone(),
    })
}
