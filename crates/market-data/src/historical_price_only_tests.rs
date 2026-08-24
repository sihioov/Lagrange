use super::*;

use crate::contract::{FetchMode, RequestMetadata, ResponseKind};
use domain::{BatchId, ContentHash, FixedPoint, InstrumentId, TradingDate, UtcTimestamp, Venue};
use uuid::Uuid;

fn date(value: &str) -> TradingDate {
    TradingDate::parse(value).expect("valid test date")
}

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(value).expect("valid test timestamp")
}

fn batch(value: u128) -> BatchId {
    BatchId::from_uuid(Uuid::from_u128(value))
}

fn hash(value: &str) -> ContentHash {
    ContentHash::from_bytes(value.as_bytes())
}

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::from_parts(symbol, Venue::Krx).expect("valid ETF instrument")
}

fn source_file(symbol: &str, window: usize) -> FileEntry {
    let name = format!("daily-bars-range-window-{window}-{symbol}-page-01.json");
    FileEntry {
        kind: ResponseKind::Bars,
        file_name: name.clone(),
        content_hash: hash(&name),
        size_bytes: 1,
        request: RequestMetadata {
            endpoint: "kis.test/stage5".to_owned(),
            query: vec![("FID_INPUT_ISCD".to_owned(), symbol.to_owned())],
            headers: Vec::new(),
            mode: FetchMode::Credentialed,
        },
    }
}

fn session_provenance(
    session_date: TradingDate,
    ordinal: u128,
) -> HistoricalPriceOnlySessionProvenance {
    HistoricalPriceOnlySessionProvenance {
        session_date,
        normalized_batch_id: batch(ordinal),
        normalized_entry_hash: hash(&format!("entry-{ordinal}")),
        normalized_bars_hash: hash(&format!("bars-{ordinal}")),
        acquired_at: timestamp("2026-08-19T00:00:00Z"),
    }
}

fn raw_bars(dates: &[TradingDate]) -> Vec<RangeCanonicalBarCandidate> {
    let mut bars = Vec::with_capacity(dates.len() * KR_ETF_CORE_SYMBOLS.len());
    for (date_index, session_date) in dates.iter().copied().enumerate() {
        for (symbol_index, symbol) in KR_ETF_CORE_SYMBOLS.iter().enumerate() {
            let base = 100 + (date_index as i128 * 10) + symbol_index as i128;
            bars.push(RangeCanonicalBarCandidate {
                instrument_id: instrument(symbol),
                session_date,
                open: FixedPoint::from_i128(base * 100, 2).expect("open"),
                high: FixedPoint::from_i128((base + 2) * 100, 2).expect("high"),
                low: FixedPoint::from_i128((base - 2) * 100, 2).expect("low"),
                close: FixedPoint::from_i128((base + 1) * 100, 2).expect("close"),
                volume: 42 + symbol_index as u64,
                trading_value: Some(FixedPoint::parse("1234.56789").expect("value")),
            });
        }
        bars[date_index * KR_ETF_CORE_SYMBOLS.len()..(date_index + 1) * KR_ETF_CORE_SYMBOLS.len()]
            .sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
    }
    bars
}

fn parts(dates: &[&str], actions: Vec<RangeAction>) -> MaterializationParts {
    let dates = dates.iter().map(|value| date(value)).collect::<Vec<_>>();
    MaterializationParts {
        range_start: *dates.first().expect("at least one date"),
        range_end: *dates.last().expect("at least one date"),
        source_batch_id: batch(1),
        source_manifest_hash: hash("stage5-manifest"),
        source_files: vec![source_file("069500", 1)],
        action_batch_id: batch(2),
        action_manifest_hash: hash("action-manifest"),
        action_file_count: REQUIRED_ACTION_KINDS.len(),
        ignored_cash_dividends: HistoricalPriceOnlyIgnoredCashDividendEvidence::new(
            1,
            hash("ignored-cash-dividend-rows"),
            hash("dividend-source-file"),
            timestamp("2026-08-19T00:00:00Z"),
        ),
        sessions: dates
            .iter()
            .copied()
            .enumerate()
            .map(|(index, session_date)| session_provenance(session_date, 10 + index as u128))
            .collect(),
        bars: raw_bars(&dates),
        actions,
    }
}

fn bonus(symbol: &str, record_date: &str, ex_date: &str, factor: &str) -> RangeAction {
    RangeAction::BonusIssue {
        instrument_id: instrument(symbol),
        record_date: date(record_date),
        ex_date: date(ex_date),
        split_factor: FixedPoint::parse(factor).expect("factor"),
        available_at: timestamp("2026-08-19T00:00:00Z"),
    }
}

fn row<'a>(
    candidate: &'a HistoricalPriceOnlyCandidate,
    symbol: &str,
    session: &str,
) -> &'a HistoricalPriceOnlyBar {
    candidate
        .bars()
        .iter()
        .find(|bar| bar.instrument_id == instrument(symbol) && bar.session_date == date(session))
        .expect("candidate row")
}

#[test]
fn no_action_is_identity_and_metadata_is_fixed() {
    let candidate = materialize_parts(parts(&["2020-01-01", "2020-01-02"], Vec::new()))
        .expect("no-action candidate");

    assert_eq!(candidate.row_count(), 22);
    assert_eq!(candidate.session_count(), 2);
    assert_eq!(candidate.source_file_count(), 1);
    assert_eq!(candidate.action_file_count(), 7);
    assert!(candidate.bonus_evidence().is_empty());
    for bar in candidate.bars() {
        assert_eq!(bar.raw_open, bar.adjusted_open);
        assert_eq!(bar.raw_high, bar.adjusted_high);
        assert_eq!(bar.raw_low, bar.adjusted_low);
        assert_eq!(bar.raw_close, bar.adjusted_close);
    }
    let metadata = candidate.metadata();
    assert!(metadata.vendor_snapshot);
    assert!(!metadata.strict_pit);
    assert_eq!(metadata.capability, Capability::PriceReturnOnly);
    assert_eq!(metadata.audience, HistoricalPriceOnlyAudience::OwnerOnly);
    assert_eq!(metadata.audience.as_str(), "OWNER_ONLY");
    assert!(!metadata.materialized);
    assert!(metadata.in_memory);
    assert!(!metadata.ready);
}

#[test]
fn one_bonus_adjusts_only_strictly_pre_ex_date_bars() {
    let candidate = materialize_parts(parts(
        &["2020-01-01", "2020-01-02", "2020-01-03"],
        vec![bonus("069500", "2020-01-01", "2020-01-02", "2")],
    ))
    .expect("bonus candidate");

    assert_eq!(
        row(&candidate, "069500", "2020-01-01").adjusted_close,
        FixedPoint::parse("50.50").unwrap()
    );
    assert_eq!(
        row(&candidate, "069500", "2020-01-02").adjusted_close,
        FixedPoint::parse("111.00").unwrap()
    );
    assert_eq!(
        row(&candidate, "069500", "2020-01-03").adjusted_close,
        FixedPoint::parse("121.00").unwrap()
    );
    assert_eq!(candidate.bonus_evidence().len(), 1);
    assert_eq!(
        candidate.bonus_evidence()[0].acquired_at,
        timestamp("2026-08-19T00:00:00Z")
    );
}

#[test]
fn multiple_later_bonuses_compound_with_scale_eight_half_even_rounding() {
    let candidate = materialize_parts(parts(
        &["2020-01-01", "2020-01-02", "2020-01-03"],
        vec![
            bonus("069500", "2020-01-01", "2020-01-02", "2"),
            bonus("069500", "2020-01-02", "2020-01-03", "3"),
        ],
    ))
    .expect("compound bonus candidate");

    // Raw closes are 101.00, 111.00, and 121.00. The first row sees
    // round_even(0.5 * round_even(1/3, 8), 8) = 0.16666667.
    assert_eq!(
        row(&candidate, "069500", "2020-01-01").adjusted_close,
        FixedPoint::parse("16.8333").unwrap()
    );
    assert_eq!(
        row(&candidate, "069500", "2020-01-02").adjusted_close,
        FixedPoint::parse("37.0000").unwrap()
    );
    assert_eq!(
        row(&candidate, "069500", "2020-01-03").adjusted_close,
        FixedPoint::parse("121.0000").unwrap()
    );
}

#[test]
fn raw_ohlcv_and_trading_value_are_preserved_separately() {
    let candidate = materialize_parts(parts(
        &["2020-01-01", "2020-01-02"],
        vec![bonus("069500", "2020-01-01", "2020-01-02", "2")],
    ))
    .expect("candidate");
    let raw = candidate
        .bars()
        .iter()
        .find(|bar| bar.instrument_id == instrument("069500"))
        .expect("raw row");
    assert_eq!(raw.raw_open, FixedPoint::parse("100.00").unwrap());
    assert_eq!(raw.raw_high, FixedPoint::parse("102.00").unwrap());
    assert_eq!(raw.raw_low, FixedPoint::parse("98.00").unwrap());
    assert_eq!(raw.raw_close, FixedPoint::parse("101.00").unwrap());
    assert_eq!(raw.raw_volume, 42);
    assert_eq!(
        raw.raw_trading_value,
        Some(FixedPoint::parse("1234.56789").unwrap())
    );
    assert_eq!(raw.adjusted_open, FixedPoint::parse("50.00").unwrap());
}

#[test]
fn raw_price_is_multiplied_before_the_single_scale_four_round() {
    let mut input = parts(
        &["2020-01-01", "2020-01-02"],
        vec![bonus("069500", "2020-01-01", "2020-01-02", "2")],
    );
    let raw_row = input
        .bars
        .iter_mut()
        .find(|bar| {
            bar.instrument_id == instrument("069500") && bar.session_date == date("2020-01-01")
        })
        .expect("raw row");
    let raw_price = FixedPoint::parse("1.00011").unwrap();
    raw_row.open = raw_price;
    raw_row.high = raw_price;
    raw_row.low = raw_price;
    raw_row.close = raw_price;

    let candidate = materialize_parts(input).expect("candidate");
    let row = row(&candidate, "069500", "2020-01-01");
    assert_eq!(row.raw_close, FixedPoint::parse("1.00011").unwrap());
    assert_eq!(row.adjusted_close, FixedPoint::parse("0.5001").unwrap());
    // Rounding 1.00011 to 1.0001 first would produce 0.5000.
    assert_ne!(row.adjusted_close, FixedPoint::parse("0.5000").unwrap());
}

#[test]
fn ordering_and_hash_are_deterministic() {
    let first = materialize_parts(parts(
        &["2020-01-01", "2020-01-02"],
        vec![bonus("069500", "2020-01-01", "2020-01-02", "2")],
    ))
    .expect("first candidate");
    let second = materialize_parts(parts(
        &["2020-01-01", "2020-01-02"],
        vec![bonus("069500", "2020-01-01", "2020-01-02", "2")],
    ))
    .expect("second candidate");

    assert_eq!(first, second);
    assert_eq!(first.content_hash(), second.content_hash());
    assert!(first.bars().windows(2).all(|pair| {
        (&pair[0].instrument_id, pair[0].session_date)
            <= (&pair[1].instrument_id, pair[1].session_date)
    }));
}

#[test]
fn malformed_duplicate_unsupported_and_noncanonical_inputs_fail_closed() {
    let mut duplicate = parts(&["2020-01-01"], Vec::new());
    duplicate.bars[1] = duplicate.bars[0].clone();
    assert!(matches!(
        materialize_parts(duplicate),
        Err(HistoricalPriceOnlyError::DuplicateBar { .. })
    ));

    let mut unsupported = parts(&["2020-01-01"], Vec::new());
    unsupported.actions.push(RangeAction::Unsupported {
        kind: "dividend".to_owned(),
        reason: "test".to_owned(),
    });
    assert!(matches!(
        materialize_parts(unsupported),
        Err(HistoricalPriceOnlyError::UnsupportedAction { kind }) if kind == "dividend"
    ));

    let mut noncanonical = parts(&["2020-01-01"], Vec::new());
    noncanonical.bars.swap(0, 1);
    assert!(matches!(
        materialize_parts(noncanonical),
        Err(HistoricalPriceOnlyError::NonCanonicalOrdering { kind }) if kind == "bars"
    ));
}

#[test]
fn empty_source_files_and_mismatched_counts_or_coverage_fail_closed() {
    let mut empty_source = parts(&["2020-01-01"], Vec::new());
    empty_source.source_files.clear();
    assert!(matches!(
        materialize_parts(empty_source),
        Err(HistoricalPriceOnlyError::InputMismatch { .. })
    ));

    let mut wrong_action_count = parts(&["2020-01-01"], Vec::new());
    wrong_action_count.action_file_count -= 1;
    assert!(matches!(
        materialize_parts(wrong_action_count),
        Err(HistoricalPriceOnlyError::InputMismatch { .. })
    ));

    let mut missing_session = parts(&["2020-01-01", "2020-01-02"], Vec::new());
    missing_session.sessions.pop();
    assert!(matches!(
        materialize_parts(missing_session),
        Err(HistoricalPriceOnlyError::InputMismatch { .. })
    ));
}

#[test]
fn source_and_bonus_action_inputs_must_already_be_canonical() {
    let mut source_files = parts(&["2020-01-01"], Vec::new());
    source_files.source_files = vec![source_file("069500", 2), source_file("069500", 1)];
    assert!(matches!(
        materialize_parts(source_files),
        Err(HistoricalPriceOnlyError::NonCanonicalOrdering { kind }) if kind == "source files"
    ));

    let mut actions = parts(
        &["2020-01-01", "2020-01-02", "2020-01-03"],
        vec![
            bonus("069500", "2020-01-02", "2020-01-03", "2"),
            bonus("069500", "2020-01-01", "2020-01-02", "2"),
        ],
    );
    assert!(matches!(
        materialize_parts(actions),
        Err(HistoricalPriceOnlyError::NonCanonicalOrdering { kind }) if kind == "bonus actions"
    ));

    actions = parts(
        &["2020-01-01", "2020-01-02", "2020-01-03"],
        vec![
            bonus("069500", "2020-01-01", "2020-01-02", "2"),
            bonus("069500", "2020-01-02", "2020-01-03", "3"),
        ],
    );
    let candidate = materialize_parts(actions).expect("canonical actions");
    assert_eq!(candidate.bonus_evidence()[0].ex_date, date("2020-01-02"));
    assert_eq!(candidate.bonus_evidence()[1].ex_date, date("2020-01-03"));
}

#[test]
fn exact_187_source_files_use_numeric_producer_order_not_lexical_file_name_order() {
    let mut numeric = parts(&["2020-01-01"], Vec::new());
    numeric.source_files = KR_ETF_CORE_SYMBOLS
        .iter()
        .flat_map(|symbol| (1..=17).map(move |window| source_file(symbol, window)))
        .collect();
    assert_eq!(numeric.source_files.len(), 187);
    materialize_parts(numeric).expect("numeric window order is canonical");

    let mut lexical = parts(&["2020-01-01"], Vec::new());
    lexical.source_files = KR_ETF_CORE_SYMBOLS
        .iter()
        .flat_map(|symbol| (1..=17).map(move |window| source_file(symbol, window)))
        .collect();
    lexical
        .source_files
        .sort_by(|left, right| left.file_name.cmp(&right.file_name));
    assert!(matches!(
        materialize_parts(lexical),
        Err(HistoricalPriceOnlyError::NonCanonicalOrdering { kind }) if kind == "source files"
    ));
}

#[test]
fn source_file_names_reject_padded_and_out_of_beta_range_windows() {
    for invalid_window in ["01", "18"] {
        let mut input = parts(&["2020-01-01"], Vec::new());
        input.source_files[0].file_name =
            format!("daily-bars-range-window-{invalid_window}-069500-page-01.json");
        assert!(matches!(
            materialize_parts(input),
            Err(HistoricalPriceOnlyError::InputMismatch { .. })
        ));
    }
}

#[test]
fn bonus_action_identity_rejects_exact_and_conflicting_duplicates() {
    let exact = bonus("069500", "2020-01-01", "2020-01-02", "2");
    let duplicate = parts(&["2020-01-01", "2020-01-02"], vec![exact.clone(), exact]);
    assert!(matches!(
        materialize_parts(duplicate),
        Err(HistoricalPriceOnlyError::DuplicateAction { .. })
    ));

    let mut conflicting = bonus("069500", "2020-01-02", "2020-01-02", "3");
    if let RangeAction::BonusIssue { available_at, .. } = &mut conflicting {
        *available_at = timestamp("2026-08-20T00:00:00Z");
    }
    let conflicting = parts(
        &["2020-01-01", "2020-01-02"],
        vec![
            bonus("069500", "2020-01-01", "2020-01-02", "2"),
            conflicting,
        ],
    );
    assert!(matches!(
        materialize_parts(conflicting),
        Err(HistoricalPriceOnlyError::ConflictingAction { .. })
    ));
}

#[test]
fn owner_scope_marker_is_hashed_and_security_provenance_changes_hash() {
    let input = parts(&["2020-01-01"], Vec::new());
    let first = materialize_parts(input).expect("first candidate");
    assert_eq!(
        first.metadata().audience,
        HistoricalPriceOnlyAudience::OwnerOnly
    );
    assert_eq!(
        HistoricalPriceOnlyAudience::OwnerOnly.as_str(),
        "OWNER_ONLY"
    );

    let mut changed = parts(&["2020-01-01"], Vec::new());
    changed.source_manifest_hash = hash("different-stage5-manifest");
    let second = materialize_parts(changed).expect("second candidate");
    assert_ne!(first.content_hash(), second.content_hash());

    let representation_input = parts(&["2020-01-01"], Vec::new());
    let source_files = canonical_source_files(representation_input.source_files.clone()).unwrap();
    let actions = canonical_actions(
        &representation_input.actions,
        representation_input.range_start,
        representation_input.range_end,
    )
    .unwrap();
    let bars = materialize_bars(&representation_input.bars, &actions).unwrap();
    let evidence = actions.iter().map(bonus_evidence).collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&canonical_representation(
        &representation_input,
        &source_files,
        &evidence,
        &bars,
    ))
    .unwrap();
    assert!(
        bytes
            .windows("OWNER_ONLY".len())
            .any(|window| window == b"OWNER_ONLY")
    );
}

#[test]
fn ignored_cash_dividend_commitment_changes_identity_but_not_price_rows() {
    let first = materialize_parts(parts(&["2020-01-01"], Vec::new())).unwrap();
    let mut changed = parts(&["2020-01-01"], Vec::new());
    changed.ignored_cash_dividends = HistoricalPriceOnlyIgnoredCashDividendEvidence::new(
        2,
        hash("different-ignored-cash-dividend-rows"),
        hash("different-dividend-source-file"),
        timestamp("2026-08-20T00:00:00Z"),
    );
    let second = materialize_parts(changed).unwrap();
    assert_eq!(first.bars(), second.bars());
    assert_eq!(first.bonus_evidence(), second.bonus_evidence());
    assert_ne!(first.content_hash(), second.content_hash());
}

#[test]
fn invalid_factors_ohlc_and_overflow_fail_closed() {
    let mut invalid_factor = parts(&["2020-01-01"], Vec::new());
    invalid_factor
        .actions
        .push(bonus("069500", "2020-01-01", "2020-01-01", "0"));
    assert!(matches!(
        materialize_parts(invalid_factor),
        Err(HistoricalPriceOnlyError::InvalidSplitFactor { .. })
    ));

    let mut invalid_ohlc = parts(&["2020-01-01"], Vec::new());
    invalid_ohlc.bars[0].high = FixedPoint::parse("1").unwrap();
    assert!(matches!(
        materialize_parts(invalid_ohlc),
        Err(HistoricalPriceOnlyError::OhlcInvariant { stage: "raw", .. })
    ));

    let mut overflow = parts(&["2020-01-01"], Vec::new());
    let huge = FixedPoint::from_i128(i128::MAX, 0).unwrap();
    for bar in &mut overflow.bars {
        bar.open = huge;
        bar.high = huge;
        bar.low = huge;
        bar.close = huge;
    }
    assert!(matches!(
        materialize_parts(overflow),
        Err(HistoricalPriceOnlyError::ArithmeticOverflow { .. })
    ));
}
