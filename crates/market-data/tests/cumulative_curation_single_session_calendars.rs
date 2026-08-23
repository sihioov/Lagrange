//! A cumulative generation must survive the calendar shape KIS actually
//! delivers.
//!
//! `chk-holiday` is normalized to the requested date, so every Raw batch
//! carries a calendar holding exactly its own session — the doc comment on
//! `curation_inputs_from_raw_entries` says so, and that is precisely why the
//! function merges sessions across batches. The existing multi-batch test
//! (`price_publication_evidence.rs`) hands both batches the *same*
//! multi-session calendar, so it never exercises that shape.
//!
//! It matters because a KIS reference document carries no `listed_at` for any
//! instrument, so the instrument master falls back to its calendar's first
//! session. With one session per batch that fallback is batch-local, every
//! batch yields a different master, and the cross-batch equality check fails
//! closed — permanently, for any generation spanning two or more dates.
use domain::{BatchId, DatasetId, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, RequestMetadata, ResponseKind};
use market_data::{
    BatchSpec, CurateRequest, CurateStore, MARKET_KR, RawEnvelope, RawStore, curate_generation,
    curation_inputs_from_raw_entries,
};
use serde_json::{Value, json};

/// One immutable Raw delivery whose calendar declares only `date`, mirroring a
/// KIS `chk-holiday` response normalized to the requested day.
fn single_session_entry(
    raw: &RawStore,
    date: TradingDate,
    retrieved_at: UtcTimestamp,
) -> market_data::ManifestEntry {
    let iso = date.to_iso();
    let calendar = json!({
        "calendar_id": "single-session-fixture",
        "schema_version": 1,
        "source": "synthetic",
        "timezone": "Asia/Seoul",
        "utc_offset": "+09:00",
        "session_times_local": { "open": "09:00:00", "close": "15:30:00" },
        "session_times_utc": { "open": "00:00:00", "close": "06:30:00" },
        "sessions": [{
            "date": iso,
            "open_utc": format!("{iso}T00:00:00Z"),
            "close_utc": format!("{iso}T06:30:00Z"),
        }],
        "holidays": [],
    });

    let mut bars: Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/kr-etf/2020-01-31/bars.json"
    ))
    .expect("fixture bars");
    bars["bars"] = Value::Array(
        bars["bars"]
            .as_array()
            .expect("bars array")
            .iter()
            .filter(|row| {
                row["date"] == Value::String(iso.clone())
                    && matches!(
                        row["instrument"].as_str(),
                        Some("069500.KRX" | "229200.KRX")
                    )
            })
            .cloned()
            .collect(),
    );
    assert!(
        !bars["bars"].as_array().expect("bars array").is_empty(),
        "fixture must contain bars for {iso}"
    );

    let batch_id = BatchId::generate();
    let request = RequestMetadata {
        endpoint: "krx.eod.bars.v1".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        mode: FetchMode::Synthetic,
    };
    let envelope = |kind: ResponseKind, name: &'static str, bytes: Vec<u8>| {
        RawEnvelope::new(batch_id, kind, name, bytes, retrieved_at, request.clone())
    };
    raw.store_batch(
        &BatchSpec {
            provider: "krx",
            market: MARKET_KR,
            date: &date,
            batch_id,
            entitlement_reference: Some("fixture://candidate-license"),
            mode: FetchMode::Synthetic,
        },
        &[
            envelope(
                ResponseKind::Bars,
                "bars.json",
                serde_json::to_vec(&bars).expect("bars fixture"),
            ),
            envelope(
                ResponseKind::Reference,
                "reference.json",
                include_bytes!("../../../tests/fixtures/kr-etf/contract/reference-response.json")
                    .to_vec(),
            ),
            envelope(
                ResponseKind::Calendar,
                "calendar.json",
                serde_json::to_vec(&calendar).expect("calendar fixture"),
            ),
            envelope(
                ResponseKind::CorporateActions,
                "corporate-actions.json",
                include_bytes!(
                    "../../../tests/fixtures/kr-etf/contract/corporate-actions-response.json"
                )
                .to_vec(),
            ),
        ],
    )
    .expect("immutable fixture Raw delivery")
}

#[test]
fn cumulative_curation_accepts_one_session_per_batch_calendars() {
    let root = tempfile::tempdir().expect("data root");
    let raw = RawStore::new(root.path());
    let first_date = TradingDate::parse("2020-01-30").expect("first date");
    let last_date = TradingDate::parse("2020-01-31").expect("last date");
    let first = single_session_entry(
        &raw,
        first_date,
        UtcTimestamp::parse_rfc3339("2020-01-30T07:00:00Z").expect("first instant"),
    );
    let last_at = UtcTimestamp::parse_rfc3339("2020-01-31T07:00:00Z").expect("last instant");
    let last = single_session_entry(&raw, last_date, last_at);
    let entries = vec![first, last];

    let (calendar, master) = curation_inputs_from_raw_entries(&raw, &entries)
        .expect("single-session calendars merge into one generation");

    // Sessions merge across batches ...
    let sessions = calendar.sessions().collect::<Vec<_>>();
    assert_eq!(sessions, vec![first_date, last_date]);
    // ... so the listing fallback is the generation's earliest session, not
    // whichever batch happened to be read first. Both batches must agree.
    for instrument in master.instruments() {
        assert_eq!(
            instrument.listed_at, first_date,
            "{} must fall back to the merged first session",
            instrument.instrument_id
        );
    }

    let dataset_id = DatasetId::parse("krx_eod_bars").expect("dataset id");
    let curated = CurateStore::new(root.path());
    let outcome = curate_generation(
        &raw,
        &entries,
        &calendar,
        &master,
        &curated,
        &CurateRequest {
            dataset_id: &dataset_id,
            market: MARKET_KR,
            source: "krx",
            now: last_at,
        },
    )
    .expect("one cumulative generation over two single-session batches");
    assert_eq!(outcome.manifest.source_batches.len(), 2);
    assert_eq!(outcome.first_session, first_date);
    assert_eq!(outcome.last_session, last_date);
}
