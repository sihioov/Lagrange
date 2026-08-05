//! Red-first contract suite for target-to-order sizing (design §8.3, §9.3).
//!
//! Covers: integer lots only, KRW fixed-point values, sell-before-buy
//! ordering, available-cash + cost reservation (buy quantities are recomputed
//! against actually available cash), minimum trade size, the rebalance
//! threshold, exit-to-cash via empty targets, typed input rejection, the
//! `TargetPortfolio -> TargetAllocation` adapter (Todo 16 reuse), and a
//! seeded property run over random accounts.

use std::collections::BTreeMap;

use domain::{Currency, FixedPoint, InstrumentId, Money, Price, Quantity, TradingDate, Weight};
use proptest::prelude::*;
use proptest::test_runner::Config as PropConfig;
use selector::{ConstraintSummary, TargetPortfolio, TargetRow};

use portfolio_model::cost::CostProfile;
use portfolio_model::error::PortfolioError;
use portfolio_model::side::Side;
use portfolio_model::sizing::{
    SizingAction, SizingInput, SkipReason, TargetAllocation, allocation_from_target_portfolio,
    plan_rebalance, weight_from_ratio,
};

const A: &str = "069500.KRX";
const B: &str = "229200.KRX";
const C: &str = "114260.KRX";

fn instrument(s: &str) -> InstrumentId {
    InstrumentId::parse(s).expect("valid instrument id")
}

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn price(amount: &str) -> Price {
    Price::parse(amount).expect("valid price")
}

fn qty(units: u64) -> Quantity {
    Quantity::parse(&units.to_string()).expect("valid quantity")
}

fn weight(amount: &str) -> Weight {
    Weight::parse(amount).expect("valid weight")
}

fn default_profile() -> CostProfile {
    CostProfile::krx_etf_default().expect("default profile")
}

fn from_cash_input(
    cash: &str,
    targets: Vec<TargetAllocation>,
    lot_sizes: &[(InstrumentId, u64)],
) -> SizingInput {
    SizingInput {
        cash: krw(cash),
        positions: BTreeMap::new(),
        open_prices: BTreeMap::from([
            (instrument(A), price("10150.0000")),
            (instrument(B), price("24850.0000")),
            (instrument(C), price("5850.0000")),
        ]),
        targets,
        lot_sizes: lot_sizes.iter().cloned().collect(),
        profile: default_profile(),
    }
}

#[test]
fn integer_lots_only() {
    // Lot size 10: every planned quantity is a multiple of 10 (FR-BT-004).
    let input = from_cash_input(
        "10000000",
        vec![
            TargetAllocation {
                instrument_id: instrument(A),
                weight: weight("0.6"),
            },
            TargetAllocation {
                instrument_id: instrument(B),
                weight: weight("0.4"),
            },
        ],
        &[(instrument(A), 10u64), (instrument(B), 10u64)],
    );
    let report = plan_rebalance(&input).expect("plan");
    assert_eq!(report.orders.len(), 2);
    for (o, expected) in report.orders.iter().zip([590u64, 160]) {
        assert_eq!(
            o.quantity.to_u64().expect("qty"),
            expected,
            "see also the exact-fee QA"
        );
        assert_eq!(
            o.quantity.to_u64().expect("qty") % 10,
            0,
            "lot size 10 enforced"
        );
    }
    // Lot size 1 (KRX ETF default): any positive integer quantity is fine.
    let input1 = from_cash_input(
        "10000000",
        vec![TargetAllocation {
            instrument_id: instrument(A),
            weight: weight("0.6"),
        }],
        &[],
    );
    let report1 = plan_rebalance(&input1).expect("plan");
    let o = &report1.orders[0];
    assert_eq!(o.side, Side::Buy);
    assert!(o.quantity.to_u64().expect("qty") > 0);
}

#[test]
fn sell_before_buy_ordering() {
    // Account holds A/B/C; the new targets shrink A and grow B/C.
    // equity = 10,000,000 + 400x10150 + 100x24850 + 50x5850 = 16,837,500
    let input = SizingInput {
        cash: krw("10000000"),
        positions: BTreeMap::from([
            (instrument(A), qty(400)),
            (instrument(B), qty(100)),
            (instrument(C), qty(50)),
        ]),
        open_prices: BTreeMap::from([
            (instrument(A), price("10150.0000")),
            (instrument(B), price("24850.0000")),
            (instrument(C), price("5850.0000")),
        ]),
        targets: vec![
            TargetAllocation {
                instrument_id: instrument(A),
                weight: weight("0.1"),
            },
            TargetAllocation {
                instrument_id: instrument(B),
                weight: weight("0.3"),
            },
            TargetAllocation {
                instrument_id: instrument(C),
                weight: weight("0.2"),
            },
        ],
        lot_sizes: BTreeMap::new(),
        profile: default_profile(),
    };
    let report = plan_rebalance(&input).expect("plan");
    assert_eq!(report.orders.len(), 3);
    assert_eq!(report.orders[0].side, Side::Sell, "A must be sold first");
    assert_eq!(report.orders[0].instrument_id, instrument(A));
    assert_eq!(
        report.orders[0].quantity,
        qty(234),
        "floor(2,376,250 / 10150)"
    );
    assert_eq!(report.orders[1].side, Side::Buy);
    assert_eq!(report.orders[2].side, Side::Buy);
    let sides: Vec<Side> = report.orders.iter().map(|o| o.side).collect();
    let first_buy = sides.iter().position(|s| *s == Side::Buy).expect("a buy");
    let last_sell = sides
        .iter()
        .rposition(|s| *s == Side::Sell)
        .expect("a sell");
    assert!(last_sell < first_buy, "sell-before-buy ordering violated");
}

#[test]
fn buy_quantities_respect_cash_and_cost_reservation() {
    // From cash: budgets are proportional to target order value; the last
    // canonical instrument receives the exact remainder. Every buy carries
    // its own fee reservation so cash can never go negative (FR-PAPER-002).
    let input = from_cash_input(
        "10000000",
        vec![
            TargetAllocation {
                instrument_id: instrument(A),
                weight: weight("0.6"),
            },
            TargetAllocation {
                instrument_id: instrument(B),
                weight: weight("0.4"),
            },
        ],
        &[],
    );
    let report = plan_rebalance(&input).expect("plan");
    assert_eq!(report.orders.len(), 2);
    assert_eq!(report.orders[0].instrument_id, instrument(A));
    assert_eq!(
        report.orders[0].quantity,
        qty(590),
        "exact-fee reservation result"
    );
    assert_eq!(report.orders[1].instrument_id, instrument(B));
    assert_eq!(
        report.orders[1].quantity,
        qty(160),
        "remainder budget result"
    );

    let mut spent = Money::zero(Currency::KRW);
    let opens = BTreeMap::from([
        (instrument(A), price("10150.0000")),
        (instrument(B), price("24850.0000")),
    ]);
    for o in &report.orders {
        let open = opens.get(&o.instrument_id).expect("open");
        let exec = default_profile()
            .execution_price(open, Side::Buy)
            .expect("exec");
        let notional = o
            .quantity
            .checked_mul_price(&exec, Currency::KRW)
            .expect("notional");
        let fees = default_profile()
            .estimate(Side::Buy, &o.quantity, &exec)
            .expect("estimate");
        assert_eq!(
            o.order_value, notional,
            "order value is notional at the exec price"
        );
        assert_eq!(
            o.estimated_fees, fees.commission,
            "buy fee estimate is the commission"
        );
        let consume = notional.checked_add(&fees.commission).expect("consume");
        assert!(
            consume.amount() <= report.available_cash.amount(),
            "reservation holds"
        );
        spent = spent.checked_add(&consume).expect("sum");
    }
    assert!(report.leftover_cash.amount() >= FixedPoint::ZERO);
    assert_eq!(
        report.leftover_cash,
        report.available_cash.checked_sub(&spent).expect("leftover"),
        "every KRW is accounted"
    );
}

#[test]
fn minimum_trade_skips_small_orders() {
    // Target value 5,000 KRW < min_trade 100,000 KRW -> skipped, not ordered.
    // Threshold is 0 so the min-trade rule (not the threshold) decides.
    let profile =
        CostProfile::custom("0.00015", "1000", "0", 10, "100000", "0").expect("custom profile");
    let input = SizingInput {
        cash: krw("10000000"),
        positions: BTreeMap::new(),
        open_prices: BTreeMap::from([(instrument(A), price("10150.0000"))]),
        targets: vec![TargetAllocation {
            instrument_id: instrument(A),
            weight: weight("0.0005"),
        }],
        lot_sizes: BTreeMap::new(),
        profile,
    };
    let report = plan_rebalance(&input).expect("plan");
    assert!(
        report.orders.is_empty(),
        "no order below the minimum trade size"
    );
    assert_eq!(report.decisions.len(), 1);
    assert!(matches!(
        report.decisions[0].action,
        SizingAction::Skip(SkipReason::BelowMinTrade { .. })
    ));
}

#[test]
fn rebalance_threshold_skips_small_weight_diffs() {
    // current weight 1.0, target 0.996: |diff| = 0.004 < 0.005 threshold.
    let profile =
        CostProfile::custom("0.001", "1000", "0", 0, "0", "0.005").expect("custom profile");
    let input = SizingInput {
        cash: krw("0"),
        positions: BTreeMap::from([(instrument(A), qty(1000))]),
        open_prices: BTreeMap::from([(instrument(A), price("10000.0000"))]),
        targets: vec![TargetAllocation {
            instrument_id: instrument(A),
            weight: weight("0.996"),
        }],
        lot_sizes: BTreeMap::new(),
        profile,
    };
    let report = plan_rebalance(&input).expect("plan");
    assert!(
        report.orders.is_empty(),
        "weight diff below threshold must not trade"
    );
    assert!(matches!(
        report.decisions[0].action,
        SizingAction::Skip(SkipReason::BelowRebalanceThreshold { .. })
    ));
}

#[test]
fn exact_target_match_does_not_trade() {
    let profile =
        CostProfile::custom("0.001", "1000", "0", 0, "0", "0.005").expect("custom profile");
    let input = SizingInput {
        cash: krw("0"),
        positions: BTreeMap::from([(instrument(A), qty(1000))]),
        open_prices: BTreeMap::from([(instrument(A), price("10000.0000"))]),
        targets: vec![TargetAllocation {
            instrument_id: instrument(A),
            weight: weight("1.0"),
        }],
        lot_sizes: BTreeMap::new(),
        profile,
    };
    let report = plan_rebalance(&input).expect("plan");
    assert!(
        report.orders.is_empty(),
        "exact target match is below any threshold"
    );
}

#[test]
fn empty_targets_sell_everything() {
    // Exit-to-cash arrives as an EMPTY target list (Todo 17 contract): the
    // whole position is planned for sale.
    let input = SizingInput {
        cash: krw("5000000"),
        positions: BTreeMap::from([(instrument(A), qty(400))]),
        open_prices: BTreeMap::from([(instrument(A), price("10150.0000"))]),
        targets: vec![],
        lot_sizes: BTreeMap::new(),
        profile: default_profile(),
    };
    let report = plan_rebalance(&input).expect("plan");
    assert_eq!(report.orders.len(), 1);
    assert_eq!(report.orders[0].side, Side::Sell);
    assert_eq!(
        report.orders[0].quantity,
        qty(400),
        "sell the full position"
    );
}

#[test]
fn missing_price_is_rejected() {
    let input = SizingInput {
        cash: krw("10000000"),
        positions: BTreeMap::new(),
        open_prices: BTreeMap::from([(instrument(A), price("10150.0000"))]),
        targets: vec![TargetAllocation {
            instrument_id: instrument(B),
            weight: weight("0.4"),
        }],
        lot_sizes: BTreeMap::new(),
        profile: default_profile(),
    };
    assert!(matches!(
        plan_rebalance(&input),
        Err(PortfolioError::MissingPrice { .. })
    ));
}

#[test]
fn position_without_price_is_rejected() {
    let input = SizingInput {
        cash: krw("10000000"),
        positions: BTreeMap::from([(instrument(B), qty(10))]),
        open_prices: BTreeMap::from([(instrument(A), price("10150.0000"))]),
        targets: vec![],
        lot_sizes: BTreeMap::new(),
        profile: default_profile(),
    };
    assert!(matches!(
        plan_rebalance(&input),
        Err(PortfolioError::MissingPrice { .. })
    ));
}

#[test]
fn zero_equity_with_targets_is_rejected() {
    let input = from_cash_input(
        "0",
        vec![TargetAllocation {
            instrument_id: instrument(A),
            weight: weight("0.6"),
        }],
        &[],
    );
    assert!(matches!(
        plan_rebalance(&input),
        Err(PortfolioError::ZeroEquity)
    ));
}

#[test]
fn krw_values_are_fixed_point_on_the_sizing_path() {
    let input = from_cash_input(
        "10000000",
        vec![
            TargetAllocation {
                instrument_id: instrument(A),
                weight: weight("0.6"),
            },
            TargetAllocation {
                instrument_id: instrument(B),
                weight: weight("0.4"),
            },
        ],
        &[],
    );
    let report = plan_rebalance(&input).expect("plan");
    assert_eq!(
        report.equity,
        krw("10000000.0000"),
        "scale-4 canonical money"
    );
    for o in &report.orders {
        let value = o.order_value.as_decimal_string();
        assert_eq!(
            value.split('.').nth(1).unwrap_or("").len(),
            4,
            "scale-4: {value}"
        );
        let fees = o.estimated_fees.as_decimal_string();
        assert_eq!(
            fees.split('.').nth(1).unwrap_or("").len(),
            4,
            "scale-4: {fees}"
        );
    }
}

#[test]
fn weight_from_ratio_converts_selector_weights_exactly() {
    // Selector weights are bps-truncated (weight_scale <= 6), so the f64 ->
    // Weight conversion at the boundary is exact.
    assert_eq!(weight_from_ratio(0.6).expect("weight"), weight("0.600000"));
    assert_eq!(
        weight_from_ratio(0.1142).expect("weight"),
        weight("0.114200")
    );
    assert_eq!(weight_from_ratio(1.0).expect("weight"), weight("1.000000"));
    assert_eq!(weight_from_ratio(0.0).expect("weight"), weight("0.000000"));
    assert!(matches!(
        weight_from_ratio(f64::NAN),
        Err(PortfolioError::NonFiniteWeight { .. })
    ));
    assert!(matches!(
        weight_from_ratio(1.5),
        Err(PortfolioError::WeightOutOfRange { .. })
    ));
    assert!(matches!(
        weight_from_ratio(-0.25),
        Err(PortfolioError::WeightOutOfRange { .. })
    ));
}

#[test]
fn allocation_from_target_portfolio_filters_zero_weights() {
    // The sizing input conversion (Todo 16 reuse): rows with target_weight 0
    // are cash, not targets; positive weights convert exactly.
    let target = TargetPortfolio {
        as_of: TradingDate::parse("2026-02-02").expect("date"),
        strategy_version: "test@1.0.0".to_owned(),
        universe_snapshot_id: "snap".to_owned(),
        factor_snapshot_hash: "hash".to_owned(),
        dataset_id: "ds".to_owned(),
        dataset_version: 1,
        targets: vec![
            TargetRow {
                instrument_id: instrument(A),
                rank: 1,
                score: 0.9,
                factors: BTreeMap::new(),
                target_weight: 0.6,
                reasons: vec![],
            },
            TargetRow {
                instrument_id: instrument(B),
                rank: 2,
                score: 0.8,
                factors: BTreeMap::new(),
                target_weight: 0.4,
                reasons: vec![],
            },
            TargetRow {
                instrument_id: instrument(C),
                rank: 3,
                score: 0.7,
                factors: BTreeMap::new(),
                target_weight: 0.0,
                reasons: vec![],
            },
        ],
        exclusions: vec![],
        cash_weight: 0.0,
        constraints: ConstraintSummary {
            top_n: 3,
            max_weight: 0.8,
            cash_floor: 0.0,
            weight_scale: 4,
            tolerance: 1e-9,
        },
        portfolio_reasons: vec![],
        portfolio_snapshot_id: "unused".to_owned(),
    };
    let allocs = allocation_from_target_portfolio(&target).expect("adapter");
    assert_eq!(allocs.len(), 2, "zero-weight row is not a target");
    assert_eq!(allocs[0].instrument_id, instrument(A));
    assert_eq!(allocs[0].weight, weight("0.600000"));
    assert_eq!(allocs[1].instrument_id, instrument(B));
    assert_eq!(allocs[1].weight, weight("0.400000"));
}

fn seeded_input(seed: u64) -> (SizingInput, CostProfile) {
    let mut s = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0xBF58_476D_1CE4_E5B9);
    let mut next = move || {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        s >> 33
    };
    let instruments = [instrument(A), instrument(B), instrument(C)];
    let prices = [price("9500.0000"), price("10150.0000"), price("24850.0000")];
    let weights = ["0.10", "0.20", "0.30", "0.40", "0.60", "0.80"];
    let mut positions = BTreeMap::new();
    let mut open_prices = BTreeMap::new();
    let mut lot_sizes = BTreeMap::new();
    for (i, id) in instruments.iter().enumerate() {
        let pos = (next() % 11) as u64 * 50;
        if pos > 0 {
            positions.insert(id.clone(), qty(pos));
        }
        open_prices.insert(id.clone(), prices[i % prices.len()]);
        lot_sizes.insert(id.clone(), if next() % 2 == 0 { 1 } else { 10 });
    }
    let mut targets = Vec::new();
    for id in instruments.iter() {
        if next() % 2 == 0 {
            targets.push(TargetAllocation {
                instrument_id: id.clone(),
                weight: weight(weights[(next() as usize) % weights.len()]),
            });
        }
    }
    if targets.is_empty() {
        targets.push(TargetAllocation {
            instrument_id: instruments[0].clone(),
            weight: weight("0.30"),
        });
    }
    let profile = default_profile();
    let cash = [krw("5000000"), krw("10000000"), krw("25000000")];
    let input = SizingInput {
        cash: cash[(next() as usize) % cash.len()],
        positions,
        open_prices,
        targets,
        lot_sizes,
        profile: profile.clone(),
    };
    (input, profile)
}

proptest! {
    #![proptest_config(PropConfig::with_cases(64))]

    /// Sizing invariants over random accounts: integer lots, sell <= position,
    /// buys bounded by available cash with cost reservation, every KRW
    /// accounted in the leftover, sell-before-buy ordering.
    #[test]
    fn sizing_properties_hold(seed in 0u64..1_000_000u64) {
        let (input, profile) = seeded_input(seed);
        let report = plan_rebalance(&input).expect("plan must never panic");
        let mut spent = Money::zero(Currency::KRW);
        let mut seen_buy = false;
        for o in &report.orders {
            let lot = input.lot_sizes.get(&o.instrument_id).copied().unwrap_or(1);
            prop_assert_eq!(o.quantity.to_u64().expect("qty") % lot, 0, "integer lots");
            if o.side == Side::Sell {
                let held = input
                    .positions
                    .get(&o.instrument_id)
                    .map(|q| q.to_u64().expect("held"))
                    .unwrap_or(0);
                prop_assert!(o.quantity.to_u64().expect("qty") <= held, "sell <= position");
            } else {
                seen_buy = true;
                let open = input.open_prices.get(&o.instrument_id).expect("price");
                let exec = profile.execution_price(open, Side::Buy).expect("exec");
                let notional = o
                    .quantity
                    .checked_mul_price(&exec, Currency::KRW)
                    .expect("notional");
                let fees = profile.estimate(Side::Buy, &o.quantity, &exec).expect("estimate");
                let consume = notional.checked_add(&fees.commission).expect("consume");
                prop_assert!(
                    consume.amount() <= report.available_cash.amount(),
                    "buy consumes <= available cash"
                );
                spent = spent.checked_add(&consume).expect("sum");
            }
        }
        prop_assert!(report.leftover_cash.amount() >= FixedPoint::ZERO);
        prop_assert_eq!(
            report.leftover_cash,
            report.available_cash.checked_sub(&spent).expect("leftover"),
            "every KRW accounted"
        );
        let sides: Vec<Side> = report.orders.iter().map(|o| o.side).collect();
        let first_buy = sides.iter().position(|s| *s == Side::Buy);
        let last_sell = sides.iter().rposition(|s| *s == Side::Sell);
        if let (Some(fb), Some(ls)) = (first_buy, last_sell) {
            prop_assert!(ls < fb, "sell-before-buy ordering");
        }
        prop_assert!(seen_buy || report.orders.is_empty() || sides.iter().all(|s| *s == Side::Sell));
    }

    /// Order list determinism: identical inputs produce identical plans.
    #[test]
    fn plans_are_deterministic(seed in 0u64..1_000_000u64) {
        let (input, _) = seeded_input(seed);
        let a = plan_rebalance(&input).expect("plan");
        let b = plan_rebalance(&input).expect("plan");
        prop_assert_eq!(a.orders.len(), b.orders.len());
        for (x, y) in a.orders.iter().zip(b.orders.iter()) {
            prop_assert_eq!(&x.instrument_id, &y.instrument_id);
            prop_assert_eq!(x.side, y.side);
            prop_assert_eq!(x.quantity, y.quantity);
            prop_assert_eq!(x.order_value, y.order_value);
        }
    }
}
