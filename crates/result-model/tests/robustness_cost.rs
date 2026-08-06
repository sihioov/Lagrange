//! Todo 21 RED tests: cost stress (FR-ROB-003, AT-04).
//!
//! AT-04: "수수료와 슬리피지를 증가 → 최종 자산이 감소하고 비용 합계가 거래
//! 내역과 일치" — increasing fees/slippage must lower the final equity and the
//! cost totals must reconcile with the trade records:
//!   - higher-cost stress ends strictly lower, EXACTLY by the fee delta
//!     (same fills, same prices — the only difference is fees);
//!   - every stressed result passes `BacktestResult::validate` (the cash
//!     ledger reconciles with fills + fees);
//!   - `summary.total_cost == sum of fee entries` (reconciled);
//!   - slippage stress moves execution prices by the documented basis points;
//!   - invalid cost profiles are typed errors, never panics.

mod common;

use domain::Price;

use result_model::backtest::BacktestResult;
use result_model::robustness::{CostStressProfile, RobustnessError, replay_with, stress_cost};

fn raw4(money: &domain::Money) -> i128 {
    common::raw4(money)
}

fn fee_total(result: &BacktestResult) -> i128 {
    result
        .fees
        .iter()
        .map(|f| raw4(&f.commission) + raw4(&f.tax))
        .sum()
}

#[test]
fn robustness_higher_cost_golden_ends_lower_with_reconciled_fees() {
    let base = common::golden_result();
    base.validate().expect("fixture must be valid");

    let stress = CostStressProfile::custom("stress-2x", 1, "0.001", "1000", "0.005", 10)
        .expect("valid profile");
    let stressed = stress_cost(&base, &stress, 10).expect("stress must succeed");
    stressed
        .validate()
        .expect("stressed result must pass integrity (cash ledger == fills + fees)");

    let base_final = raw4(&base.summary.final_equity);
    let stress_final = raw4(&stressed.summary.final_equity);
    assert!(
        stress_final < base_final,
        "AT-04: higher cost must end lower (base {base_final} vs stressed {stress_final})"
    );

    // Fills and execution prices are UNCHANGED; the entire difference is the
    // fee delta — assert the exact equality in scale-4 units.
    let fee_delta = fee_total(&stressed) - fee_total(&base);
    assert_eq!(
        base_final - stress_final,
        fee_delta,
        "AT-04: the final-equity drop must equal the fee increase exactly"
    );

    // Reconciled fees: summary.total_cost == sum of fee entries.
    assert_eq!(
        raw4(&stressed.summary.total_cost),
        fee_total(&stressed),
        "AT-04: cost totals must reconcile with the trade records"
    );
    // Hand-verified ground truth: 3013 + 46237 == 49250 KRW.
    assert_eq!(fee_total(&stressed), 492_500_000_i128);
    assert_eq!(raw4(&stressed.summary.final_equity), 98_757_500_000_i128);
}

#[test]
fn robustness_higher_costs_never_improve_terminal_equity() {
    let base = common::golden_result();
    let profiles = [
        CostStressProfile::custom("stress-2x", 1, "0.001", "1000", "0.005", 10).unwrap(),
        CostStressProfile::custom("stress-4x", 1, "0.002", "2000", "0.010", 10).unwrap(),
        CostStressProfile::custom("stress-8x", 1, "0.004", "4000", "0.020", 10).unwrap(),
    ];
    let mut previous = raw4(&base.summary.final_equity);
    for profile in &profiles {
        let stressed = stress_cost(&base, profile, 10).unwrap();
        stressed.validate().unwrap();
        let final_equity = raw4(&stressed.summary.final_equity);
        assert!(
            final_equity < previous,
            "monotonic: higher cost {final_equity} must be below previous {previous}"
        );
        assert_eq!(raw4(&stressed.summary.total_cost), fee_total(&stressed));
        previous = final_equity;
    }
}

#[test]
fn robustness_slippage_stress_moves_execution_prices() {
    let base = common::golden_result();
    // KRX_ETF_DEFAULT embeds 10 bps; stress to 30 bps → buy +0.20%, sell -0.20%.
    let stress = CostStressProfile::custom("slip-30bps", 1, "0.00015", "1000", "0", 30).unwrap();
    let stressed = stress_cost(&base, &stress, 10).unwrap();
    stressed.validate().unwrap();

    // fill 1: BUY 069500.KRX 200 @ 10000 -> 10020.0000
    assert_eq!(
        stressed.fills[0].price,
        Price::parse("10020.0000").unwrap(),
        "buy fill must be marked up by the slippage delta"
    );
    // fill 4: SELL 069500.KRX 100 @ 10500 -> 10479.0000
    assert_eq!(
        stressed.fills[3].price,
        Price::parse("10479.0000").unwrap(),
        "sell fill must be marked down by the slippage delta"
    );
    // Slippage cost + fees must leave the run lower than the base.
    assert!(raw4(&stressed.summary.final_equity) < raw4(&base.summary.final_equity));
}

#[test]
fn robustness_equal_slippage_keeps_execution_prices_identical() {
    let base = common::golden_result();
    let stress = CostStressProfile::custom("same-slip", 1, "0.00015", "1000", "0", 10).unwrap();
    let stressed = stress_cost(&base, &stress, 10).unwrap();
    for (a, b) in stressed.fills.iter().zip(base.fills.iter()) {
        assert_eq!(a.price, b.price, "equal slippage must not move prices");
    }
}

#[test]
fn robustness_invalid_cost_profiles_are_typed_errors() {
    let error = CostStressProfile::custom("bad-rate", 1, "1.5", "1000", "0", 10)
        .expect_err("commission rate above 1 must be rejected");
    assert!(matches!(error, RobustnessError::InvalidCostProfile { .. }));

    let error = CostStressProfile::custom("bad-slip", 1, "0.00015", "1000", "0", 20_000)
        .expect_err("slippage above 10_000 bps must be rejected");
    assert!(matches!(error, RobustnessError::InvalidCostProfile { .. }));
}

#[test]
fn robustness_replay_preserves_identity_when_nothing_changes() {
    let base = common::golden_result();
    let identity = replay_with(&base, |f| f.clone(), |f| (f.commission, f.tax))
        .expect("identity replay must succeed");
    assert_eq!(identity.equity, base.equity);
    assert_eq!(identity.summary.final_equity, base.summary.final_equity);
    assert_eq!(identity.fees, base.fees);
    identity
        .validate()
        .expect("identity replay must stay valid");
}

#[test]
fn robustness_replay_rejects_unsorted_fills() {
    let base = common::golden_result();
    let error = replay_with(
        &base,
        |f| {
            let mut f = f.clone();
            // swap timestamps of the first two fills -> unsorted input
            if f.fill_id == "fill-1" {
                f.ts = domain::UtcTimestamp::parse_rfc3339("2020-01-09T00:00:00Z").unwrap();
            }
            f
        },
        |f| (f.commission, f.tax),
    )
    .expect_err("unsorted fills must be rejected as a typed replay error");
    assert!(matches!(error, RobustnessError::Replay { .. }));
}
