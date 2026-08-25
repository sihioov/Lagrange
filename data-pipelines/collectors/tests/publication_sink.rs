mod common;

use std::collections::BTreeMap;
use std::error::Error as _;

use collectors::{
    PostgresPublicationSink, PublicationSink, PublicationState, PublishOutcome, SinkError,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::FetchMode;
use market_data::publication::{CalendarSessionType, DataBatchKind};
use sqlx::Row;

use common::{ScratchDb, credentialed_normalized_bundle, synthetic_bundle};

fn assert_conflict(error: SinkError) {
    assert!(matches!(error, SinkError::Conflict(_)), "{error:?}");
    assert!(!error.is_retryable());
}

fn set_calendar_hash(bundle: &mut market_data::publication::PublicationBundle, hash: String) {
    bundle
        .files
        .iter_mut()
        .find(|file| file.kind == DataBatchKind::Calendar)
        .expect("calendar publication file")
        .content_sha256 = hash.clone();
    for fact in &mut bundle.calendar_facts {
        fact.content_sha256 = hash.clone();
    }
}

async fn counts(db: &ScratchDb) -> (i64, i64, i64) {
    (
        sqlx::query_scalar("SELECT count(*) FROM data_batches")
            .fetch_one(&db.supervisor)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
            .fetch_one(&db.supervisor)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT count(*) FROM trading_calendars")
            .fetch_one(&db.supervisor)
            .await
            .unwrap(),
    )
}

async fn seed_batch_rows(db: &ScratchDb, bundle: &market_data::publication::PublicationBundle) {
    for file in &bundle.files {
        sqlx::query(
            "INSERT INTO data_batches \
             (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, \
              source_batch_id, source_file_name, fetch_mode) \
             VALUES ('KRX','KR',$1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(bundle.target_date.as_naive_date())
        .bind(file.kind.as_db_str())
        .bind(&file.storage_path)
        .bind(&file.content_sha256)
        .bind(file.bytes_size as i64)
        .bind(bundle.retrieved_at.as_datetime())
        .bind(bundle.source_batch_id.as_uuid())
        .bind(&file.file_name)
        .bind(bundle.fetch_mode.as_str())
        .execute(&db.supervisor)
        .await
        .unwrap();
    }
}

#[test]
fn public_sink_contract_is_exposed_and_pool_errors_are_retryable() {
    fn assert_sink<T: PublicationSink>() {}
    let _ = assert_sink::<PostgresPublicationSink>;
    let error = SinkError::from_sqlx(sqlx::Error::PoolClosed);
    assert!(error.is_retryable());
}

#[tokio::test]
async fn publishes_verified_bundle_with_exact_lineage_and_reports_state_and_eod() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());
    assert_eq!(
        sink.publication_state(fixture.bundle.source_batch_id)
            .await
            .unwrap(),
        PublicationState::Missing
    );

    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::Published
    );
    assert_eq!(
        sink.publication_state(fixture.bundle.source_batch_id)
            .await
            .unwrap(),
        PublicationState::Complete
    );
    assert!(
        sink.has_eod(TradingDate::parse("2020-01-31").unwrap())
            .await
            .unwrap()
    );
    let lookup_index: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(&db.supervisor)
    .await
    .unwrap();
    assert_eq!(
        lookup_index.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
    );

    let rows = sqlx::query(
        "SELECT provider, market, batch_date, kind, storage_path, content_sha256, \
                bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode \
         FROM data_batches ORDER BY source_file_name",
    )
    .fetch_all(&db.supervisor)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4);
    let expected: BTreeMap<_, _> = fixture
        .bundle
        .files
        .iter()
        .map(|file| (file.file_name.as_str(), file))
        .collect();
    for row in rows {
        let file_name: String = row.get("source_file_name");
        let file = expected[file_name.as_str()];
        assert_eq!(row.get::<String, _>("provider"), "KRX");
        assert_eq!(row.get::<String, _>("market"), "KR");
        assert_eq!(
            row.get::<chrono::NaiveDate, _>("batch_date"),
            fixture.bundle.target_date.as_naive_date()
        );
        assert_eq!(row.get::<String, _>("kind"), file.kind.as_db_str());
        assert_eq!(row.get::<String, _>("storage_path"), file.storage_path);
        assert_eq!(row.get::<String, _>("content_sha256"), file.content_sha256);
        assert_eq!(row.get::<i64, _>("bytes_size"), file.bytes_size as i64);
        assert_eq!(
            row.get::<chrono::DateTime<chrono::Utc>, _>("retrieved_at"),
            fixture.bundle.retrieved_at.as_datetime()
        );
        assert_eq!(
            row.get::<uuid::Uuid, _>("source_batch_id"),
            fixture.bundle.source_batch_id.as_uuid()
        );
        assert_eq!(file_name, file.file_name);
        assert_eq!(row.get::<String, _>("fetch_mode"), "synthetic");
    }
    let history_count: i64 = sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
        .fetch_one(&db.supervisor)
        .await
        .unwrap();
    let projection_count: i64 = sqlx::query_scalar("SELECT count(*) FROM trading_calendars")
        .fetch_one(&db.supervisor)
        .await
        .unwrap();
    assert_eq!(history_count as usize, fixture.bundle.calendar_facts.len());
    assert_eq!(
        projection_count as usize,
        fixture.bundle.calendar_facts.len()
    );
    let fact = fixture
        .bundle
        .calendar_facts
        .first()
        .expect("calendar fact");
    type PersistedCalendarFact = (
        String,
        chrono::NaiveDate,
        String,
        String,
        String,
        String,
        uuid::Uuid,
        String,
        chrono::DateTime<chrono::Utc>,
    );
    let history: PersistedCalendarFact = sqlx::query_as(
        "SELECT exchange, session_date, session_type, timezone, source, source_version, \
                source_batch_id, content_sha256, retrieved_at \
         FROM trading_calendar_versions \
         WHERE exchange=$1 AND session_date=$2 AND source_version=$3",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .bind(&fact.source_version)
    .fetch_one(&db.supervisor)
    .await
    .unwrap();
    let projection: PersistedCalendarFact = sqlx::query_as(
        "SELECT exchange, session_date, session_type, timezone, source, source_version, \
                source_batch_id, content_sha256, retrieved_at \
         FROM trading_calendars WHERE exchange=$1 AND session_date=$2",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .fetch_one(&db.supervisor)
    .await
    .unwrap();
    for persisted in [&history, &projection] {
        assert_eq!(persisted.0, fact.exchange);
        assert_eq!(persisted.1, fact.session_date.as_naive_date());
        assert_eq!(persisted.2, fact.session_type.as_db_str());
        assert_eq!(persisted.3, fact.timezone);
        assert_eq!(persisted.4, fact.source);
        assert_eq!(persisted.5, fact.source_version);
        assert_eq!(persisted.6, fixture.bundle.source_batch_id.as_uuid());
        assert_eq!(persisted.7, fact.content_sha256);
        assert_eq!(persisted.8, fixture.bundle.retrieved_at.as_datetime());
    }

    let before = counts(&db).await;
    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::AlreadyPublished
    );
    assert_eq!(counts(&db).await, before);
    db.drop_db().await;
}

#[tokio::test]
async fn publishes_credentialed_kis_normalized_scope_and_replays_idempotently() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = credentialed_normalized_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());

    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::Published
    );
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT provider, market, fetch_mode FROM data_batches ORDER BY source_file_name",
    )
    .fetch_all(&db.supervisor)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|(provider, market, mode)| {
        provider == "KRX" && market == "KR" && mode == "credentialed"
    }));

    let before = counts(&db).await;
    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::AlreadyPublished
    );
    assert_eq!(counts(&db).await, before);
    db.drop_db().await;
}

#[tokio::test]
async fn fractional_retrieval_time_replays_at_durable_manifest_precision() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let mut fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    fixture.bundle.retrieved_at =
        UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00.123456789Z").unwrap();
    let sink = PostgresPublicationSink::new(db.writer.clone());
    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::Published
    );
    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::AlreadyPublished
    );

    let expected = UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z")
        .unwrap()
        .as_datetime();
    let batch_times: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT retrieved_at FROM data_batches")
            .fetch_all(&db.supervisor)
            .await
            .unwrap();
    assert_eq!(batch_times.len(), 4);
    assert!(
        batch_times.iter().all(|value| *value == expected),
        "stored batch times: {batch_times:?}"
    );
    let history_times: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT retrieved_at FROM trading_calendar_versions")
            .fetch_all(&db.supervisor)
            .await
            .unwrap();
    assert!(!history_times.is_empty());
    assert!(
        history_times.iter().all(|value| *value == expected),
        "stored history times: {history_times:?}"
    );
    let projection_times: Vec<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT retrieved_at FROM trading_calendars")
            .fetch_all(&db.supervisor)
            .await
            .unwrap();
    assert!(!projection_times.is_empty());
    assert!(
        projection_times.iter().all(|value| *value == expected),
        "stored projection times: {projection_times:?}"
    );

    // Older releases persisted PostgreSQL's microsecond precision even though
    // batch.json serialized the same UtcTimestamp at whole-second precision.
    // Simulate those existing rows, then replay the timestamp reconstructed
    // from the immutable manifest. The content and lineage still match, so
    // the subsecond-only legacy difference must remain idempotent.
    let legacy = UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00.123456Z")
        .unwrap()
        .as_datetime();
    for statement in [
        "UPDATE data_batches SET retrieved_at=$1",
        "UPDATE trading_calendars SET retrieved_at=$1",
    ] {
        sqlx::query(statement)
            .bind(legacy)
            .execute(&db.supervisor)
            .await
            .unwrap();
    }
    fixture.bundle.retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-05T07:00:00Z").unwrap();
    assert_eq!(
        sink.publish(&fixture.bundle).await.unwrap(),
        PublishOutcome::AlreadyPublished
    );
    db.drop_db().await;
}

#[tokio::test]
async fn publication_state_marks_every_nonzero_noncanonical_shape_partial() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let sink = PostgresPublicationSink::new(db.writer.clone());
    let partial = BatchId::generate();
    let duplicate = BatchId::generate();
    let unexpected = BatchId::generate();
    for (batch, kinds) in [
        (partial, vec!["EOD"]),
        (duplicate, vec!["EOD", "REFERENCE", "REFERENCE", "CALENDAR"]),
        (unexpected, vec!["EOD", "REFERENCE", "CALENDAR", "MYSTERY"]),
    ] {
        for (index, kind) in kinds.into_iter().enumerate() {
            sqlx::query(
                "INSERT INTO data_batches \
                 (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, \
                  retrieved_at, source_batch_id, source_file_name, fetch_mode) \
                 VALUES ('KRX', 'KR', '2020-01-31', $1, $2, $3, 1, '2026-08-05T07:00:00Z', $4, $5, 'synthetic')",
            )
            .bind(kind)
            .bind(format!("seed/{batch}/{index}"))
            .bind(format!("{index:064x}"))
            .bind(batch.as_uuid())
            .bind(format!("file-{index}.json"))
            .execute(&db.supervisor)
            .await
            .unwrap();
        }
    }
    for batch in [partial, duplicate, unexpected] {
        assert_eq!(
            sink.publication_state(batch).await.unwrap(),
            PublicationState::Partial
        );
    }
    db.drop_db().await;
}

#[tokio::test]
async fn conflicting_replays_and_preseeded_partial_state_are_never_repaired() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&fixture.bundle).await.unwrap();
    let baseline = counts(&db).await;

    let mut mutations = Vec::new();
    let mut changed = fixture.bundle.clone();
    changed.files[0].kind = DataBatchKind::Reference;
    mutations.push(changed);
    let mut changed = fixture.bundle.clone();
    changed.files[0].content_sha256 = "f".repeat(64);
    mutations.push(changed);
    let mut changed = fixture.bundle.clone();
    changed.files[0].storage_path.push_str(".changed");
    mutations.push(changed);
    let mut changed = fixture.bundle.clone();
    changed.files[0].bytes_size += 1;
    mutations.push(changed);
    let mut changed = fixture.bundle.clone();
    changed.fetch_mode = FetchMode::Credentialed;
    mutations.push(changed);
    let mut changed = fixture.bundle.clone();
    changed.target_date = TradingDate::parse("2020-02-01").unwrap();
    mutations.push(changed);
    let mut changed = fixture.bundle.clone();
    changed.retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-05T07:00:01Z").unwrap();
    mutations.push(changed);
    for mutation in mutations {
        assert_conflict(sink.publish(&mutation).await.unwrap_err());
        assert_eq!(counts(&db).await, baseline);
    }

    let partial_fixture = synthetic_bundle("2026-08-06T07:00:00Z");
    let file = &partial_fixture.bundle.files[0];
    sqlx::query(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, \
          source_batch_id, source_file_name, fetch_mode) \
         VALUES ('KRX','KR',$1,$2,$3,$4,$5,$6,$7,$8,'synthetic')",
    )
    .bind(partial_fixture.bundle.target_date.as_naive_date())
    .bind(file.kind.as_db_str())
    .bind(&file.storage_path)
    .bind(&file.content_sha256)
    .bind(file.bytes_size as i64)
    .bind(partial_fixture.bundle.retrieved_at.as_datetime())
    .bind(partial_fixture.bundle.source_batch_id.as_uuid())
    .bind(&file.file_name)
    .execute(&db.supervisor)
    .await
    .unwrap();
    let partial_baseline = counts(&db).await;
    assert_conflict(sink.publish(&partial_fixture.bundle).await.unwrap_err());
    assert_eq!(counts(&db).await, partial_baseline);
    db.drop_db().await;
}

#[tokio::test]
async fn calendar_history_is_immutable_and_projection_is_newer_wins() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let original = synthetic_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&original.bundle).await.unwrap();
    let target = original.bundle.calendar_facts[0].session_date;

    let mut later = original.bundle.clone();
    later.source_batch_id = BatchId::generate();
    later.retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-06T07:00:00Z").unwrap();
    for fact in &mut later.calendar_facts {
        fact.source_version.push_str(":correction");
        fact.session_type = CalendarSessionType::Closed;
    }
    set_calendar_hash(&mut later, "a".repeat(64));
    assert_eq!(
        sink.publish(&later).await.unwrap(),
        PublishOutcome::Published
    );
    let projection: (String, String, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT session_type, source_version, retrieved_at FROM trading_calendars \
         WHERE exchange='KRX' AND session_date=$1",
    )
    .bind(target.as_naive_date())
    .fetch_one(&db.supervisor)
    .await
    .unwrap();
    assert_eq!(projection.0, "CLOSED");
    assert!(projection.1.ends_with(":correction"));
    assert_eq!(projection.2, later.retrieved_at.as_datetime());

    let history_after_later = counts(&db).await.1;
    let mut older = original.bundle.clone();
    older.source_batch_id = BatchId::generate();
    older.retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-04T07:00:00Z").unwrap();
    for fact in &mut older.calendar_facts {
        fact.source_version.push_str(":older");
    }
    set_calendar_hash(&mut older, "b".repeat(64));
    sink.publish(&older).await.unwrap();
    assert!(counts(&db).await.1 > history_after_later);
    let still_later: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT retrieved_at FROM trading_calendars WHERE exchange='KRX' AND session_date=$1",
    )
    .bind(target.as_naive_date())
    .fetch_one(&db.supervisor)
    .await
    .unwrap();
    assert_eq!(still_later, later.retrieved_at.as_datetime());
    db.drop_db().await;
}

#[tokio::test]
async fn calendar_equal_time_and_source_version_conflicts_roll_back_the_whole_batch() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let original = synthetic_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&original.bundle).await.unwrap();

    let mut refetch = original.bundle.clone();
    refetch.source_batch_id = BatchId::generate();
    assert_eq!(
        sink.publish(&refetch).await.unwrap(),
        PublishOutcome::Published
    );
    let history_after_refetch = counts(&db).await.1;
    assert_eq!(
        history_after_refetch as usize,
        original.bundle.calendar_facts.len()
    );

    let baseline = counts(&db).await;
    let mut equal_conflict = original.bundle.clone();
    equal_conflict.source_batch_id = BatchId::generate();
    for fact in &mut equal_conflict.calendar_facts {
        fact.source_version.push_str(":equal-conflict");
        fact.session_type = CalendarSessionType::Closed;
    }
    set_calendar_hash(&mut equal_conflict, "c".repeat(64));
    assert_conflict(sink.publish(&equal_conflict).await.unwrap_err());
    assert_eq!(counts(&db).await, baseline);

    let mut version_conflict = original.bundle.clone();
    version_conflict.source_batch_id = BatchId::generate();
    set_calendar_hash(&mut version_conflict, "d".repeat(64));
    assert_conflict(sink.publish(&version_conflict).await.unwrap_err());
    assert_eq!(counts(&db).await, baseline);
    db.drop_db().await;
}

/// A per-date calendar source publishes a different document every day under
/// one unchanging source version, and every day after the first must still
/// publish.
///
/// KIS `chk-holiday` is normalized to the date requested, while the version
/// string is built from `calendar_id` and `schema_version` alone -- both
/// constant. The cross-date rule that used to live in the sink read that as one
/// source version claiming two documents and refused, so the pipeline could
/// publish exactly one session and no more. The sibling test above pins what
/// survives: the same date coming back with different bytes is still a
/// conflict.
#[tokio::test]
async fn one_source_version_may_span_dates_that_each_have_their_own_document() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let original = synthetic_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&original.bundle).await.unwrap();
    let first_version = original.bundle.calendar_facts[0].source_version.clone();
    let already_published: Vec<_> = original
        .bundle
        .calendar_facts
        .iter()
        .map(|fact| fact.session_date)
        .collect();

    // The next day: same source version, a session this generation has not
    // recorded yet, and necessarily a different document hash.
    let next_day = TradingDate::parse("2020-02-04").expect("next session");
    assert!(!already_published.contains(&next_day));
    let mut following = original.bundle.clone();
    following.source_batch_id = BatchId::generate();
    following.calendar_facts.truncate(1);
    following.calendar_facts[0].session_date = next_day;
    following.calendar_facts[0].session_type = CalendarSessionType::Trading;
    assert_eq!(following.calendar_facts[0].source_version, first_version);
    set_calendar_hash(&mut following, "e".repeat(64));

    assert_eq!(
        sink.publish(&following).await.unwrap(),
        PublishOutcome::Published,
        "a second session under one source version must publish"
    );
    let persisted: String = sqlx::query_scalar(
        "SELECT content_sha256 FROM trading_calendar_versions \
         WHERE exchange='KRX' AND session_date=$1 AND source_version=$2",
    )
    .bind(next_day.as_naive_date())
    .bind(&first_version)
    .fetch_one(&db.supervisor)
    .await
    .expect("second session calendar history");
    assert_eq!(persisted, "e".repeat(64));
    db.drop_db().await;
}

#[tokio::test]
async fn calendar_facts_must_be_anchored_to_the_calendar_file_hash() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let mut fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    fixture.bundle.calendar_facts[0].content_sha256 = "e".repeat(64);
    let sink = PostgresPublicationSink::new(db.writer.clone());
    let error = sink.publish(&fixture.bundle).await.unwrap_err();
    assert!(matches!(error, SinkError::Invariant(_)), "{error:?}");
    assert_eq!(counts(&db).await, (0, 0, 0));
    db.drop_db().await;
}

#[tokio::test]
async fn replay_with_missing_calendar_evidence_conflicts_instead_of_repairing() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    seed_batch_rows(&db, &fixture.bundle).await;
    let sink = PostgresPublicationSink::new(db.writer.clone());
    assert_conflict(sink.publish(&fixture.bundle).await.unwrap_err());
    assert_eq!(counts(&db).await, (4, 0, 0));

    for fact in &fixture.bundle.calendar_facts {
        sqlx::query(
            "INSERT INTO trading_calendar_versions \
             (exchange, session_date, session_type, timezone, source, source_version, \
              source_batch_id, content_sha256, retrieved_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&fact.exchange)
        .bind(fact.session_date.as_naive_date())
        .bind(fact.session_type.as_db_str())
        .bind(&fact.timezone)
        .bind(&fact.source)
        .bind(&fact.source_version)
        .bind(fixture.bundle.source_batch_id.as_uuid())
        .bind(&fact.content_sha256)
        .bind(fixture.bundle.retrieved_at.as_datetime())
        .execute(&db.supervisor)
        .await
        .unwrap();
    }
    let before = counts(&db).await;
    assert_conflict(sink.publish(&fixture.bundle).await.unwrap_err());
    assert_eq!(counts(&db).await, before);
    assert_eq!(before.2, 0);
    db.drop_db().await;
}

#[tokio::test]
async fn replay_rejects_a_newer_projection_without_matching_history() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&fixture.bundle).await.unwrap();
    let fact = &fixture.bundle.calendar_facts[0];
    sqlx::query(
        "UPDATE trading_calendars SET source_version='unsupported-v2', content_sha256=$3, \
         retrieved_at='2026-08-06T07:00:00Z' WHERE exchange=$1 AND session_date=$2",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .bind("9".repeat(64))
    .execute(&db.supervisor)
    .await
    .unwrap();
    let before = counts(&db).await;
    assert_conflict(sink.publish(&fixture.bundle).await.unwrap_err());
    assert_eq!(counts(&db).await, before);
    db.drop_db().await;
}

#[tokio::test]
async fn verified_publication_advances_a_legacy_projection() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let fact = &fixture.bundle.calendar_facts[0];
    sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version) \
         VALUES ($1,$2,'CLOSED','Asia/Seoul','legacy','legacy-v1')",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .execute(&db.supervisor)
    .await
    .unwrap();
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&fixture.bundle).await.unwrap();
    let projection: (String, uuid::Uuid, String, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT source_version, source_batch_id, content_sha256, retrieved_at \
             FROM trading_calendars WHERE exchange=$1 AND session_date=$2",
    )
    .bind(&fact.exchange)
    .bind(fact.session_date.as_naive_date())
    .fetch_one(&db.supervisor)
    .await
    .unwrap();
    assert_eq!(projection.0, fact.source_version);
    assert_eq!(projection.1, fixture.bundle.source_batch_id.as_uuid());
    assert_eq!(projection.2, fact.content_sha256);
    assert_eq!(projection.3, fixture.bundle.retrieved_at.as_datetime());
    db.drop_db().await;
}

#[tokio::test]
async fn concurrent_different_batches_leave_the_newest_projection() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let mut older = fixture.bundle.clone();
    older.source_batch_id = BatchId::generate();
    older.retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-04T07:00:00Z").unwrap();
    for fact in &mut older.calendar_facts {
        fact.source_version.push_str(":older-concurrent");
    }
    set_calendar_hash(&mut older, "1".repeat(64));
    let mut newer = fixture.bundle.clone();
    newer.source_batch_id = BatchId::generate();
    newer.retrieved_at = UtcTimestamp::parse_rfc3339("2026-08-06T07:00:00Z").unwrap();
    for fact in &mut newer.calendar_facts {
        fact.source_version.push_str(":newer-concurrent");
        fact.session_type = CalendarSessionType::Closed;
    }
    set_calendar_hash(&mut newer, "2".repeat(64));

    let left = PostgresPublicationSink::new(db.writer.clone());
    let right = PostgresPublicationSink::new(db.writer.clone());
    let (a, b) = tokio::join!(left.publish(&older), right.publish(&newer));
    assert_eq!(a.unwrap(), PublishOutcome::Published);
    assert_eq!(b.unwrap(), PublishOutcome::Published);
    for fact in &newer.calendar_facts {
        let row: (String, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            "SELECT source_version, retrieved_at FROM trading_calendars \
             WHERE exchange=$1 AND session_date=$2",
        )
        .bind(&fact.exchange)
        .bind(fact.session_date.as_naive_date())
        .fetch_one(&db.supervisor)
        .await
        .unwrap();
        assert_eq!(row.0, fact.source_version);
        assert_eq!(row.1, newer.retrieved_at.as_datetime());
    }
    db.drop_db().await;
}

#[tokio::test]
/// Two publishers landing different sessions under one source version at the
/// same time must both succeed.
///
/// This used to assert the opposite: whichever lost the race got a conflict.
/// That rule assumed a source version names exactly one document, which holds
/// for a published calendar file and cannot hold for KIS `chk-holiday`, whose
/// response is normalized to the date requested while the version string is
/// built only from `calendar_id` and `schema_version`. Under the old rule the
/// pipeline published one session and refused every later one.
///
/// The advisory lock is unchanged and still serializes these two, so each lands
/// its own history row and its own projection rather than interleaving.
async fn concurrent_disjoint_dates_may_share_one_source_version() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let mut first = fixture.bundle.clone();
    first.source_batch_id = BatchId::generate();
    first.calendar_facts.truncate(1);
    first.calendar_facts[0].session_date = TradingDate::parse("2020-02-03").unwrap();
    set_calendar_hash(&mut first, "3".repeat(64));
    let mut second = fixture.bundle.clone();
    second.source_batch_id = BatchId::generate();
    second.calendar_facts.truncate(1);
    second.calendar_facts[0].session_date = TradingDate::parse("2020-02-04").unwrap();
    set_calendar_hash(&mut second, "4".repeat(64));
    assert_eq!(
        first.calendar_facts[0].source_version,
        second.calendar_facts[0].source_version
    );

    let left = PostgresPublicationSink::new(db.writer.clone());
    let right = PostgresPublicationSink::new(db.writer.clone());
    let (a, b) = tokio::join!(left.publish(&first), right.publish(&second));
    assert_eq!(
        a.expect("first disjoint session"),
        PublishOutcome::Published
    );
    assert_eq!(
        b.expect("second disjoint session"),
        PublishOutcome::Published
    );
    // Four data_batches rows per bundle, then one history row and one
    // projection per distinct session date.
    assert_eq!(counts(&db).await, (8, 2, 2));
    for (date, hash) in [("2020-02-03", "3"), ("2020-02-04", "4")] {
        let persisted: String = sqlx::query_scalar(
            "SELECT content_sha256 FROM trading_calendar_versions \
             WHERE exchange='KRX' AND session_date=$1",
        )
        .bind(TradingDate::parse(date).unwrap().as_naive_date())
        .fetch_one(&db.supervisor)
        .await
        .unwrap_or_else(|error| panic!("history for {date}: {error}"));
        assert_eq!(persisted, hash.repeat(64));
    }
    db.drop_db().await;
}

#[tokio::test]
async fn failures_roll_back_and_concurrent_same_batch_is_deterministic() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    sqlx::raw_sql(
        "CREATE FUNCTION fail_calendar_publish() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected' USING ERRCODE='40001'; END $$; \
         CREATE TRIGGER fail_calendar_publish BEFORE INSERT ON trading_calendar_versions \
         FOR EACH ROW EXECUTE FUNCTION fail_calendar_publish();",
    )
    .execute(&db.supervisor)
    .await
    .unwrap();
    let sink = PostgresPublicationSink::new(db.writer.clone());
    let failure = sink.publish(&fixture.bundle).await.unwrap_err();
    assert!(failure.is_retryable(), "{failure:?}");
    assert_eq!(counts(&db).await, (0, 0, 0));
    sqlx::raw_sql("DROP TRIGGER fail_calendar_publish ON trading_calendar_versions; DROP FUNCTION fail_calendar_publish()")
        .execute(&db.supervisor)
        .await
        .unwrap();

    let left = PostgresPublicationSink::new(db.writer.clone());
    let right = PostgresPublicationSink::new(db.writer.clone());
    let (a, b) = tokio::join!(
        left.publish(&fixture.bundle),
        right.publish(&fixture.bundle)
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    assert!(outcomes.contains(&PublishOutcome::Published));
    assert!(outcomes.contains(&PublishOutcome::AlreadyPublished));
    assert_eq!(counts(&db).await.0, 4);
    db.drop_db().await;
}

#[tokio::test]
async fn advisory_lock_timeout_is_retryable_and_writes_nothing() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    let lock_key = i64::from_be_bytes(
        fixture.bundle.source_batch_id.as_uuid().as_bytes()[..8]
            .try_into()
            .unwrap(),
    );
    let mut blocker = db.supervisor.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let timed_writer = db.writer_with_lock_timeout().await;
    let sink = PostgresPublicationSink::new(timed_writer.clone());

    let error = sink.publish(&fixture.bundle).await.unwrap_err();

    assert!(matches!(&error, SinkError::RetryableDatabase(_)));
    assert!(error.is_retryable());
    assert_eq!(error.to_string(), "retryable database failure");
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<sqlx::Error>())
        .expect("retryable error retains SQLx source");
    assert_eq!(
        source
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("55P03")
    );
    assert_eq!(counts(&db).await, (0, 0, 0));
    blocker.rollback().await.unwrap();
    timed_writer.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn database_unique_conflict_retains_a_sanitized_sqlx_source() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    sqlx::raw_sql(
        "CREATE FUNCTION fail_unique_publication() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected unique detail' USING ERRCODE='23505'; END $$; \
         CREATE TRIGGER fail_unique_publication BEFORE INSERT ON data_batches \
         FOR EACH ROW EXECUTE FUNCTION fail_unique_publication();",
    )
    .execute(&db.supervisor)
    .await
    .unwrap();
    let sink = PostgresPublicationSink::new(db.writer.clone());

    let error = sink.publish(&fixture.bundle).await.unwrap_err();

    assert!(matches!(&error, SinkError::DatabaseConflict { .. }));
    assert!(!error.is_retryable());
    assert_eq!(
        error.to_string(),
        "publication database conflict: source-file lineage is already occupied"
    );
    assert!(!error.to_string().contains("injected unique detail"));
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<sqlx::Error>())
        .expect("database conflict retains SQLx source");
    assert_eq!(
        source
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("23505")
    );
    assert_eq!(counts(&db).await, (0, 0, 0));
    db.drop_db().await;
}

#[tokio::test]
async fn database_constraint_failure_is_permanent_and_retains_its_source() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let fixture = synthetic_bundle("2026-08-05T07:00:00Z");
    sqlx::raw_sql(
        "CREATE FUNCTION fail_constraint_publication() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected constraint detail' USING ERRCODE='23514'; END $$; \
         CREATE TRIGGER fail_constraint_publication BEFORE INSERT ON data_batches \
         FOR EACH ROW EXECUTE FUNCTION fail_constraint_publication();",
    )
    .execute(&db.supervisor)
    .await
    .unwrap();
    let sink = PostgresPublicationSink::new(db.writer.clone());

    let error = sink.publish(&fixture.bundle).await.unwrap_err();

    assert!(matches!(&error, SinkError::PermanentDatabase(_)));
    assert!(!error.is_retryable());
    assert_eq!(error.to_string(), "permanent database failure");
    assert!(!error.to_string().contains("injected constraint detail"));
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<sqlx::Error>())
        .expect("permanent database error retains SQLx source");
    assert_eq!(
        source
            .as_database_error()
            .and_then(|database| database.code())
            .as_deref(),
        Some("23514")
    );
    assert_eq!(counts(&db).await, (0, 0, 0));
    db.drop_db().await;
}

#[tokio::test]
async fn research_writer_is_least_privileged_and_eod_unavailable_does_not_count() {
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let mut unavailable = synthetic_bundle("2026-08-05T07:00:00Z");
    unavailable.bundle.files[0].kind = DataBatchKind::EodUnavailable;
    let sink = PostgresPublicationSink::new(db.writer.clone());
    sink.publish(&unavailable.bundle).await.unwrap();
    assert!(!sink.has_eod(unavailable.bundle.target_date).await.unwrap());

    for statement in [
        "SELECT * FROM orders",
        "DELETE FROM data_batches",
        "TRUNCATE TABLE data_batches",
        "DELETE FROM trading_calendar_versions",
        "CREATE TABLE publication_escape(id integer)",
    ] {
        let error = sqlx::query(statement)
            .execute(&db.writer)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    db.drop_db().await;
}
