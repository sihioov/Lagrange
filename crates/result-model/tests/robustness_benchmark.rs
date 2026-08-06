//! Todo 21 RED tests: benchmark comparison (design §9.5
//! `BenchmarkComparison`).
//!
//! The comparison aligns strategy and benchmark by date (inner join), then
//! reports the total returns, excess return, tracking error, underperforming
//! days, win rate, and the recent-window excess. A missing benchmark series
//! is a typed error; identical inputs yield identical comparisons.

mod common;

use domain::{Currency, Money};

use result_model::backtest::{BacktestResult, BenchmarkPoint};
use result_model::robustness::{BenchmarkComparison, RobustnessError, compare_benchmark};

fn money(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).unwrap()
}

/// Golden result with a benchmark aligned to every equity date.
fn result_with_daily_benchmark(flat: bool) -> BacktestResult {
    let mut result = common::golden_result();
    result.benchmark = result
        .equity
        .iter()
        .enumerate()
        .map(|(i, point)| BenchmarkPoint {
            ts: point.ts,
            value: if flat {
                money("10000000.0000")
            } else {
                money(&format!("{:.4}", 9_900_000.0 * (1.0 + 0.001 * i as f64)))
            },
        })
        .collect();
    result
}

#[test]
fn benchmark_comparison_reports_expected_metrics() {
    let result = result_with_daily_benchmark(false);
    let comparison = compare_benchmark(&result, "069500.KRX")
        .expect("comparison with an aligned benchmark must succeed");

    assert_eq!(comparison.benchmark_id, "069500.KRX");
    assert_eq!(comparison.comparison_start, "2020-01-01");
    assert_eq!(comparison.comparison_end, "2020-01-16");

    // Strategy total return over the joined window: 9,921,987 / 10,000,000 - 1
    let expected_strategy = 9_921_987.0 / 10_000_000.0 - 1.0;
    assert!((comparison.strategy_total_return.value() - expected_strategy).abs() < 1e-9);
    // Benchmark: 9,959,400 / 9,900,000 - 1 (base 9.9M, +0.1% per step, 7 pts)
    let expected_benchmark = 9_959_400.0 / 9_900_000.0 - 1.0;
    assert!((comparison.benchmark_total_return.value() - expected_benchmark).abs() < 1e-9);
    assert!(
        (comparison.excess_return.value()
            - (expected_strategy - expected_benchmark))
            .abs()
            < 1e-9
    );
    assert!(comparison.tracking_error.value() >= 0.0);
    assert!(comparison.win_rate.value() >= 0.0 && comparison.win_rate.value() <= 1.0);
    assert!(comparison.underperforming_days <= comparison.equity_days);
    assert!(comparison.recent_excess.value().is_finite());
}

#[test]
fn benchmark_missing_is_a_typed_error() {
    let mut result = common::golden_result();
    result.benchmark.clear();
    let error = compare_benchmark(&result, "069500.KRX")
        .expect_err("a missing benchmark series must be a typed error");
    assert!(matches!(error, RobustnessError::NoBenchmarkData { .. }));
}

#[test]
fn benchmark_comparison_is_deterministic() {
    let a = result_with_daily_benchmark(true);
    let b = result_with_daily_benchmark(true);
    let ca = compare_benchmark(&a, "069500.KRX").unwrap();
    let cb = compare_benchmark(&b, "069500.KRX").unwrap();
    assert_eq!(ca, cb);
}

#[test]
fn flat_benchmark_exposes_underperformance_and_recent_excess() {
    let result = result_with_daily_benchmark(true);
    let comparison = compare_benchmark(&result, "069500.KRX").unwrap();

    // Flat benchmark -> benchmark total return is zero; strategy lost money.
    assert!(comparison.benchmark_total_return.value().abs() < 1e-9);
    assert!(comparison.excess_return.value() < 0.0);
    assert!(comparison.recent_excess.value() < 0.0);
    // Strategy underperformed at least once -> win rate strictly below 1.
    assert!(comparison.win_rate.value() < 1.0);
    assert!(comparison.underperforming_days > 0);
}

#[test]
fn empty_equity_is_a_typed_error() {
    let mut result = common::golden_result();
    result.equity.clear();
    let error = compare_benchmark(&result, "069500.KRX")
        .expect_err("an empty equity curve must be a typed error");
    assert!(matches!(error, RobustnessError::EmptySeries { .. }));
}
