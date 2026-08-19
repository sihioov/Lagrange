//! Raw-only ingestion tests for the OpenDART disclosure adapter.
//!
//! Covers exactly the three approved surfaces (`list.json`, `corpCode.xml`,
//! `company.json`): validation, pagination, the never-persist-the-key
//! contract, and storage into the immutable Raw zone. No network I/O; all
//! bytes here are fixture bytes returned by [`FixtureReader`].

use std::sync::{Arc, Mutex};

use domain::{TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_OPENDART, ResponseKind};
use market_data::storage::RawStore;
use market_data::{
    DISCLOSURE_LIST_MAX_PAGES, ManifestEntry, OPENDART_DISCLOSURE_LIST_ENDPOINT,
    OPENDART_ENTITY_COMPANY_ENDPOINT, OPENDART_ENTITY_CORPCODE_ENDPOINT, OpenDartError,
    OpenDartOutcome, OpenDartProvider, OpenDartRead,
};
use serde_json::json;

const NOW: &str = "2026-08-19T08:00:00Z";

fn new_store() -> (tempfile::TempDir, RawStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path());
    (temp, store)
}

fn fixed_date() -> TradingDate {
    TradingDate::new(2026, 8, 19).expect("valid date")
}

fn fixed_retrieved_at() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(NOW).expect("valid timestamp")
}

/// One recorded call against [`FixtureReader`].
#[derive(Debug, Clone)]
struct Call {
    path: String,
    query: Vec<(String, String)>,
}

/// Fixture [`OpenDartRead`] implementation: returns canned response bytes in
/// order, one per call, and records every call it received.
#[derive(Debug, Clone)]
struct FixtureReader {
    calls: Arc<Mutex<Vec<Call>>>,
    pages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FixtureReader {
    fn new(pages: Vec<Vec<u8>>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            pages: Arc::new(Mutex::new(pages)),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// The requested `page_no` query value from every recorded call, in
    /// call order.
    fn page_no_sequence(&self) -> Vec<u32> {
        self.calls()
            .iter()
            .filter_map(|call| {
                call.query
                    .iter()
                    .find(|(key, _)| key == "page_no")
                    .and_then(|(_, value)| value.parse().ok())
            })
            .collect()
    }
}

impl OpenDartRead for FixtureReader {
    async fn get(&self, path: &str, query: &[(String, String)]) -> Result<Vec<u8>, OpenDartError> {
        self.calls.lock().unwrap().push(Call {
            path: path.to_owned(),
            query: query.to_vec(),
        });
        let mut pages = self.pages.lock().unwrap();
        assert!(
            !pages.is_empty(),
            "FixtureReader exhausted: unexpected extra call to {path}"
        );
        Ok(pages.remove(0))
    }
}

fn list_row(
    corp_code: &str,
    corp_name: &str,
    stock_code: &str,
    rcept_no: &str,
) -> serde_json::Value {
    json!({
        "corp_cls": "Y",
        "corp_name": corp_name,
        "corp_code": corp_code,
        "stock_code": stock_code,
        "report_nm": "SYNTHETIC-DISCLOSURE (fixture only)",
        "rcept_no": rcept_no,
        "flr_nm": "SYNTHETIC-FILER",
        "rcept_dt": "20260101",
        "rm": "",
    })
}

fn list_page_body(
    status: &str,
    page_no: u32,
    total_count: u64,
    total_page: u32,
    rows: Vec<serde_json::Value>,
) -> Vec<u8> {
    json!({
        "status": status,
        "message": "SYNTHETIC",
        "page_no": page_no,
        "page_count": 100,
        "total_count": total_count,
        "total_page": total_page,
        "list": rows,
    })
    .to_string()
    .into_bytes()
}

fn no_data_body() -> Vec<u8> {
    json!({ "status": "013", "message": "SYNTHETIC no-data" })
        .to_string()
        .into_bytes()
}

fn company_body(status: &str) -> Vec<u8> {
    json!({
        "status": status,
        "message": "SYNTHETIC",
        "corp_code": "00000001",
        "corp_name": "SYNTHETIC CORP",
        "stock_code": "000000",
    })
    .to_string()
    .into_bytes()
}

fn zip_like_body(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x50, 0x4b, 0x03, 0x04];
    bytes.extend_from_slice(payload);
    bytes
}

fn corp_code_error_body() -> Vec<u8> {
    json!({ "status": "020", "message": "SYNTHETIC error body, not a real archive" })
        .to_string()
        .into_bytes()
}

fn manifest_text(store: &RawStore) -> String {
    std::fs::read_to_string(store.manifest_path(PROVIDER_OPENDART, MARKET_KR)).unwrap_or_default()
}

/// Lists the file names inside `entry`'s batch dir, sorted. Used to prove a
/// secret sweep is not vacuous before it concludes the key is absent.
fn batch_dir_file_names(
    store: &RawStore,
    date: &TradingDate,
    entry: &ManifestEntry,
) -> Vec<String> {
    let dir = store.batch_dir(PROVIDER_OPENDART, MARKET_KR, date, &entry.batch_id);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("batch dir exists")
        .map(|file| {
            file.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Reads one named file out of `entry`'s batch dir as text.
fn batch_dir_file_text(
    store: &RawStore,
    date: &TradingDate,
    entry: &ManifestEntry,
    file_name: &str,
) -> String {
    let dir = store.batch_dir(PROVIDER_OPENDART, MARKET_KR, date, &entry.batch_id);
    std::fs::read_to_string(dir.join(file_name)).expect("read stored metadata file")
}

/// Every value recorded against the `crtfc_key` query parameter anywhere in
/// `text`, found by walking parsed JSON rather than by substring match.
///
/// `text` may be a single JSON document or JSONL; each line is parsed
/// independently so the same walk serves `batch.json` and `manifest.jsonl`.
fn recorded_crtfc_key_values(text: &str) -> Vec<String> {
    fn walk(value: &serde_json::Value, found: &mut Vec<String>) {
        match value {
            serde_json::Value::Array(items) => {
                // A recorded query pair is serialized as a two-element array.
                if let [
                    serde_json::Value::String(key),
                    serde_json::Value::String(recorded),
                ] = items.as_slice()
                    && key == "crtfc_key"
                {
                    found.push(recorded.clone());
                }
                for item in items {
                    walk(item, found);
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values() {
                    walk(item, found);
                }
            }
            _ => {}
        }
    }

    let mut found = Vec::new();
    for line in text.split('\n').filter(|line| !line.trim().is_empty()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            walk(&value, &mut found);
        }
    }
    if found.is_empty()
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
    {
        walk(&value, &mut found);
    }
    found
}

/// Reads back every stored file inside `entry`'s batch dir, `batch.json`
/// included (it lives in the same directory).
fn batch_dir_file_bytes(
    store: &RawStore,
    date: &TradingDate,
    entry: &ManifestEntry,
) -> Vec<Vec<u8>> {
    let dir = store.batch_dir(PROVIDER_OPENDART, MARKET_KR, date, &entry.batch_id);
    std::fs::read_dir(&dir)
        .expect("batch dir exists")
        .map(|file| std::fs::read(file.expect("dir entry").path()).expect("read stored file"))
        .collect()
}

// ---------------------------------------------------------------------
// Test 1 (happy path, one per surface) + redacted-query/endpoint/mode checks
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_happy_path_single_page() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let body = list_page_body(
        "000",
        1,
        1,
        1,
        vec![list_row(
            "00000001",
            "SYNTHETIC CORP A",
            "",
            "20260101000001",
        )],
    );
    let reader = FixtureReader::new(vec![body]);
    let provider = OpenDartProvider::new(reader.clone());

    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("single-page list.json ingest should succeed");

    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome");
    };
    assert_eq!(entry.provider, PROVIDER_OPENDART);
    assert_eq!(entry.files.len(), 1);
    let file = &entry.files[0];
    assert_eq!(file.kind, ResponseKind::DisclosureIndex);
    assert_eq!(file.request.endpoint, OPENDART_DISCLOSURE_LIST_ENDPOINT);
    assert_eq!(file.request.mode, FetchMode::Synthetic);
    assert!(
        file.request
            .query
            .iter()
            .any(|(key, value)| key == "crtfc_key" && value == "REDACTED"),
        "recorded query must carry the redacted placeholder: {:?}",
        file.request.query
    );
    // The query handed to the reader carries no credential at all: only the
    // transport appends one, on its own side of the boundary.
    assert!(
        !reader.calls()[0]
            .query
            .iter()
            .any(|(key, _)| key == "crtfc_key"),
        "the query passed to the reader must carry no credential parameter: {:?}",
        reader.calls()[0].query
    );
    assert_eq!(reader.page_no_sequence(), vec![1]);
    assert_eq!(reader.calls()[0].path, "/api/list.json");
}

/// `total_page == 0` is a documented terminal condition distinct from
/// `page_no >= total_page`: the walk must stop after page 1 and still store
/// a real (if empty-list) batch, rather than treating it as `status=013`.
#[tokio::test]
async fn disclosure_index_total_page_zero_is_terminal_after_one_page() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let body = list_page_body("000", 1, 0, 0, vec![]);
    let reader = FixtureReader::new(vec![body]);
    let provider = OpenDartProvider::new(reader.clone());

    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("total_page=0 must terminate immediately, not error");
    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome (total_page=0 is not the same as status=013)");
    };
    assert_eq!(entry.files.len(), 1);
    assert_eq!(reader.page_no_sequence(), vec![1]);
}

#[tokio::test]
async fn entity_master_happy_path_zip_magic_accepted_and_stored_byte_for_byte() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let payload = zip_like_body(b"SYNTHETIC-ZIP-PAYLOAD-NOT-A-REAL-ARCHIVE");
    let reader = FixtureReader::new(vec![payload.clone()]);
    let provider = OpenDartProvider::new(reader.clone());

    let outcome = provider
        .ingest_entity_master(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("ZIP-magic corpCode.xml ingest should succeed");
    assert_eq!(reader.calls()[0].path, "/api/corpCode.xml");

    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome");
    };
    assert_eq!(entry.files.len(), 1);
    let file = &entry.files[0];
    assert_eq!(file.kind, ResponseKind::DisclosureEntityMaster);
    assert_eq!(file.request.endpoint, OPENDART_ENTITY_CORPCODE_ENDPOINT);
    assert_eq!(file.request.mode, FetchMode::Synthetic);
    assert!(
        file.request
            .query
            .iter()
            .any(|(key, value)| key == "crtfc_key" && value == "REDACTED")
    );

    // Never unzipped, never parsed: the stored bytes are exactly the wire bytes.
    let stored_path = store
        .batch_dir(PROVIDER_OPENDART, MARKET_KR, &date, &entry.batch_id)
        .join(&file.file_name);
    let stored_bytes = std::fs::read(stored_path).expect("stored archive readable");
    assert_eq!(stored_bytes, payload);
}

#[tokio::test]
async fn entity_profile_happy_path() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let reader = FixtureReader::new(vec![company_body("000")]);
    let provider = OpenDartProvider::new(reader.clone());

    let outcome = provider
        .ingest_entity_profile(
            &store,
            MARKET_KR,
            &date,
            retrieved_at,
            FetchMode::Synthetic,
            "00000001",
        )
        .await
        .expect("company.json ingest should succeed");
    assert_eq!(reader.calls()[0].path, "/api/company.json");

    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome");
    };
    let file = &entry.files[0];
    assert_eq!(file.kind, ResponseKind::DisclosureEntityProfile);
    assert_eq!(file.request.endpoint, OPENDART_ENTITY_COMPANY_ENDPOINT);
    assert_eq!(file.request.mode, FetchMode::Synthetic);
    assert!(
        file.request
            .query
            .iter()
            .any(|(key, value)| key == "crtfc_key" && value == "REDACTED")
    );
    assert!(
        file.request
            .query
            .iter()
            .any(|(key, value)| key == "corp_code" && value == "00000001")
    );
}

// ---------------------------------------------------------------------
// Test 2: no live credential reaches disk anywhere, and the placeholder
// does.
// ---------------------------------------------------------------------

/// What reaches disk for the credential parameter, asserted positively.
///
/// Scanning for a sentinel key would be vacuous here: this crate no longer
/// holds a `crtfc_key` value, so no sentinel is ever injected and an absence
/// check could not fail. The real, falsifiable invariant is that **every**
/// `crtfc_key` pair persisted in `batch.json` and in the manifest carries
/// exactly the placeholder — which would break the moment anyone made this
/// module record a live value again. The sweep's non-vacuity is asserted
/// first, so the invariant is known to have been applied to real files.
#[tokio::test]
async fn no_live_credential_reaches_stored_bytes_batch_json_or_manifest() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let pages = vec![
        list_page_body(
            "000",
            1,
            2,
            2,
            vec![list_row(
                "00000001",
                "SYNTHETIC CORP A",
                "",
                "20260101000001",
            )],
        ),
        list_page_body(
            "000",
            2,
            2,
            2,
            vec![list_row(
                "00000002",
                "SYNTHETIC CORP B",
                "",
                "20260101000002",
            )],
        ),
    ];
    let reader = FixtureReader::new(pages);
    let provider = OpenDartProvider::new(reader);

    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("multi-page ingest should succeed");
    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome");
    };

    // Guard against a vacuous scan: this is the load-bearing proof for the one
    // hard security requirement, so assert the sweep actually saw the two page
    // bodies plus `batch.json` before concluding the key is absent.
    let names = batch_dir_file_names(&store, &date, &entry);
    assert_eq!(
        names.len(),
        3,
        "expected two page files plus batch.json: {names:?}"
    );
    assert!(
        names.iter().any(|name| name == "batch.json"),
        "the sweep must cover batch.json, where request metadata is persisted: {names:?}"
    );

    let scanned = batch_dir_file_bytes(&store, &date, &entry);
    assert_eq!(scanned.len(), names.len());

    // Every persisted `crtfc_key` pair must be the placeholder, in both
    // durable locations. Asserted over parsed structure rather than by
    // substring, so a live value recorded alongside a placeholder elsewhere
    // in the file could not slip through.
    let batch_json = batch_dir_file_text(&store, &date, &entry, "batch.json");
    let manifest = manifest_text(&store);
    assert!(
        !manifest.is_empty(),
        "manifest must exist and be non-empty for these assertions to mean anything"
    );

    for (label, text) in [("batch.json", &batch_json), ("manifest.jsonl", &manifest)] {
        let recorded = recorded_crtfc_key_values(text);
        assert!(
            !recorded.is_empty(),
            "{label} must record the crtfc_key parameter so the manifest reflects the live request"
        );
        for value in recorded {
            assert_eq!(
                value, "REDACTED",
                "{label} recorded a crtfc_key value that is not the placeholder"
            );
        }
    }
}

// ---------------------------------------------------------------------
// Test 3: status=013 is a typed empty outcome, not an error, no batch.
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_no_data_status_is_typed_empty_not_error() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let reader = FixtureReader::new(vec![no_data_body()]);
    let provider = OpenDartProvider::new(reader);

    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("status=013 must not be an error");
    assert_eq!(outcome, OpenDartOutcome::Empty);

    // No misleading batch: nothing at all was written for this provider/market.
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
    assert!(!store.manifest_path(PROVIDER_OPENDART, MARKET_KR).exists());
}

// ---------------------------------------------------------------------
// Test 4: failing status / missing status / malformed JSON / undocumented
// shape each fail closed with a distinct typed error.
// ---------------------------------------------------------------------

#[tokio::test]
async fn entity_profile_status_and_shape_failures_are_distinct_typed_errors() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("failing_status", company_body("500")),
        (
            "missing_status",
            json!({"message": "no status field at all"})
                .to_string()
                .into_bytes(),
        ),
        ("malformed_json", b"{ this is not valid json".to_vec()),
        (
            "undocumented_shape",
            json!(["not", "an", "object"]).to_string().into_bytes(),
        ),
    ];

    for (label, body) in cases {
        let (_temp, store) = new_store();
        let date = fixed_date();
        let retrieved_at = fixed_retrieved_at();
        let reader = FixtureReader::new(vec![body]);
        let provider = OpenDartProvider::new(reader);

        let result = provider
            .ingest_entity_profile(
                &store,
                MARKET_KR,
                &date,
                retrieved_at,
                FetchMode::Synthetic,
                "00000001",
            )
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{label} must fail closed but succeeded"),
        };

        let matches_expected = match label {
            "failing_status" => matches!(error, OpenDartError::UnexpectedStatus { .. }),
            "missing_status" => matches!(error, OpenDartError::MissingStatus),
            "malformed_json" => matches!(error, OpenDartError::MalformedJson),
            "undocumented_shape" => matches!(error, OpenDartError::UndocumentedShape),
            _ => unreachable!(),
        };
        assert!(matches_expected, "{label}: unexpected error {error:?}");
        assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
    }
}

// ---------------------------------------------------------------------
// Test 5: multi-page walk terminates correctly; exact page_no sequence.
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_multi_page_walk_terminates_and_records_page_sequence() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let pages = vec![
        list_page_body(
            "000",
            1,
            250,
            3,
            vec![list_row(
                "00000001",
                "SYNTHETIC CORP A",
                "",
                "20260101000001",
            )],
        ),
        list_page_body(
            "000",
            2,
            250,
            3,
            vec![list_row(
                "00000002",
                "SYNTHETIC CORP B",
                "",
                "20260101000002",
            )],
        ),
        list_page_body(
            "000",
            3,
            250,
            3,
            vec![list_row(
                "00000003",
                "SYNTHETIC CORP C",
                "",
                "20260101000003",
            )],
        ),
    ];
    let reader = FixtureReader::new(pages);
    let provider = OpenDartProvider::new(reader.clone());

    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("three-page walk should terminate cleanly");
    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome");
    };
    assert_eq!(entry.files.len(), 3);
    assert_eq!(reader.page_no_sequence(), vec![1, 2, 3]);
}

// ---------------------------------------------------------------------
// Test 6: total_count/total_page changing mid-walk fails closed.
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_total_page_change_mid_walk_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let pages = vec![
        list_page_body(
            "000",
            1,
            250,
            3,
            vec![list_row(
                "00000001",
                "SYNTHETIC CORP A",
                "",
                "20260101000001",
            )],
        ),
        list_page_body(
            "000",
            2,
            999, // total_count shifted mid-walk
            3,
            vec![list_row(
                "00000002",
                "SYNTHETIC CORP B",
                "",
                "20260101000002",
            )],
        ),
    ];
    let reader = FixtureReader::new(pages);
    let provider = OpenDartProvider::new(reader);

    let error = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect_err("a shifting result set must fail closed");
    assert!(matches!(error, OpenDartError::InconsistentPagination));
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
}

// ---------------------------------------------------------------------
// Test 7: identical bytes for two different requested pages fails closed.
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_identical_bytes_across_pages_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let page = list_page_body(
        "000",
        1,
        500,
        5,
        vec![list_row(
            "00000001",
            "SYNTHETIC CORP A",
            "",
            "20260101000001",
        )],
    );
    let reader = FixtureReader::new(vec![page.clone(), page]);
    let provider = OpenDartProvider::new(reader);

    let error = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect_err("byte-identical pages must fail closed");
    assert!(matches!(
        error,
        OpenDartError::DuplicatePageBytes {
            page_no: 2,
            duplicate_of: 1
        }
    ));
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
}

// ---------------------------------------------------------------------
// Test 8: exceeding the 10-page bound fails closed.
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_exceeding_page_bound_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let pages: Vec<Vec<u8>> = (1..=10u32)
        .map(|page_no| {
            let rcept_no = format!("20260101{page_no:06}");
            list_page_body(
                "000",
                page_no,
                5000,
                999, // never terminal within the bound
                vec![list_row(
                    &format!("{page_no:08}"),
                    "SYNTHETIC CORP",
                    "",
                    &rcept_no,
                )],
            )
        })
        .collect();
    let reader = FixtureReader::new(pages);
    let provider = OpenDartProvider::new(reader.clone());

    let error = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect_err("a walk that never terminates must fail closed at the bound");
    assert!(matches!(
        error,
        OpenDartError::PaginationBoundExceeded {
            max_pages: DISCLOSURE_LIST_MAX_PAGES
        }
    ));
    assert_eq!(reader.page_no_sequence(), (1..=10).collect::<Vec<u32>>());
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
}

// ---------------------------------------------------------------------
// Test 9: rcept_no that is not exactly 14 digits fails closed.
// ---------------------------------------------------------------------

#[tokio::test]
async fn disclosure_index_rejects_rcept_no_not_14_digits() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let body = list_page_body(
        "000",
        1,
        1,
        1,
        vec![list_row("00000001", "SYNTHETIC CORP A", "", "12345")],
    );
    let reader = FixtureReader::new(vec![body]);
    let provider = OpenDartProvider::new(reader);

    let error = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect_err("a non-14-digit rcept_no must fail closed");
    assert!(matches!(error, OpenDartError::InvalidRceptNo));
}

// ---------------------------------------------------------------------
// Test 10 (second half): corpCode.xml JSON error body fails closed instead
// of being stored as if it were the archive.
// ---------------------------------------------------------------------

#[tokio::test]
async fn entity_master_json_error_body_fails_closed_instead_of_being_stored() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let reader = FixtureReader::new(vec![corp_code_error_body()]);
    let provider = OpenDartProvider::new(reader);

    let error = provider
        .ingest_entity_master(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect_err("a JSON error body must never be stored as the archive");
    assert!(matches!(error, OpenDartError::UnexpectedStatus { .. }));
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
}

// ---------------------------------------------------------------------
// Additional coverage beyond the numbered list, tied to explicit prose
// requirements in the brief.
// ---------------------------------------------------------------------

/// A `013` arriving after at least one page already succeeded must not
/// resolve to a clean empty outcome: the result set may have shifted
/// mid-walk, and completeness cannot be proven.
#[tokio::test]
async fn disclosure_index_no_data_after_prior_page_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let pages = vec![
        list_page_body(
            "000",
            1,
            200,
            2,
            vec![list_row(
                "00000001",
                "SYNTHETIC CORP A",
                "",
                "20260101000001",
            )],
        ),
        no_data_body(),
    ];
    let reader = FixtureReader::new(pages);
    let provider = OpenDartProvider::new(reader);

    let error = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect_err("013 after a prior success must fail closed, not resolve to Empty");
    assert!(matches!(
        error,
        OpenDartError::UnexpectedEmptyMidWalk { page_no: 2 }
    ));
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
}

/// Single-page surfaces must reject a pagination-like marker in the body.
#[tokio::test]
async fn entity_profile_rejects_pagination_like_marker() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let body = json!({
        "status": "000",
        "message": "SYNTHETIC",
        "corp_code": "00000001",
        "total_page": 1,
    })
    .to_string()
    .into_bytes();
    let reader = FixtureReader::new(vec![body]);
    let provider = OpenDartProvider::new(reader);

    let error = provider
        .ingest_entity_profile(
            &store,
            MARKET_KR,
            &date,
            retrieved_at,
            FetchMode::Synthetic,
            "00000001",
        )
        .await
        .expect_err("a pagination-like marker on a single-page surface must fail closed");
    assert!(matches!(error, OpenDartError::UnexpectedPagination));
}

// `OpenDartProvider` no longer holds a `crtfc_key` value, so a test built
// around a caller-supplied query *value* colliding with a configured key no
// longer describes anything real: there is no key here to collide with, and
// `redacted_metadata`'s guard is now structural (it rejects a query pair by
// *name*, not by value). None of this module's public `ingest_*` methods
// let a caller choose a query pair's name -- every one is hardcoded
// (`page_no`, `page_count`, `corp_code`) -- so the guard cannot be reached
// through this crate's public API at all. That structural unreachability is
// itself evidence for the "no `crtfc_key` path in `market-data`" property;
// see `redacted_metadata_rejects_a_caller_supplied_crtfc_key_pair` and
// `redacted_metadata_allows_a_query_value_that_merely_resembles_a_key` in
// `crates/market-data/src/providers/opendart.rs`'s in-module unit tests,
// which reach past the public API to exercise the guard directly.

/// Fixture-hygiene requirement: real ETF11 KRX short codes may be used ONLY
/// to exercise the corp_code/stock_code field-carrying machinery of this
/// adapter's JSON validation and storage pipeline. This test proves that
/// machinery works end-to-end (the values survive validation and storage
/// unmodified). It does NOT establish that OpenDART actually covers these
/// ETFs' corporate disclosures -- that question is open until the owner
/// supplies a real key or the real `corpCode.xml` archive. Every corp code,
/// corp name, and filer name here remains synthetic.
#[tokio::test]
async fn disclosure_index_carries_real_etf_short_codes_as_stock_code_without_corruption() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let rows = vec![
        list_row(
            "00000101",
            "SYNTHETIC ETF ISSUER A",
            "069500",
            "20260101000001",
        ),
        list_row(
            "00000102",
            "SYNTHETIC ETF ISSUER B",
            "102110",
            "20260101000002",
        ),
        list_row(
            "00000103",
            "SYNTHETIC ETF ISSUER C",
            "114260",
            "20260101000003",
        ),
    ];
    let body = list_page_body("000", 1, 3, 1, rows);
    let reader = FixtureReader::new(vec![body]);
    let provider = OpenDartProvider::new(reader);

    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("synthetic rows carrying real ETF short codes must validate");
    let OpenDartOutcome::Stored(entry) = outcome else {
        panic!("expected Stored outcome");
    };
    let stored_path = store
        .batch_dir(PROVIDER_OPENDART, MARKET_KR, &date, &entry.batch_id)
        .join(&entry.files[0].file_name);
    let stored_bytes = std::fs::read(stored_path).expect("stored file readable");
    let value: serde_json::Value = serde_json::from_slice(&stored_bytes).expect("valid JSON");
    let stock_codes: Vec<&str> = value["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|row| row["stock_code"].as_str().expect("stock_code is a string"))
        .collect();
    assert_eq!(stock_codes, vec!["069500", "102110", "114260"]);
}

/// The official `응답 결과` table names `page_no`, `page_count`, `total_count`,
/// and `total_page` without stating their JSON type, while the matching request
/// parameters are documented as `STRING`. Both representations of the same
/// documented value are therefore accepted; value validation stays strict.
#[tokio::test]
async fn disclosure_index_accepts_string_typed_envelope_integers() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let body = json!({
        "status": "000",
        "message": "SYNTHETIC",
        "page_no": "1",
        "page_count": "100",
        "total_count": "1",
        "total_page": "1",
        "list": [list_row("00000001", "SYNTHETIC CORP A", "", "20260101000001")],
    })
    .to_string()
    .into_bytes();

    let provider = OpenDartProvider::new(FixtureReader::new(vec![body]));
    let outcome = provider
        .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
        .await
        .expect("string-typed envelope integers are a documented-value representation");
    assert!(matches!(outcome, OpenDartOutcome::Stored(_)));
}

/// A non-integer representation is still an undocumented shape: tolerance
/// covers how the documented value is written, never what it may contain.
#[tokio::test]
async fn disclosure_index_rejects_non_integer_envelope_values() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    for bad in [
        json!("1.5"),
        json!(""),
        json!("-1"),
        json!("1 "),
        json!(true),
    ] {
        let body = json!({
            "status": "000",
            "message": "SYNTHETIC",
            "page_no": 1,
            "page_count": 100,
            "total_count": 1,
            "total_page": bad,
            "list": [list_row("00000001", "SYNTHETIC CORP A", "", "20260101000001")],
        })
        .to_string()
        .into_bytes();

        let provider = OpenDartProvider::new(FixtureReader::new(vec![body]));
        let error = provider
            .ingest_disclosure_index(&store, MARKET_KR, &date, retrieved_at, FetchMode::Synthetic)
            .await
            .expect_err("a non-integer envelope value must fail closed");
        assert!(
            matches!(error, OpenDartError::UndocumentedShape),
            "expected UndocumentedShape, got {error:?}"
        );
    }
}
