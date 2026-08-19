use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;

use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS_DAILY_RANGE, RequestMetadata, ResponseKind,
};
use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
use market_data::range_normalize::{
    ExpectedRangeSessions, RangeNormalizeError, normalize_kis_daily_range_batch,
};
use market_data::storage::{BatchSpec, ManifestEntry, RawStore};
use market_data::{PublicationBundle, curation_inputs_from_raw};
use serde_json::{Value, json};

const ENDPOINT: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const NOW: &str = "2026-08-19T00:00:00Z";

fn date(value: &str) -> TradingDate {
    TradingDate::parse(value).expect("valid fixture date")
}

fn digits(value: TradingDate) -> String {
    value.to_iso().replace('-', "")
}

fn body(symbol: &str, dates: &[TradingDate]) -> Vec<u8> {
    let rows = dates
        .iter()
        .map(|date| {
            json!({
                "stck_bsop_date": digits(*date),
                "stck_oprc": "99",
                "stck_hgpr": "101",
                "stck_lwpr": "98",
                "stck_clpr": "100",
                "acml_vol": "100",
                "acml_tr_pbmn": "10000"
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "rt_cd": "0",
        "output1": {"current_price_that_must_not_be_used": "999"},
        "output2": rows,
        "symbol_that_must_not_be_used": symbol
    }))
    .expect("fixture JSON")
}

fn query(symbol: &str, start: TradingDate, end: TradingDate) -> RequestMetadata {
    RequestMetadata {
        endpoint: ENDPOINT.to_owned(),
        query: vec![
            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
            ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
            ("FID_INPUT_DATE_1".to_owned(), digits(start)),
            ("FID_INPUT_DATE_2".to_owned(), digits(end)),
            ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
            ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
        ],
        headers: vec![("tr_cont".to_owned(), String::new())],
        mode: FetchMode::Credentialed,
    }
}

fn fixture_source(
    root: &std::path::Path,
    start: TradingDate,
    end: TradingDate,
    rows_by_symbol: &BTreeMap<&str, Vec<TradingDate>>,
) -> (RawStore, ManifestEntry) {
    let store = RawStore::new(root);
    let batch_id = BatchId::generate();
    let now = UtcTimestamp::parse_rfc3339(NOW).expect("timestamp");
    let envelopes = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            let bytes = body(symbol, rows_by_symbol.get(symbol).expect("symbol rows"));
            market_data::RawEnvelope::new(
                batch_id,
                ResponseKind::Bars,
                format!("daily-bars-range-window-1-{symbol}-page-01.json"),
                bytes,
                now,
                query(symbol, start, end),
            )
        })
        .collect::<Vec<_>>();
    let spec = BatchSpec {
        provider: PROVIDER_KIS_DAILY_RANGE,
        market: MARKET_KR,
        date: &start,
        batch_id,
        entitlement_reference: Some("fixture-entitlement"),
        mode: FetchMode::Credentialed,
    };
    let entry = store
        .store_batch(&spec, &envelopes)
        .expect("store source range");
    (store, entry)
}

fn rows_for(start: TradingDate, end: TradingDate) -> BTreeMap<&'static str, Vec<TradingDate>> {
    KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| (*symbol, vec![end, start]))
        .map(|(symbol, values)| {
            if start == end {
                (symbol, vec![start])
            } else {
                (symbol, values)
            }
        })
        .collect()
}

fn fixture_multi_window(
    root: &std::path::Path,
    start: TradingDate,
    end: TradingDate,
    duplicate: bool,
    second_end: TradingDate,
) -> (RawStore, ManifestEntry) {
    let store = RawStore::new(root);
    let batch_id = BatchId::generate();
    let now = UtcTimestamp::parse_rfc3339(NOW).expect("timestamp");
    let mut envelopes = Vec::new();
    for symbol in KR_ETF_CORE_SYMBOLS {
        let first_rows = if duplicate {
            vec![end, start]
        } else {
            vec![end]
        };
        let second_rows = vec![start];
        for (window, (window_end, rows)) in [(1, (end, first_rows)), (2, (second_end, second_rows))]
        {
            let bytes = body(symbol, &rows);
            envelopes.push(market_data::RawEnvelope::new(
                batch_id,
                ResponseKind::Bars,
                format!("daily-bars-range-window-{window}-{symbol}-page-01.json"),
                bytes,
                now,
                query(symbol, start, window_end),
            ));
        }
    }
    let spec = BatchSpec {
        provider: PROVIDER_KIS_DAILY_RANGE,
        market: MARKET_KR,
        date: &start,
        batch_id,
        entitlement_reference: Some("fixture-entitlement"),
        mode: FetchMode::Credentialed,
    };
    let entry = store
        .store_batch(&spec, &envelopes)
        .expect("store source range");
    (store, entry)
}

fn expected(start: TradingDate, end: TradingDate) -> ExpectedRangeSessions {
    ExpectedRangeSessions::approved_xkrx(start, end).expect("approved XKRX fixture range")
}

#[test]
fn normalizes_exact_fixed_universe_into_one_batch_per_approved_session() {
    let start = date("2020-01-31");
    let end = date("2020-02-03");
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, source) = fixture_source(temp.path(), start, end, &rows_for(start, end));
    let outputs = normalize_kis_daily_range_batch(&store, &source, &expected(start, end))
        .expect("normalize range");
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].entry.provider, "kis-daily-range-normalized");
    assert_eq!(outputs[0].entry.files.len(), 1);
    assert_eq!(outputs[0].lineage.source_rows.len(), 11);
    assert_eq!(outputs[0].lineage.listing_snapshot_id, "kr-etf-core-v1");
    assert!(!outputs[0].lineage.availability_evidence);
    assert!(!outputs[0].lineage.revision_evidence);
    assert!(!outputs[0].lineage.knowledge_time_evidence);
    let document: Value = serde_json::from_slice(&outputs[0].files[0].bytes).expect("document");
    assert_eq!(document["schema_version"], 1);
    assert_eq!(document["dataset_kind"], "kis-daily-range-bars");
    assert_eq!(document["bars"].as_array().expect("bars").len(), 11);
    assert_eq!(document["pit"]["strict"], false);
    assert_eq!(document["acquired_at"], NOW);
    assert!(document.get("available_at").is_none());
    assert!(document["_lineage"]["source_files"].is_array());
    assert!(document["_lineage"]["source_rows"].is_array());
    assert!(!document["bars"][0].get("adjusted_close").is_some());
}

#[test]
fn replay_is_idempotent_and_publication_and_curation_reject_intermediate_scope() {
    let start = date("2020-01-31");
    let end = start;
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, source) = fixture_source(temp.path(), start, end, &rows_for(start, end));
    let expected = expected(start, end);
    let first =
        normalize_kis_daily_range_batch(&store, &source, &expected).expect("first normalize");
    let second =
        normalize_kis_daily_range_batch(&store, &source, &expected).expect("replay normalize");
    assert_eq!(first, second);
    let manifest = store
        .read_manifest("kis-daily-range-normalized", MARKET_KR)
        .expect("normalized manifest");
    assert_eq!(manifest.len(), 1);
    let publication = PublicationBundle::from_raw(&store, &first[0].entry)
        .expect_err("intermediate scope must not publish");
    assert!(
        publication
            .to_string()
            .contains("kis-daily-range-normalized")
    );
    let curate = curation_inputs_from_raw(&store, &first[0].entry)
        .expect_err("intermediate scope must not curate");
    assert!(curate.to_string().contains("kis-daily-range-normalized"));
}

#[test]
fn concurrent_normalization_converges_to_one_exact_batch() {
    let start = date("2020-01-31");
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, source) = fixture_source(temp.path(), start, start, &rows_for(start, start));
    let expected = expected(start, start);
    let store = Arc::new(store);
    let source = Arc::new(source);
    let expected = Arc::new(expected);
    let mut handles = Vec::new();
    for _ in 0..4 {
        let store = Arc::clone(&store);
        let source = Arc::clone(&source);
        let expected = Arc::clone(&expected);
        handles.push(thread::spawn(move || {
            normalize_kis_daily_range_batch(&store, &source, &expected).expect("normalize")
        }));
    }
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect::<Vec<_>>();
    assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(
        store
            .read_manifest("kis-daily-range-normalized", MARKET_KR)
            .expect("manifest")
            .len(),
        1
    );
}

#[test]
fn missing_symbol_is_permanent_coverage_failure() {
    let start = date("2020-01-31");
    let temp = tempfile::tempdir().expect("tempdir");
    let mut rows = rows_for(start, start);
    rows.get_mut("069500").expect("symbol").clear();
    let (store, source) = fixture_source(temp.path(), start, start, &rows);
    let error = normalize_kis_daily_range_batch(&store, &source, &expected(start, start))
        .expect_err("missing symbol must fail closed");
    assert!(matches!(error, RangeNormalizeError::Malformed { .. }));
}

#[test]
fn weekend_row_is_rejected_by_validated_session_list() {
    let start = date("2020-01-31");
    let end = date("2020-02-03");
    let temp = tempfile::tempdir().expect("tempdir");
    let mut rows = rows_for(start, end);
    for values in rows.values_mut() {
        *values = vec![date("2020-02-01"), start];
    }
    let (store, source) = fixture_source(temp.path(), start, end, &rows);
    let error = normalize_kis_daily_range_batch(&store, &source, &expected(start, end))
        .expect_err("non-session row must fail closed");
    assert!(matches!(error, RangeNormalizeError::OutOfSession { .. }));
}

#[test]
fn adjusted_price_query_is_rejected_before_normalization() {
    let start = date("2020-01-31");
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, mut source) = fixture_source(temp.path(), start, start, &rows_for(start, start));
    source.files[0]
        .request
        .query
        .iter_mut()
        .find(|(key, _)| key == "FID_ORG_ADJ_PRC")
        .expect("adjust query")
        .1 = "0".to_owned();
    let error = normalize_kis_daily_range_batch(&store, &source, &expected(start, start))
        .expect_err("adjusted request must fail closed");
    assert!(matches!(error, RangeNormalizeError::InvalidQuery { .. }));
}

#[test]
fn multi_window_union_preserves_each_source_window_and_row_lineage() {
    let start = date("2020-01-31");
    let end = date("2020-02-03");
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, source) = fixture_multi_window(temp.path(), start, end, false, date("2020-02-02"));
    let outputs = normalize_kis_daily_range_batch(&store, &source, &expected(start, end))
        .expect("multi-window normalize");
    assert_eq!(outputs.len(), 2);
    for output in outputs {
        assert_eq!(output.lineage.source_rows.len(), 11);
        assert!(
            output
                .lineage
                .source_rows
                .iter()
                .all(|row| row.row_date == output.session_date)
        );
        assert!(
            output
                .lineage
                .source_rows
                .iter()
                .all(|row| row.source_file_name.contains("window-1")
                    || row.source_file_name.contains("window-2"))
        );
    }
}

#[test]
fn overlapping_multi_window_windows_fail_closed() {
    let start = date("2020-01-31");
    let end = date("2020-02-03");
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, source) = fixture_multi_window(temp.path(), start, end, true, date("2020-02-02"));
    let error = normalize_kis_daily_range_batch(&store, &source, &expected(start, end))
        .expect_err("overlap must fail closed");
    assert!(matches!(error, RangeNormalizeError::DuplicateRow { .. }));
}

#[test]
fn non_contiguous_multi_window_bounds_fail_closed() {
    let start = date("2020-01-31");
    let end = date("2020-02-03");
    let temp = tempfile::tempdir().expect("tempdir");
    // The first response's oldest row is 2020-02-03, so the next request
    // must end on 2020-02-02.  Deliberately use 2020-02-01 to model a gap.
    let (store, source) = fixture_multi_window(temp.path(), start, end, false, date("2020-02-01"));
    let error = normalize_kis_daily_range_batch(&store, &source, &expected(start, end))
        .expect_err("non-contiguous windows must fail closed");
    assert!(matches!(error, RangeNormalizeError::InvalidQuery { .. }));
}

#[test]
fn custom_calendar_identity_is_rejected_even_with_approved_sessions() {
    let start = date("2020-01-31");
    let end = date("2020-01-31");
    let approved = expected(start, end);
    let error = ExpectedRangeSessions::new(
        "operator-calendar",
        approved.calendar_hash.clone(),
        start,
        end,
        approved.sessions.clone(),
    )
    .expect_err("custom calendar identity must not bypass embedded approval");
    assert!(matches!(
        error,
        RangeNormalizeError::InvalidExpectedSessions { .. }
    ));
}

#[test]
fn custom_calendar_hash_and_weekend_session_are_rejected() {
    let start = date("2020-01-31");
    let end = date("2020-02-03");
    let approved = expected(start, end);
    let bad_hash = ExpectedRangeSessions::new(
        approved.calendar_id.clone(),
        domain::ContentHash::from_bytes(b"untrusted-calendar"),
        start,
        end,
        approved.sessions.clone(),
    )
    .expect_err("custom calendar hash must not bypass embedded approval");
    assert!(matches!(
        bad_hash,
        RangeNormalizeError::InvalidExpectedSessions { .. }
    ));

    let mut sessions = approved.sessions;
    sessions.insert(1, date("2020-02-01"));
    let bad_session = ExpectedRangeSessions::new(
        approved.calendar_id,
        approved.calendar_hash,
        start,
        end,
        sessions,
    )
    .expect_err("weekend session must not bypass embedded approval");
    assert!(matches!(
        bad_session,
        RangeNormalizeError::InvalidExpectedSessions { .. }
    ));
}

#[test]
fn calendar_loader_rejects_outside_approved_effective_range() {
    let error = ExpectedRangeSessions::approved_xkrx(date("2019-12-31"), date("2020-01-31"))
        .expect_err("pre-effective range must fail closed");
    assert!(matches!(
        error,
        RangeNormalizeError::CalendarRangeOutOfBounds { .. }
    ));
}
