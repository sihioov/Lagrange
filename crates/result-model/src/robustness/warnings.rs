//! Concentration, recent-degradation, and parameter-neighborhood analysis
//! (FR-ROB-002/005, plan Todo 21).
//!
//! FR-ROB-005: "수익 집중도와 최근 성과 약화를 경고해야 한다. 상위 거래
//! 기여도, 특정 연도 기여도, 최근 구간 벤치마크 하회가 표시된다" — three
//! warnings:
//!   - `return_concentration` when the top-3 realized (gross, FIFO) trades
//!     carry more than 50% of the total |realized PnL|;
//!   - `year_concentration` when one calendar year contributes more than 40%
//!     of the total |yearly contribution|;
//!   - `recent_degradation` when the recent-window strategy return is below
//!     the benchmark's over the same window.
//!
//! FR-ROB-002: "최적값 주변 결과 분포와 성과 급변 경고" — [`analyze_neighborhood`]
//! reports the neighborhood return distribution and warns
//! (`performance_sudden_change`) when an adjacent parameter delta jumps more
//! than [`SUDDEN_CHANGE_THRESHOLD`].

use std::collections::BTreeMap;

use domain::ReportedStat;
use serde_json::json;

use crate::backtest::{BacktestResult, OrderSide};
use crate::robustness::RobustnessError;
use crate::robustness::benchmark::{RECENT_WINDOW_SESSIONS, compare_benchmark};
use crate::Warning;

/// Top-trade concentration threshold (top-3 |PnL| share above this warns).
pub const TOP_TRADE_CONCENTRATION_THRESHOLD: f64 = 0.5;
/// Single-year contribution threshold.
pub const YEAR_CONCENTRATION_THRESHOLD: f64 = 0.4;
/// Adjacent-delta return jump that triggers the sudden-change warning.
pub const SUDDEN_CHANGE_THRESHOLD: f64 = 0.10;
/// How many top trades the concentration metric examines.
pub const TOP_TRADES: usize = 3;

fn warning(
    code: &str,
    message: impl Into<String>,
    details: serde_json::Value,
) -> Warning {
    Warning::new(code, message, crate::WarningSeverity::Warning).with_details(details)
}

/// Realized (gross, FIFO) PnL of every round trip, instrument by instrument.
///
/// Buys accumulate a cost basis; sells realize `(price - avg_cost) x qty`.
/// Fees are excluded (the gross trade PnL; documented in the warning details).
fn realized_pnls(result: &BacktestResult) -> Vec<(String, f64)> {
    let mut basis: BTreeMap<String, (f64, f64)> = BTreeMap::new(); // instrument -> (qty, avg_cost)
    let mut realized: Vec<(String, f64)> = Vec::new();
    let f64_ = |amount: i128| amount as f64 / 10_000.0;
    for fill in &result.fills {
        let key = fill.instrument.as_str();
        let qty = fill.quantity.amount().bits() as f64;
        let price = f64_(fill.price.amount().bits());
        let (held, avg_cost) = basis.entry(key.to_owned()).or_insert((0.0, 0.0));
        match fill.side {
            OrderSide::Buy => {
                let total = *held * *avg_cost + qty * price;
                *held += qty;
                *avg_cost = if *held > 0.0 { total / *held } else { 0.0 };
            }
            OrderSide::Sell => {
                let sell = qty.min(*held);
                if sell > 0.0 {
                    realized.push((key.to_owned(), (price - *avg_cost) * sell));
                    *held -= sell;
                }
            }
        }
    }
    realized
}

/// `return_concentration` warning when the top-3 trades dominate the gross
/// realized PnL (FR-ROB-005).
pub fn top_trade_concentration_warning(result: &BacktestResult) -> Option<Warning> {
    let mut pnls = realized_pnls(result);
    if pnls.is_empty() {
        return None;
    }
    pnls.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    let total: f64 = pnls.iter().map(|(_, p)| p.abs()).sum();
    if total <= 0.0 {
        return None;
    }
    let top: f64 = pnls.iter().take(TOP_TRADES).map(|(_, p)| p.abs()).sum();
    let share = top / total;
    if share > TOP_TRADE_CONCENTRATION_THRESHOLD {
        let trades: Vec<serde_json::Value> = pnls
            .iter()
            .take(TOP_TRADES)
            .map(|(instrument, pnl)| json!({"instrument": instrument, "realized_pnl": pnl}))
            .collect();
        return Some(warning(
            "return_concentration",
            format!(
                "top-{TOP_TRADES} trades carry {:.1}% of the gross realized PnL",
                share * 100.0
            ),
            json!({
                "top_3_share": share,
                "top_trades": trades,
                "total_gross_pnl": total,
            }),
        ));
    }
    None
}

/// `year_concentration` warning when one year dominates the total |yearly
/// contribution| (FR-ROB-005). Yearly contribution compounds the calendar
/// year's monthly returns: `year_return = Π(1 + r_m) - 1`.
pub fn year_concentration_warning(result: &BacktestResult) -> Option<Warning> {
    let mut by_year: BTreeMap<String, f64> = BTreeMap::new();
    for m in &result.monthly_returns {
        let year = m.month[..4].to_owned();
        let product = by_year.entry(year).or_insert(1.0);
        *product *= 1.0 + m.return_.value();
    }
    let contributions: Vec<(String, f64)> = by_year
        .into_iter()
        .map(|(year, product)| (year, product - 1.0))
        .collect();
    let total: f64 = contributions.iter().map(|(_, c)| c.abs()).sum();
    if total <= 0.0 {
        return None;
    }
    let (worst_year, worst) = contributions
        .iter()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .expect("non-empty year map");
    let share = worst.abs() / total;
    if share > YEAR_CONCENTRATION_THRESHOLD {
        let years: Vec<serde_json::Value> = contributions
            .iter()
            .map(|(year, contrib)| json!({"year": year, "contribution": contrib}))
            .collect();
        return Some(warning(
            "year_concentration",
            format!("year {worst_year} carries {:.1}% of the total yearly contribution", share * 100.0),
            json!({
                "max_year_share": share,
                "worst_year": worst_year,
                "years": years,
            }),
        ));
    }
    None
}

/// `recent_degradation` warning when the recent window underperforms the
/// benchmark (FR-ROB-005).
pub fn recent_degradation_warning(
    result: &BacktestResult,
    benchmark_id: &str,
) -> Option<Warning> {
    let comparison = compare_benchmark(result, benchmark_id).ok()?;
    if comparison.recent_excess.value() < 0.0 {
        return Some(warning(
            "recent_degradation",
            format!(
                "recent {RECENT_WINDOW_SESSIONS}-session window underperforms benchmark {benchmark_id}"
            ),
            json!({
                "benchmark_id": benchmark_id,
                "recent_window_sessions": RECENT_WINDOW_SESSIONS,
                "recent_excess": comparison.recent_excess.value(),
                "recent_strategy_return": comparison.strategy_total_return.value(),
            }),
        ));
    }
    None
}

/// One parameter-neighborhood run: the delta and its total return.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NeighborhoodPoint {
    pub delta: serde_json::Value,
    pub total_return: ReportedStat,
}

/// The result distribution around the optimal parameter value (FR-ROB-002).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NeighborhoodAnalysis {
    pub points: Vec<NeighborhoodPoint>,
    pub mean_return: ReportedStat,
    /// Standard deviation of the neighborhood returns.
    pub dispersion: ReportedStat,
    /// Largest |return - mean| in the neighborhood.
    pub max_deviation_from_mean: ReportedStat,
    /// `performance_sudden_change` warning when an adjacent delta jumps more
    /// than [`SUDDEN_CHANGE_THRESHOLD`].
    pub sudden_change: Option<Warning>,
}

/// Analyzes a parameter neighborhood. Deltas must arrive in ascending order
/// (the caller's declared neighborhood); returns are decimal fractions.
pub fn analyze_neighborhood(
    points: Vec<(serde_json::Value, ReportedStat)>,
) -> Result<NeighborhoodAnalysis, RobustnessError> {
    if points.is_empty() {
        return Err(RobustnessError::EmptySeries {
            what: "parameter neighborhood".to_owned(),
        });
    }
    let returns: Vec<f64> = points.iter().map(|(_, r)| r.value()).collect();
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|r| (r - mean) * (r - mean))
        .sum::<f64>()
        / returns.len() as f64;
    let dispersion = variance.sqrt();
    let max_deviation = returns
        .iter()
        .map(|r| (r - mean).abs())
        .fold(0.0f64, f64::max);

    let mut sudden_change = None;
    for pair in points.windows(2) {
        let jump = (pair[1].1.value() - pair[0].1.value()).abs();
        if jump > SUDDEN_CHANGE_THRESHOLD {
            sudden_change = Some(warning(
                "performance_sudden_change",
                format!(
                    "performance jumps {jump:.4} between adjacent parameter deltas"
                ),
                json!({
                    "from_delta": pair[0].0,
                    "to_delta": pair[1].0,
                    "from_return": pair[0].1.value(),
                    "to_return": pair[1].1.value(),
                    "jump": jump,
                    "threshold": SUDDEN_CHANGE_THRESHOLD,
                }),
            ));
            break;
        }
    }

    let stat = |v: f64| {
        ReportedStat::from_f64(v).map_err(|e| RobustnessError::NonFinite {
            field: format!("neighborhood metric: {e}"),
        })
    };
    Ok(NeighborhoodAnalysis {
        points: points
            .into_iter()
            .map(|(delta, total_return)| NeighborhoodPoint { delta, total_return })
            .collect(),
        mean_return: stat(mean)?,
        dispersion: stat(dispersion)?,
        max_deviation_from_mean: stat(max_deviation)?,
        sudden_change,
    })
}
