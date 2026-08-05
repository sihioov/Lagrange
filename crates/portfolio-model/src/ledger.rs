//! The canonical ledger shared by backtest, Paper, and Live (design §9.2,
//! §10) — ONE implementation, never mode-specific arithmetic.
//!
//! ## Canonical transitions
//!
//! - `OrderPlaced` records an open order (idempotency surface for fills).
//! - `Fill` debits/credits cash by `+/-qty x price -/+(commission + tax)`,
//!   moves the position, and records the fill. Fill prices are EXECUTION
//!   prices (slippage already embedded by the execution layer via
//!   [`CostProfile::execution_price`]) — the ledger never re-applies
//!   slippage, so the same ledger is exact for NT backtest fills, Paper
//!   next-open fills, and KIS Live fills.
//! - `MarkToMarket` prices every held position (fail-closed), merges the
//!   marks, and appends the daily equity `cash + sum(qty x mark)` to the
//!   equity curve.
//!
//! ## Invariants (property-tested)
//!
//! 1. Cash is NEVER negative: any debit that would cross zero is a typed
//!    [`PortfolioError::InsufficientCash`] and the event is NOT applied.
//! 2. Positions are integer `Quantity`s; sells can never drive a position
//!    negative (no shorting).
//! 3. Fees are always balanced against cash: per fill,
//!    `cash_after == cash_before +/- notional -/+ (commission + tax)`, and
//!    every KRW is accounted in `equity == cash + sum(qty x mark)`.
//! 4. Replay is deterministic and idempotent: the same event stream applied
//!    to the same initial cash yields the same state, byte for byte
//!    ([`LedgerState::canonical_bytes`]).
//! 5. Events carry strictly increasing sequence numbers; out-of-order,
//!    duplicate, unknown, over-filled, mismatched, zero-quantity, and
//!    impossible-precision events are typed rejects — never panics, never
//!    silent.

use std::collections::BTreeMap;

use domain::{
    Currency, DomainError, FillId, FixedPoint, InstrumentId, Money, OrderId, PRICE_SCALE, Price,
    Quantity, TradingDate,
};
use serde::{Deserialize, Serialize};

use crate::cost::{CostBreakdown, CostProfile};
use crate::error::PortfolioError;
use crate::side::Side;

/// One event in the canonical ledger stream (deterministic replay unit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LedgerEvent {
    /// Credits cash (initial funding or a Paper top-up).
    CashDeposit { seq: u64, amount: Money },
    /// Opens an order against which fills reconcile.
    OrderPlaced {
        seq: u64,
        order_id: OrderId,
        instrument_id: InstrumentId,
        side: Side,
        quantity: Quantity,
    },
    /// A (possibly partial) execution at the EXECUTION price.
    Fill {
        seq: u64,
        fill_id: FillId,
        order_id: OrderId,
        instrument_id: InstrumentId,
        side: Side,
        quantity: Quantity,
        /// Raw fixed-point price; the ledger rejects lossy precision.
        price: FixedPoint,
    },
    /// Daily marking: every held position must be priced (fail-closed).
    MarkToMarket {
        seq: u64,
        date: TradingDate,
        prices: BTreeMap<InstrumentId, Price>,
    },
}

/// The recorded fill (append-only; every fill is explained by an event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillRecord {
    /// The event sequence.
    pub seq: u64,
    /// The unique fill id (duplicates are rejected).
    pub fill_id: FillId,
    /// The order this fill belongs to.
    pub order_id: OrderId,
    /// The instrument.
    pub instrument_id: InstrumentId,
    /// Buy or sell.
    pub side: Side,
    /// Integer quantity filled.
    pub quantity: Quantity,
    /// The execution price (slippage embedded).
    pub price: Price,
    /// `quantity x price`.
    pub notional: Money,
    /// Commission charged (cash item).
    pub commission: Money,
    /// Sell tax charged (cash item).
    pub tax: Money,
    /// Cash immediately before the fill.
    pub cash_before: Money,
    /// Cash immediately after the fill.
    pub cash_after: Money,
}

/// An order placed on the ledger with its cumulative fills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenOrder {
    /// The order id.
    pub order_id: OrderId,
    /// The instrument.
    pub instrument_id: InstrumentId,
    /// The side.
    pub side: Side,
    /// The total order quantity.
    pub quantity: Quantity,
    /// The quantity filled so far (partial fills aggregate).
    pub filled_quantity: Quantity,
}

impl OpenOrder {
    /// `quantity - filled_quantity` (typed; zero when fully filled).
    pub fn remaining(&self) -> Result<Quantity, PortfolioError> {
        Ok(self.quantity.checked_sub(&self.filled_quantity)?)
    }
}

/// The result of applying one event (used by execution layers for traces).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEffect {
    /// Cash before the event.
    pub cash_before: Money,
    /// Cash after the event.
    pub cash_after: Money,
    /// The fee breakdown for a fill event.
    pub fees: Option<CostBreakdown>,
    /// The daily equity for a mark event.
    pub equity_after: Option<Money>,
}

/// The full, replayable account state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerState {
    /// The account currency (KRW for this product).
    pub base_currency: Currency,
    /// The versioned cost profile used for every fill (config, not mode).
    pub cost_profile: CostProfile,
    /// Cash, NEVER negative at any point.
    pub cash: Money,
    /// Positions, always integer and never negative.
    pub positions: BTreeMap<InstrumentId, Quantity>,
    /// Orders (placed) with cumulative fills.
    pub orders: BTreeMap<OrderId, OpenOrder>,
    /// Append-only fill records.
    pub fills: Vec<FillRecord>,
    /// The latest mark per instrument (set by mark events).
    pub marks: BTreeMap<InstrumentId, Price>,
    /// `date -> cash + sum(qty x mark)` at each mark event.
    pub equity_curve: BTreeMap<TradingDate, Money>,
    /// The last applied sequence number.
    pub last_seq: u64,
}

impl LedgerState {
    /// A fresh account: initial cash, the profile, no positions/orders.
    pub fn new(initial_cash: Money, cost_profile: CostProfile) -> Self {
        Self {
            base_currency: initial_cash.currency(),
            cost_profile,
            cash: initial_cash,
            positions: BTreeMap::new(),
            orders: BTreeMap::new(),
            fills: Vec::new(),
            marks: BTreeMap::new(),
            equity_curve: BTreeMap::new(),
            last_seq: 0,
        }
    }

    /// Applies one event. On a typed reject the state is UNCHANGED
    /// (validation happens before any mutation).
    pub fn apply(&mut self, event: LedgerEvent) -> Result<LedgerEffect, PortfolioError> {
        match event {
            LedgerEvent::CashDeposit { seq, amount } => {
                self.check_seq(seq)?;
                let before = self.cash;
                self.cash = before.checked_add(&amount)?;
                self.last_seq = seq;
                Ok(LedgerEffect {
                    cash_before: before,
                    cash_after: self.cash,
                    fees: None,
                    equity_after: None,
                })
            }
            LedgerEvent::OrderPlaced {
                seq,
                order_id,
                instrument_id,
                side,
                quantity,
            } => {
                self.check_seq(seq)?;
                if quantity.is_zero() {
                    return Err(PortfolioError::ZeroQuantity {
                        kind: "order",
                        id: order_id.to_string(),
                    });
                }
                if self.orders.contains_key(&order_id) {
                    return Err(PortfolioError::DuplicateOrder { order_id });
                }
                self.orders.insert(
                    order_id,
                    OpenOrder {
                        order_id,
                        instrument_id,
                        side,
                        quantity,
                        filled_quantity: Quantity::zero()?,
                    },
                );
                self.last_seq = seq;
                Ok(LedgerEffect {
                    cash_before: self.cash,
                    cash_after: self.cash,
                    fees: None,
                    equity_after: None,
                })
            }
            LedgerEvent::Fill {
                seq,
                fill_id,
                order_id,
                instrument_id,
                side,
                quantity,
                price,
            } => {
                self.check_seq(seq)?;
                if quantity.is_zero() {
                    return Err(PortfolioError::ZeroQuantity {
                        kind: "fill",
                        id: fill_id.to_string(),
                    });
                }
                let exec = lossless_price(price)?;
                let order = self
                    .orders
                    .get(&order_id)
                    .ok_or(PortfolioError::UnknownOrder { order_id })?;
                if order.instrument_id != instrument_id {
                    return Err(PortfolioError::InstrumentMismatch {
                        order_id,
                        order_instrument: order.instrument_id.clone(),
                        fill_instrument: instrument_id,
                    });
                }
                if order.side != side {
                    return Err(PortfolioError::SideMismatch {
                        order_id,
                        order_side: order.side,
                        fill_side: side,
                    });
                }
                let remaining = order.remaining()?;
                if quantity.amount() > remaining.amount() {
                    return Err(PortfolioError::OverFill {
                        order_id,
                        remaining,
                        fill_quantity: quantity,
                    });
                }
                if self.fills.iter().any(|f| f.fill_id == fill_id) {
                    return Err(PortfolioError::DuplicateFill { fill_id });
                }
                let breakdown = self.cost_profile.estimate(side, &quantity, &exec)?;
                let fees = breakdown.cash_fees()?;
                let notional = quantity.checked_mul_price(&exec, self.base_currency)?;
                let before = self.cash;
                match side {
                    Side::Buy => {
                        let total = notional.checked_add(&fees)?;
                        let after = self.cash.checked_sub(&total).map_err(|e| match e {
                            DomainError::NegativeMoney { .. } => PortfolioError::InsufficientCash {
                                needed: total,
                                available: self.cash,
                            },
                            other => other.into(),
                        })?;
                        let pos = self
                            .positions
                            .get(&instrument_id)
                            .copied()
                            .unwrap_or(Quantity::zero()?);
                        self.positions
                            .insert(instrument_id.clone(), pos.checked_add(&quantity)?);
                        self.cash = after;
                    }
                    Side::Sell => {
                        let pos = self
                            .positions
                            .get(&instrument_id)
                            .copied()
                            .unwrap_or(Quantity::zero()?);
                        if quantity.amount() > pos.amount() {
                            return Err(PortfolioError::SellWithoutPosition {
                                instrument_id,
                                fill_quantity: quantity,
                                position: pos,
                            });
                        }
                        let proceeds = notional.checked_sub(&fees).map_err(|e| match e {
                            DomainError::NegativeMoney { .. } => {
                                PortfolioError::FeesExceedProceeds { notional, fees }
                            }
                            other => other.into(),
                        })?;
                        let new_pos = pos.checked_sub(&quantity)?;
                        if new_pos.is_zero() {
                            self.positions.remove(&instrument_id);
                        } else {
                            self.positions.insert(instrument_id.clone(), new_pos);
                        }
                        self.cash = before.checked_add(&proceeds)?;
                    }
                }
                let order = self
                    .orders
                    .get_mut(&order_id)
                    .expect("order validated above");
                order.filled_quantity = order.filled_quantity.checked_add(&quantity)?;
                self.fills.push(FillRecord {
                    seq,
                    fill_id,
                    order_id,
                    instrument_id,
                    side,
                    quantity,
                    price: exec,
                    notional,
                    commission: breakdown.commission,
                    tax: breakdown.tax,
                    cash_before: before,
                    cash_after: self.cash,
                });
                self.last_seq = seq;
                Ok(LedgerEffect {
                    cash_before: before,
                    cash_after: self.cash,
                    fees: Some(breakdown),
                    equity_after: None,
                })
            }
            LedgerEvent::MarkToMarket { seq, date, prices } => {
                self.check_seq(seq)?;
                for id in self.positions.keys() {
                    if !prices.contains_key(id) {
                        return Err(PortfolioError::MissingMark {
                            instrument_id: id.clone(),
                        });
                    }
                }
                self.marks.extend(prices);
                let equity = self.equity()?;
                self.equity_curve.insert(date, equity);
                self.last_seq = seq;
                Ok(LedgerEffect {
                    cash_before: self.cash,
                    cash_after: self.cash,
                    fees: None,
                    equity_after: Some(equity),
                })
            }
        }
    }

    /// The current positions (integer, never negative; flat = absent).
    pub fn positions(&self) -> &BTreeMap<InstrumentId, Quantity> {
        &self.positions
    }

    /// The current position for an instrument (None = flat).
    pub fn position(&self, instrument_id: &InstrumentId) -> Option<&Quantity> {
        self.positions.get(instrument_id)
    }

    /// The current cash.
    pub fn cash(&self) -> Money {
        self.cash
    }

    /// The append-only fill records.
    pub fn fills(&self) -> &[FillRecord] {
        &self.fills
    }

    /// The latest marks per instrument.
    pub fn marks(&self) -> &BTreeMap<InstrumentId, Price> {
        &self.marks
    }

    /// The daily equity curve (`date -> equity` at each mark event).
    pub fn equity_curve(&self) -> &BTreeMap<TradingDate, Money> {
        &self.equity_curve
    }

    /// `cash + sum(quantity x mark)` over every held position; a held
    /// position without a mark is a typed error (fail-closed).
    pub fn equity(&self) -> Result<Money, PortfolioError> {
        let mut total = self.cash;
        for (id, qty) in &self.positions {
            let mark = self.marks.get(id).ok_or(PortfolioError::MissingMark {
                instrument_id: id.clone(),
            })?;
            total = total.checked_add(&qty.checked_mul_price(mark, self.base_currency)?)?;
        }
        Ok(total)
    }

    /// Deterministic replay: folds the whole stream from a fresh account.
    pub fn replay(
        initial_cash: Money,
        cost_profile: CostProfile,
        events: &[LedgerEvent],
    ) -> Result<Self, PortfolioError> {
        let mut state = Self::new(initial_cash, cost_profile);
        for event in events {
            state.apply(event.clone())?;
        }
        Ok(state)
    }

    /// Idempotent append: replays events onto a snapshot of this state.
    pub fn replay_onto(&self, events: &[LedgerEvent]) -> Result<Self, PortfolioError> {
        let mut state = self.clone();
        for event in events {
            state.apply(event.clone())?;
        }
        Ok(state)
    }

    /// Canonical serialization: deterministic bytes for the same state
    /// (BTreeMaps sort keys; fixed scales; append-only vectors).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PortfolioError> {
        serde_json::to_vec(self).map_err(|e| PortfolioError::Serialization {
            detail: format!("ledger canonical serialization failed: {e}"),
        })
    }

    fn check_seq(&self, seq: u64) -> Result<(), PortfolioError> {
        if seq <= self.last_seq {
            Err(PortfolioError::OutOfOrderEvent {
                seq,
                last_seq: self.last_seq,
            })
        } else {
            Ok(())
        }
    }
}

/// Converts a raw fill price to a `Price`, rejecting precision that cannot
/// be represented losslessly at the canonical scale (KRW is scale 4).
fn lossless_price(price: FixedPoint) -> Result<Price, PortfolioError> {
    let rescaled = price.with_scale(PRICE_SCALE)?;
    if rescaled != price {
        return Err(PortfolioError::PrecisionExceeded {
            value: price,
            max_scale: PRICE_SCALE,
        });
    }
    Ok(Price::from_fixed(rescaled)?)
}
