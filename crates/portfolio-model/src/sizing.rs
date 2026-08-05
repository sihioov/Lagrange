//! Target-to-order sizing (design §8.3, §9.3).
//!
//! ```text
//! TargetValue_i   = TotalEquity x TargetWeight_i
//! OrderValue_i    = TargetValue_i - CurrentMarketValue_i
//! ```
//!
//! Rules (all fixed-point, all deterministic):
//!
//! 1. **Integer lots only** — every planned quantity is a whole multiple of
//!    the instrument's lot size (KRX ETF default 1). No fractional
//!    quantities can ever leave the sizer (`Quantity` is scale-0 by type).
//! 2. **Sell-before-buy** — sells are planned first (order list = all sells
//!    in canonical instrument order, then all buys), exactly as the design
//!    mandates ("매도 주문을 먼저 계산").
//! 3. **Available-cash + cost reservation** — after sells, buy budgets are
//!    allocated proportionally to order value (the LAST canonical buy gets
//!    the exact remainder so budgets sum to available cash). Each buy
//!    reserves `min_commission + qty x exec x (1 + commission_rate)`, so
//!    `cash` can never go negative even with fees (FR-PAPER-002).
//! 4. **Minimum trade** — orders below `profile.min_trade` are skipped.
//! 5. **Rebalance threshold** — `|target - current| < threshold` is not
//!    traded (weight space, `Weight` scale 6).
//! 6. **Exit to cash** — an empty target list sells the full positions
//!    (Todo 17 contract: exit arrives as an empty-target portfolio).
//!
//! KRW values are [`Money`] at scale 4; weights are [`Weight`] at scale 6.
//! The only `f64 -> Weight` conversion in the crate is
//! [`weight_from_ratio`] at the selector boundary (selector weights are
//! bps-truncated, so the conversion is exact).

use std::collections::{BTreeMap, BTreeSet};

use domain::{FixedPoint, InstrumentId, Money, Price, Quantity, WEIGHT_SCALE, Weight};
use selector::TargetPortfolio;

use crate::cost::CostProfile;
use crate::error::PortfolioError;
use crate::side::Side;

/// One target instrument with its weight (the sizing input per instrument).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetAllocation {
    /// The canonical instrument id.
    pub instrument_id: InstrumentId,
    /// The target weight in [0, 1] (scale 6).
    pub weight: Weight,
}

/// The full sizing input: current account + execution prices + targets.
///
/// `open_prices` are the raw T+1 session opens (Todo 10 execution-price
/// basis); the profile turns them into execution prices for affordability
/// and fee estimation. Positions are always lot-aligned by construction.
#[derive(Debug, Clone)]
pub struct SizingInput {
    /// Current cash (KRW).
    pub cash: Money,
    /// Current positions (always integer/lot-aligned).
    pub positions: BTreeMap<InstrumentId, Quantity>,
    /// Execution-price basis: raw open per instrument.
    pub open_prices: BTreeMap<InstrumentId, Price>,
    /// Target weights (from `TargetPortfolio`; weight-0 rows are cash).
    pub targets: Vec<TargetAllocation>,
    /// Lot size per instrument (Todo 9 instrument master); absent = 1.
    pub lot_sizes: BTreeMap<InstrumentId, u64>,
    /// The cost profile driving affordability, fees, threshold, min trade.
    pub profile: CostProfile,
}

/// Why an instrument produced no order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// |target - current| weight difference below the rebalance threshold.
    BelowRebalanceThreshold { weight_diff: FixedPoint },
    /// Order value (or buy budget) below the minimum trade size.
    BelowMinTrade { order_value: Money },
    /// No cash available after sells to fund buys.
    NoAvailableCash,
    /// The budget cannot afford one lot after fee reservation.
    NoAffordableLot { budget: Money },
}

/// What the sizer decided for one instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingAction {
    /// Sell this exact quantity (<= current position).
    Sell(Quantity),
    /// Buy this exact quantity (cash + fees reserved).
    Buy(Quantity),
    /// No order, with the reason.
    Skip(SkipReason),
}

/// The per-instrument sizing detail (explainability).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizingDecision {
    /// The instrument.
    pub instrument_id: InstrumentId,
    /// `TotalEquity x TargetWeight_i` (scale-4 KRW).
    pub target_value: Money,
    /// `CurrentMarketValue_i = qty x open` (scale-4 KRW).
    pub current_value: Money,
    /// `TargetValue_i - CurrentMarketValue_i`; negative = sell (scale 4).
    pub order_value: FixedPoint,
    /// The resulting action.
    pub action: SizingAction,
}

/// One planned order (sells first, then buys; each group canonical order).
///
/// Order ids are NOT minted here: the execution layer wraps these into
/// `LedgerEvent::OrderPlaced` with its own (deterministic) ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRequest {
    /// The instrument.
    pub instrument_id: InstrumentId,
    /// Buy or sell.
    pub side: Side,
    /// Integer lots only.
    pub quantity: Quantity,
    /// Expected notional at the execution price.
    pub order_value: Money,
    /// Estimated explicit cash fees (commission + sell tax).
    pub estimated_fees: Money,
}

/// The complete sizing result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizingReport {
    /// `cash + sum(qty x open)` used for targeting.
    pub equity: Money,
    /// Cash available for buys after sells (net of sell fees).
    pub available_cash: Money,
    /// Cash left after every planned buy and its fees (>= 0 by construction).
    pub leftover_cash: Money,
    /// Per-instrument decisions in canonical instrument order.
    pub decisions: Vec<SizingDecision>,
    /// Planned orders: all sells, then all buys (canonical order within each).
    pub orders: Vec<OrderRequest>,
}

/// The only `f64 -> Weight` conversion in the crate (selector boundary).
///
/// Selector weights are truncated to `weight_scale <= 6` decimal places, so
/// `round_ties_even(x * 10^6)` recovers them exactly. Non-finite or
/// out-of-range values are typed errors.
pub fn weight_from_ratio(ratio: f64) -> Result<Weight, PortfolioError> {
    if !ratio.is_finite() {
        return Err(PortfolioError::NonFiniteWeight { value: ratio });
    }
    if !(0.0..=1.0).contains(&ratio) {
        return Err(PortfolioError::WeightOutOfRange {
            value: ratio,
            min: 0.0,
            max: 1.0,
        });
    }
    let bits = (ratio * 10f64.powi(i32::from(WEIGHT_SCALE))).round_ties_even() as i128;
    Weight::from_fixed(FixedPoint::from_i128(bits, WEIGHT_SCALE)?).map_err(Into::into)
}

/// Converts a Todo 16 `TargetPortfolio` into sizing targets.
///
/// Rows with `target_weight <= 0` are cash, not targets; positive weights
/// convert exactly via [`weight_from_ratio`].
pub fn allocation_from_target_portfolio(
    portfolio: &TargetPortfolio,
) -> Result<Vec<TargetAllocation>, PortfolioError> {
    portfolio
        .targets
        .iter()
        .filter(|row| row.target_weight > 0.0)
        .map(|row| {
            Ok(TargetAllocation {
                instrument_id: row.instrument_id.clone(),
                weight: weight_from_ratio(row.target_weight)?,
            })
        })
        .collect()
}

/// Plans the rebalance: sells first (with exact fees), then buys recomputed
/// against the actually available cash (design §8.3). Deterministic for
/// identical inputs; typed errors, never panics.
pub fn plan_rebalance(input: &SizingInput) -> Result<SizingReport, PortfolioError> {
    // --- validation (fail closed before any computation) ---
    for target in &input.targets {
        if !input.open_prices.contains_key(&target.instrument_id) {
            return Err(PortfolioError::MissingPrice {
                instrument_id: target.instrument_id.clone(),
            });
        }
    }
    for id in input.positions.keys() {
        if !input.open_prices.contains_key(id) {
            return Err(PortfolioError::MissingPrice {
                instrument_id: id.clone(),
            });
        }
    }
    for (_id, lot) in &input.lot_sizes {
        if *lot == 0 {
            return Err(PortfolioError::InvalidLotSize { lot_size: *lot });
        }
    }

    let currency = input.cash.currency();

    // --- total equity at the execution-price basis ---
    let mut equity = input.cash;
    for (id, qty) in &input.positions {
        let open = input.open_prices.get(id).expect("validated above");
        equity = equity.checked_add(&qty.checked_mul_price(open, currency)?)?;
    }
    if equity.is_zero() && !input.targets.is_empty() {
        return Err(PortfolioError::ZeroEquity);
    }

    // --- phase 1: per-instrument order values (canonical union order) ---
    let mut targets: BTreeMap<InstrumentId, Weight> = BTreeMap::new();
    for target in &input.targets {
        targets.insert(target.instrument_id.clone(), target.weight);
    }
    let mut union: BTreeSet<InstrumentId> = targets.keys().cloned().collect();
    union.extend(input.positions.keys().cloned());

    let mut pre: Vec<PreDecision> = Vec::new();
    let mut buy_candidates: Vec<(InstrumentId, Money)> = Vec::new();
    let mut sells: Vec<OrderRequest> = Vec::new();

    for id in union {
        let open = input.open_prices.get(&id).expect("validated above");
        let target_weight = targets.get(&id).copied().unwrap_or(Weight::zero()?);
        let current_qty = input
            .positions
            .get(&id)
            .copied()
            .unwrap_or(Quantity::zero()?);
        let current_value = if current_qty.is_zero() {
            Money::zero(currency)
        } else {
            current_qty.checked_mul_price(open, currency)?
        };
        let target_value = if target_weight.is_zero() {
            Money::zero(currency)
        } else {
            equity.checked_mul(&target_weight.amount())?
        };
        let order_value = target_value.amount().checked_sub(&current_value.amount())?;

        // Rebalance threshold (weight space).
        let current_weight = if equity.is_zero() {
            FixedPoint::ZERO
        } else {
            current_value
                .amount()
                .checked_div(&equity.amount(), WEIGHT_SCALE)?
        };
        let weight_diff = target_weight.amount().checked_sub(&current_weight)?.abs();
        if weight_diff < input.profile.rebalance_threshold {
            pre.push(PreDecision {
                instrument_id: id,
                target_value,
                current_value,
                order_value,
                action: PreAction::Skip(SkipReason::BelowRebalanceThreshold { weight_diff }),
            });
            continue;
        }

        // Minimum trade (money space, absolute order value).
        let order_abs = order_value.abs();
        if order_abs < input.profile.min_trade.amount() {
            pre.push(PreDecision {
                instrument_id: id,
                target_value,
                current_value,
                order_value,
                action: PreAction::Skip(SkipReason::BelowMinTrade {
                    order_value: Money::from_fixed(order_abs, currency)?,
                }),
            });
            continue;
        }

        if order_value.is_negative() {
            // SELL: quantity from |order_value| at the raw open, floored to
            // lots, capped at the current position (shorting unsupported).
            let qty_raw = floor_adjust(
                order_abs.checked_div(&open.amount(), 0)?,
                &order_abs,
                &open.amount(),
            )?;
            let lot = input.lot_sizes.get(&id).copied().unwrap_or(1);
            let mut qty = floor_to_lot(qty_raw, lot)?;
            let held = current_qty.to_u64()?;
            qty = qty.min(held);
            if qty == 0 {
                pre.push(PreDecision {
                    instrument_id: id,
                    target_value,
                    current_value,
                    order_value,
                    action: PreAction::Skip(SkipReason::NoAffordableLot {
                        budget: Money::from_fixed(order_abs, currency)?,
                    }),
                });
                continue;
            }
            let qty_q = Quantity::from_fixed(FixedPoint::from_i128(i128::from(qty), 0)?)?;
            let exec = input.profile.execution_price(open, Side::Sell)?;
            let notional = qty_q.checked_mul_price(&exec, currency)?;
            let fees = input.profile.estimate(Side::Sell, &qty_q, &exec)?;
            let fees_cash = fees.cash_fees()?;
            if fees_cash.amount() > notional.amount() {
                return Err(PortfolioError::FeesExceedProceeds {
                    notional,
                    fees: fees_cash,
                });
            }
            sells.push(OrderRequest {
                instrument_id: id.clone(),
                side: Side::Sell,
                quantity: qty_q,
                order_value: notional,
                estimated_fees: fees_cash,
            });
            pre.push(PreDecision {
                instrument_id: id,
                target_value,
                current_value,
                order_value,
                action: PreAction::Sell(qty_q),
            });
        } else {
            buy_candidates.push((id.clone(), Money::from_fixed(order_value, currency)?));
            pre.push(PreDecision {
                instrument_id: id,
                target_value,
                current_value,
                order_value,
                action: PreAction::Buy,
            });
        }
    }

    // --- phase 2a: available cash = cash + net sell proceeds ---
    let mut available = input.cash;
    for sell in &sells {
        let proceeds = sell.order_value.checked_sub(&sell.estimated_fees)?;
        available = available.checked_add(&proceeds)?;
    }

    // --- phase 2b: buy budgets with cost reservation ---
    let mut buy_actions: BTreeMap<InstrumentId, SizingAction> = BTreeMap::new();
    let mut leftover = available;
    if buy_candidates.is_empty() {
        // nothing to do
    } else if available.is_zero() {
        for (id, _) in &buy_candidates {
            buy_actions.insert(id.clone(), SizingAction::Skip(SkipReason::NoAvailableCash));
        }
    } else {
        let mut total_buy = Money::zero(currency);
        for (_, ov) in &buy_candidates {
            total_buy = total_buy.checked_add(ov)?;
        }
        if total_buy.is_zero() {
            for (id, ov) in &buy_candidates {
                buy_actions.insert(
                    id.clone(),
                    SizingAction::Skip(SkipReason::NoAffordableLot { budget: *ov }),
                );
            }
        } else {
            // Budgets: proportional shares, LAST canonical instrument gets the
            // exact remainder (so the sum of budgets == available exactly).
            let mut budgets: Vec<(InstrumentId, Money)> = Vec::new();
            let mut consumed = Money::zero(currency);
            for (i, (id, ov)) in buy_candidates.iter().enumerate() {
                if i + 1 == buy_candidates.len() {
                    budgets.push((id.clone(), available.checked_sub(&consumed)?));
                } else {
                    let share = ov.amount().checked_div(&total_buy.amount(), WEIGHT_SCALE)?;
                    let budget = floor_mul(available.amount(), share)?;
                    consumed = consumed.checked_add(&Money::from_fixed(budget, currency)?)?;
                    budgets.push((id.clone(), Money::from_fixed(budget, currency)?));
                }
            }

            for (id, budget) in &budgets {
                if budget.amount() < input.profile.min_trade.amount() {
                    buy_actions.insert(
                        id.clone(),
                        SizingAction::Skip(SkipReason::BelowMinTrade {
                            order_value: *budget,
                        }),
                    );
                    continue;
                }
                let open = input.open_prices.get(id).expect("validated above");
                let exec = input.profile.execution_price(open, Side::Buy)?;
                let one_plus =
                    FixedPoint::parse("1")?.checked_add(&input.profile.commission_rate)?;
                let denom = exec.amount().checked_mul(&one_plus)?;
                let reserved = budget
                    .amount()
                    .checked_sub(&input.profile.min_commission.amount())?;
                if reserved.is_negative() {
                    buy_actions.insert(
                        id.clone(),
                        SizingAction::Skip(SkipReason::NoAffordableLot { budget: *budget }),
                    );
                    continue;
                }
                let qty_raw = floor_adjust(reserved.checked_div(&denom, 0)?, &reserved, &denom)?;
                let lot = input.lot_sizes.get(id).copied().unwrap_or(1);
                let mut qty = floor_to_lot(qty_raw, lot)?;
                if qty == 0 {
                    buy_actions.insert(
                        id.clone(),
                        SizingAction::Skip(SkipReason::NoAffordableLot { budget: *budget }),
                    );
                    continue;
                }
                // Verify the reservation against the REAL fee; decrement one
                // lot until it holds (bounded; deterministic; never silent).
                loop {
                    let q = Quantity::from_fixed(FixedPoint::from_i128(i128::from(qty), 0)?)?;
                    let notional = q.checked_mul_price(&exec, currency)?;
                    let fees = input.profile.estimate(Side::Buy, &q, &exec)?;
                    let consume = notional.checked_add(&fees.commission)?;
                    if consume.amount() <= budget.amount() {
                        leftover = leftover.checked_sub(&consume)?;
                        buy_actions.insert(id.clone(), SizingAction::Buy(q));
                        break;
                    }
                    if qty < lot {
                        buy_actions.insert(
                            id.clone(),
                            SizingAction::Skip(SkipReason::NoAffordableLot { budget: *budget }),
                        );
                        break;
                    }
                    qty -= lot;
                }
            }
        }
    }

    // --- phase 3: assemble decisions and the order list ---
    // Orders: ALL sells first (canonical instrument order), then ALL buys
    // (canonical) - sell-before-buy is structural, never incidental.
    let mut orders: Vec<OrderRequest> = Vec::new();
    for p in &pre {
        if let PreAction::Sell(_) = p.action {
            let sell = sells
                .iter()
                .find(|o| o.instrument_id == p.instrument_id)
                .expect("sell order exists");
            orders.push(sell.clone());
        }
    }
    for p in &pre {
        if let PreAction::Buy = p.action {
            if let Some(SizingAction::Buy(q)) = buy_actions.get(&p.instrument_id).cloned() {
                let exec = input.profile.execution_price(
                    input
                        .open_prices
                        .get(&p.instrument_id)
                        .expect("validated above"),
                    Side::Buy,
                )?;
                let notional = q.checked_mul_price(&exec, currency)?;
                let fees = input.profile.estimate(Side::Buy, &q, &exec)?;
                orders.push(OrderRequest {
                    instrument_id: p.instrument_id.clone(),
                    side: Side::Buy,
                    quantity: q,
                    order_value: notional,
                    estimated_fees: fees.cash_fees()?,
                });
            }
        }
    }

    let mut decisions: Vec<SizingDecision> = Vec::new();
    for p in pre {
        let action = match p.action {
            PreAction::Skip(reason) => SizingAction::Skip(reason),
            PreAction::Sell(quantity) => SizingAction::Sell(quantity),
            PreAction::Buy => match buy_actions.get(&p.instrument_id).cloned() {
                Some(action) => action,
                None => {
                    return Err(PortfolioError::SizingInternal {
                        detail: format!("missing buy action for {}", p.instrument_id),
                    });
                }
            },
        };
        decisions.push(SizingDecision {
            instrument_id: p.instrument_id,
            target_value: p.target_value,
            current_value: p.current_value,
            order_value: p.order_value,
            action,
        });
    }

    Ok(SizingReport {
        equity,
        available_cash: available,
        leftover_cash: leftover,
        decisions,
        orders,
    })
}

/// One instrument's sizing detail before buy quantities are final.
struct PreDecision {
    instrument_id: InstrumentId,
    target_value: Money,
    current_value: Money,
    order_value: FixedPoint,
    action: PreAction,
}

enum PreAction {
    Skip(SkipReason),
    Sell(Quantity),
    Buy,
}

/// Corrects half-even division overshoot: ensures `q x den <= num`.
fn floor_adjust(
    q: FixedPoint,
    num: &FixedPoint,
    den: &FixedPoint,
) -> Result<FixedPoint, PortfolioError> {
    let one = FixedPoint::parse("1")?;
    let prod = q.checked_mul(den)?;
    if prod > *num {
        Ok(q.checked_sub(&one)?)
    } else {
        Ok(q)
    }
}

/// Floors a scale-0 fixed-point quantity to a whole multiple of `lot`.
fn floor_to_lot(q: FixedPoint, lot: u64) -> Result<u64, PortfolioError> {
    if q.bits() <= 0 {
        return Ok(0);
    }
    let units = u64::try_from(q.bits()).map_err(|_| PortfolioError::SizingInternal {
        detail: format!("negative quantity {q}"),
    })?;
    let lots = units / lot;
    Ok(lots.saturating_mul(lot))
}

/// `floor(a x b)` where `a` is scale 4 and `b` is scale 6: exact integer
/// math, so the sum of proportional budgets can never exceed `available`.
fn floor_mul(a: FixedPoint, b: FixedPoint) -> Result<FixedPoint, PortfolioError> {
    let product = a
        .bits()
        .checked_mul(b.bits())
        .ok_or_else(|| PortfolioError::SizingInternal {
            detail: "budget multiplication overflow".to_owned(),
        })?;
    // product is at scale 10; truncate to scale 4 (both operands are >= 0).
    let bits = product / 1_000_000;
    FixedPoint::from_i128(bits, 4).map_err(Into::into)
}
