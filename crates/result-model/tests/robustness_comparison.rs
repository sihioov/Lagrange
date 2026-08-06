//! Todo 21 RED tests: two-run comparison (FR-BT-010).
//!
//! FR-BT-010: "두 실행의 Equity Curve, 거래, 설정을 비교해야 한다. 차이를
//! 기간·종목·비용·체결 기준으로 확인할 수 있다" — the comparison reports
//! period/equity diffs, trade (order/fill) diffs, cost deltas, and
//! configuration diffs. Identical runs compare equal; deterministic inputs
//! yield deterministic comparisons.

mod common;

use domain::Currency;

use result_model::robustness::{CostStressProfile, RunComparison, compare_runs, stress_cost};

fn raw4(money: &domain::Money) -> i128 {
    common::raw4(money)
}

#[test]
fn identical_runs_compare_equal() {
    let left = common::golden_result();
    let right = common::golden_result();
    let comparison = compare_runs(&left, &right);
    assert!(comparison.identical, "identical runs must compare equal");
    assert!(comparison.equity_diffs.is_empty());
    assert!(comparison.summary_diffs.is_empty());
    assert!(comparison.order_diffs.is_empty());
    assert!(comparison.fill_diffs.is_empty());
    assert!(comparison.config_diffs.is_empty());
    assert_eq!(comparison.cost_delta_raw, 0);
}

#[test]
fn cost_stress_differs_on_equity_and_costs_but_not_config() {
    let base = common::golden_result();
    let stress = CostStressProfile::custom("stress-2x", 1, "0.001", "1000", "0.005", 10).unwrap();
    let stressed = stress_cost(&base, &stress, 10).unwrap();

    let comparison = compare_runs(&base, &stressed);
    assert!(!comparison.identical, "stressed run must differ from the base");

    // Cost basis: the fee delta (+46,237 KRW == 462,370,000 scale-4 units).
    assert_eq!(
        comparison.cost_delta_raw,
        462_370_000_i128,
        "cost delta must equal the exact fee increase"
    );
    // Config basis: same provenance -> no config diffs.
    assert!(comparison.config_diffs.is_empty());
    // Period basis: shared dates with non-zero equity deltas.
    assert!(!comparison.equity_diffs.is_empty());
    assert!(comparison
        .equity_diffs
        .iter()
        .all(|d| d.delta_raw != 0));
    // Summary basis: the changed fields are reported by name.
    let summary_fields: Vec<&str> = comparison
        .summary_diffs
        .iter()
        .map(|d| d.field.as_str())
        .collect();
    assert!(summary_fields.contains(&"final_equity"));
    assert!(summary_fields.contains(&"total_cost"));
    assert!(summary_fields.contains(&"total_return"));
}

#[test]
fn provenance_drift_reports_config_diffs() {
    let left = common::golden_result();
    let mut right = common::golden_result();
    right.provenance.strategy_version =
        domain::version::StrategyVersion::parse("2.0.0").unwrap();
    let comparison = compare_runs(&left, &right);
    assert!(comparison.config_diffs.contains(&"strategy_version".to_owned()));
    assert!(!comparison.identical);
}

#[test]
fn mutated_fill_is_reported() {
    let left = common::golden_result();
    let mut right = common::golden_result();
    // One fill executes at a different price.
    let mut fill = right.fills[0].clone();
    fill.price = domain::Price::parse("10050.0000").unwrap();
    right.fills[0] = fill;
    let comparison = compare_runs(&left, &right);
    assert!(!comparison.identical);
    assert!(comparison.changed_fills.contains(&"fill-1".to_owned()));
    // The other fills/orders are untouched.
    assert!(comparison.order_diffs.is_empty());
}

#[test]
fn order_presence_differs() {
    let left = common::golden_result();
    let mut right = common::golden_result();
    right.orders.pop();
    right.fills.pop();
    let comparison = compare_runs(&left, &right);
    assert!(!comparison.identical);
    assert_eq!(comparison.order_diffs.len(), 1);
    assert_eq!(comparison.order_diffs[0].only_in, "left");
    assert_eq!(comparison.fill_diffs.len(), 1);
}

#[test]
fn comparison_is_deterministic() {
    let left = common::golden_result();
    let stress = CostStressProfile::custom("stress-2x", 1, "0.001", "1000", "0.005", 10).unwrap();
    let right = stress_cost(&left, &stress, 10).unwrap();
    let a = compare_runs(&left, &right);
    let b = compare_runs(&left, &right);
    assert_eq!(a, b);
}

#[test]
fn cost_delta_sign_matches_order() {
    let base = common::golden_result();
    let stress = CostStressProfile::custom("stress-2x", 1, "0.001", "1000", "0.005", 10).unwrap();
    let stressed = stress_cost(&base, &stress, 10).unwrap();
    let forward = compare_runs(&base, &stressed);
    let backward = compare_runs(&stressed, &base);
    assert!(forward.cost_delta_raw > 0, "base -> stressed adds cost");
    assert_eq!(forward.cost_delta_raw, -backward.cost_delta_raw);
    assert_eq!(forward.cost_delta_raw, raw4(&stressed.summary.total_cost) - raw4(&base.summary.total_cost));
    let _ = Currency::KRW;
}
