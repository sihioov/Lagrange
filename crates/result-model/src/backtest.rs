//! `BacktestResult` — the platform common model for normalized backtest
//! results (design §6.10, plan Todo 20).
//!
//! This module is the CONTRACT: the `nt/backtest-worker` normalizer produces
//! exactly the 13 sections declared here (`summary, equity, drawdown,
//! monthly_returns, orders, fills, positions, cash, fees, benchmark, metrics,
//! warnings, provenance`). Large arrays are stored as Parquet by the worker;
//! this Rust model defines the canonical shape, units, and integrity checks.
//!
//! Units: money is fixed-point [`domain::Money`] (KRW scale-4, decimal strings
//! across JSON); returns and drawdowns are DECIMAL FRACTIONS (0.0106 = 1.06%,
//! -0.35 = -35% MDD); quantities are whole units; timestamps are RFC 3339 UTC;
//! `month` is `YYYY-MM`; dates are `YYYY-MM-DD`.
//!
//! NaN / ±Infinity are structurally impossible here: every float is a
//! [`domain::ReportedStat`] (finite by construction and at the JSON boundary),
//! and every money/price/quantity is fixed-point. The Python normalizer still
//! rejects non-finite values in the RAW NT results before conversion (same
//! semantic, tested on both sides).
//!
//! [`BacktestResult::validate`] enforces the design §6.10 integrity checks
//! that are derivable from the common model: monotonic dates, fill-quantity
//! sums reconciling with position snapshots, the cash ledger reconciling with
//! fill notionals + fees, and summary returns/drawdown/monthly consistency.
//! The worker additionally verifies `cash + positions_value == equity` at the
//! raw level (where NT reports the independent breakdown). Nothing may be
//! published through [`PublicationGate`] until validation succeeds.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use domain::provenance::RunProvenance;
use domain::{Currency, InstrumentId, Money, Price, Quantity, ReportedStat, UtcTimestamp};

use crate::Warning;

/// A typed failure of a [`BacktestResult`] integrity check or publication gate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BacktestError {
    /// A non-finite value appeared in a reported statistic.
    #[error("non-finite value at {field}: {value}")]
    NonFiniteValue { field: String, value: String },
    /// Dates in a timestamped array did not increase monotonically.
    #[error("date regression in {field} at index {index}")]
    DateRegression { field: &'static str, index: usize },
    /// A ledger invariant failed to reconcile (fills vs positions, cash vs
    /// fills+fees, returns vs equity, ...).
    #[error("ledger mismatch: {detail}")]
    LedgerMismatch { detail: String },
    /// Publication was attempted without a successful integrity validation.
    #[error("publication denied: {reason}")]
    PublicationDenied { reason: String },
}

/// Buy/sell direction of an order or fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// One point of the daily equity curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquityPoint {
    pub ts: UtcTimestamp,
    pub equity: Money,
}

/// One point of the drawdown curve (negative decimal fraction).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawdownPoint {
    pub ts: UtcTimestamp,
    pub drawdown: ReportedStat,
}

/// One monthly return (decimal fraction) keyed by `YYYY-MM`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthlyReturn {
    pub month: String,
    #[serde(rename = "return")]
    pub return_: ReportedStat,
}

/// A submitted order (normalized).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderRecord {
    pub order_id: String,
    pub client_order_id: String,
    pub instrument: InstrumentId,
    pub side: OrderSide,
    pub quantity: Quantity,
    pub order_type: String,
    pub signal_date: Option<String>,
    pub created_ts: Option<UtcTimestamp>,
    pub execution_ts_target: Option<UtcTimestamp>,
    pub state: String,
}

/// A filled execution with its cost breakdown (normalized).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillRecord {
    pub fill_id: String,
    pub order_id: String,
    pub client_order_id: String,
    pub instrument: InstrumentId,
    pub side: OrderSide,
    pub quantity: Quantity,
    pub price: Price,
    pub ts: UtcTimestamp,
    pub commission: Money,
    pub tax: Money,
}

/// A per-date, per-instrument position snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionSnapshot {
    pub date: String,
    pub instrument: InstrumentId,
    pub quantity: Quantity,
}

/// One entry of the cash ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CashLedgerEntry {
    pub ts: UtcTimestamp,
    pub cash: Money,
}

/// One fee line item (commission + tax).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeeEntry {
    pub ts: UtcTimestamp,
    pub commission: Money,
    pub tax: Money,
}

/// One point of the benchmark series (buy-and-hold reference portfolio value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkPoint {
    pub ts: UtcTimestamp,
    pub value: Money,
}

/// Run headline facts (design §6.10 `summary`). Returns/drawdown are decimal
/// fractions; money is scale-4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestSummary {
    pub currency: Currency,
    pub initial_equity: Money,
    pub final_equity: Money,
    pub total_return: ReportedStat,
    pub cagr: ReportedStat,
    pub max_drawdown: ReportedStat,
    pub volatility: ReportedStat,
    pub sharpe: ReportedStat,
    pub sortino: ReportedStat,
    pub calmar: ReportedStat,
    pub turnover: ReportedStat,
    pub total_cost: Money,
    pub n_orders: u64,
    pub n_fills: u64,
    pub start_date: String,
    pub end_date: String,
}

/// The normalized backtest result (design §6.10 `BacktestResult`).
///
/// Field names, units, and ordering are the worker contract; do not rename
/// without updating `nt/backtest-worker` and the T3 `result_artifacts` types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestResult {
    pub summary: BacktestSummary,
    pub equity: Vec<EquityPoint>,
    pub drawdown: Vec<DrawdownPoint>,
    pub monthly_returns: Vec<MonthlyReturn>,
    pub orders: Vec<OrderRecord>,
    pub fills: Vec<FillRecord>,
    pub positions: Vec<PositionSnapshot>,
    pub cash: Vec<CashLedgerEntry>,
    pub fees: Vec<FeeEntry>,
    pub benchmark: Vec<BenchmarkPoint>,
    pub metrics: BTreeMap<String, ReportedStat>,
    pub warnings: Vec<Warning>,
    pub provenance: RunProvenance,
}

/// Reconciliation tolerance for the cash ledger, in raw scale-4 units
/// (100 units = 0.01 KRW), absorbing per-fill rounding of commissions.
const CASH_TOLERANCE_RAW: i128 = 100;

/// Relative tolerance for float consistency checks (returns, drawdowns).
const FLOAT_TOLERANCE: f64 = 1e-6;

fn raw4(money: &Money) -> i128 {
    money.amount().bits()
}

fn f64_money(money: &Money) -> f64 {
    money.amount().bits() as f64 / 10_000.0
}

impl BacktestResult {
    /// Runs every integrity check (design §6.10). Returns the first failure.
    pub fn validate(&self) -> Result<(), BacktestError> {
        self.validate_dates()?;
        self.validate_fills_to_positions()?;
        self.validate_cash_ledger()?;
        self.validate_return_equity()?;
        self.validate_drawdown()?;
        self.validate_monthly_returns()?;
        Ok(())
    }

    fn validate_dates(&self) -> Result<(), BacktestError> {
        let fields: [(&'static str, Vec<&UtcTimestamp>); 5] = [
            ("equity", self.equity.iter().map(|p| &p.ts).collect()),
            ("drawdown", self.drawdown.iter().map(|p| &p.ts).collect()),
            ("cash", self.cash.iter().map(|p| &p.ts).collect()),
            ("fees", self.fees.iter().map(|p| &p.ts).collect()),
            ("benchmark", self.benchmark.iter().map(|p| &p.ts).collect()),
        ];
        for (field, ts) in fields {
            for pair in ts.windows(2).enumerate() {
                if pair.1[0] > pair.1[1] {
                    return Err(BacktestError::DateRegression {
                        field,
                        index: pair.0 + 1,
                    });
                }
            }
        }
        for pair in self
            .fills
            .iter()
            .map(|p| &p.ts)
            .collect::<Vec<_>>()
            .windows(2)
            .enumerate()
        {
            if pair.1[0] > pair.1[1] {
                return Err(BacktestError::DateRegression {
                    field: "fills",
                    index: pair.0 + 1,
                });
            }
        }
        for pair in self
            .positions
            .iter()
            .map(|p| &p.date)
            .collect::<Vec<_>>()
            .windows(2)
            .enumerate()
        {
            if pair.1[0] > pair.1[1] {
                return Err(BacktestError::DateRegression {
                    field: "positions",
                    index: pair.0 + 1,
                });
            }
        }
        for pair in self
            .monthly_returns
            .iter()
            .map(|p| &p.month)
            .collect::<Vec<_>>()
            .windows(2)
            .enumerate()
        {
            if pair.1[0] > pair.1[1] {
                return Err(BacktestError::DateRegression {
                    field: "monthly_returns",
                    index: pair.0 + 1,
                });
            }
        }
        let created: Vec<&UtcTimestamp> = self
            .orders
            .iter()
            .filter_map(|o| o.created_ts.as_ref())
            .collect();
        for pair in created.windows(2).enumerate() {
            if pair.1[0] > pair.1[1] {
                return Err(BacktestError::DateRegression {
                    field: "orders",
                    index: pair.0 + 1,
                });
            }
        }
        let signals: Vec<&String> = self
            .orders
            .iter()
            .filter_map(|o| o.signal_date.as_ref())
            .collect();
        for pair in signals.windows(2).enumerate() {
            if pair.1[0] > pair.1[1] {
                return Err(BacktestError::DateRegression {
                    field: "orders.signal_date",
                    index: pair.0 + 1,
                });
            }
        }
        Ok(())
    }

    /// Fill-quantity sums must reconcile with position snapshots (design:
    /// "체결 수량 합계와 포지션 변화 일치"). A snapshot at date D must equal
    /// the cumulative signed fill quantity for that instrument up to D.
    fn validate_fills_to_positions(&self) -> Result<(), BacktestError> {
        let mut cumulative: HashMap<InstrumentId, i128> = HashMap::new();
        let mut fills = self.fills.iter().peekable();
        for (i, snapshot) in self.positions.iter().enumerate() {
            let day = snapshot.date.as_str();
            while let Some(fill) = fills.peek() {
                if &fill.ts.to_rfc3339()[..10] <= day {
                    let delta = match fill.side {
                        OrderSide::Buy => fill.quantity.amount().bits(),
                        OrderSide::Sell => -fill.quantity.amount().bits(),
                    };
                    *cumulative.entry(fill.instrument.clone()).or_insert(0) += delta;
                    fills.next();
                } else {
                    break;
                }
            }
            let expected = cumulative.get(&snapshot.instrument).copied().unwrap_or(0);
            if snapshot.quantity.amount().bits() != expected {
                return Err(BacktestError::LedgerMismatch {
                    detail: format!(
                        "position {} at date {} is {}, fills sum to {}",
                        snapshot.instrument, snapshot.date, snapshot.quantity, expected
                    ),
                });
            }
            let _ = i;
        }
        Ok(())
    }

    /// Cash ledger must reconcile with initial equity minus fill notionals and
    /// fees (design: "비용 합계와 현금 장부 일치").
    fn validate_cash_ledger(&self) -> Result<(), BacktestError> {
        let initial = raw4(&self.summary.initial_equity);
        let fill_date = |f: &FillRecord| f.ts.to_rfc3339()[..10].to_owned();
        let fee_date = |f: &FeeEntry| f.ts.to_rfc3339()[..10].to_owned();
        let mut spent: Vec<(String, i128)> = Vec::new();
        for fill in &self.fills {
            spent.push((
                fill_date(fill).to_owned(),
                fill.quantity.amount().bits() * fill.price.amount().bits(),
            ));
        }
        for fee in &self.fees {
            let total = raw4(&fee.commission) + raw4(&fee.tax);
            spent.push((fee_date(fee).to_owned(), total));
        }
        for (i, entry) in self.cash.iter().enumerate() {
            let day = &entry.ts.to_rfc3339()[..10];
            let spent_by: i128 = spent
                .iter()
                .filter(|(d, _)| d.as_str() <= day)
                .map(|(_, v)| v)
                .sum();
            let expected = initial - spent_by;
            if (expected - raw4(&entry.cash)).abs() > CASH_TOLERANCE_RAW {
                return Err(BacktestError::LedgerMismatch {
                    detail: format!(
                        "cash at index {i} ({day}) is {}, expected {expected} (initial minus fills+fees)",
                        entry.cash
                    ),
                });
            }
        }
        Ok(())
    }

    /// Summary returns must agree with the equity curve and initial/final
    /// equity must match the curve endpoints (design: "초기·최종 자산과 수익률 일치").
    fn validate_return_equity(&self) -> Result<(), BacktestError> {
        let first = self
            .equity
            .first()
            .ok_or_else(|| BacktestError::LedgerMismatch {
                detail: "equity curve is empty".to_owned(),
            })?;
        let last = self
            .equity
            .last()
            .ok_or_else(|| BacktestError::LedgerMismatch {
                detail: "equity curve is empty".to_owned(),
            })?;
        if self.summary.initial_equity != first.equity {
            return Err(BacktestError::LedgerMismatch {
                detail: format!(
                    "summary initial equity {} does not match first equity point {}",
                    self.summary.initial_equity, first.equity
                ),
            });
        }
        if self.summary.final_equity != last.equity {
            return Err(BacktestError::LedgerMismatch {
                detail: format!(
                    "summary final equity {} does not match last equity point {}",
                    self.summary.final_equity, last.equity
                ),
            });
        }
        let initial = f64_money(&self.summary.initial_equity);
        if initial <= 0.0 {
            return Err(BacktestError::LedgerMismatch {
                detail: "initial equity must be positive".to_owned(),
            });
        }
        let actual = f64_money(&self.summary.final_equity) / initial - 1.0;
        if (actual - self.summary.total_return.value()).abs() > FLOAT_TOLERANCE {
            return Err(BacktestError::LedgerMismatch {
                detail: format!(
                    "summary total_return {} disagrees with equity curve return {actual:.8}",
                    self.summary.total_return
                ),
            });
        }
        Ok(())
    }

    /// The drawdown curve must equal equity / running-peak - 1, and the summary
    /// max_drawdown must be the minimum of the curve.
    fn validate_drawdown(&self) -> Result<(), BacktestError> {
        let mut peak = f64::NEG_INFINITY;
        let mut min_dd = 0.0f64;
        for (i, point) in self.equity.iter().enumerate() {
            let value = f64_money(&point.equity);
            peak = peak.max(value);
            let expected = if peak > 0.0 { value / peak - 1.0 } else { 0.0 };
            let actual = self
                .drawdown
                .get(i)
                .map(|d| d.drawdown.value())
                .unwrap_or(f64::NEG_INFINITY);
            if (actual - expected).abs() > FLOAT_TOLERANCE {
                return Err(BacktestError::LedgerMismatch {
                    detail: format!(
                        "drawdown at index {i} is {actual:.8}, expected {expected:.8} from the equity curve"
                    ),
                });
            }
            min_dd = min_dd.min(actual);
        }
        if (min_dd - self.summary.max_drawdown.value()).abs() > FLOAT_TOLERANCE {
            return Err(BacktestError::LedgerMismatch {
                detail: format!(
                    "summary max_drawdown {} disagrees with the drawdown curve minimum {min_dd:.8}",
                    self.summary.max_drawdown
                ),
            });
        }
        Ok(())
    }

    /// The compounded monthly returns must equal the total return.
    fn validate_monthly_returns(&self) -> Result<(), BacktestError> {
        let mut product = 1.0f64;
        for m in &self.monthly_returns {
            product *= 1.0 + m.return_.value();
        }
        let initial = f64_money(&self.summary.initial_equity);
        if initial <= 0.0 {
            return Err(BacktestError::LedgerMismatch {
                detail: "initial equity must be positive".to_owned(),
            });
        }
        let actual = f64_money(&self.summary.final_equity) / initial;
        if (product - actual).abs() > FLOAT_TOLERANCE {
            return Err(BacktestError::LedgerMismatch {
                detail: format!(
                    "compounded monthly returns {product:.8} disagree with equity ratio {actual:.8}"
                ),
            });
        }
        Ok(())
    }
}

/// Gate that refuses publication until [`BacktestResult::validate`] succeeds.
///
/// A failed validation permanently latches the gate: the result must never be
/// published after an integrity failure (design §6.10; plan Todo 20
/// "publication after integrity failure" rejection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PublicationGate {
    state: GateState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GateState {
    #[default]
    New,
    Validated,
    Failed,
}

impl PublicationGate {
    /// A fresh gate; nothing may be published until it is validated.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates the result, latching the gate. On failure the gate is
    /// permanently closed.
    pub fn validate(&mut self, result: &BacktestResult) -> Result<(), BacktestError> {
        match result.validate() {
            Ok(()) => {
                self.state = GateState::Validated;
                Ok(())
            }
            Err(error) => {
                self.state = GateState::Failed;
                Err(error)
            }
        }
    }

    /// Returns `Ok(())` only when a validation succeeded; refuses otherwise.
    pub fn publish(&self) -> Result<(), BacktestError> {
        match self.state {
            GateState::Validated => Ok(()),
            GateState::New => Err(BacktestError::PublicationDenied {
                reason: "result was never validated".to_owned(),
            }),
            GateState::Failed => Err(BacktestError::PublicationDenied {
                reason: "integrity validation failed; the result must not be published".to_owned(),
            }),
        }
    }
}
