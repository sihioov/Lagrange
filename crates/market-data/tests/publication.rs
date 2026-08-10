use std::fs;

use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, RawEnvelope, RequestMetadata, ResponseKind, MARKET_KR, PROVIDER_KRX,
};
use market_data::ingest::{ingest_bundle, IngestRequest};
use market_data::provider::{KrxProvider, RecordedBundle};
use market_data::publication::{
    CalendarSessionType, DataBatchKind, PublicationBundle, PublicationError,
};
use market_data::storage::{BatchSpec, ManifestEntry, RawStore};

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

fn metadata() -> RequestMetadata {
    RequestMetadata {
        endpoint: "test".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        mode: FetchMode::Synthetic,
    }
}

fn store_files(store: &RawStore, files: &[(ResponseKind, &str, Vec<u8>)]) -> ManifestEntry {
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
                metadata(),
            )
        })
        .collect();
    store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KRX,
                market: MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: None,
                mode: FetchMode::Synthetic,
            },
            &envelopes,
        )
        .expect("store batch")
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

    let publication = PublicationBundle::from_raw(&store, &outcome.entry).expect("publication");

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
    assert!(publication.files[0]
        .content_sha256
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()));
    assert!(!publication.files[0].content_sha256.contains("sha256:"));
    assert_eq!(
        publication.files[0].storage_path,
        store
            .batch_dir(
                PROVIDER_KRX,
                MARKET_KR,
                &outcome.entry.date,
                &outcome.entry.batch_id
            )
            .join("bars-response.json")
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
    let outcome = ingest_bundle(&store, &provider, &request, None).expect("ingest");

    let publication = PublicationBundle::from_raw(&store, &outcome.entry).expect("publication");

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
}

#[test]
fn from_raw_publishes_explicit_calendar_holidays_as_closed_without_inferring_missing_dates() {
    let store = RawStore::new(temp_root("calendar-holidays"));
    let calendar = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/kr-etf/2020-01-31/calendar.json"
    ))
    .expect("calendar fixture");
    let entry = store_files(
        &store,
        &[(ResponseKind::Calendar, "calendar.json", calendar)],
    );

    let publication = PublicationBundle::from_raw(&store, &entry).expect("publication");

    assert!(publication.calendar_facts.iter().any(|fact| {
        fact.session_date.to_iso() == "2020-01-24"
            && fact.session_type == CalendarSessionType::Closed
    }));
    assert!(!publication
        .calendar_facts
        .iter()
        .any(|fact| fact.session_date.to_iso() == "2020-01-25"));
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
    let entry = store_files(
        &store,
        &[(ResponseKind::Bars, "bars.json", br#"{"bars":[]}"#.to_vec())],
    );

    let publication = PublicationBundle::from_raw(&store, &entry).expect("publication");

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

    assert!(matches!(error, PublicationError::Store(_)));
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
    let duplicate = store_files(
        &store,
        &[
            (ResponseKind::Calendar, "first.json", session.clone()),
            (ResponseKind::Calendar, "second.json", session),
        ],
    );
    let publication = PublicationBundle::from_raw(&store, &duplicate).expect("duplicates dedupe");
    assert_eq!(publication.calendar_facts.len(), 1);

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
        PublicationError::ConflictingCalendarFact { .. }
    ));
}
