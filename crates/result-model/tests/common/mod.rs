//! Shared test helpers for the Todo 21 robustness suite.
//!
//! Every robustness test file includes this module via `mod common;`; the
//! helpers build deterministic, `BacktestResult::validate`-clean fixtures so
//! tests focus on the robustness contract rather than fixture plumbing.
//! Each test binary uses a different subset of the helpers, so dead-code
//! warnings are suppressed here by design.

#![allow(dead_code)]

use domain::provenance::{Engine, RandomSeed, RunProvenance};
use domain::version::{SemVer, StrategyVersion};
use domain::{
    CodeCommit, ContentHash, Currency, DatasetVersionId, FixedPoint, InstrumentId, Money, Price,
    Quantity, ReportedStat, StrategyId, UtcTimestamp, Zone,
};

use result_model::backtest::{
    BacktestResult, BacktestSummary, BenchmarkPoint, CashLedgerEntry, DrawdownPoint, EquityPoint,
    FeeEntry, FillRecord, MonthlyReturn, OrderRecord, OrderSide, PositionSnapshot,
};

/// A deterministic parent provenance (dual_momentum 1.2.0 on the pinned
/// engine/data versions) used by the lineage and suite tests.
pub fn provenance() -> RunProvenance {
    RunProvenance {
        engine: Engine::NautilusTrader,
        engine_version: SemVer::parse("1.231.0").unwrap(),
        strategy_id: StrategyId::parse("dual_momentum").unwrap(),
        strategy_version: StrategyVersion::parse("1.2.0").unwrap(),
        dataset_version: DatasetVersionId::parse("kr-etf-daily-20260804.1").unwrap(),
        config_hash: ContentHash::from_bytes(b"parent-config"),
        code_commit: CodeCommit::parse("0123456789abcdef").unwrap(),
        random_seed: RandomSeed::new(42),
        timezone: Zone::SEOUL,
    }
}

/// The derived provenance: identical pinning to [`provenance`], different
/// config hash (the axis changed the configuration).
pub fn derived_provenance() -> RunProvenance {
    let mut p = provenance();
    p.config_hash = ContentHash::from_bytes(b"derived-config");
    p
}

/// Ten million KRW in scale-4 (the golden-scenario initial capital).
pub fn ten_million() -> Money {
    Money::parse("10000000.0000", Currency::KRW).unwrap()
}

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).unwrap()
}

fn price(amount: &str) -> Price {
    Price::parse(amount).unwrap()
}

fn qty(amount: u64) -> Quantity {
    Quantity::parse(&amount.to_string()).unwrap()
}

fn ts(day: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(&format!("{day}T00:00:00Z")).unwrap()
}

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::parse(symbol).unwrap()
}

fn stat(value: f64) -> ReportedStat {
    ReportedStat::from_f64(value).unwrap()
}

fn day_before(day: &str) -> &'static str {
    match day {
        "2020-01-02" => "2020-01-01",
        "2020-01-03" => "2020-01-02",
        "2020-01-06" => "2020-01-03",
        "2020-01-09" => "2020-01-08",
        "2020-01-14" => "2020-01-13",
        "2020-01-16" => "2020-01-15",
        _ => panic!("unknown fixture day {day}"),
    }
}

/// The deterministic golden-scenario result (initial 10,000,000 KRW, three
/// instruments, six fills, fees 500/600/500/400/570/443).
///
/// Ground truth (hand-verified, all values scale-4 KRW):
///   final equity = 9,921,987.0000; total cost = 3,013.0000;
///   cash at end = 8,871,987.0000; open position = 069500.KRX x100.
/// The equity curve uses point-in-time marks (last fill price per
/// instrument, up to and including each date).
pub fn golden_result() -> BacktestResult {
    let initial = ten_million();

    let fills = vec![
        FillRecord {
            fill_id: "fill-1".to_owned(),
            order_id: "ord-1".to_owned(),
            client_order_id: "co-1".to_owned(),
            instrument: instrument("069500.KRX"),
            side: OrderSide::Buy,
            quantity: qty(200),
            price: price("10000.0000"),
            ts: ts("2020-01-02"),
            commission: krw("500.0000"),
            tax: krw("0.0000"),
        },
        FillRecord {
            fill_id: "fill-2".to_owned(),
            order_id: "ord-2".to_owned(),
            client_order_id: "co-2".to_owned(),
            instrument: instrument("229200.KRX"),
            side: OrderSide::Buy,
            quantity: qty(150),
            price: price("20000.0000"),
            ts: ts("2020-01-03"),
            commission: krw("600.0000"),
            tax: krw("0.0000"),
        },
        FillRecord {
            fill_id: "fill-3".to_owned(),
            order_id: "ord-3".to_owned(),
            client_order_id: "co-3".to_owned(),
            instrument: instrument("114260.KRX"),
            side: OrderSide::Buy,
            quantity: qty(100),
            price: price("30000.0000"),
            ts: ts("2020-01-06"),
            commission: krw("500.0000"),
            tax: krw("0.0000"),
        },
        FillRecord {
            fill_id: "fill-4".to_owned(),
            order_id: "ord-4".to_owned(),
            client_order_id: "co-4".to_owned(),
            instrument: instrument("069500.KRX"),
            side: OrderSide::Sell,
            quantity: qty(100),
            price: price("10500.0000"),
            ts: ts("2020-01-09"),
            commission: krw("400.0000"),
            tax: krw("0.0000"),
        },
        FillRecord {
            fill_id: "fill-5".to_owned(),
            order_id: "ord-5".to_owned(),
            client_order_id: "co-5".to_owned(),
            instrument: instrument("229200.KRX"),
            side: OrderSide::Sell,
            quantity: qty(150),
            price: price("19500.0000"),
            ts: ts("2020-01-14"),
            commission: krw("570.0000"),
            tax: krw("0.0000"),
        },
        FillRecord {
            fill_id: "fill-6".to_owned(),
            order_id: "ord-6".to_owned(),
            client_order_id: "co-6".to_owned(),
            instrument: instrument("114260.KRX"),
            side: OrderSide::Sell,
            quantity: qty(100),
            price: price("29000.0000"),
            ts: ts("2020-01-16"),
            commission: krw("443.0000"),
            tax: krw("0.0000"),
        },
    ];

    let orders: Vec<OrderRecord> = fills
        .iter()
        .map(|f| OrderRecord {
            order_id: f.order_id.clone(),
            client_order_id: f.client_order_id.clone(),
            instrument: f.instrument.clone(),
            side: f.side,
            quantity: f.quantity,
            order_type: "MARKET".to_owned(),
            signal_date: Some(day_before(&f.ts.to_rfc3339()[..10]).to_owned()),
            created_ts: Some(ts(day_before(&f.ts.to_rfc3339()[..10]))),
            execution_ts_target: Some(f.ts),
            state: "FILLED".to_owned(),
        })
        .collect();

    let fees: Vec<FeeEntry> = fills
        .iter()
        .map(|f| FeeEntry {
            ts: f.ts,
            commission: f.commission,
            tax: f.tax,
        })
        .collect();

    // equity/cash ground truth (scale-4 units: 1 KRW = 10_000), open-of-day
    // equity + close-of-day cash.
    let equity_days = [
        ("2020-01-01", 100_000_000_000_i128, 100_000_000_000_i128),
        ("2020-01-02", 99_995_000_000, 79_995_000_000),
        ("2020-01-03", 99_989_000_000, 49_989_000_000),
        ("2020-01-06", 99_984_000_000, 19_984_000_000),
        ("2020-01-09", 100_980_000_000, 30_480_000_000),
        ("2020-01-14", 100_224_300_000, 59_724_300_000),
        ("2020-01-16", 99_219_870_000, 88_719_870_000),
    ];

    let equity: Vec<EquityPoint> = equity_days
        .iter()
        .map(|(day, raw, _)| EquityPoint {
            ts: ts(day),
            equity: Money::from_fixed(FixedPoint::from_i128(*raw, 4).unwrap(), Currency::KRW)
                .unwrap(),
        })
        .collect();

    let cash: Vec<CashLedgerEntry> = equity_days
        .iter()
        .map(|(day, _, raw)| CashLedgerEntry {
            ts: ts(day),
            cash: Money::from_fixed(FixedPoint::from_i128(*raw, 4).unwrap(), Currency::KRW)
                .unwrap(),
        })
        .collect();

    // point-in-time marks (last fill price per instrument up to each date)
    let positions = [
        ("2020-01-02", vec![("069500.KRX", 200_i128)]),
        ("2020-01-03", vec![("069500.KRX", 200), ("229200.KRX", 150)]),
        (
            "2020-01-06",
            vec![
                ("069500.KRX", 200),
                ("114260.KRX", 100),
                ("229200.KRX", 150),
            ],
        ),
        (
            "2020-01-09",
            vec![
                ("069500.KRX", 100),
                ("114260.KRX", 100),
                ("229200.KRX", 150),
            ],
        ),
        ("2020-01-14", vec![("069500.KRX", 100), ("114260.KRX", 100)]),
        ("2020-01-16", vec![("069500.KRX", 100)]),
    ];

    let positions: Vec<PositionSnapshot> = positions
        .iter()
        .flat_map(|(day, rows)| {
            rows.iter()
                .map(|(symbol, q)| PositionSnapshot {
                    date: (*day).to_owned(),
                    instrument: instrument(symbol),
                    quantity: Quantity::parse(&format!("{q}")).unwrap(),
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // drawdown curve (peak tracking over equity, same algorithm as validate)
    let mut peak = 0.0f64;
    let mut drawdown = Vec::new();
    let mut max_drawdown = 0.0f64;
    for point in &equity {
        let value = point.equity.amount().bits() as f64 / 10_000.0;
        peak = peak.max(value);
        let dd = value / peak - 1.0;
        max_drawdown = max_drawdown.min(dd);
        drawdown.push(DrawdownPoint {
            ts: point.ts,
            drawdown: stat(dd),
        });
    }

    // monthly returns (compounded product == final/initial)
    let final_raw = equity.last().unwrap().equity.amount().bits() as f64 / 10_000.0;
    let initial_f64 = initial.amount().bits() as f64 / 10_000.0;
    let monthly_returns = vec![MonthlyReturn {
        month: "2020-01".to_owned(),
        return_: stat(final_raw / initial_f64 - 1.0),
    }];

    // reported metrics (finite only)
    let days = 15.0;
    let cagr = (final_raw / initial_f64).powf(365.25 / days) - 1.0;
    let daily_returns: Vec<f64> = equity
        .windows(2)
        .map(|w| {
            let a = w[0].equity.amount().bits() as f64 / 10_000.0;
            let b = w[1].equity.amount().bits() as f64 / 10_000.0;
            b / a - 1.0
        })
        .collect();
    let mean = daily_returns.iter().sum::<f64>() / daily_returns.len() as f64;
    let variance = daily_returns
        .iter()
        .map(|r| (r - mean) * (r - mean))
        .sum::<f64>()
        / daily_returns.len() as f64;
    let vol = variance.sqrt() * 252.0f64.sqrt();
    let sharpe = if vol > 0.0 {
        mean / vol * 252.0f64.sqrt()
    } else {
        0.0
    };
    let downside = daily_returns
        .iter()
        .map(|r| if *r < 0.0 { r * r } else { 0.0 })
        .sum::<f64>()
        / daily_returns.len() as f64;
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
    let mean_equity = equity
        .iter()
        .map(|p| p.equity.amount().bits() as f64 / 10_000.0)
        .sum::<f64>()
        / equity.len() as f64;
    let turnover = if mean_equity > 0.0 {
        fills
            .iter()
            .map(|f| f.quantity.amount().bits() as f64 * f.price.amount().bits() as f64 / 10_000.0)
            .sum::<f64>()
            / mean_equity
    } else {
        0.0
    };

    let summary = BacktestSummary {
        currency: Currency::KRW,
        initial_equity: initial,
        final_equity: equity.last().unwrap().equity,
        total_return: stat(final_raw / initial_f64 - 1.0),
        cagr: stat(cagr),
        max_drawdown: stat(max_drawdown),
        volatility: stat(vol),
        sharpe: stat(sharpe),
        sortino: stat(sortino),
        calmar: stat(calmar),
        turnover: stat(turnover),
        total_cost: krw("3013.0000"),
        n_orders: 6,
        n_fills: 6,
        start_date: "2020-01-01".to_owned(),
        end_date: "2020-01-16".to_owned(),
    };

    let metrics = [
        ("total_return", final_raw / initial_f64 - 1.0),
        ("max_drawdown", max_drawdown),
        ("volatility", vol),
        ("sharpe", sharpe),
        ("sortino", sortino),
        ("calmar", calmar),
        ("turnover", turnover),
        ("total_cost", 3013.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), stat(v)))
    .collect();

    let benchmark = vec![
        BenchmarkPoint {
            ts: ts("2020-01-01"),
            value: initial,
        },
        BenchmarkPoint {
            ts: ts("2020-01-08"),
            value: krw("10050000.0000"),
        },
        BenchmarkPoint {
            ts: ts("2020-01-16"),
            value: krw("9980000.0000"),
        },
    ];

    BacktestResult {
        summary,
        equity,
        drawdown,
        monthly_returns,
        orders,
        fills,
        positions,
        cash,
        fees,
        benchmark,
        metrics,
        warnings: Vec::new(),
        provenance: provenance(),
    }
}

/// Raw scale-4 units of a Money value.
pub fn raw4(money: &Money) -> i128 {
    money.amount().bits()
}
