//! Two-run comparison (FR-BT-010, plan Todo 21).
//!
//! FR-BT-010: "두 실행의 Equity Curve, 거래, 설정을 비교해야 한다. 차이를
//! 기간·종목·비용·체결 기준으로 확인할 수 있다" — [`compare_runs`] reports
//! the period/equity diffs (date-aligned), trade diffs (orders and fills
//! present in one run only, plus fills whose price/quantity changed), the
//! exact cost delta (scale-4 KRW), and the configuration diffs (provenance
//! fields that differ).

use std::collections::BTreeMap;

use domain::Money;

use crate::backtest::BacktestResult;

/// One date-aligned equity difference (scale-4 KRW, signed).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EquityDiff {
    pub date: String,
    pub left_raw: i128,
    pub right_raw: i128,
    /// `right_raw - left_raw` (scale-4 KRW, signed).
    pub delta_raw: i128,
}

/// One differing summary field (name plus stringified values).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SummaryDiff {
    pub field: String,
    pub left: String,
    pub right: String,
}

/// An order or fill present in exactly one of the two runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PresenceDiff {
    pub id: String,
    /// `left` or `right` (which run contains it).
    pub only_in: String,
}

/// A per-(date, instrument) position difference.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PositionDiff {
    pub date: String,
    pub instrument: String,
    pub left_qty: i128,
    pub right_qty: i128,
}

/// The full comparison of two runs (FR-BT-010).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunComparison {
    pub left_run_id: String,
    pub right_run_id: String,
    pub identical: bool,
    /// Date-aligned equity differences (shared dates only).
    pub equity_diffs: Vec<EquityDiff>,
    /// Dates present in one run's equity curve only.
    pub left_only_dates: Vec<String>,
    pub right_only_dates: Vec<String>,
    /// Differing summary fields.
    pub summary_diffs: Vec<SummaryDiff>,
    /// Orders present in one run only.
    pub order_diffs: Vec<PresenceDiff>,
    /// Fills present in one run only.
    pub fill_diffs: Vec<PresenceDiff>,
    /// Fills present in BOTH runs whose price/quantity/side/ts changed.
    pub changed_fills: Vec<String>,
    /// Per-(date, instrument) position differences.
    pub position_diffs: Vec<PositionDiff>,
    /// `right.total_cost - left.total_cost` (scale-4 KRW, signed).
    pub cost_delta_raw: i128,
    /// Provenance fields that differ.
    pub config_diffs: Vec<String>,
    /// Warning codes present in one run only.
    pub warnings_delta: Vec<String>,
}

fn raw4(money: &Money) -> i128 {
    money.amount().bits()
}

/// Compares two runs on every documented basis (FR-BT-010).
pub fn compare_runs(left: &BacktestResult, right: &BacktestResult) -> RunComparison {
    let day = |ts: &domain::UtcTimestamp| ts.to_rfc3339()[..10].to_owned();

    let left_equity: BTreeMap<String, i128> = left
        .equity
        .iter()
        .map(|p| (day(&p.ts), raw4(&p.equity)))
        .collect();
    let right_equity: BTreeMap<String, i128> = right
        .equity
        .iter()
        .map(|p| (day(&p.ts), raw4(&p.equity)))
        .collect();

    let mut equity_diffs = Vec::new();
    let mut left_only_dates = Vec::new();
    let mut right_only_dates = Vec::new();
    let mut dates: Vec<&String> = left_equity.keys().chain(right_equity.keys()).collect();
    dates.sort();
    dates.dedup();
    for date in dates {
        match (left_equity.get(date), right_equity.get(date)) {
            (Some(l), Some(r)) => {
                if l != r {
                    equity_diffs.push(EquityDiff {
                        date: date.clone(),
                        left_raw: *l,
                        right_raw: *r,
                        delta_raw: r - l,
                    });
                }
            }
            (Some(_), None) => left_only_dates.push(date.clone()),
            (None, Some(_)) => right_only_dates.push(date.clone()),
            (None, None) => unreachable!("date came from one of the two maps"),
        }
    }

    let summary_fields: Vec<(&'static str, String, String)> = vec![
        (
            "currency",
            left.summary.currency.to_string(),
            right.summary.currency.to_string(),
        ),
        (
            "initial_equity",
            left.summary.initial_equity.to_string(),
            right.summary.initial_equity.to_string(),
        ),
        (
            "final_equity",
            left.summary.final_equity.to_string(),
            right.summary.final_equity.to_string(),
        ),
        (
            "total_return",
            left.summary.total_return.to_string(),
            right.summary.total_return.to_string(),
        ),
        (
            "cagr",
            left.summary.cagr.to_string(),
            right.summary.cagr.to_string(),
        ),
        (
            "max_drawdown",
            left.summary.max_drawdown.to_string(),
            right.summary.max_drawdown.to_string(),
        ),
        (
            "volatility",
            left.summary.volatility.to_string(),
            right.summary.volatility.to_string(),
        ),
        (
            "sharpe",
            left.summary.sharpe.to_string(),
            right.summary.sharpe.to_string(),
        ),
        (
            "sortino",
            left.summary.sortino.to_string(),
            right.summary.sortino.to_string(),
        ),
        (
            "calmar",
            left.summary.calmar.to_string(),
            right.summary.calmar.to_string(),
        ),
        (
            "turnover",
            left.summary.turnover.to_string(),
            right.summary.turnover.to_string(),
        ),
        (
            "total_cost",
            left.summary.total_cost.to_string(),
            right.summary.total_cost.to_string(),
        ),
        (
            "n_orders",
            left.summary.n_orders.to_string(),
            right.summary.n_orders.to_string(),
        ),
        (
            "n_fills",
            left.summary.n_fills.to_string(),
            right.summary.n_fills.to_string(),
        ),
        (
            "start_date",
            left.summary.start_date.clone(),
            right.summary.start_date.clone(),
        ),
        (
            "end_date",
            left.summary.end_date.clone(),
            right.summary.end_date.clone(),
        ),
    ];
    let summary_diffs: Vec<SummaryDiff> = summary_fields
        .into_iter()
        .filter(|(_, l, r)| l != r)
        .map(|(field, l, r)| SummaryDiff {
            field: field.to_owned(),
            left: l,
            right: r,
        })
        .collect();

    let order_diffs = presence(
        &left
            .orders
            .iter()
            .map(|o| o.order_id.clone())
            .collect::<Vec<_>>(),
        &right
            .orders
            .iter()
            .map(|o| o.order_id.clone())
            .collect::<Vec<_>>(),
    );
    let fill_diffs = presence(
        &left
            .fills
            .iter()
            .map(|f| f.fill_id.clone())
            .collect::<Vec<_>>(),
        &right
            .fills
            .iter()
            .map(|f| f.fill_id.clone())
            .collect::<Vec<_>>(),
    );

    let right_fills: BTreeMap<&str, &crate::backtest::FillRecord> = right
        .fills
        .iter()
        .map(|f| (f.fill_id.as_str(), f))
        .collect();
    let mut changed_fills = Vec::new();
    for fill in &left.fills {
        if let Some(other) = right_fills.get(fill.fill_id.as_str())
            && (fill.price != other.price
                || fill.quantity != other.quantity
                || fill.side != other.side
                || fill.ts != other.ts)
        {
            changed_fills.push(fill.fill_id.clone());
        }
    }

    let mut position_diffs = Vec::new();
    let left_positions: BTreeMap<(String, String), i128> = position_map(&left.positions);
    let right_positions: BTreeMap<(String, String), i128> = position_map(&right.positions);
    let mut keys: Vec<&(String, String)> = left_positions
        .keys()
        .chain(right_positions.keys())
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let l = left_positions.get(key).copied().unwrap_or(0);
        let r = right_positions.get(key).copied().unwrap_or(0);
        if l != r {
            position_diffs.push(PositionDiff {
                date: key.0.clone(),
                instrument: key.1.clone(),
                left_qty: l,
                right_qty: r,
            });
        }
    }

    let config_diffs = provenance_diffs(&left.provenance, &right.provenance);

    let left_warnings: BTreeMap<&str, ()> = left
        .warnings
        .iter()
        .map(|w| (w.code.as_str(), ()))
        .collect();
    let right_warnings: BTreeMap<&str, ()> = right
        .warnings
        .iter()
        .map(|w| (w.code.as_str(), ()))
        .collect();
    let mut warnings_delta = Vec::new();
    for code in left_warnings.keys() {
        if !right_warnings.contains_key(code) {
            warnings_delta.push((*code).to_owned());
        }
    }
    for code in right_warnings.keys() {
        if !left_warnings.contains_key(code) {
            warnings_delta.push((*code).to_owned());
        }
    }
    warnings_delta.sort();

    RunComparison {
        left_run_id: left.provenance.config_hash.to_string(),
        right_run_id: right.provenance.config_hash.to_string(),
        identical: equity_diffs.is_empty()
            && left_only_dates.is_empty()
            && right_only_dates.is_empty()
            && summary_diffs.is_empty()
            && order_diffs.is_empty()
            && fill_diffs.is_empty()
            && changed_fills.is_empty()
            && position_diffs.is_empty()
            && cost_delta(left, right) == 0
            && config_diffs.is_empty()
            && warnings_delta.is_empty(),
        equity_diffs,
        left_only_dates,
        right_only_dates,
        summary_diffs,
        order_diffs,
        fill_diffs,
        changed_fills,
        position_diffs,
        cost_delta_raw: cost_delta(left, right),
        config_diffs,
        warnings_delta,
    }
}

fn cost_delta(left: &BacktestResult, right: &BacktestResult) -> i128 {
    raw4(&right.summary.total_cost) - raw4(&left.summary.total_cost)
}

fn presence(left: &[String], right: &[String]) -> Vec<PresenceDiff> {
    let right_set: BTreeMap<&str, ()> = right.iter().map(|id| (id.as_str(), ())).collect();
    let left_set: BTreeMap<&str, ()> = left.iter().map(|id| (id.as_str(), ())).collect();
    let mut diffs = Vec::new();
    for id in left {
        if !right_set.contains_key(id.as_str()) {
            diffs.push(PresenceDiff {
                id: id.clone(),
                only_in: "left".to_owned(),
            });
        }
    }
    for id in right {
        if !left_set.contains_key(id.as_str()) {
            diffs.push(PresenceDiff {
                id: id.clone(),
                only_in: "right".to_owned(),
            });
        }
    }
    diffs
}

fn position_map(
    positions: &[crate::backtest::PositionSnapshot],
) -> BTreeMap<(String, String), i128> {
    positions
        .iter()
        .map(|p| {
            (
                (p.date.clone(), p.instrument.as_str().to_owned()),
                p.quantity.amount().bits(),
            )
        })
        .collect()
}

fn provenance_diffs(
    left: &domain::provenance::RunProvenance,
    right: &domain::provenance::RunProvenance,
) -> Vec<String> {
    let engine_name = |e: &domain::provenance::Engine| match e {
        domain::provenance::Engine::NautilusTrader => "nautilustrader",
    };
    let fields: Vec<(&'static str, String, String)> = vec![
        (
            "engine",
            engine_name(&left.engine).to_owned(),
            engine_name(&right.engine).to_owned(),
        ),
        (
            "engine_version",
            left.engine_version.to_string(),
            right.engine_version.to_string(),
        ),
        (
            "strategy_id",
            left.strategy_id.to_string(),
            right.strategy_id.to_string(),
        ),
        (
            "strategy_version",
            left.strategy_version.to_string(),
            right.strategy_version.to_string(),
        ),
        (
            "dataset_version",
            left.dataset_version.to_string(),
            right.dataset_version.to_string(),
        ),
        (
            "config_hash",
            left.config_hash.to_string(),
            right.config_hash.to_string(),
        ),
        (
            "code_commit",
            left.code_commit.to_string(),
            right.code_commit.to_string(),
        ),
        (
            "random_seed",
            left.random_seed.to_string(),
            right.random_seed.to_string(),
        ),
        (
            "timezone",
            left.timezone.name().to_owned(),
            right.timezone.name().to_owned(),
        ),
    ];
    fields
        .into_iter()
        .filter(|(_, l, r)| l != r)
        .map(|(name, _, _)| name.to_owned())
        .collect()
}
