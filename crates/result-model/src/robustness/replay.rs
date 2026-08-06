//! Deterministic fill→ledger→equity replay (plan Todo 21).
//!
//! Every derived-run simulation (cost stress, execution delay) re-derives a
//! full, integrity-valid [`BacktestResult`] from a set of fills: the cash
//! ledger is rebuilt from fill notionals and fees, positions from cumulative
//! signed quantities, daily equity from cash plus position marks (the last
//! fill price per instrument, point-in-time), and drawdown/monthly
//! returns/summary from the resulting curve. The rebuilt result MUST pass
//! [`BacktestResult::validate`] before it is returned — a derived run that
//! cannot reconcile is a typed [`RobustnessError::Replay`], never a partial
//! result (same publication gate discipline as Todo 20).
//!
//! Conventions: money is scale-4 KRW [`domain::Money`]; cash is close-of-day;
//! the equity curve starts with an initial point the day before the first
//! fill (open-of-day convention on fill dates); positions never go negative
//! and cash never goes negative (Todo 18 ledger invariants).

use std::collections::BTreeMap;

use domain::{Currency, FixedPoint, InstrumentId, Money, UtcTimestamp};

use crate::backtest::{
    BacktestResult, BacktestSummary, BenchmarkPoint, CashLedgerEntry, DrawdownPoint, EquityPoint,
    FeeEntry, FillRecord, MonthlyReturn, OrderRecord, OrderSide, PositionSnapshot,
};
use crate::robustness::RobustnessError;
use crate::Warning;

/// Inputs of a deterministic replay.
#[derive(Debug, Clone)]
pub struct ReplaySpec<'a> {
    pub initial_equity: &'a Money,
    pub currency: Currency,
    /// The fills to replay, chronological.
    pub fills: Vec<FillRecord>,
    /// One fee entry per fill (parallel arrays).
    pub fees: Vec<FeeEntry>,
    pub orders: &'a [OrderRecord],
    pub warnings: &'a [Warning],
    pub provenance: &'a domain::provenance::RunProvenance,
    pub benchmark: &'a [BenchmarkPoint],
}

/// Replays fills into a full, validated [`BacktestResult`].
pub fn replay(spec: ReplaySpec) -> Result<BacktestResult, RobustnessError> {
    if spec.fills.len() != spec.fees.len() {
        return Err(RobustnessError::Replay {
            detail: format!(
                "fill/fee count mismatch: {} fills vs {} fees",
                spec.fills.len(),
                spec.fees.len()
            ),
        });
    }
    for pair in spec.fills.windows(2) {
        if pair[0].ts > pair[1].ts {
            return Err(RobustnessError::Replay {
                detail: "fills are not in chronological order".to_owned(),
            });
        }
    }

    let initial_raw = raw4(spec.initial_equity);
    let mut cash = initial_raw;
    // instrument -> (quantity, last fill price), both scale-consistent
    let mut positions: BTreeMap<InstrumentId, i128> = BTreeMap::new();
    let mut marks: BTreeMap<InstrumentId, i128> = BTreeMap::new();

    let day0 = spec
        .fills
        .first()
        .map(|f| previous_calendar_day(&f.ts.to_rfc3339()[..10]))
        .unwrap_or_else(|| "1970-01-01".to_owned());

    let mut equity: Vec<(String, i128)> = vec![(day0.clone(), initial_raw)];
    let mut cash_entries: Vec<(String, i128)> = vec![(day0.clone(), initial_raw)];
    // (date, instrument) -> cumulative quantity at that date (point-in-time)
    let mut day_positions: BTreeMap<(String, String), i128> = BTreeMap::new();

    for (fill, fee) in spec.fills.iter().zip(spec.fees.iter()) {
        let date = fill.ts.to_rfc3339()[..10].to_owned();
        let notional = raw4(&fill.quantity.checked_mul_price(&fill.price, spec.currency).map_err(
            |e| RobustnessError::Replay {
                detail: format!("notional for {}: {e}", fill.fill_id),
            },
        )?);
        let fee_total = raw4(&fee.commission) + raw4(&fee.tax);
        let delta = match fill.side {
            OrderSide::Buy => -notional - fee_total,
            OrderSide::Sell => notional - fee_total,
        };
        cash += delta;
        if cash < 0 {
            return Err(RobustnessError::Replay {
                detail: format!("cash went negative at {}", fill.fill_id),
            });
        }
        let qty = fill.quantity.amount().bits();
        let position = positions.entry(fill.instrument.clone()).or_insert(0);
        *position += match fill.side {
            OrderSide::Buy => qty,
            OrderSide::Sell => -qty,
        };
        if *position < 0 {
            return Err(RobustnessError::Replay {
                detail: format!("position went negative at {}", fill.fill_id),
            });
        }
        marks.insert(fill.instrument.clone(), fill.price.amount().bits());
        day_positions.insert((date.clone(), fill.instrument.as_str()), *position);

        let mut marked = cash;
        for (instrument, position_qty) in &positions {
            let mark = marks.get(instrument).copied().unwrap_or(0);
            marked += position_qty * mark;
        }
        equity.push((date.clone(), marked));
        // Cash entries are close-of-day: one entry per date holding the
        // day's FINAL cash (the ledger check reconciles each entry against
        // every fill/fee up to and including that date).
        match cash_entries.last_mut() {
            Some((last_date, last_cash)) if *last_date == date => *last_cash = cash,
            _ => cash_entries.push((date.clone(), cash)),
        }
    }

    // position snapshots: one row per (date, held instrument), date-ordered
    let mut position_rows: Vec<PositionSnapshot> = Vec::new();
    for ((date, symbol), qty) in day_positions.iter() {
        if *qty == 0 {
            continue;
        }
        position_rows.push(PositionSnapshot {
            date: date.clone(),
            instrument: domain::InstrumentId::parse(symbol).map_err(|e| {
                RobustnessError::Replay {
                    detail: format!("instrument {symbol}: {e}"),
                }
            })?,
            quantity: domain::Quantity::parse(&qty.to_string()).map_err(|e| {
                RobustnessError::Replay {
                    detail: format!("position quantity: {e}"),
                }
            })?,
        });
    }

    let equity_points: Vec<EquityPoint> = equity
        .iter()
        .map(|(date, raw)| {
            Ok(EquityPoint {
                ts: parse_ts(date)?,
                equity: money_from_raw(*raw, spec.currency)?,
            })
        })
        .collect::<Result<Vec<_>, RobustnessError>>()?;
    let cash_ledger: Vec<CashLedgerEntry> = cash_entries
        .iter()
        .map(|(date, raw)| {
            Ok(CashLedgerEntry {
                ts: parse_ts(date)?,
                cash: money_from_raw(*raw, spec.currency)?,
            })
        })
        .collect::<Result<Vec<_>, RobustnessError>>()?;

    let drawdown = drawdown_curve(&equity_points)?;
    let monthly = monthly_returns(&equity_points)?;
    let summary = build_summary(
        &equity_points,
        &spec.fills,
        &spec.fees,
        spec.currency,
        spec.orders.len(),
    )?;

    let result = BacktestResult {
        summary,
        equity: equity_points,
        drawdown,
        monthly_returns: monthly,
        orders: spec.orders.to_vec(),
        fills: spec.fills,
        positions: position_rows,
        cash: cash_ledger,
        fees: spec.fees,
        benchmark: spec.benchmark.to_vec(),
        metrics: BTreeMap::new(),
        warnings: spec.warnings.to_vec(),
        provenance: spec.provenance.clone(),
    };
    // The rebuilt result must reconcile before it may leave this function.
    result.validate().map_err(|e| RobustnessError::Replay {
        detail: format!("rebuilt result failed integrity: {e}"),
    })?;
    Ok(result)
}

/// Replays an original result under a fill transform and fee function.
///
/// `transform` adjusts each fill (price, timestamps); `fee_fn` computes the
/// (commission, tax) of each TRANSFORMED fill. Used by cost stress and
/// execution delay so every derived run shares one ledger path.
pub fn replay_with(
    original: &BacktestResult,
    transform: impl Fn(&FillRecord) -> FillRecord,
    fee_fn: impl Fn(&FillRecord) -> (Money, Money),
) -> Result<BacktestResult, RobustnessError> {
    let fills: Vec<FillRecord> = original.fills.iter().map(transform).collect();
    let fees: Vec<FeeEntry> = fills
        .iter()
        .map(|f| {
            let (commission, tax) = fee_fn(f);
            FeeEntry {
                ts: f.ts,
                commission,
                tax,
            }
        })
        .collect();
    replay(ReplaySpec {
        initial_equity: &original.summary.initial_equity,
        currency: original.summary.currency,
        fills,
        fees,
        orders: &original.orders,
        warnings: &original.warnings,
        provenance: &original.provenance,
        benchmark: &original.benchmark,
    })
}

fn raw4(money: &Money) -> i128 {
    money.amount().bits()
}

fn money_from_raw(raw: i128, currency: Currency) -> Result<Money, RobustnessError> {
    Money::from_fixed(FixedPoint::from_i128(raw, 4).map_err(|e| RobustnessError::Replay {
        detail: format!("money from raw: {e}"),
    })?, currency)
    .map_err(|e| RobustnessError::Replay {
        detail: format!("money from raw: {e}"),
    })
}

fn parse_ts(date: &str) -> Result<UtcTimestamp, RobustnessError> {
    UtcTimestamp::parse_rfc3339(&format!("{date}T00:00:00Z")).map_err(|e| RobustnessError::Replay {
        detail: format!("timestamp {date}: {e}"),
    })
}

fn previous_calendar_day(date: &str) -> String {
    let y: i32 = date[..4].parse().unwrap();
    let m: u32 = date[5..7].parse().unwrap();
    let d: u32 = date[8..10].parse().unwrap();
    if d > 1 {
        return format!("{y:04}-{m:02}-{:02}", d - 1);
    }
    let (pm, pd) = match m {
        1 => (12, 31),
        3 => (2, 28),
        5 | 7 | 10 | 12 => (m - 1, 30),
        _ => (m - 1, 31),
    };
    let py = if m == 1 { y - 1 } else { y };
    format!("{py:04}-{pm:02}-{pd:02}")
}

fn drawdown_curve(equity: &[EquityPoint]) -> Result<Vec<DrawdownPoint>, RobustnessError> {
    let mut peak = f64::NEG_INFINITY;
    let mut curve = Vec::new();
    for point in equity {
        let value = raw4(&point.equity) as f64 / 10_000.0;
        peak = peak.max(value);
        let dd = if peak > 0.0 { value / peak - 1.0 } else { 0.0 };
        curve.push(DrawdownPoint {
            ts: point.ts,
            drawdown: domain::ReportedStat::from_f64(dd).map_err(|e| {
                RobustnessError::NonFinite {
                    field: format!("drawdown: {e}"),
                }
            })?,
        });
    }
    Ok(curve)
}

fn monthly_returns(equity: &[EquityPoint]) -> Result<Vec<MonthlyReturn>, RobustnessError> {
    let initial = equity
        .first()
        .ok_or_else(|| RobustnessError::EmptySeries {
            what: "equity".to_owned(),
        })?
        .equity
        .amount()
        .bits() as f64
        / 10_000.0;
    let mut monthly: Vec<(String, f64)> = Vec::new();
    for point in equity {
        let month = point.ts.to_rfc3339()[..7].to_owned();
        let value = raw4(&point.equity) as f64 / 10_000.0;
        if let Some((last_month, last_value)) = monthly.last_mut() {
            if *last_month == month {
                *last_value = value;
                continue;
            }
        }
        monthly.push((month, value));
    }
    let mut previous = initial;
    let mut out = Vec::new();
    for (month, value) in monthly {
        out.push(MonthlyReturn {
            month,
            return_: domain::ReportedStat::from_f64(value / previous - 1.0).map_err(|e| {
                RobustnessError::NonFinite {
                    field: format!("monthly return: {e}"),
                }
            })?,
        });
        previous = value;
    }
    Ok(out)
}

fn build_summary(
    equity: &[EquityPoint],
    fills: &[FillRecord],
    fees: &[FeeEntry],
    currency: Currency,
    n_orders: usize,
) -> Result<BacktestSummary, RobustnessError> {
    let first = equity
        .first()
        .ok_or_else(|| RobustnessError::EmptySeries {
            what: "equity".to_owned(),
        })?;
    let last = equity
        .last()
        .ok_or_else(|| RobustnessError::EmptySeries {
            what: "equity".to_owned(),
        })?;
    let initial = raw4(&first.equity) as f64 / 10_000.0;
    let final_value = raw4(&last.equity) as f64 / 10_000.0;
    if initial <= 0.0 {
        return Err(RobustnessError::Replay {
            detail: "initial equity must be positive".to_owned(),
        });
    }
    let total_return = final_value / initial - 1.0;

    let mut peak = initial;
    let mut max_drawdown = 0.0f64;
    for point in equity {
        let value = raw4(&point.equity) as f64 / 10_000.0;
        peak = peak.max(value);
        max_drawdown = max_drawdown.min(value / peak - 1.0);
    }

    let days = match (equity.first(), equity.last()) {
        (Some(first_point), Some(last_point)) => {
            day_span(&first_point.ts.to_rfc3339()[..10], &last_point.ts.to_rfc3339()[..10])
                .max(1) as f64
        }
        _ => 1.0,
    };
    let cagr = (final_value / initial).powf(365.25 / days) - 1.0;

    let mut daily = Vec::new();
    for pair in equity.windows(2) {
        let a = raw4(&pair[0].equity) as f64 / 10_000.0;
        let b = raw4(&pair[1].equity) as f64 / 10_000.0;
        daily.push(b / a - 1.0);
    }
    let n = daily.len().max(1) as f64;
    let mean = daily.iter().sum::<f64>() / n;
    let variance = daily.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / n;
    let volatility = variance.sqrt() * 252.0f64.sqrt();
    let sharpe = if volatility > 0.0 {
        mean / volatility * 252.0f64.sqrt()
    } else {
        0.0
    };
    let downside = daily
        .iter()
        .map(|r| if *r < 0.0 { r * r } else { 0.0 })
        .sum::<f64>()
        / n;
    let sortino = if downside > 0.0 {
        mean / downside.sqrt() * 252.0f64.sqrt()
    } else {
        0.0
    };
    let calmar = if max_drawdown < 0.0 {
        cagr / max_drawdown.abs()
    } else {
        0.0
    };
    let mean_equity =
        equity.iter().map(|p| raw4(&p.equity) as f64 / 10_000.0).sum::<f64>() / equity.len() as f64;
    let mut turnover_notional = 0_i128;
    for fill in fills {
        let notional = fill.quantity.checked_mul_price(&fill.price, currency).map_err(|e| {
            RobustnessError::Replay {
                detail: format!("turnover notional: {e}"),
            }
        })?;
        turnover_notional += raw4(&notional);
    }
    let turnover = if mean_equity > 0.0 {
        turnover_notional as f64 / 10_000.0 / mean_equity
    } else {
        0.0
    };

    let total_cost = fees.iter().fold(0_i128, |acc, f| {
        acc + raw4(&f.commission) + raw4(&f.tax)
    });
    let stat = |v: f64| {
        domain::ReportedStat::from_f64(v).map_err(|e| RobustnessError::NonFinite {
            field: format!("summary metric: {e}"),
        })
    };

    Ok(BacktestSummary {
        currency,
        initial_equity: first.equity,
        final_equity: last.equity,
        total_return: stat(total_return)?,
        cagr: stat(cagr)?,
        max_drawdown: stat(max_drawdown)?,
        volatility: stat(volatility)?,
        sharpe: stat(sharpe)?,
        sortino: stat(sortino)?,
        calmar: stat(calmar)?,
        turnover: stat(turnover)?,
        total_cost: money_from_raw(total_cost, currency)?,
        n_orders: n_orders as u64,
        n_fills: fills.len() as u64,
        start_date: equity.first().map(|p| p.ts.to_rfc3339()[..10].to_owned()).unwrap_or_default(),
        end_date: equity.last().map(|p| p.ts.to_rfc3339()[..10].to_owned()).unwrap_or_default(),
    })
}

/// Calendar-day span between two `YYYY-MM-DD` strings (deterministic).
fn day_span(from: &str, to: &str) -> i64 {
    ordinal(to) - ordinal(from)
}

/// Days since 0000-01-01 (proleptic Gregorian, deterministic).
fn ordinal(date: &str) -> i64 {
    let y: i64 = date[..4].parse().unwrap();
    let m: i64 = date[5..7].parse().unwrap();
    let d: i64 = date[8..10].parse().unwrap();
    let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
