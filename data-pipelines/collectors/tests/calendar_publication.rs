mod common;

use collectors::{
    CalendarPublicationSink, PostgresPublicationSink, PublicationState, PublishOutcome, SinkError,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS_CALENDAR, RawEnvelope, RequestMetadata, ResponseKind,
};
use market_data::normalize::{KIS_CALENDAR_SOURCE_VERSION, normalize_kis_calendar_batch};
use market_data::publication::CalendarPublicationBundle;
use market_data::storage::{BatchSpec, ManifestEntry, RawStore};
use sqlx::Row;

use common::ScratchDb;

const CALENDAR_PATH: &str = "/uapi/domestic-stock/v1/quotations/chk-holiday";
const CALENDAR_TR_ID: &str = "CTCA0903R";
const TARGET_DATE: &str = "2026-09-14";
const RETRIEVED_AT: &str = "2026-09-14T00:05:00Z";

fn target() -> TradingDate {
    TradingDate::parse(TARGET_DATE).expect("target date")
}

fn retrieved_at() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(RETRIEVED_AT).expect("retrieved at")
}

fn source_body(open: bool) -> Vec<u8> {
    let value = if open { "Y" } else { "N" };
    format!(r#"{{"rt_cd":"0","output":[{{"bass_dt":"20260914","opnd_yn":"{value}"}}]}}"#)
        .into_bytes()
}

fn store_calendar_source(store: &RawStore, batch_id: BatchId, open: bool) -> ManifestEntry {
    let body = source_body(open);
    let date = target();
    let at = retrieved_at();
    let envelope = RawEnvelope::new(
        batch_id,
        ResponseKind::Calendar,
        "calendar-page-01.json",
        body,
        at,
        RequestMetadata {
            endpoint: CALENDAR_PATH.to_owned(),
            query: vec![
                ("BASS_DT".to_owned(), "20260914".to_owned()),
                ("CTX_AREA_FK".to_owned(), String::new()),
                ("CTX_AREA_NK".to_owned(), String::new()),
            ],
            headers: vec![
                ("authorization".to_owned(), "[REDACTED]".to_owned()),
                ("appkey".to_owned(), "[REDACTED]".to_owned()),
                ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                ("tr_id".to_owned(), CALENDAR_TR_ID.to_owned()),
                ("tr_cont".to_owned(), String::new()),
            ],
            mode: FetchMode::Credentialed,
        },
    );
    store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KIS_CALENDAR,
                market: MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: Some("entitlement://kis-calendar-v1"),
                mode: FetchMode::Credentialed,
            },
            &[envelope],
        )
        .expect("calendar source batch")
}

fn calendar_bundle(store: &RawStore, source: &ManifestEntry) -> CalendarPublicationBundle {
    let normalized = normalize_kis_calendar_batch(store, source).expect("calendar normalization");
    CalendarPublicationBundle::from_raw(store, &normalized.entry)
        .expect("calendar publication bundle")
}

async fn counts(db: &ScratchDb) -> (i64, i64, i64) {
    (
        sqlx::query_scalar("SELECT count(*) FROM data_batches")
            .fetch_one(&db.supervisor)
            .await
            .expect("data batch count"),
        sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
            .fetch_one(&db.supervisor)
            .await
            .expect("history count"),
        sqlx::query_scalar("SELECT count(*) FROM trading_calendars")
            .fetch_one(&db.supervisor)
            .await
            .expect("projection count"),
    )
}

fn assert_conflict(error: SinkError) {
    assert!(matches!(error, SinkError::Conflict(_)), "{error:?}");
    assert!(!error.is_retryable());
}

#[test]
fn calendar_sink_contract_is_separate_from_the_eod_sink() {
    fn assert_calendar_sink<T: CalendarPublicationSink>() {}
    assert_calendar_sink::<PostgresPublicationSink>();
}

#[tokio::test]
async fn publishes_calendar_atomically_idempotently_and_rejects_wrong_history() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let temp = tempfile::tempdir().expect("raw root");
    let store = RawStore::new(temp.path().join("data"));
    let sink = PostgresPublicationSink::new(db.writer.clone());

    let first_source = store_calendar_source(&store, BatchId::generate(), true);
    let first = calendar_bundle(&store, &first_source);
    assert_eq!(
        first.calendar_facts()[0].source_version,
        KIS_CALENDAR_SOURCE_VERSION
    );
    assert_eq!(
        sink.calendar_publication_state(first.source_batch_id())
            .await
            .expect("calendar publication state"),
        PublicationState::Missing
    );
    assert_eq!(
        sink.publish_calendar(&first)
            .await
            .expect("first publication"),
        PublishOutcome::Published
    );
    assert_eq!(
        sink.calendar_publication_state(first.source_batch_id())
            .await
            .expect("calendar publication state"),
        PublicationState::Complete
    );

    let published: (String, String, String, String, i64, uuid::Uuid) = sqlx::query_as(
        "SELECT provider, market, kind, source_file_name, bytes_size, source_batch_id \
         FROM data_batches WHERE source_batch_id=$1",
    )
    .bind(first.source_batch_id().as_uuid())
    .fetch_one(&db.supervisor)
    .await
    .expect("published calendar data row");
    assert_eq!(published.0, "KRX");
    assert_eq!(published.1, "KR");
    assert_eq!(published.2, "CALENDAR");
    assert_eq!(published.3, "calendar.json");
    assert_eq!(published.4, first.file().bytes_size as i64);
    assert_eq!(published.5, first.source_batch_id().as_uuid());
    let fetch_mode: String =
        sqlx::query_scalar("SELECT fetch_mode FROM data_batches WHERE source_batch_id=$1")
            .bind(first.source_batch_id().as_uuid())
            .fetch_one(&db.supervisor)
            .await
            .expect("published fetch mode");
    assert_eq!(fetch_mode, "credentialed");

    let before_replay = counts(&db).await;
    assert_eq!(
        sink.publish_calendar(&first)
            .await
            .expect("calendar replay"),
        PublishOutcome::AlreadyPublished
    );
    assert_eq!(counts(&db).await, before_replay);

    let second_source = store_calendar_source(&store, BatchId::generate(), false);
    let second = calendar_bundle(&store, &second_source);
    assert_ne!(
        first.file().content_sha256,
        second.file().content_sha256,
        "the wrong-history fixture must have different immutable bytes"
    );
    let before_conflict = counts(&db).await;
    assert_conflict(
        sink.publish_calendar(&second)
            .await
            .expect_err("same-date wrong-history publication must fail"),
    );
    assert_eq!(counts(&db).await, before_conflict);
    assert_eq!(
        sink.calendar_publication_state(second.source_batch_id())
            .await
            .expect("wrong-history publication state"),
        PublicationState::Missing
    );

    let stored_history: (String, String, String, uuid::Uuid) = sqlx::query_as(
        "SELECT session_type, source, source_version, source_batch_id \
         FROM trading_calendar_versions WHERE exchange='KRX' AND session_date=$1",
    )
    .bind(target().as_naive_date())
    .fetch_one(&db.supervisor)
    .await
    .expect("published history");
    assert_eq!(stored_history.0, "TRADING");
    assert_eq!(stored_history.1, "kis");
    assert_eq!(stored_history.2, KIS_CALENDAR_SOURCE_VERSION);
    assert_eq!(stored_history.3, first.source_batch_id().as_uuid());
    db.drop_db().await;
}
