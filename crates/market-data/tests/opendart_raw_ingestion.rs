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
    CredentialRef, DISCLOSURE_LIST_MAX_PAGES, ManifestEntry, OPENDART_DISCLOSURE_LIST_ENDPOINT,
    OPENDART_ENTITY_COMPANY_ENDPOINT, OPENDART_ENTITY_CORPCODE_ENDPOINT, OpenDartError,
    OpenDartLiveReader, OpenDartOutcome, OpenDartProvider, OpenDartRead,
};
use serde_json::json;

const NOW: &str = "2026-08-19T08:00:00Z";
const SENTINEL_KEY: &str = "sentinel-key-do-not-persist-to-disk";

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
    let provider = OpenDartProvider::new(reader.clone(), SENTINEL_KEY);

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
    assert!(
        !file
            .request
            .query
            .iter()
            .any(|(_, value)| value == SENTINEL_KEY),
        "recorded query must never carry the live key"
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
    let provider = OpenDartProvider::new(reader.clone(), SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader.clone(), SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader.clone(), SENTINEL_KEY);

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
// Test 2: the sentinel key never reaches disk anywhere.
// ---------------------------------------------------------------------

#[tokio::test]
async fn sentinel_key_never_reaches_stored_bytes_batch_json_or_manifest() {
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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    for bytes in scanned {
        assert!(
            !contains_bytes(&bytes, SENTINEL_KEY.as_bytes()),
            "sentinel leaked into a stored batch file"
        );
    }
    let manifest = manifest_text(&store);
    assert!(
        !manifest.is_empty(),
        "manifest must exist and be non-empty for this assertion to mean anything"
    );
    assert!(
        !manifest.contains(SENTINEL_KEY),
        "sentinel leaked into manifest.jsonl: {manifest}"
    );
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
        let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader.clone(), SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader.clone(), SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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

/// The redaction scan is wired for real: if a caller-supplied query value
/// happens to equal the configured key, that must be rejected rather than
/// silently persisted (defense in depth beyond the structural placeholder).
#[tokio::test]
async fn key_leak_is_detected_when_a_caller_supplied_value_matches_the_configured_key() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let retrieved_at = fixed_retrieved_at();

    let reader = FixtureReader::new(vec![company_body("000")]);
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

    let error = provider
        .ingest_entity_profile(
            &store,
            MARKET_KR,
            &date,
            retrieved_at,
            FetchMode::Synthetic,
            SENTINEL_KEY, // corp_code accidentally equals the configured key
        )
        .await
        .expect_err("a query value matching the configured key must be rejected");
    assert!(matches!(error, OpenDartError::KeyLeakDetected));
    assert!(!store.provider_dir(PROVIDER_OPENDART, MARKET_KR).exists());
}

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
    let provider = OpenDartProvider::new(reader, SENTINEL_KEY);

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

/// The live reader stub cannot be built without an explicit credential
/// reference, and every call fails closed without attempting any I/O.
#[tokio::test]
async fn live_reader_requires_a_credential_reference_and_always_fails_closed() {
    let reader = OpenDartLiveReader::new(CredentialRef::new("env:OPENDART_CRTFC_KEY_REF"));
    let result = reader.get("/api/company.json", &[]).await;
    assert!(matches!(result, Err(OpenDartError::NotConfigured)));
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

    let provider = OpenDartProvider::new(FixtureReader::new(vec![body]), SENTINEL_KEY);
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

        let provider = OpenDartProvider::new(FixtureReader::new(vec![body]), SENTINEL_KEY);
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
