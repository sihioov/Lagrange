//! Todo 21 RED tests: concentration and recent-degradation warnings
//! (FR-ROB-005) plus parameter-neighborhood analysis (FR-ROB-002).
//!
//! - top-trade contribution (top-3 realized-PnL share > 50%) warns;
//! - single-year contribution (> 40% of total |year contribution|) warns;
//! - recent-window underperformance vs the benchmark warns;
//! - parameter-neighborhood dispersion is reported and an adjacent-delta
//!   performance jump warns (성과 급변 경고).

mod common;

use domain::ReportedStat;
use serde_json::json;

use domain::{Currency, InstrumentId, Money, Price, Quantity, UtcTimestamp};

use result_model::backtest::{BacktestResult, FillRecord, OrderSide};
use result_model::robustness::{
    NeighborhoodAnalysis, ReplaySpec, RobustnessError, analyze_neighborhood,
    recent_degradation_warning, replay, top_trade_concentration_warning,
    year_concentration_warning,
};

fn stat(value: f64) -> ReportedStat {
    ReportedStat::from_f64(value).unwrap()
}

fn fill(
    id: &str,
    day: &str,
    instrument: &str,
    side: OrderSide,
    qty: u64,
    price: &str,
) -> FillRecord {
    FillRecord {
        fill_id: id.to_owned(),
        order_id: format!("ord-{id}"),
        client_order_id: format!("co-{id}"),
        instrument: InstrumentId::parse(instrument).unwrap(),
        side,
        quantity: Quantity::parse(&qty.to_string()).unwrap(),
        price: Price::parse(price).unwrap(),
        ts: UtcTimestamp::parse_rfc3339(&format!("{day}T00:00:00Z")).unwrap(),
        commission: Money::parse("0.0000", Currency::KRW).unwrap(),
        tax: Money::parse("0.0000", Currency::KRW).unwrap(),
    }
}

/// Builds a valid result via the public replay (zero fees) from raw fills.
fn result_from_fills(fills: Vec<FillRecord>) -> BacktestResult {
    let fees: Vec<result_model::backtest::FeeEntry> = fills
        .iter()
        .map(|f| result_model::backtest::FeeEntry {
            ts: f.ts,
            commission: Money::parse("0.0000", Currency::KRW).unwrap(),
            tax: Money::parse("0.0000", Currency::KRW).unwrap(),
        })
        .collect();
    replay(ReplaySpec {
        initial_equity: &common::ten_million(),
        currency: Currency::KRW,
        fills,
        fees,
        orders: &[],
        warnings: &[],
        provenance: &common::provenance(),
        benchmark: &[],
    })
    .expect("synthetic replay must produce a valid result")
}

#[test]
fn concentrated_returns_fire_top_trade_warning() {
    // Golden scenario: three round trips of very different sizes.
    let result = common::golden_result();
    let warning = top_trade_concentration_warning(&result)
        .expect("top-3 share of 1.0 must fire the concentration warning");
    assert_eq!(warning.code, "return_concentration");
    assert_eq!(warning.severity, result_model::WarningSeverity::Warning);
    let details = warning.details.as_ref().expect("warning carries details");
    assert_eq!(details["top_3_share"], json!(1.0));
}

#[test]
fn diversified_returns_do_not_warn() {
    // Six equal round trips: top-3 share == 0.5, at the threshold -> no warn.
    let mut fills = Vec::new();
    let instruments = [
        "069500.KRX",
        "114260.KRX",
        "229200.KRX",
        "102110.KRX",
        "143850.KRX",
        "133690.KRX",
    ];
    // All buys first (chronological), then all sells (chronological).
    for (i, instrument) in instruments.iter().enumerate() {
        fills.push(fill(&format!("b{i}"), &format!("2020-01-{:02}", i * 2 + 2), instrument, OrderSide::Buy, 100, "10000.0000"));
    }
    for (i, instrument) in instruments.iter().enumerate() {
        fills.push(fill(&format!("s{i}"), &format!("2020-02-{:02}", i + 2), instrument, OrderSide::Sell, 100, "10500.0000"));
    }
    let result = result_from_fills(fills);
    assert!(
        top_trade_concentration_warning(&result).is_none(),
        "top-3 share of 0.5 must NOT warn (threshold is strict >)"
    );
}

#[test]
fn single_year_dominance_fires_year_warning() {
    let result = common::golden_result();
    let warning = year_concentration_warning(&result)
        .expect("a single-year contribution of 1.0 must warn");
    assert_eq!(warning.code, "year_concentration");
    assert_eq!(warning.severity, result_model::WarningSeverity::Warning);
    let details = warning.details.as_ref().unwrap();
    assert_eq!(details["max_year_share"], json!(1.0));
}

#[test]
fn spread_years_do_not_warn() {
    // Three equal-magnitude round trips, one per year (2019/2020/2021):
    // each year contributes 1/3 < 0.4 -> no warning.
    let mut fills = Vec::new();
    for (i, (year, instrument)) in [
        ("2019", "069500.KRX"),
        ("2020", "114260.KRX"),
        ("2021", "229200.KRX"),
    ]
    .iter()
    .enumerate()
    {
        fills.push(fill(
            &format!("b{i}"),
            &format!("{year}-06-02"),
            instrument,
            OrderSide::Buy,
            100,
            "10000.0000",
        ));
        fills.push(fill(
            &format!("s{i}"),
            &format!("{year}-07-02"),
            instrument,
            OrderSide::Sell,
            100,
            "11000.0000",
        ));
    }
    let result = result_from_fills(fills);
    assert!(
        year_concentration_warning(&result).is_none(),
        "evenly spread years must NOT warn"
    );
}

#[test]
fn recent_underperformance_warns() {
    let result = common::golden_result();
    let warning = recent_degradation_warning(&result, "069500.KRX")
        .expect("recent strategy return below benchmark must warn");
    assert_eq!(warning.code, "recent_degradation");
    assert_eq!(warning.severity, result_model::WarningSeverity::Warning);
    let details = warning.details.as_ref().unwrap();
    assert_eq!(details["benchmark_id"], json!("069500.KRX"));
    assert!(details["recent_excess"].as_f64().unwrap() < 0.0);
}

#[test]
fn recent_outperformance_does_not_warn() {
    let mut result = common::golden_result();
    // Benchmark crashing harder than the strategy -> no degradation warning.
    result.benchmark = vec![
        result_model::backtest::BenchmarkPoint {
            ts: result.equity.first().unwrap().ts,
            value: Money::parse("10000000.0000", Currency::KRW).unwrap(),
        },
        result_model::backtest::BenchmarkPoint {
            ts: result.equity.last().unwrap().ts,
            value: Money::parse("8000000.0000", Currency::KRW).unwrap(),
        },
    ];
    assert!(
        recent_degradation_warning(&result, "069500.KRX").is_none(),
        "strategy outperforming the benchmark must NOT warn"
    );
}

#[test]
fn neighborhood_sudden_change_warns_and_stats_are_reported() {
    // Returns climb then collapse: the 0.04 -> -0.20 jump exceeds 10%.
    let analysis: NeighborhoodAnalysis = analyze_neighborhood(vec![
        (json!({"fast_ma": 10}), stat(0.05)),
        (json!({"fast_ma": 20}), stat(0.04)),
        (json!({"fast_ma": 30}), stat(-0.20)),
    ])
    .expect("analysis over a non-empty neighborhood succeeds");
    let warning = analysis
        .sudden_change
        .expect("a 24-point jump must fire the sudden-change warning");
    assert_eq!(warning.code, "performance_sudden_change");
    assert!(analysis.dispersion.value() > 0.0);
    assert!(analysis.max_deviation_from_mean.value() > 0.0);
}

#[test]
fn neighborhood_without_jumps_does_not_warn() {
    let analysis = analyze_neighborhood(vec![
        (json!({"fast_ma": 10}), stat(0.05)),
        (json!({"fast_ma": 20}), stat(0.04)),
        (json!({"fast_ma": 30}), stat(0.045)),
    ])
    .expect("analysis over a non-empty neighborhood succeeds");
    assert!(
        analysis.sudden_change.is_none(),
        "gentle neighborhood must not warn"
    );
    let mean = analysis.mean_return.value();
    assert!((mean - 0.045).abs() < 1e-9, "mean of 0.05/0.04/0.045 is 0.045");
}

#[test]
fn empty_neighborhood_is_a_typed_error() {
    let error =
        analyze_neighborhood(Vec::<(serde_json::Value, ReportedStat)>::new())
            .expect_err("an empty neighborhood must be a typed error");
    assert!(matches!(error, RobustnessError::EmptySeries { .. }));
}
