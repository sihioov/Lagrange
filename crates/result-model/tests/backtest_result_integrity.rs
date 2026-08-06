//! Red tests (Todo 20): the `BacktestResult` common model, its integrity
//! invariants (NaN/Infinity, date regressions, ledger mismatch), and the
//! publication gate. Design §6.10 + plan Todo 20: the crate is the contract —
//! the `nt/backtest-worker` normalizer produces exactly what this declares.

use std::collections::{BTreeMap, BTreeSet};

use domain::provenance::{Engine, RandomSeed, RunProvenance};
use domain::{
    CodeCommit, ContentHash, Currency, DatasetVersionId, InstrumentId, Money, Price, Quantity,
    ReportedStat, SemVer, StrategyId, StrategyVersion, UtcTimestamp, Zone,
};
use result_model::backtest::{
    BacktestResult, BacktestSummary, BenchmarkPoint, CashLedgerEntry, DrawdownPoint, EquityPoint,
    FillRecord, MonthlyReturn, OrderRecord, OrderSide, PositionSnapshot,
};
use result_model::{BacktestError, PublicationGate, Warning};

fn ts(s: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(s).unwrap()
}

fn krw(s: &str) -> Money {
    Money::parse(s, Currency::KRW).unwrap()
}

fn provenance() -> RunProvenance {
    RunProvenance {
        engine: Engine::NautilusTrader,
        engine_version: SemVer::parse("1.231.0").unwrap(),
        strategy_id: StrategyId::parse("ma200_trend").unwrap(),
        strategy_version: StrategyVersion::parse("1.0.0").unwrap(),
        dataset_version: DatasetVersionId::parse("kr-etf-daily-20260804.1").unwrap(),
        config_hash: ContentHash::from_bytes(b"config"),
        code_commit: CodeCommit::parse("abcdef1234567").unwrap(),
        random_seed: RandomSeed::new(42),
        timezone: Zone::SEOUL,
    }
}

/// A fully consistent fixture: buy 3300 shares of 069500.KRX at 10106.0960 on
/// 2020-01-02, hold to 2020-12-31, final equity 101060960.0000.
fn valid_result() -> BacktestResult {
    let initial = krw("100000000.0000");
    let notional = krw("33350116.8000"); // 3300 x 10106.0960
    let cash_after = initial.checked_sub(&notional).unwrap();
    let final_equity = krw("101060960.0000");
    let t1 = ts("2020-01-02T00:00:00Z");
    let t2 = ts("2020-12-31T00:00:00Z");
    let instrument = InstrumentId::parse("069500.KRX").unwrap();

    let mut metrics = BTreeMap::new();
    metrics.insert(
        "total_return".to_owned(),
        ReportedStat::from_f64(0.010_609_6).unwrap(),
    );
    metrics.insert(
        "max_drawdown".to_owned(),
        ReportedStat::from_f64(0.0).unwrap(),
    );
    metrics.insert("sharpe".to_owned(), ReportedStat::from_f64(0.75).unwrap());

    BacktestResult {
        summary: BacktestSummary {
            currency: Currency::KRW,
            initial_equity: initial,
            final_equity,
            total_return: ReportedStat::from_f64(0.010_609_6).unwrap(),
            cagr: ReportedStat::from_f64(0.010_609_6).unwrap(),
            max_drawdown: ReportedStat::from_f64(0.0).unwrap(),
            volatility: ReportedStat::from_f64(0.0).unwrap(),
            sharpe: ReportedStat::from_f64(0.75).unwrap(),
            sortino: ReportedStat::from_f64(1.0).unwrap(),
            calmar: ReportedStat::from_f64(0.0).unwrap(),
            turnover: ReportedStat::from_f64(0.333_501_168).unwrap(),
            total_cost: krw("0.0000"),
            n_orders: 1,
            n_fills: 1,
            start_date: "2020-01-02".to_owned(),
            end_date: "2020-12-31".to_owned(),
        },
        equity: vec![
            EquityPoint {
                ts: t1,
                equity: initial,
            },
            EquityPoint {
                ts: t2,
                equity: final_equity,
            },
        ],
        drawdown: vec![
            DrawdownPoint {
                ts: t1,
                drawdown: ReportedStat::from_f64(0.0).unwrap(),
            },
            DrawdownPoint {
                ts: t2,
                drawdown: ReportedStat::from_f64(0.0).unwrap(),
            },
        ],
        monthly_returns: vec![
            MonthlyReturn {
                month: "2020-01".to_owned(),
                return_: ReportedStat::from_f64(0.0).unwrap(),
            },
            MonthlyReturn {
                month: "2020-12".to_owned(),
                return_: ReportedStat::from_f64(0.010_609_6).unwrap(),
            },
        ],
        orders: vec![OrderRecord {
            order_id: "O-1".to_owned(),
            client_order_id: "C-1".to_owned(),
            instrument: instrument.clone(),
            side: OrderSide::Buy,
            quantity: Quantity::parse("3300").unwrap(),
            order_type: "MARKET".to_owned(),
            signal_date: Some("2020-01-01".to_owned()),
            created_ts: Some(t1),
            execution_ts_target: Some(t1),
            state: "FILLED".to_owned(),
        }],
        fills: vec![FillRecord {
            fill_id: "F-1".to_owned(),
            order_id: "O-1".to_owned(),
            client_order_id: "C-1".to_owned(),
            instrument: instrument.clone(),
            side: OrderSide::Buy,
            quantity: Quantity::parse("3300").unwrap(),
            price: Price::parse("10106.0960").unwrap(),
            ts: t1,
            commission: krw("0.0000"),
            tax: krw("0.0000"),
        }],
        positions: vec![PositionSnapshot {
            date: "2020-01-02".to_owned(),
            instrument,
            quantity: Quantity::parse("3300").unwrap(),
        }],
        cash: vec![
            CashLedgerEntry {
                ts: t1,
                cash: cash_after,
            },
            CashLedgerEntry {
                ts: t2,
                cash: cash_after,
            },
        ],
        fees: vec![],
        benchmark: vec![
            BenchmarkPoint {
                ts: t1,
                value: initial,
            },
            BenchmarkPoint {
                ts: t2,
                value: krw("102000000.0000"),
            },
        ],
        metrics,
        warnings: vec![Warning::info("ok", "synthetic fixture")],
        provenance: provenance(),
    }
}

#[test]
fn common_model_has_all_documented_fields() {
    let json = serde_json::to_value(valid_result()).unwrap();
    let obj = json.as_object().unwrap();
    let keys: BTreeSet<String> = obj.keys().cloned().collect();
    let expected: BTreeSet<String> = [
        "summary",
        "equity",
        "drawdown",
        "monthly_returns",
        "orders",
        "fills",
        "positions",
        "cash",
        "fees",
        "benchmark",
        "metrics",
        "warnings",
        "provenance",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(keys, expected);
}

#[test]
fn valid_result_round_trips_and_validates() {
    let result = valid_result();
    let json = serde_json::to_string(&result).unwrap();
    let back: BacktestResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back, result);
    result.validate().expect("consistent fixture must validate");
}

#[test]
fn non_finite_metric_is_rejected_at_the_json_boundary() {
    // 1e999 is a valid JSON number token that overflows f64 -> non-finite.
    let json = serde_json::to_string(&valid_result()).unwrap();
    let poisoned = json.replace("0.0106096", "1e999");
    let err = serde_json::from_str::<BacktestResult>(&poisoned);
    assert!(err.is_err(), "1e999 must be rejected, got {err:?}");
}

#[test]
fn non_finite_drawdown_is_rejected_at_the_json_boundary() {
    let mut json = serde_json::to_value(valid_result()).unwrap();
    json["drawdown"][0]["drawdown"] = serde_json::json!("Infinity");
    let err = serde_json::from_str::<BacktestResult>(&serde_json::to_string(&json).unwrap());
    assert!(
        err.is_err(),
        "Infinity drawdown must be rejected, got {err:?}"
    );
}

#[test]
fn date_regression_in_equity_is_rejected() {
    let mut result = valid_result();
    result.equity[0].ts = ts("2021-01-01T00:00:00Z"); // later than point 1 -> regression
    match result.validate() {
        Err(BacktestError::DateRegression { .. }) => {}
        other => panic!("expected DateRegression, got {other:?}"),
    }
}

#[test]
fn date_regression_in_fills_is_rejected() {
    let mut result = valid_result();
    let t0 = ts("2019-12-30T00:00:00Z");
    result.fills.push(FillRecord {
        fill_id: "F-0".to_owned(),
        order_id: "O-0".to_owned(),
        client_order_id: "C-0".to_owned(),
        instrument: InstrumentId::parse("069500.KRX").unwrap(),
        side: OrderSide::Sell,
        quantity: Quantity::parse("1").unwrap(),
        price: Price::parse("10106.0960").unwrap(),
        ts: t0, // earlier than the first fill -> regression at index 1
        commission: krw("0.0000"),
        tax: krw("0.0000"),
    });
    match result.validate() {
        Err(BacktestError::DateRegression { .. }) => {}
        other => panic!("expected DateRegression, got {other:?}"),
    }
}

#[test]
fn ledger_mismatch_between_fills_and_positions_is_rejected() {
    let mut result = valid_result();
    // positions say 3300 but fills sum to 3300; break the reconciliation:
    result.positions[0].quantity = Quantity::parse("3200").unwrap();
    match result.validate() {
        Err(BacktestError::LedgerMismatch { detail }) if detail.contains("position") => {}
        other => panic!("expected position LedgerMismatch, got {other:?}"),
    }
}

#[test]
fn ledger_mismatch_between_cash_and_fills_is_rejected() {
    let mut result = valid_result();
    // cash says 1000 more than initial - notional - fees allows
    result.cash[0].cash = krw("66650883.2000");
    result.cash[1].cash = krw("66650883.2000");
    match result.validate() {
        Err(BacktestError::LedgerMismatch { detail }) if detail.contains("cash") => {}
        other => panic!("expected cash LedgerMismatch, got {other:?}"),
    }
}

#[test]
fn return_equity_inconsistency_is_rejected() {
    let mut result = valid_result();
    // summary claims a 20% return but the equity curve only gained 1.06%
    result.summary.total_return = ReportedStat::from_f64(0.20).unwrap();
    match result.validate() {
        Err(BacktestError::LedgerMismatch { detail }) if detail.contains("return") => {}
        other => panic!("expected return LedgerMismatch, got {other:?}"),
    }
}

#[test]
fn publication_is_refused_before_validation() {
    let gate = PublicationGate::new();
    match gate.publish() {
        Err(BacktestError::PublicationDenied { .. }) => {}
        other => panic!("expected PublicationDenied, got {other:?}"),
    }
}

#[test]
fn publication_is_refused_after_a_failed_validation() {
    let mut result = valid_result();
    result.equity[0].ts = ts("2021-01-01T00:00:00Z");
    let mut gate = PublicationGate::new();
    assert!(gate.validate(&result).is_err());
    match gate.publish() {
        Err(BacktestError::PublicationDenied { .. }) => {}
        other => panic!("expected PublicationDenied after failed validate, got {other:?}"),
    }
}

#[test]
fn publication_is_allowed_after_a_successful_validation() {
    let result = valid_result();
    let mut gate = PublicationGate::new();
    gate.validate(&result).expect("fixture is consistent");
    gate.publish().expect("validated result may be published");
}

#[test]
fn sell_credit_reconciles_in_cash_ledger() {
    // A SELL fill must CREDIT the cash ledger: initial - (-notional) == cash.
    let t1 = ts("2020-01-02T00:00:00Z");
    let t2 = ts("2020-02-03T00:00:00Z");
    let mut metrics = BTreeMap::new();
    metrics.insert(
        "total_return".to_owned(),
        ReportedStat::from_f64(0.10).unwrap(),
    );
    metrics.insert(
        "max_drawdown".to_owned(),
        ReportedStat::from_f64(0.0).unwrap(),
    );
    metrics.insert("sharpe".to_owned(), ReportedStat::from_f64(0.0).unwrap());

    let result = BacktestResult {
        summary: BacktestSummary {
            currency: Currency::KRW,
            initial_equity: krw("100000000.0000"),
            final_equity: krw("110000000.0000"),
            total_return: ReportedStat::from_f64(0.10).unwrap(),
            cagr: ReportedStat::from_f64(0.10).unwrap(),
            max_drawdown: ReportedStat::from_f64(0.0).unwrap(),
            volatility: ReportedStat::from_f64(0.0).unwrap(),
            sharpe: ReportedStat::from_f64(0.0).unwrap(),
            sortino: ReportedStat::from_f64(0.0).unwrap(),
            calmar: ReportedStat::from_f64(0.0).unwrap(),
            turnover: ReportedStat::from_f64(0.0).unwrap(),
            total_cost: krw("0.0000"),
            n_orders: 1,
            n_fills: 1,
            start_date: "2020-01-02".to_owned(),
            end_date: "2020-02-03".to_owned(),
        },
        equity: vec![
            EquityPoint {
                ts: t1,
                equity: krw("100000000.0000"),
            },
            EquityPoint {
                ts: t2,
                equity: krw("110000000.0000"),
            },
        ],
        drawdown: vec![
            DrawdownPoint {
                ts: t1,
                drawdown: ReportedStat::from_f64(0.0).unwrap(),
            },
            DrawdownPoint {
                ts: t2,
                drawdown: ReportedStat::from_f64(0.0).unwrap(),
            },
        ],
        monthly_returns: vec![
            MonthlyReturn {
                month: "2020-01".to_owned(),
                return_: ReportedStat::from_f64(0.0).unwrap(),
            },
            MonthlyReturn {
                month: "2020-02".to_owned(),
                return_: ReportedStat::from_f64(0.10).unwrap(),
            },
        ],
        orders: vec![],
        fills: vec![FillRecord {
            fill_id: "S-1".to_owned(),
            order_id: "O-1".to_owned(),
            client_order_id: "C-1".to_owned(),
            instrument: InstrumentId::parse("069500.KRX").unwrap(),
            side: OrderSide::Sell,
            quantity: Quantity::parse("1000").unwrap(),
            price: Price::parse("10000.0000").unwrap(),
            ts: t2,
            commission: krw("0.0000"),
            tax: krw("0.0000"),
        }],
        positions: vec![],
        cash: vec![
            CashLedgerEntry {
                ts: t1,
                cash: krw("100000000.0000"),
            },
            CashLedgerEntry {
                ts: t2,
                cash: krw("110000000.0000"),
            },
        ],
        fees: vec![],
        benchmark: vec![
            BenchmarkPoint {
                ts: t1,
                value: krw("100000000.0000"),
            },
            BenchmarkPoint {
                ts: t2,
                value: krw("100000000.0000"),
            },
        ],
        metrics,
        warnings: vec![],
        provenance: provenance(),
    };
    result
        .validate()
        .expect("a sell credit must reconcile the cash ledger");
}

#[test]
fn provenance_shape_matches_design_execution_metadata() {
    let json = serde_json::to_value(valid_result().provenance).unwrap();
    for key in [
        "engine",
        "engine_version",
        "strategy_id",
        "strategy_version",
        "dataset_version",
        "config_hash",
        "code_commit",
        "random_seed",
        "timezone",
    ] {
        assert!(json.get(key).is_some(), "provenance missing {key}");
    }
    assert_eq!(json["engine"], "nautilustrader");
    assert_eq!(json["timezone"], "Asia/Seoul");
    assert_eq!(json["random_seed"], 42);
}
