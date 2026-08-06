//! Period segments and walk-forward folds (FR-ROB-001/004, plan Todo 21).
//!
//! [`split_period`] partitions a result's equity curve into disjoint,
//! exhaustive train/validation/test segments under a [`PeriodSplit`] (the
//! same boundaries the selection barrier enforces). [`walk_forward`] produces
//! the documented (train window → next validation window) folds of a
//! [`WalkForwardPlan`] (FR-ROB-004: "각 학습 창과 다음 검증 창의 결과가 구분
//! 되어 표시된다").

use domain::ReportedStat;

use crate::backtest::{BacktestResult, EquityPoint};
use crate::robustness::RobustnessError;
use crate::robustness::holdout::PeriodSplit;

/// Reported metrics of one segment (decimal fractions).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SegmentMetrics {
    pub start_date: String,
    pub end_date: String,
    pub n_points: usize,
    pub total_return: ReportedStat,
    pub max_drawdown: ReportedStat,
    pub volatility: ReportedStat,
}

/// An equity segment (train, validation, or test).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub points: Vec<EquityPoint>,
    pub metrics: SegmentMetrics,
}

/// The three segments of a period split.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeriodSegments {
    pub train: Segment,
    pub validation: Segment,
    pub test: Segment,
}

/// The walk-forward plan (sessions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WalkForwardPlan {
    pub window_sessions: u32,
    pub step_sessions: u32,
}

/// One walk-forward fold: a train window and the immediately following
/// validation window.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WalkForwardFold {
    pub index: usize,
    pub train: Segment,
    pub validation: Segment,
}

fn day(point: &EquityPoint) -> String {
    point.ts.to_rfc3339()[..10].to_owned()
}

fn segment(points: Vec<EquityPoint>) -> Segment {
    let start_date = points.first().map(day).unwrap_or_default();
    let end_date = points.last().map(day).unwrap_or_default();
    let n_points = points.len();
    let (total_return, max_drawdown, volatility) = metrics(&points);
    Segment {
        points,
        metrics: SegmentMetrics {
            start_date,
            end_date,
            n_points,
            total_return,
            max_drawdown,
            volatility,
        },
    }
}

/// Computes the segment metrics from its equity points.
fn metrics(points: &[EquityPoint]) -> (ReportedStat, ReportedStat, ReportedStat) {
    let value = |p: &EquityPoint| p.equity.amount().bits() as f64 / 10_000.0;
    let total_return = match (points.first(), points.last()) {
        (Some(first), Some(last)) => value(last) / value(first) - 1.0,
        _ => 0.0,
    };
    let mut peak = f64::NEG_INFINITY;
    let mut max_drawdown = 0.0f64;
    for point in points {
        let v = value(point);
        peak = peak.max(v);
        max_drawdown = max_drawdown.min(v / peak - 1.0);
    }
    let mut daily = Vec::new();
    for pair in points.windows(2) {
        daily.push(value(&pair[1]) / value(&pair[0]) - 1.0);
    }
    let n = daily.len().max(1) as f64;
    let mean = daily.iter().sum::<f64>() / n;
    let variance = daily.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / n;
    let volatility = variance.sqrt() * 252.0f64.sqrt();
    let stat = |v: f64| {
        ReportedStat::from_f64(v).unwrap_or_else(|e| {
            panic!("segment metric must stay finite: {e}")
        })
    };
    (stat(total_return), stat(max_drawdown), stat(volatility))
}

/// Partitions the equity curve into train/validation/test segments.
pub fn split_period(
    result: &BacktestResult,
    split: &PeriodSplit,
) -> Result<PeriodSegments, RobustnessError> {
    split.validate()?;
    let mut train = Vec::new();
    let mut validation = Vec::new();
    let mut test = Vec::new();
    for point in &result.equity {
        let date = day(point);
        if date.as_str() <= split.train_end.as_str() {
            train.push(point.clone());
        } else if date.as_str() <= split.validation_end.as_str() {
            validation.push(point.clone());
        } else {
            test.push(point.clone());
        }
    }
    Ok(PeriodSegments {
        train: segment(train),
        validation: segment(validation),
        test: segment(test),
    })
}

/// Produces the complete (train, validation) folds of a walk-forward plan.
///
/// Fold `i` trains on points `[i*step, i*step+window)` and validates on the
/// following `step` points. Plans that do not fit the data are typed
/// [`RobustnessError::InsufficientData`] errors (never truncated silently).
pub fn walk_forward(
    result: &BacktestResult,
    plan: &WalkForwardPlan,
) -> Result<Vec<WalkForwardFold>, RobustnessError> {
    let n = result.equity.len();
    if plan.window_sessions == 0 || plan.step_sessions == 0 {
        return Err(RobustnessError::InsufficientData {
            what: "walk-forward window/step",
            need: 1,
            have: 0,
        });
    }
    if plan.window_sessions as usize > n {
        return Err(RobustnessError::InsufficientData {
            what: "walk-forward window",
            need: plan.window_sessions as usize,
            have: n,
        });
    }
    let mut folds = Vec::new();
    let mut index = 0_usize;
    loop {
        let start = index * plan.step_sessions as usize;
        let train_end = start + plan.window_sessions as usize;
        let validation_end = train_end + plan.step_sessions as usize;
        if validation_end > n {
            break;
        }
        folds.push(WalkForwardFold {
            index,
            train: segment(result.equity[start..train_end].to_vec()),
            validation: segment(result.equity[train_end..validation_end].to_vec()),
        });
        index += 1;
    }
    Ok(folds)
}
