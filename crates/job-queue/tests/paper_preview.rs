use std::collections::BTreeMap;

use domain::{
    ContentHash, Currency, InstrumentId, Money, Price, Quantity, TradingDate, UtcTimestamp, Weight,
};
use job_queue::paper_preview::{
    PaperPreviewError, PreviewCalculationInput, PreviewLineage, calculate_preview,
    load_recommendation_closes,
};
use market_data::CurateStore;
use market_data::curate::schema::{CuratedBar, write_bars};
use portfolio_model::CostProfile;
use portfolio_model::sizing::TargetAllocation;
use uuid::Uuid;

fn instrument(value: &str) -> InstrumentId {
    InstrumentId::parse(value).unwrap()
}

fn target(value: &str, weight: &str) -> TargetAllocation {
    TargetAllocation {
        instrument_id: instrument(value),
        weight: Weight::parse(weight).unwrap(),
    }
}

fn lineage() -> PreviewLineage {
    PreviewLineage {
        account_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        recommendation_run_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
        target_portfolio_id: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
        strategy_config_id: Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap(),
        dataset_version_id: Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap(),
        dataset_manifest_sha256: "a".repeat(64),
        account_state_version: 7,
        account_state_sha256: "b".repeat(64),
        target_portfolio_sha256: "c".repeat(64),
    }
}

fn calculation_input() -> PreviewCalculationInput {
    PreviewCalculationInput {
        cash: Money::parse("1000000", Currency::KRW).unwrap(),
        positions: BTreeMap::from([(instrument("069500.KRX"), Quantity::parse("100").unwrap())]),
        close_prices: BTreeMap::from([
            (instrument("069500.KRX"), Price::parse("10000").unwrap()),
            (instrument("229200.KRX"), Price::parse("10000").unwrap()),
        ]),
        targets: vec![
            target("069500.KRX", "0.250000"),
            target("229200.KRX", "0.750000"),
        ],
        lot_sizes: BTreeMap::new(),
        profile: CostProfile::krx_etf_default().unwrap(),
        price_date: TradingDate::parse("2026-05-08").unwrap(),
        proposed_effective_date: TradingDate::parse("2026-05-12").unwrap(),
        lineage: lineage(),
    }
}

#[test]
fn calculation_is_sell_first_explainable_and_deterministic() {
    let input = calculation_input();
    let (result, token) = calculate_preview(input.clone()).unwrap();

    assert_eq!(result.schema_version, 1);
    assert_eq!(result.price_basis, "RECOMMENDATION_CLOSE");
    assert_eq!(result.price_date, "2026-05-08");
    assert_eq!(result.proposed_effective_date, "2026-05-12");
    assert_eq!(result.equity, "2000000.0000");
    assert_eq!(result.cash_before, "1000000.0000");
    assert_eq!(result.warning_code, "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED");
    assert_eq!(result.orders.len(), 2);
    assert_eq!(result.orders[0].instrument_id, "069500.KRX");
    assert_eq!(result.orders[0].side, "SELL");
    assert_eq!(result.orders[1].instrument_id, "229200.KRX");
    assert_eq!(result.orders[1].side, "BUY");
    assert!(result.explicit_fees.parse::<f64>().unwrap() > 0.0);
    assert!(result.informational_slippage.parse::<f64>().unwrap() > 0.0);
    assert!(result.leftover_cash.parse::<f64>().unwrap() >= 0.0);
    assert_eq!(token.len(), 64);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );

    let (same_result, same_token) = calculate_preview(input).unwrap();
    assert_eq!(same_result, result);
    assert_eq!(same_token, token);
}

#[test]
fn calculation_is_canonical_when_target_input_order_changes() {
    let input = calculation_input();
    let (expected, expected_token) = calculate_preview(input.clone()).unwrap();
    let mut reordered = input;
    reordered.targets.reverse();

    let (actual, actual_token) = calculate_preview(reordered).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual_token, expected_token);
}

#[test]
fn calculation_all_cash_sells_every_position_and_never_buys() {
    let mut input = calculation_input();
    input.targets.clear();

    let (result, _) = calculate_preview(input).unwrap();
    assert_eq!(result.orders.len(), 1);
    assert_eq!(result.orders[0].side, "SELL");
    assert_eq!(result.orders[0].quantity, "100");
    assert_eq!(result.buy_notional, "0.0000");
    assert!(result.sell_notional.parse::<f64>().unwrap() > 0.0);
}

#[test]
fn calculation_fails_closed_when_a_held_instrument_has_no_close() {
    let mut input = calculation_input();
    input.close_prices.remove(&instrument("069500.KRX"));

    let error = calculate_preview(input).unwrap_err();
    assert!(matches!(
        error,
        PaperPreviewError::MissingPrice { instrument_id }
            if instrument_id == "069500.KRX"
    ));
}

#[test]
fn calculation_explains_no_trade_and_minimum_trade_skips() {
    let mut no_trade = calculation_input();
    no_trade.cash = Money::zero(Currency::KRW);
    no_trade.targets = vec![target("069500.KRX", "1.000000")];
    let (no_trade_result, _) = calculate_preview(no_trade).unwrap();
    assert!(no_trade_result.orders.is_empty());
    assert_eq!(no_trade_result.decisions[0].action, "SKIP");
    assert_eq!(
        no_trade_result.decisions[0].skip_reason.as_deref(),
        Some("BELOW_REBALANCE_THRESHOLD")
    );

    let mut below_minimum = calculation_input();
    below_minimum.cash = Money::parse("50000", Currency::KRW).unwrap();
    below_minimum.positions.clear();
    below_minimum.targets = vec![target("229200.KRX", "1.000000")];
    let (minimum_result, _) = calculate_preview(below_minimum).unwrap();
    assert!(minimum_result.orders.is_empty());
    assert_eq!(minimum_result.decisions[0].action, "SKIP");
    assert_eq!(
        minimum_result.decisions[0].skip_reason.as_deref(),
        Some("BELOW_MIN_TRADE")
    );
}

#[test]
fn calculation_rejects_duplicate_target_identity() {
    let mut input = calculation_input();
    input.targets.push(target("069500.KRX", "0.100000"));
    assert!(matches!(
        calculate_preview(input),
        Err(PaperPreviewError::InvalidPayload(detail)) if detail.contains("duplicate target")
    ));
}

fn curated_bar(instrument_id: &str, date: &str, close: &str, close_at: &str) -> CuratedBar {
    let price = Price::parse(close).unwrap();
    CuratedBar {
        instrument_id: instrument(instrument_id),
        trading_date: TradingDate::parse(date).unwrap(),
        market_open_ts: UtcTimestamp::parse_rfc3339("2026-05-08T00:00:00Z").unwrap(),
        market_close_ts: UtcTimestamp::parse_rfc3339(close_at).unwrap(),
        open: price,
        high: price,
        low: price,
        close: price,
        volume: 1,
        trading_value: Some(1),
        currency: Currency::KRW,
        source: "qa".into(),
        ingested_at: UtcTimestamp::parse_rfc3339("2026-05-08T07:00:00Z").unwrap(),
        batch_id: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        raw_hash: ContentHash::from_bytes(b"paper-preview"),
    }
}

fn write_preview_bars(
    root: &std::path::Path,
    partition_instrument: &str,
    version: u32,
    rows: &[CuratedBar],
) {
    let store = CurateStore::new(root.join("curated"));
    let path = store.bars_path("kr", partition_instrument, 2026, version);
    write_bars(&path, rows).unwrap();
}

#[test]
fn close_loader_reads_exact_raw_close_from_the_attested_version() {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "069500.KRX",
            "2026-05-08",
            "12345.6700",
            "2026-05-08T06:30:00Z",
        )],
    );

    let closes = load_recommendation_closes(
        directory.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap();
    assert_eq!(
        closes[&instrument("069500.KRX")].as_decimal_string(),
        "12345.6700"
    );
}

#[test]
fn close_loader_fails_closed_for_missing_version_or_date() {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        6,
        &[curated_bar(
            "069500.KRX",
            "2026-05-07",
            "10000",
            "2026-05-07T06:30:00Z",
        )],
    );

    for (version, date) in [(7, "2026-05-07"), (6, "2026-05-08")] {
        let error = load_recommendation_closes(
            directory.path(),
            version,
            TradingDate::parse(date).unwrap(),
            &[instrument("069500.KRX")],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            PaperPreviewError::MissingPrice { instrument_id }
                if instrument_id == "069500.KRX"
        ));
    }
}

#[test]
fn close_loader_rejects_partition_identity_mismatch_and_malformed_parquet() {
    let wrong_identity = tempfile::tempdir().unwrap();
    write_preview_bars(
        wrong_identity.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "229200.KRX",
            "2026-05-08",
            "10000",
            "2026-05-08T06:30:00Z",
        )],
    );
    let mismatch = load_recommendation_closes(
        wrong_identity.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap_err();
    assert!(matches!(
        mismatch,
        PaperPreviewError::MalformedCuratedData(_)
    ));

    let malformed = tempfile::tempdir().unwrap();
    let store = CurateStore::new(malformed.path().join("curated"));
    let path = store.bars_path("kr", "069500.KRX", 2026, 7);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"not parquet").unwrap();
    let error = load_recommendation_closes(
        malformed.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap_err();
    assert!(matches!(error, PaperPreviewError::MalformedCuratedData(_)));
}

#[test]
fn close_loader_rejects_a_close_that_is_not_yet_available() {
    let directory = tempfile::tempdir().unwrap();
    write_preview_bars(
        directory.path(),
        "069500.KRX",
        7,
        &[curated_bar(
            "069500.KRX",
            "2026-05-08",
            "10000",
            "2099-05-08T06:30:00Z",
        )],
    );

    let error = load_recommendation_closes(
        directory.path(),
        7,
        TradingDate::parse("2026-05-08").unwrap(),
        &[instrument("069500.KRX")],
    )
    .unwrap_err();
    assert!(matches!(error, PaperPreviewError::PreviewUnavailable(_)));
}
