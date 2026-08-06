//! Benchmark comparison (design §9.5 `BenchmarkComparison`, plan Todo 21).
//!
//! Aligns the strategy's equity curve with the benchmark series by date
//! (inner join) and reports the documented comparison: total returns, excess
//! return, tracking error, underperforming days, win rate, and the
//! recent-window excess return (the input of the recent-degradation warning
//! and the stability score's recent-performance component).

use std::collections::BTreeMap;

use domain::ReportedStat;

use crate::backtest::BacktestResult;
use crate::robustness::RobustnessError;

/// The recent-window length for [`BenchmarkComparison::recent_excess`]
/// (three calendar months of KRX sessions).
pub const RECENT_WINDOW_SESSIONS: usize = 63;

/// The result of comparing a run against a benchmark series.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkComparison {
    pub benchmark_id: String,
    /// Number of joined (strategy, benchmark) days.
    pub equity_days: u64,
    /// Strategy total return over the joined window (decimal fraction).
    pub strategy_total_return: ReportedStat,
    /// Benchmark total return over the joined window (decimal fraction).
    pub benchmark_total_return: ReportedStat,
    /// `strategy_total_return - benchmark_total_return`.
    pub excess_return: ReportedStat,
    /// Annualized standard deviation of daily return differences.
    pub tracking_error: ReportedStat,
    /// Days the strategy return was below the benchmark return.
    pub underperforming_days: u64,
    /// Fraction of joined days where the strategy met or beat the benchmark.
    pub win_rate: ReportedStat,
    /// Strategy minus benchmark return over the recent window.
    pub recent_excess: ReportedStat,
    pub comparison_start: String,
    pub comparison_end: String,
}

/// Compares a run's equity curve against its benchmark series.
pub fn compare_benchmark(
    result: &BacktestResult,
    benchmark_id: &str,
) -> Result<BenchmarkComparison, RobustnessError> {
    if result.equity.is_empty() {
        return Err(RobustnessError::EmptySeries {
            what: "equity".to_owned(),
        });
    }
    if result.benchmark.is_empty() {
        return Err(RobustnessError::NoBenchmarkData {
            benchmark_id: benchmark_id.to_owned(),
        });
    }
    let day = |ts: &domain::UtcTimestamp| ts.to_rfc3339()[..10].to_owned();
    let strategy: BTreeMap<String, f64> = result
        .equity
        .iter()
        .map(|p| (day(&p.ts), p.equity.amount().bits() as f64 / 10_000.0))
        .collect();
    let benchmark: BTreeMap<String, f64> = result
        .benchmark
        .iter()
        .map(|p| (day(&p.ts), p.value.amount().bits() as f64 / 10_000.0))
        .collect();

    let joined: Vec<(&String, f64, f64)> = strategy
        .iter()
        .filter_map(|(date, value)| benchmark.get(date).map(|b| (date, *value, *b)))
        .collect();
    if joined.len() < 2 {
        return Err(RobustnessError::NoBenchmarkData {
            benchmark_id: format!(
                "{benchmark_id}: fewer than two overlapping days ({} joined)",
                joined.len()
            ),
        });
    }

    let first = joined[0];
    let last = *joined.last().expect("non-empty join");
    let strategy_total = last.1 / first.1 - 1.0;
    let benchmark_total = last.2 / first.2 - 1.0;
    let excess = strategy_total - benchmark_total;

    let mut diffs = Vec::new();
    let mut underperforming = 0_u64;
    for pair in joined.windows(2) {
        let strategy_daily = pair[1].1 / pair[0].1 - 1.0;
        let benchmark_daily = pair[1].2 / pair[0].2 - 1.0;
        diffs.push(strategy_daily - benchmark_daily);
        if strategy_daily < benchmark_daily {
            underperforming += 1;
        }
    }
    let n = diffs.len().max(1) as f64;
    let mean = diffs.iter().sum::<f64>() / n;
    let variance = diffs.iter().map(|d| (d - mean) * (d - mean)).sum::<f64>() / n;
    let tracking_error = variance.sqrt() * 252.0f64.sqrt();
    let win_rate = 1.0 - underperforming as f64 / diffs.len().max(1) as f64;

    // recent window: last RECENT_WINDOW_SESSIONS joined points (or all)
    let recent_start = joined.len().saturating_sub(RECENT_WINDOW_SESSIONS);
    let recent = &joined[recent_start..];
    let recent_strategy = recent.last().expect("non-empty recent").1 / recent[0].1 - 1.0;
    let recent_benchmark = recent.last().expect("non-empty recent").2 / recent[0].2 - 1.0;
    let recent_excess = recent_strategy - recent_benchmark;

    let stat = |v: f64| {
        ReportedStat::from_f64(v).map_err(|e| RobustnessError::NonFinite {
            field: format!("benchmark metric: {e}"),
        })
    };

    Ok(BenchmarkComparison {
        benchmark_id: benchmark_id.to_owned(),
        equity_days: joined.len() as u64,
        strategy_total_return: stat(strategy_total)?,
        benchmark_total_return: stat(benchmark_total)?,
        excess_return: stat(excess)?,
        tracking_error: stat(tracking_error)?,
        underperforming_days: underperforming,
        win_rate: stat(win_rate)?,
        recent_excess: stat(recent_excess)?,
        comparison_start: first.0.clone(),
        comparison_end: last.0.clone(),
    })
}
