use std::future::Future;
use std::sync::{Arc, Mutex};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use kis_client::{KisError, MarketDataReply};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS_CALENDAR, PROVIDER_KIS_CALENDAR_NORMALIZED, RawEnvelope,
    RequestMetadata, ResponseKind,
};
use market_data::ingest::{IngestRequest, ingest_kis_bundle, ingest_kis_calendar_with_batch_id};
use market_data::normalize::{
    KIS_CALENDAR_DOCUMENT_ID, KIS_CALENDAR_NORMALIZER, KIS_CALENDAR_SOURCE_VERSION,
    deterministic_kis_calendar_normalized_batch_id, normalize_kis_batch,
    normalize_kis_calendar_batch, normalize_kis_calendar_envelopes,
};
use market_data::providers::kis::{KisProvider, KisRead};
use market_data::publication::{CalendarPublicationBundle, PublicationBundle, PublicationError};
use market_data::storage::{BatchSpec, RawStore};
use serde_json::json;
use tempfile::TempDir;

const CALENDAR_PATH: &str = "/uapi/domestic-stock/v1/quotations/chk-holiday";
const CALENDAR_TR_ID: &str = "CTCA0903R";
const TARGET_DATE: &str = "2026-09-14";
const RETRIEVED_AT: &str = "2026-09-14T00:05:00Z";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedCall {
    path: String,
    tr_id: String,
    query: Vec<(String, String)>,
    continuation: Option<String>,
}

#[derive(Debug, Clone)]
struct FixtureReader {
    body: Arc<Mutex<Vec<u8>>>,
    response_continuation: Option<String>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl FixtureReader {
    fn new(body: Vec<u8>) -> Self {
        Self {
            body: Arc::new(Mutex::new(body)),
            response_continuation: None,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_response_continuation(body: Vec<u8>, marker: &str) -> Self {
        Self {
            body: Arc::new(Mutex::new(body)),
            response_continuation: Some(marker.to_owned()),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().expect("fixture calls lock").clone()
    }

    fn eod_body(path: &str, query: &[(String, String)]) -> Vec<u8> {
        let symbol = query
            .iter()
            .find(|(key, _)| key == "FID_INPUT_ISCD")
            .map(|(_, value)| value.as_str())
            .unwrap_or("069500");
        let value = if path.ends_with("inquire-daily-itemchartprice") {
            json!({
                "rt_cd": "0",
                "output1": {
                    "hts_kor_isnm": format!("ETF {symbol}"),
                    "stck_shrn_iscd": symbol
                },
                "output2": [{
                    "stck_bsop_date": "20260914",
                    "stck_oprc": "100.00",
                    "stck_hgpr": "102.00",
                    "stck_lwpr": "99.00",
                    "stck_clpr": "101.00",
                    "acml_vol": "1300",
                    "acml_tr_pbmn": "131300"
                }]
            })
        } else if path.ends_with("inquire-price") {
            json!({"rt_cd": "0", "output": {"stck_shrn_iscd": symbol}})
        } else {
            json!({"rt_cd": "0", "output1": []})
        };
        serde_json::to_vec(&value).expect("EOD fixture JSON")
    }
}

impl KisRead for FixtureReader {
    async fn get(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        self.calls
            .lock()
            .expect("fixture calls lock")
            .push(RecordedCall {
                path: path.to_owned(),
                tr_id: tr_id.to_owned(),
                query: query.to_vec(),
                continuation: continuation.map(str::to_owned),
            });
        let body = if path.ends_with("chk-holiday") {
            self.body.lock().expect("fixture body lock").clone()
        } else {
            Self::eod_body(path, query)
        };
        Ok(MarketDataReply {
            body,
            continuation: self.response_continuation.clone(),
        })
    }
}

fn target() -> TradingDate {
    TradingDate::parse(TARGET_DATE).expect("target date")
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime")
        .block_on(future)
}

fn request() -> IngestRequest {
    IngestRequest::new(
        MARKET_KR.to_owned(),
        target(),
        UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at"),
    )
}

fn calendar_body(rows: &str) -> Vec<u8> {
    format!(r#"{{"rt_cd":"0","output":[{rows}]}}"#).into_bytes()
}

fn valid_body() -> Vec<u8> {
    calendar_body(r#"{"bass_dt":"20260914","opnd_yn":"Y"}"#)
}

fn assert_one_exact_call(reader: &FixtureReader) {
    assert_eq!(
        reader.calls(),
        vec![RecordedCall {
            path: CALENDAR_PATH.to_owned(),
            tr_id: CALENDAR_TR_ID.to_owned(),
            query: vec![
                ("BASS_DT".to_owned(), "20260914".to_owned()),
                ("CTX_AREA_FK".to_owned(), String::new()),
                ("CTX_AREA_NK".to_owned(), String::new()),
            ],
            continuation: None,
        }]
    );
}

fn ingest_valid() -> (TempDir, RawStore, market_data::IngestOutcome, FixtureReader) {
    let temp = tempfile::tempdir().expect("raw root");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FixtureReader::new(valid_body());
    let provider = KisProvider::kr_etf_core(reader.clone());
    let batch_id = BatchId::generate();
    let outcome = block_on(ingest_kis_calendar_with_batch_id(
        &store,
        &provider,
        &request(),
        Some("entitlement://kis-calendar-v1"),
        batch_id,
    ))
    .expect("calendar capture");
    (temp, store, outcome, reader)
}

#[test]
fn bootstrap_calendar_is_reused_without_a_second_calendar_get_through_eod() {
    let (_temp, store, calendar_outcome, reader) = ingest_valid();
    let provider = KisProvider::kr_etf_core(reader.clone());
    let result = block_on(ingest_kis_bundle(
        &store,
        &provider,
        &request(),
        Some("entitlement://kis-calendar-v1"),
    ));
    let outcome = result.expect("EOD must reuse the committed calendar source");
    assert_eq!(outcome.entry.files.len(), 30);
    assert!(
        outcome
            .entry
            .files
            .iter()
            .find(|file| file.kind == ResponseKind::Calendar)
            .and_then(|file| file.copied_from.as_ref())
            .is_some_and(|reference| reference.provider == PROVIDER_KIS_CALENDAR)
    );
    let calls = reader.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.path == CALENDAR_PATH)
            .count(),
        1,
        "the dedicated calendar source must be the only chk-holiday request"
    );
    assert!(calls.len() > 1, "EOD must proceed to non-calendar classes");
    let standalone = normalize_kis_calendar_batch(&store, &calendar_outcome.entry)
        .expect("standalone calendar normalization");
    let normalized =
        normalize_kis_batch(&store, &outcome.entry).expect("complete EOD normalization");
    let standalone_bytes = store
        .read_batch_bytes(
            &standalone.entry.provider,
            &standalone.entry.market,
            &standalone.entry,
        )
        .expect("standalone bytes")
        .pop()
        .expect("standalone file")
        .bytes;
    let normalized_calendar = store
        .read_batch_bytes(
            &normalized.entry.provider,
            &normalized.entry.market,
            &normalized.entry,
        )
        .expect("normalized bytes")
        .into_iter()
        .find(|file| file.file_name == "calendar.json")
        .expect("normalized calendar")
        .bytes;
    assert_eq!(normalized_calendar, standalone_bytes);
    let bundle = PublicationBundle::from_raw(&store, &normalized.entry)
        .expect("complete normalized publication");
    assert_eq!(
        bundle
            .calendar_evidence
            .as_ref()
            .expect("calendar evidence")
            .source_batch_id,
        standalone.entry.batch_id
    );
    let later_request = IngestRequest::new(
        MARKET_KR.to_owned(),
        target(),
        UtcTimestamp::parse_rfc3339("2026-09-14T00:06:00Z").expect("later retrieval"),
    );
    let second = block_on(ingest_kis_bundle(
        &store,
        &KisProvider::kr_etf_core(reader.clone()),
        &later_request,
        Some("entitlement://kis-calendar-v1"),
    ))
    .expect("second EOD source reuses the same calendar");
    let second_normalized =
        normalize_kis_batch(&store, &second.entry).expect("second complete EOD normalization");
    let second_bundle = PublicationBundle::from_raw(&store, &second_normalized.entry)
        .expect("second complete normalized publication");
    assert_eq!(second_bundle.calendar_evidence, bundle.calendar_evidence);

    let temp = tempfile::tempdir().expect("raw root");
    let store = RawStore::new(temp.path());
    let claim = temp.path().join("raw/.calendar-bootstrap");
    std::fs::create_dir_all(&claim).unwrap();
    std::fs::write(claim.join("2026-09-14.json"), b"interrupted attempt").unwrap();
    let reader = FixtureReader::new(valid_body());
    let provider = KisProvider::kr_etf_core(reader.clone());
    assert!(
        block_on(ingest_kis_bundle(
            &store,
            &provider,
            &request(),
            Some("entitlement://kis-calendar-v1")
        ))
        .is_err()
    );
    assert!(reader.calls().is_empty());
}

#[tokio::test]
async fn calendar_ingest_is_one_exact_blank_continuation_request() {
    let temp = tempfile::tempdir().expect("raw root");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FixtureReader::new(valid_body());
    let provider = KisProvider::kr_etf_core(reader.clone());
    let batch_id = BatchId::generate();
    let outcome = ingest_kis_calendar_with_batch_id(
        &store,
        &provider,
        &request(),
        Some("entitlement://kis-calendar-v1"),
        batch_id,
    )
    .await
    .expect("calendar capture");

    assert_eq!(outcome.batch_id, batch_id);
    assert_eq!(outcome.entry.provider, PROVIDER_KIS_CALENDAR);
    assert_eq!(outcome.entry.market, MARKET_KR);
    assert_eq!(outcome.entry.files.len(), 1);
    assert_eq!(outcome.entry.files[0].kind, ResponseKind::Calendar);
    assert_eq!(outcome.entry.files[0].file_name, "calendar-page-01.json");
    assert_eq!(outcome.entry.files[0].request.endpoint, CALENDAR_PATH);
    assert_eq!(outcome.entry.files[0].request.mode, FetchMode::Credentialed);
    assert_eq!(
        outcome.entry.files[0].request.headers,
        vec![
            ("authorization".to_owned(), "[REDACTED]".to_owned()),
            ("appkey".to_owned(), "[REDACTED]".to_owned()),
            ("appsecret".to_owned(), "[REDACTED]".to_owned()),
            ("tr_id".to_owned(), CALENDAR_TR_ID.to_owned()),
            ("tr_cont".to_owned(), String::new()),
        ]
    );
    assert_eq!(outcome.entry.files[0].response_continuation, None);
    assert_one_exact_call(&reader);
    assert!(!store.manifest_path("kis", MARKET_KR).exists());
}

#[tokio::test]
async fn invalid_calendar_response_never_makes_raw_visible() {
    let bodies = [
        calendar_body(r#"{"bass_dt":"20260913","opnd_yn":"Y"}"#),
        calendar_body(
            r#"{"bass_dt":"20260914","opnd_yn":"Y"},{"bass_dt":"20260914","opnd_yn":"N"}"#,
        ),
        calendar_body(r#"{"bass_dt":"20260914","opnd_yn":"X"}"#),
        br#"{"rt_cd":"1","output":[]}"#.to_vec(),
        br#"not-json"#.to_vec(),
    ];

    for body in bodies {
        let temp = tempfile::tempdir().expect("raw root");
        let store = RawStore::new(temp.path().join("data"));
        let reader = FixtureReader::new(body);
        let provider = KisProvider::kr_etf_core(reader.clone());
        let result = ingest_kis_calendar_with_batch_id(
            &store,
            &provider,
            &request(),
            Some("entitlement://kis-calendar-v1"),
            BatchId::generate(),
        )
        .await;
        assert!(result.is_err());
        assert_one_exact_call(&reader);
        assert!(
            !store
                .provider_dir(PROVIDER_KIS_CALENDAR, MARKET_KR)
                .exists()
        );
        assert!(
            !store
                .manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR)
                .exists()
        );
    }
}

#[tokio::test]
async fn calendar_continuation_marker_is_rejected_without_raw_visibility() {
    let temp = tempfile::tempdir().expect("raw root");
    let store = RawStore::new(temp.path().join("data"));
    let reader = FixtureReader::with_response_continuation(valid_body(), "M");
    let provider = KisProvider::kr_etf_core(reader.clone());

    let result = ingest_kis_calendar_with_batch_id(
        &store,
        &provider,
        &request(),
        Some("entitlement://kis-calendar-v1"),
        BatchId::generate(),
    )
    .await;
    assert!(result.is_err());
    assert_one_exact_call(&reader);
    assert!(
        !store
            .manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR)
            .exists()
    );
}

#[test]
fn calendar_normalization_and_publication_bundle_are_deterministic_and_separate_from_eod() {
    let (_temp, store, source, reader) = ingest_valid();
    assert_one_exact_call(&reader);
    let first = normalize_kis_calendar_batch(&store, &source.entry).expect("first normalization");
    let second = normalize_kis_calendar_batch(&store, &source.entry).expect("replay normalization");

    assert_eq!(
        first.entry.batch_id,
        deterministic_kis_calendar_normalized_batch_id(source.batch_id)
    );
    assert_eq!(first.entry, second.entry);
    assert_eq!(first.files, second.files);
    assert_eq!(first.entry.provider, PROVIDER_KIS_CALENDAR_NORMALIZED);
    assert_eq!(first.entry.files.len(), 1);
    assert_eq!(first.entry.files[0].file_name, "calendar.json");
    assert_eq!(first.lineage.normalizer, KIS_CALENDAR_NORMALIZER);
    assert_eq!(first.lineage.schema_version, 1);

    let document: serde_json::Value =
        serde_json::from_slice(&first.files[0].bytes).expect("canonical calendar JSON");
    assert_eq!(document["calendar_id"], KIS_CALENDAR_DOCUMENT_ID);
    assert_eq!(document["schema_version"], 1);
    assert_eq!(document["source"], "kis");
    assert_eq!(document["sessions"].as_array().expect("sessions").len(), 1);
    assert_eq!(
        document["sessions"][0]["date"],
        serde_json::Value::String(TARGET_DATE.to_owned())
    );
    assert_eq!(
        document["_lineage"]["upstream_batch_id"],
        source.batch_id.to_string()
    );

    let bundle = CalendarPublicationBundle::from_raw(&store, &first.entry)
        .expect("validated calendar publication bundle");
    assert_eq!(bundle.source_batch_id(), first.entry.batch_id);
    assert_eq!(bundle.target_date(), target());
    assert_eq!(bundle.calendar_facts().len(), 1);
    assert_eq!(
        bundle.calendar_facts()[0].source_version,
        KIS_CALENDAR_SOURCE_VERSION
    );
    let eod_result = market_data::PublicationBundle::from_raw(&store, &first.entry);
    assert!(matches!(
        eod_result,
        Err(PublicationError::UnsupportedManifestScope { .. })
    ));
}

#[test]
fn calendar_bundle_rejects_corrupted_source_canonical_hash_and_lineage() {
    let (_temp, store, source, _reader) = ingest_valid();
    let normalized = normalize_kis_calendar_batch(&store, &source.entry).expect("normalize");

    let source_path = store
        .batch_dir(
            PROVIDER_KIS_CALENDAR,
            MARKET_KR,
            &source.entry.date,
            &source.entry.batch_id,
        )
        .join("calendar-page-01.json");
    std::fs::write(&source_path, b"corrupted source").expect("corrupt source fixture");
    assert!(CalendarPublicationBundle::from_raw(&store, &normalized.entry).is_err());

    let (_temp, store, source, _reader) = ingest_valid();
    let normalized = normalize_kis_calendar_batch(&store, &source.entry).expect("normalize");
    let canonical_path = store
        .batch_dir(
            PROVIDER_KIS_CALENDAR_NORMALIZED,
            MARKET_KR,
            &normalized.entry.date,
            &normalized.entry.batch_id,
        )
        .join("calendar.json");
    std::fs::write(&canonical_path, b"corrupted canonical").expect("corrupt canonical fixture");
    assert!(CalendarPublicationBundle::from_raw(&store, &normalized.entry).is_err());

    let (_temp, store, source, _reader) = ingest_valid();
    let normalized = normalize_kis_calendar_batch(&store, &source.entry).expect("normalize");
    let mut bad_hash = normalized.entry.clone();
    bad_hash.files[0].content_hash = ContentHash::from_bytes(b"different hash");
    assert!(CalendarPublicationBundle::from_raw(&store, &bad_hash).is_err());

    let (_temp, store, source, _reader) = ingest_valid();
    let source_stored = store
        .read_batch_bytes(PROVIDER_KIS_CALENDAR, MARKET_KR, &source.entry)
        .expect("source readback");
    let mut envelopes = normalize_kis_calendar_envelopes(&source.entry, &source_stored)
        .expect("canonical fixture envelope");
    let envelope = envelopes.pop().expect("canonical envelope");
    let bad_lineage_entry = store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KIS_CALENDAR_NORMALIZED,
                market: MARKET_KR,
                date: &source.entry.date,
                batch_id: envelope.batch_id,
                entitlement_reference: source.entry.entitlement_reference.as_deref(),
                mode: FetchMode::Credentialed,
            },
            &[RawEnvelope::new(
                envelope.batch_id,
                envelope.kind,
                envelope.file_name,
                envelope.bytes,
                envelope.retrieved_at,
                envelope.request,
            )],
        )
        .expect("bad lineage fixture");
    assert!(CalendarPublicationBundle::from_raw(&store, &bad_lineage_entry).is_err());
}

#[test]
fn calendar_normalizer_rejects_a_noncommitted_source_manifest() {
    let temp = tempfile::tempdir().expect("raw root");
    let store = RawStore::new(temp.path().join("data"));
    let source = market_data::ManifestEntry {
        batch_id: BatchId::generate(),
        provider: PROVIDER_KIS_CALENDAR.to_owned(),
        market: MARKET_KR.to_owned(),
        date: target(),
        retrieved_at: UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at"),
        mode: FetchMode::Credentialed,
        entitlement_reference: None,
        files: vec![market_data::storage::FileEntry {
            kind: ResponseKind::Calendar,
            file_name: "calendar-page-01.json".to_owned(),
            content_hash: ContentHash::from_bytes(&valid_body()),
            size_bytes: valid_body().len() as u64,
            request: RequestMetadata {
                endpoint: CALENDAR_PATH.to_owned(),
                query: vec![
                    ("BASS_DT".to_owned(), "20260914".to_owned()),
                    ("CTX_AREA_FK".to_owned(), String::new()),
                    ("CTX_AREA_NK".to_owned(), String::new()),
                ],
                headers: vec![],
                mode: FetchMode::Credentialed,
            },
            response_continuation: None,
            copied_from: None,
        }],
    };
    store
        .read_reconciled_manifest(PROVIDER_KIS_CALENDAR, MARKET_KR)
        .expect("empty source manifest");
    let error = normalize_kis_calendar_batch(&store, &source).expect_err("uncommitted source");
    assert!(matches!(
        error,
        market_data::normalize::NormalizeError::CalendarSourceManifestConflict { .. }
    ));
}
