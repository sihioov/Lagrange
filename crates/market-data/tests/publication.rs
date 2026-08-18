use std::fs;

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX, RawEnvelope,
    RequestMetadata, ResponseKind,
};
use market_data::ingest::{IngestRequest, ingest_bundle};
use market_data::normalize::{
    NormalizationLineage, NormalizationSourceFile, deterministic_kis_normalized_batch_id,
};
use market_data::provider::{KrxProvider, RecordedBundle};
use market_data::publication::{
    CalendarSessionType, DataBatchKind, PublicationBundle, PublicationError,
};
use market_data::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use serde_json::{Value, json};

const CONTRACT_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract"
);

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ls-publication-{tag}-{}", BatchId::generate()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test root");
    dir
}

fn metadata_for_mode(mode: FetchMode) -> RequestMetadata {
    RequestMetadata {
        endpoint: "test".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        mode,
    }
}

fn store_files(store: &RawStore, files: &[(ResponseKind, &str, Vec<u8>)]) -> ManifestEntry {
    store_files_with_scope(store, files, PROVIDER_KRX, FetchMode::Synthetic)
}

fn store_files_with_scope(
    store: &RawStore,
    files: &[(ResponseKind, &str, Vec<u8>)],
    provider: &str,
    mode: FetchMode,
) -> ManifestEntry {
    let batch_id = BatchId::generate();
    let date = TradingDate::parse("2020-01-31").expect("date");
    let envelopes: Vec<_> = files
        .iter()
        .map(|(kind, name, bytes)| {
            RawEnvelope::new(
                batch_id,
                *kind,
                (*name).to_owned(),
                bytes.clone(),
                UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").expect("time"),
                metadata_for_mode(mode),
            )
        })
        .collect();
    store
        .store_batch(
            &BatchSpec {
                provider,
                market: MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: None,
                mode,
            },
            &envelopes,
        )
        .expect("store batch")
}

fn reopen_manifest(store: &RawStore) -> ManifestEntry {
    store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("reopen manifest")
        .pop()
        .expect("stored manifest entry")
}

fn calendar_payload(
    timezone: &str,
    source: &str,
    open: &str,
    close: &str,
    sessions: &str,
    holidays: &str,
) -> Vec<u8> {
    format!(
        r#"{{"calendar_id":"krx-test","schema_version":1,"source":"{source}","timezone":"{timezone}","session_times_local":{{"open":"{open}","close":"{close}"}},"sessions":{sessions},"holidays":{holidays}}}"#
    )
    .into_bytes()
}

fn calendar_payload_with_id(calendar_id: &str, sessions: &str) -> Vec<u8> {
    String::from_utf8(calendar_payload(
        "Asia/Seoul",
        "source",
        "09:00:00",
        "15:30:00",
        sessions,
        "[]",
    ))
    .expect("calendar payload UTF-8")
    .replace("krx-test", calendar_id)
    .into_bytes()
}

fn normalized_lineage(upstream_batch_id: BatchId) -> NormalizationLineage {
    NormalizationLineage {
        schema_version: 1,
        normalizer: "kis-wire-to-canonical-v2".to_owned(),
        upstream_provider: PROVIDER_KIS.to_owned(),
        upstream_market: MARKET_KR.to_owned(),
        upstream_batch_id,
        upstream_files: vec![
            NormalizationSourceFile {
                kind: ResponseKind::Bars,
                file_name: "wire-bars.json".to_owned(),
                content_hash: ContentHash::from_bytes(b"wire-bars"),
            },
            NormalizationSourceFile {
                kind: ResponseKind::Reference,
                file_name: "wire-reference.json".to_owned(),
                content_hash: ContentHash::from_bytes(b"wire-reference"),
            },
            NormalizationSourceFile {
                kind: ResponseKind::Calendar,
                file_name: "wire-calendar.json".to_owned(),
                content_hash: ContentHash::from_bytes(b"wire-calendar"),
            },
            NormalizationSourceFile {
                kind: ResponseKind::CorporateActions,
                file_name: "wire-actions.json".to_owned(),
                content_hash: ContentHash::from_bytes(b"wire-actions"),
            },
        ],
    }
}

fn normalized_request(
    kind: ResponseKind,
    lineage: &NormalizationLineage,
    mode: FetchMode,
) -> RequestMetadata {
    RequestMetadata {
        endpoint: format!("kis.normalized/kis-wire-to-canonical-v2/{kind}"),
        query: vec![
            (
                "upstream_batch_id".to_owned(),
                lineage.upstream_batch_id.to_string(),
            ),
            (
                "upstream_lineage".to_owned(),
                serde_json::to_string(lineage).expect("lineage JSON"),
            ),
        ],
        headers: Vec::new(),
        mode,
    }
}

fn store_normalized_files(
    store: &RawStore,
    files: &[(ResponseKind, &str, Vec<u8>)],
    lineage: &NormalizationLineage,
    mode: FetchMode,
) -> ManifestEntry {
    let batch_id = deterministic_kis_normalized_batch_id(lineage.upstream_batch_id);
    let date = TradingDate::parse("2020-01-31").expect("date");
    let envelopes: Vec<_> = files
        .iter()
        .map(|(kind, name, bytes)| {
            RawEnvelope::new(
                batch_id,
                *kind,
                (*name).to_owned(),
                bytes.clone(),
                UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").expect("time"),
                normalized_request(*kind, lineage, mode),
            )
        })
        .collect();
    store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KIS_NORMALIZED,
                market: MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: None,
                mode,
            },
            &envelopes,
        )
        .expect("store normalized batch")
}

fn with_lineage(document: Value, lineage: &NormalizationLineage) -> Vec<u8> {
    let mut object = document.as_object().expect("canonical object").clone();
    object.insert(
        "_lineage".to_owned(),
        serde_json::to_value(lineage).expect("lineage value"),
    );
    serde_json::to_vec(&Value::Object(object)).expect("canonical JSON")
}

fn normalized_files(lineage: &NormalizationLineage) -> Vec<(ResponseKind, &'static str, Vec<u8>)> {
    vec![
        (
            ResponseKind::Bars,
            "bars.json",
            with_lineage(
                json!({
                    "dataset_id":"kr-etf-daily-2020-01-31",
                    "schema_version":1,
                    "currency":"KRW",
                    "instruments":[{"symbol":"069500","currency":"KRW","lot_size":1}],
                    "bars":[{"instrument":"069500","date":"2020-01-31","open":1,"high":2,"low":1,"close":2,"volume":100}]
                }),
                lineage,
            ),
        ),
        (
            ResponseKind::Reference,
            "reference.json",
            with_lineage(
                json!({
                    "source":"kis-inquire-price-and-daily-bars-v1",
                    "instruments":[{"symbol":"069500","name":"ETF 069500","lot_size":1,"currency":"KRW","kind":"equity-etf"}]
                }),
                lineage,
            ),
        ),
        (
            ResponseKind::Calendar,
            "calendar.json",
            with_lineage(
                json!({
                    "calendar_id":"kis-chk-holiday-v1",
                    "schema_version":1,
                    "source":"kis",
                    "timezone":"Asia/Seoul",
                    "session_times_local":{"open":"09:00:00","close":"15:30:00"},
                    "sessions":[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}],
                    "holidays":[]
                }),
                lineage,
            ),
        ),
        (
            ResponseKind::CorporateActions,
            "corporate-actions.json",
            with_lineage(
                json!({
                    "dataset_id":"kr-etf-daily-2020-01-31",
                    "schema_version":1,
                    "actions":[]
                }),
                lineage,
            ),
        ),
    ]
}

fn normalized_fixture(store: &RawStore, mode: FetchMode) -> ManifestEntry {
    let lineage = normalized_lineage(BatchId::generate());
    let files = normalized_files(&lineage);
    store_normalized_files(store, &files, &lineage, mode)
}

#[test]
fn from_raw_maps_contract_bars_for_target_date_to_eod() {
    let store = RawStore::new(temp_root("contract"));
    let provider = KrxProvider::synthetic(RecordedBundle::open(CONTRACT_BUNDLE).expect("bundle"));
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse("2020-01-31").expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").expect("time"),
    );
    let outcome = ingest_bundle(&store, &provider, &request, None).expect("ingest");
    let reopened = reopen_manifest(&store);

    let publication = PublicationBundle::from_raw(&store, &reopened).expect("publication");

    assert_eq!(publication.files.len(), 4);
    assert_eq!(publication.files[0].kind, DataBatchKind::Eod);
    assert_eq!(publication.files[0].file_name, "bars-response.json");
    assert_eq!(
        publication
            .files
            .iter()
            .map(|file| file.kind)
            .collect::<Vec<_>>(),
        vec![
            DataBatchKind::Eod,
            DataBatchKind::Reference,
            DataBatchKind::Calendar,
            DataBatchKind::CorporateActions,
        ]
    );
    assert_eq!(publication.files[0].content_sha256.len(), 64);
    assert!(
        publication.files[0]
            .content_sha256
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    );
    assert!(!publication.files[0].content_sha256.contains("sha256:"));
    assert_eq!(
        publication.files[0].storage_path,
        fs::canonicalize(
            store
                .batch_dir(
                    PROVIDER_KRX,
                    MARKET_KR,
                    &outcome.entry.date,
                    &outcome.entry.batch_id
                )
                .join("bars-response.json")
        )
        .expect("canonical raw object")
        .to_string_lossy()
    );
    assert_eq!(
        publication.files[0].bytes_size,
        outcome.entry.files[0].size_bytes
    );
    assert_eq!(publication.source_batch_id, outcome.entry.batch_id);
    assert_eq!(
        outcome.entry,
        store
            .read_manifest(PROVIDER_KRX, MARKET_KR)
            .expect("manifest")[0]
    );
}

#[test]
fn from_raw_publishes_calendar_trading_sessions_with_stable_provenance() {
    let store = RawStore::new(temp_root("calendar-sessions"));
    let provider = KrxProvider::synthetic(RecordedBundle::open(CONTRACT_BUNDLE).expect("bundle"));
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse("2020-01-31").expect("date"),
        UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").expect("time"),
    );
    ingest_bundle(&store, &provider, &request, None).expect("ingest");
    let reopened = reopen_manifest(&store);

    let publication = PublicationBundle::from_raw(&store, &reopened).expect("publication");

    assert_eq!(publication.calendar_facts.len(), 2);
    assert_eq!(publication.calendar_facts[0].exchange, "KRX");
    assert_eq!(
        publication.calendar_facts[0].session_date.to_iso(),
        "2020-01-30"
    );
    assert_eq!(
        publication.calendar_facts[0].session_type,
        CalendarSessionType::Trading
    );
    assert_eq!(publication.calendar_facts[0].timezone, "Asia/Seoul");
    assert_eq!(publication.calendar_facts[0].source, "synthetic");
    assert_eq!(
        publication.calendar_facts[0].source_version,
        "krx-2020-01-synthetic:schema-1"
    );
    assert_eq!(
        publication.calendar_facts[0].content_sha256,
        reopened.files[2]
            .content_hash
            .as_str()
            .strip_prefix("sha256:")
            .unwrap()
    );
}

#[test]
fn from_raw_publishes_explicit_calendar_holidays_as_closed_without_inferring_missing_dates() {
    let store = RawStore::new(temp_root("calendar-holidays"));
    let calendar = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/kr-etf/2020-01-31/calendar.json"
    ))
    .expect("calendar fixture");
    store_files(
        &store,
        &[(ResponseKind::Calendar, "calendar.json", calendar)],
    );
    let reopened = reopen_manifest(&store);

    let publication = PublicationBundle::from_raw(&store, &reopened).expect("publication");

    assert!(publication.calendar_facts.iter().any(|fact| {
        fact.session_date.to_iso() == "2020-01-24"
            && fact.session_type == CalendarSessionType::Closed
    }));
    assert!(
        !publication
            .calendar_facts
            .iter()
            .any(|fact| fact.session_date.to_iso() == "2020-01-25")
    );
}

#[test]
fn from_raw_rejects_manifest_size_above_postgres_bigint() {
    let store = RawStore::new(temp_root("oversize"));
    let mut entry = store_files(
        &store,
        &[(ResponseKind::Reference, "reference.json", b"{}".to_vec())],
    );
    entry.files[0].size_bytes = i64::MAX as u64 + 1;

    let error = PublicationBundle::from_raw(&store, &entry).expect_err("oversize must fail");

    assert!(matches!(
        error,
        PublicationError::SizeExceedsPostgresBigint { .. }
    ));
}

#[test]
fn from_raw_validates_all_evidence_before_parsing_an_earlier_malformed_payload() {
    let store = RawStore::new(temp_root("evidence-before-parse"));
    let mut entry = store_files(
        &store,
        &[
            (ResponseKind::Bars, "malformed-bars.json", b"{".to_vec()),
            (ResponseKind::Reference, "oversized.json", b"{}".to_vec()),
        ],
    );
    entry.files[1].size_bytes = i64::MAX as u64 + 1;

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("later evidence validation must win over malformed payload parsing");

    assert!(matches!(
        error,
        PublicationError::SizeExceedsPostgresBigint {
            ref file_name,
            ..
        } if file_name == "oversized.json"
    ));
}

#[test]
fn from_raw_marks_valid_empty_bars_as_eod_unavailable() {
    let store = RawStore::new(temp_root("unavailable"));
    store_files(
        &store,
        &[(ResponseKind::Bars, "bars.json", br#"{"bars":[]}"#.to_vec())],
    );
    let reopened = reopen_manifest(&store);

    let publication = PublicationBundle::from_raw(&store, &reopened).expect("publication");

    assert_eq!(publication.files[0].kind, DataBatchKind::EodUnavailable);
}

#[test]
fn from_raw_rejects_tampered_later_file_before_returning_a_bundle() {
    let store = RawStore::new(temp_root("tampered"));
    let entry = store_files(
        &store,
        &[
            (ResponseKind::Bars, "bars.json", br#"{"bars":[]}"#.to_vec()),
            (ResponseKind::Reference, "reference.json", br#"{}"#.to_vec()),
        ],
    );
    fs::write(
        store
            .batch_dir(PROVIDER_KRX, MARKET_KR, &entry.date, &entry.batch_id)
            .join("reference.json"),
        b"tampered",
    )
    .expect("tamper");

    let error = PublicationBundle::from_raw(&store, &entry).expect_err("tampering must fail");

    assert!(matches!(
        error,
        PublicationError::Store(market_data::storage::StoreError::ContentHashMismatch { .. })
    ));
}

#[test]
fn from_raw_rejects_manifest_size_mismatch() {
    let store = RawStore::new(temp_root("size-mismatch"));
    let mut entry = store_files(
        &store,
        &[(ResponseKind::Reference, "reference.json", b"{}".to_vec())],
    );
    entry.files[0].size_bytes += 1;

    let error = PublicationBundle::from_raw(&store, &entry).expect_err("mismatch must fail");

    assert!(matches!(error, PublicationError::SizeMismatch { .. }));
}

#[test]
fn from_raw_rejects_malformed_bars_json_and_invalid_dates_in_any_row() {
    let store = RawStore::new(temp_root("bad-bars"));
    let malformed = store_files(&store, &[(ResponseKind::Bars, "bars.json", b"{".to_vec())]);
    let malformed_error =
        PublicationBundle::from_raw(&store, &malformed).expect_err("json must fail");
    assert!(matches!(
        malformed_error,
        PublicationError::MalformedBars { .. }
    ));

    let invalid = store_files(
        &store,
        &[(
            ResponseKind::Bars,
            "invalid-bars.json",
            br#"{"bars":[{"date":"2020-01-31"},{"date":"not-a-date"}]}"#.to_vec(),
        )],
    );
    let date_error =
        PublicationBundle::from_raw(&store, &invalid).expect_err("bad row date must fail");
    assert!(matches!(
        date_error,
        PublicationError::InvalidBarDate { .. }
    ));
}

#[test]
fn from_raw_rejects_invalid_calendar_provenance_timezone_and_declared_times() {
    let store = RawStore::new(temp_root("bad-calendar-fields"));
    let timezone = store_files(
        &store,
        &[(
            (ResponseKind::Calendar),
            "timezone.json",
            calendar_payload("UTC", "source", "09:00:00", "15:30:00", "[]", "[]"),
        )],
    );
    let timezone_error =
        PublicationBundle::from_raw(&store, &timezone).expect_err("timezone must fail");
    assert!(matches!(
        timezone_error,
        PublicationError::UnsupportedCalendarTimezone { .. }
    ));

    let source = store_files(
        &store,
        &[(
            ResponseKind::Calendar,
            "source.json",
            calendar_payload("Asia/Seoul", "", "09:00:00", "15:30:00", "[]", "[]"),
        )],
    );
    let source_error = PublicationBundle::from_raw(&store, &source).expect_err("source must fail");
    assert!(matches!(
        source_error,
        PublicationError::MalformedCalendar { .. }
    ));

    let times = store_files(
        &store,
        &[(
            ResponseKind::Calendar,
            "times.json",
            calendar_payload("Asia/Seoul", "source", "09:01:00", "15:30:00", "[]", "[]"),
        )],
    );
    let time_error = PublicationBundle::from_raw(&store, &times).expect_err("times must fail");
    assert!(matches!(
        time_error,
        PublicationError::InvalidCalendarSessionTimes { .. }
    ));
}

#[test]
fn from_raw_rejects_inconsistent_calendar_instants_and_dates_present_in_both_lists() {
    let store = RawStore::new(temp_root("bad-calendar-values"));
    let inconsistent = store_files(
        &store,
        &[(
            ResponseKind::Calendar,
            "instant.json",
            calendar_payload(
                "Asia/Seoul",
                "source",
                "09:00:00",
                "15:30:00",
                r#"[{"date":"2020-01-31","open_utc":"2020-01-31T01:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]"#,
                "[]",
            ),
        )],
    );
    let instant_error =
        PublicationBundle::from_raw(&store, &inconsistent).expect_err("instant must fail");
    assert!(matches!(
        instant_error,
        PublicationError::InconsistentCalendarInstant { .. }
    ));

    let both = store_files(
        &store,
        &[(
            ResponseKind::Calendar,
            "both.json",
            calendar_payload(
                "Asia/Seoul",
                "source",
                "09:00:00",
                "15:30:00",
                r#"[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]"#,
                r#"[{"date":"2020-01-31"}]"#,
            ),
        )],
    );
    let both_error = PublicationBundle::from_raw(&store, &both).expect_err("overlap must fail");
    assert!(matches!(
        both_error,
        PublicationError::CalendarDateBothSessionAndHoliday { .. }
    ));
}

#[test]
fn from_raw_rejects_calendar_session_instants_with_fractional_seconds() {
    let store = RawStore::new(temp_root("calendar-fractional-instant"));
    let entry = store_files(
        &store,
        &[(
            ResponseKind::Calendar,
            "fractional.json",
            calendar_payload(
                "Asia/Seoul",
                "source",
                "09:00:00",
                "15:30:00",
                r#"[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00.500Z","close_utc":"2020-01-31T06:30:00Z"}]"#,
                "[]",
            ),
        )],
    );

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("fractional-second session instant must fail");

    assert!(matches!(
        error,
        PublicationError::InconsistentCalendarInstant {
            ref field,
            ref file_name,
            ..
        } if field == "open_utc" && file_name == "fractional.json"
    ));
}

#[test]
fn from_raw_deduplicates_identical_calendar_facts_and_rejects_conflicts() {
    let store = RawStore::new(temp_root("calendar-duplicates"));
    let session = calendar_payload(
        "Asia/Seoul",
        "source",
        "09:00:00",
        "15:30:00",
        r#"[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]"#,
        "[]",
    );
    store_files(
        &store,
        &[
            (ResponseKind::Calendar, "first.json", session.clone()),
            (ResponseKind::Calendar, "second.json", session.clone()),
        ],
    );
    let reopened_duplicate = reopen_manifest(&store);
    let publication =
        PublicationBundle::from_raw(&store, &reopened_duplicate).expect("duplicates dedupe");
    assert_eq!(publication.calendar_facts.len(), 1);
    assert_eq!(
        publication.calendar_facts[0].content_sha256,
        reopened_duplicate.files[0]
            .content_hash
            .as_str()
            .strip_prefix("sha256:")
            .unwrap()
    );

    let mut semantically_identical_different_bytes = session.clone();
    semantically_identical_different_bytes.push(b' ');
    let different_bytes = store_files(
        &store,
        &[
            (ResponseKind::Calendar, "original.json", session.clone()),
            (
                ResponseKind::Calendar,
                "whitespace.json",
                semantically_identical_different_bytes,
            ),
        ],
    );
    let different_bytes_error = PublicationBundle::from_raw(&store, &different_bytes)
        .expect_err("same source version must not point at different raw bytes");
    assert!(matches!(
        different_bytes_error,
        PublicationError::ConflictingCalendarProvenance { .. }
    ));

    let conflict = store_files(
        &store,
        &[
            (
                ResponseKind::Calendar,
                "session.json",
                calendar_payload(
                    "Asia/Seoul",
                    "source",
                    "09:00:00",
                    "15:30:00",
                    r#"[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]"#,
                    "[]",
                ),
            ),
            (
                ResponseKind::Calendar,
                "holiday.json",
                calendar_payload(
                    "Asia/Seoul",
                    "source",
                    "09:00:00",
                    "15:30:00",
                    "[]",
                    r#"[{"date":"2020-01-31"}]"#,
                ),
            ),
        ],
    );
    let error = PublicationBundle::from_raw(&store, &conflict).expect_err("conflict must fail");
    assert!(matches!(
        error,
        PublicationError::ConflictingCalendarProvenance { .. }
    ));
}

#[test]
fn publication_rejects_non_krx_manifest_scope_before_raw_path_access() {
    let store = RawStore::new(temp_root("publication-scope"));
    let mut entry = store_files(
        &store,
        &[(ResponseKind::Reference, "reference.json", b"{}".to_vec())],
    );
    entry.market = "other-market".to_owned();

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("publication scope must be constrained before raw reads");

    assert!(matches!(
        error,
        PublicationError::UnsupportedManifestScope { .. }
    ));
}

#[test]
fn from_raw_accepts_the_credentialed_kis_normalized_scope_and_reads_that_scope() {
    let store = RawStore::new(temp_root("publication-kis-normalized"));
    let entry = normalized_fixture(&store, FetchMode::Credentialed);

    let publication = PublicationBundle::from_raw(&store, &entry).expect("normalized publication");

    assert_eq!(publication.provider, PROVIDER_KIS_NORMALIZED);
    assert_eq!(publication.market, MARKET_KR);
    assert_eq!(publication.fetch_mode, FetchMode::Credentialed);
    assert_eq!(publication.files.len(), 4);
    assert_eq!(publication.files[0].kind, DataBatchKind::Eod);
    assert_eq!(publication.files[0].file_name, "bars.json");
    assert_eq!(publication.calendar_facts.len(), 1);
    assert_eq!(publication.calendar_facts[0].source, "kis");
}

#[test]
fn from_raw_accepts_a_normalized_closed_target_as_eod_unavailable() {
    let store = RawStore::new(temp_root("publication-kis-normalized-holiday"));
    let lineage = normalized_lineage(BatchId::generate());
    let mut files = normalized_files(&lineage);
    files[0].2 = with_lineage(
        json!({
            "dataset_id":"kr-etf-daily-2020-01-31",
            "schema_version":1,
            "currency":"KRW",
            "instruments":[],
            "bars":[]
        }),
        &lineage,
    );
    files[2].2 = with_lineage(
        json!({
            "calendar_id":"kis-chk-holiday-v1",
            "schema_version":1,
            "source":"kis",
            "timezone":"Asia/Seoul",
            "session_times_local":{"open":"09:00:00","close":"15:30:00"},
            "sessions":[],
            "holidays":[{"date":"2020-01-31"}]
        }),
        &lineage,
    );
    let entry = store_normalized_files(&store, &files, &lineage, FetchMode::Credentialed);

    let publication = PublicationBundle::from_raw(&store, &entry).expect("holiday publication");

    assert_eq!(publication.files[0].kind, DataBatchKind::EodUnavailable);
    assert_eq!(publication.calendar_facts.len(), 1);
    assert_eq!(
        publication.calendar_facts[0].session_type,
        CalendarSessionType::Closed
    );
}

#[test]
fn from_raw_rejects_wire_kis_scope_before_readback() {
    let store = RawStore::new(temp_root("publication-kis-wire"));
    let entry = store_files_with_scope(
        &store,
        &[(ResponseKind::Reference, "reference.json", b"{}".to_vec())],
        PROVIDER_KIS,
        FetchMode::Credentialed,
    );

    let error =
        PublicationBundle::from_raw(&store, &entry).expect_err("wire KIS is not publishable");

    assert!(matches!(
        error,
        PublicationError::UnsupportedManifestScope { .. }
    ));
}

#[test]
fn from_raw_rejects_synthetic_normalized_scope() {
    let store = RawStore::new(temp_root("publication-kis-normalized-mode"));
    let entry = normalized_fixture(&store, FetchMode::Synthetic);

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("normalized scope must be credentialed");

    assert!(matches!(
        error,
        PublicationError::UnsupportedManifestMode {
            expected: FetchMode::Credentialed,
            actual: FetchMode::Synthetic,
            ..
        }
    ));
}

#[test]
fn from_raw_rejects_noncanonical_normalized_file_shape() {
    let store = RawStore::new(temp_root("publication-kis-normalized-shape"));
    let lineage = normalized_lineage(BatchId::generate());
    let mut files = normalized_files(&lineage);
    files[0].1 = "bars-response.json";
    let entry = store_normalized_files(&store, &files, &lineage, FetchMode::Credentialed);

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("normalized scope must use canonical file names");

    assert!(matches!(
        error,
        PublicationError::NonCanonicalNormalizedManifest { .. }
    ));
}

#[test]
fn from_raw_rejects_missing_normalized_lineage_as_typed_permanent_error() {
    let store = RawStore::new(temp_root("publication-kis-normalized-missing-lineage"));
    let lineage = normalized_lineage(BatchId::generate());
    let mut files = normalized_files(&lineage);
    let mut bars: Value = serde_json::from_slice(&files[0].2).expect("bars JSON");
    bars.as_object_mut()
        .expect("bars object")
        .remove("_lineage");
    files[0].2 = serde_json::to_vec(&bars).expect("bars without lineage");
    let entry = store_normalized_files(&store, &files, &lineage, FetchMode::Credentialed);

    let error =
        PublicationBundle::from_raw(&store, &entry).expect_err("missing lineage must fail closed");

    assert!(matches!(
        error,
        PublicationError::InvalidCanonicalProvenance { file_name, .. }
            if file_name == "bars.json"
    ));
}

#[test]
fn from_raw_rejects_normalized_request_lineage_mismatch() {
    let store = RawStore::new(temp_root("publication-kis-normalized-request-lineage"));
    let mut entry = normalized_fixture(&store, FetchMode::Credentialed);
    let bars = entry
        .files
        .iter_mut()
        .find(|file| file.kind == ResponseKind::Bars)
        .expect("bars manifest file");
    let lineage_query = bars
        .request
        .query
        .iter_mut()
        .find(|(key, _)| key == "upstream_lineage")
        .expect("lineage query");
    lineage_query.1.push_str("-mismatch");

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("request lineage mismatch must fail closed");

    assert!(matches!(
        error,
        PublicationError::InvalidCanonicalProvenance { file_name, .. }
            if file_name == "bars.json"
    ));
}

#[test]
fn from_raw_keeps_hash_verified_readback_for_normalized_scope() {
    let store = RawStore::new(temp_root("publication-kis-normalized-hash"));
    let entry = normalized_fixture(&store, FetchMode::Credentialed);
    fs::write(
        store
            .batch_dir(
                PROVIDER_KIS_NORMALIZED,
                MARKET_KR,
                &entry.date,
                &entry.batch_id,
            )
            .join("bars.json"),
        b"tampered",
    )
    .expect("tamper fixture");

    let error = PublicationBundle::from_raw(&store, &entry).expect_err("tampering must fail");

    assert!(matches!(
        error,
        PublicationError::Store(StoreError::ContentHashMismatch { .. })
    ));
}

#[test]
fn calendar_source_version_cannot_span_different_raw_bytes_for_disjoint_dates() {
    let store = RawStore::new(temp_root("calendar-provenance-across-dates"));
    let entry = store_files(
        &store,
        &[
            (
                ResponseKind::Calendar,
                "first.json",
                calendar_payload(
                    "Asia/Seoul",
                    "source",
                    "09:00:00",
                    "15:30:00",
                    r#"[{"date":"2020-01-30","open_utc":"2020-01-30T00:00:00Z","close_utc":"2020-01-30T06:30:00Z"}]"#,
                    "[]",
                ),
            ),
            (
                ResponseKind::Calendar,
                "second.json",
                calendar_payload(
                    "Asia/Seoul",
                    "source",
                    "09:00:00",
                    "15:30:00",
                    r#"[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]"#,
                    "[]",
                ),
            ),
        ],
    );

    let error = PublicationBundle::from_raw(&store, &entry)
        .expect_err("one source version must not silently refer to different raw bytes");

    assert!(matches!(
        error,
        PublicationError::ConflictingCalendarProvenance { .. }
    ));
}

#[test]
fn calendar_facts_are_content_addressed_and_canonically_ordered() {
    let store = RawStore::new(temp_root("calendar-ordering"));
    let date_31 = r#"[{"date":"2020-01-31","open_utc":"2020-01-31T00:00:00Z","close_utc":"2020-01-31T06:30:00Z"}]"#;
    let date_30 = r#"[{"date":"2020-01-30","open_utc":"2020-01-30T00:00:00Z","close_utc":"2020-01-30T06:30:00Z"}]"#;
    store_files(
        &store,
        &[
            (
                ResponseKind::Calendar,
                "z.json",
                calendar_payload_with_id("z-version", date_31),
            ),
            (
                ResponseKind::Calendar,
                "a.json",
                calendar_payload_with_id("a-version", date_31),
            ),
            (
                ResponseKind::Calendar,
                "earlier.json",
                calendar_payload_with_id("earlier-version", date_30),
            ),
        ],
    );
    let reopened = reopen_manifest(&store);

    let publication = PublicationBundle::from_raw(&store, &reopened).expect("publication");

    assert_eq!(DataBatchKind::Eod.as_db_str(), "EOD");
    assert_eq!(DataBatchKind::EodUnavailable.as_db_str(), "EOD_UNAVAILABLE");
    assert_eq!(DataBatchKind::Reference.as_db_str(), "REFERENCE");
    assert_eq!(DataBatchKind::Calendar.as_db_str(), "CALENDAR");
    assert_eq!(
        DataBatchKind::CorporateActions.as_db_str(),
        "CORPORATE_ACTIONS"
    );
    assert_eq!(CalendarSessionType::Trading.as_db_str(), "TRADING");
    assert_eq!(CalendarSessionType::Closed.as_db_str(), "CLOSED");
    assert_eq!(
        publication
            .calendar_facts
            .iter()
            .map(|fact| (fact.session_date.to_iso(), fact.source_version.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                "2020-01-30".to_owned(),
                "earlier-version:schema-1".to_owned()
            ),
            ("2020-01-31".to_owned(), "a-version:schema-1".to_owned()),
            ("2020-01-31".to_owned(), "z-version:schema-1".to_owned()),
        ]
    );
}
