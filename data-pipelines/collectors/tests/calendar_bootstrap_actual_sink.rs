mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Timelike, Utc};
use collectors::calendar_bootstrap::{
    CalendarSourceMode, run_calendar_once_with_postgres_pool_for_test,
};
use collectors::{current_kst_date, ensure_kis_calendar_source};
use domain::UtcTimestamp;
use kis_client::{KisError, MarketDataReply};
use market_data::contract::{MARKET_KR, PROVIDER_KIS_CALENDAR};
use market_data::ingest::IngestRequest;
use market_data::normalize::KIS_CALENDAR_SOURCE_VERSION;
use market_data::providers::kis::{KisProvider, KisRead};
use market_data::storage::RawStore;
use uuid::Uuid;

use common::ScratchDb;

#[derive(Debug, Clone)]
struct CalendarFixtureReader {
    calls: Arc<AtomicUsize>,
}

impl KisRead for CalendarFixtureReader {
    async fn get(
        &self,
        _path: &str,
        _tr_id: &str,
        query: &[(String, String)],
        _continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let date = query
            .iter()
            .find(|(key, _)| key == "BASS_DT")
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        Ok(MarketDataReply {
            body: format!(r#"{{"rt_cd":"0","output":[{{"bass_dt":"{date}","opnd_yn":"Y"}}]}}"#)
                .into_bytes(),
            continuation: None,
        })
    }
}

fn source_bytes(store: &RawStore, source: &market_data::ManifestEntry) -> Vec<Vec<u8>> {
    store
        .read_batch_bytes(PROVIDER_KIS_CALENDAR, MARKET_KR, source)
        .expect("source bytes")
        .into_iter()
        .map(|file| file.bytes)
        .collect()
}

async fn exercise_actual_sink(db: &ScratchDb) -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| format!("raw tempdir: {error}"))?;
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(data_root.join("raw"))
        .map_err(|error| format!("raw directory: {error}"))?;
    let store = RawStore::new(&data_root);
    let date = current_kst_date(Utc::now());
    let entitlement = "entitlement://calendar-bootstrap-actual-sink-test";
    let retrieved_at = (Utc::now() - chrono::Duration::seconds(5))
        .with_nanosecond(0)
        .ok_or_else(|| "failed to make whole-second retrieval time".to_owned())?;
    let request = IngestRequest::new(
        MARKET_KR.to_owned(),
        date,
        UtcTimestamp::from_datetime(retrieved_at),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = KisProvider::kr_etf_core(CalendarFixtureReader {
        calls: calls.clone(),
    });
    let source = ensure_kis_calendar_source(&store, &provider, &request, Some(entitlement))
        .await
        .map_err(|error| format!("EOD-first source: {error}"))?
        .ok_or_else(|| "EOD-first fixture did not create a dedicated source".to_owned())?;
    if calls.load(Ordering::SeqCst) != 1 {
        return Err("EOD-first fixture made an unexpected number of broker calls".to_owned());
    }

    let source_manifest_path = store.manifest_path(PROVIDER_KIS_CALENDAR, MARKET_KR);
    let source_manifest_before = std::fs::read(&source_manifest_path)
        .map_err(|error| format!("read source manifest before: {error}"))?;
    let source_bytes_before = source_bytes(&store, &source);
    let claim_path = data_root
        .join("raw/.calendar-bootstrap")
        .join(format!("{}.json", date.to_iso()));
    let claim_before =
        std::fs::read(&claim_path).map_err(|error| format!("read claim before: {error}"))?;

    let first = run_calendar_once_with_postgres_pool_for_test(
        &data_root,
        date,
        CalendarSourceMode::ReuseExisting,
        entitlement,
        db.writer.clone(),
    )
    .await
    .map_err(|error| format!("actual sink first publish: {error}"))?;
    let replay = run_calendar_once_with_postgres_pool_for_test(
        &data_root,
        date,
        CalendarSourceMode::ReuseExisting,
        entitlement,
        db.writer.clone(),
    )
    .await
    .map_err(|error| format!("actual sink reuse replay: {error}"))?;
    let explicit_replay = run_calendar_once_with_postgres_pool_for_test(
        &data_root,
        date,
        CalendarSourceMode::Explicit(source.batch_id),
        entitlement,
        db.writer.clone(),
    )
    .await
    .map_err(|error| format!("actual sink explicit replay: {error}"))?;
    let wrong = run_calendar_once_with_postgres_pool_for_test(
        &data_root,
        date,
        CalendarSourceMode::Explicit(domain::BatchId::generate()),
        entitlement,
        db.writer.clone(),
    )
    .await;
    if wrong.as_ref().err().map(|error| error.0) != Some("CALENDAR_ATTEMPT_ID_CONFLICT") {
        return Err(format!("wrong UUID was not rejected: {wrong:?}"));
    }

    if !first.reused_existing_source
        || first.source_batch_id != source.batch_id
        || first.source_batch_id != replay.source_batch_id
        || first.source_batch_id != explicit_replay.source_batch_id
        || first.normalized_batch_id != replay.normalized_batch_id
        || first.normalized_batch_id != explicit_replay.normalized_batch_id
    {
        return Err("standalone summaries did not preserve source identity".to_owned());
    }
    if calls.load(Ordering::SeqCst) != 1 {
        return Err("standalone reuse caused an additional provider call".to_owned());
    }

    let source_manifest_after = std::fs::read(&source_manifest_path)
        .map_err(|error| format!("read source manifest after: {error}"))?;
    if source_manifest_after != source_manifest_before
        || source_bytes(&store, &source) != source_bytes_before
        || std::fs::read(&claim_path).map_err(|error| format!("read claim after: {error}"))?
            != claim_before
    {
        return Err("standalone publication mutated immutable source or claim evidence".to_owned());
    }

    let canonical_batch_id = first.normalized_batch_id.as_uuid();
    let batch_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM data_batches
          WHERE provider='KRX' AND market='KR' AND kind='CALENDAR'
            AND source_batch_id=$1 AND source_file_name='calendar.json'",
    )
    .bind(canonical_batch_id)
    .fetch_one(&db.supervisor)
    .await
    .map_err(|error| format!("count published calendar batches: {error}"))?;
    let history_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM trading_calendar_versions
          WHERE exchange='KRX' AND session_date=$1 AND source_version=$2",
    )
    .bind(date.as_naive_date())
    .bind(KIS_CALENDAR_SOURCE_VERSION)
    .fetch_one(&db.supervisor)
    .await
    .map_err(|error| format!("count calendar history: {error}"))?;
    let projection_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM trading_calendars
          WHERE exchange='KRX' AND session_date=$1 AND source_version=$2",
    )
    .bind(date.as_naive_date())
    .bind(KIS_CALENDAR_SOURCE_VERSION)
    .fetch_one(&db.supervisor)
    .await
    .map_err(|error| format!("count calendar projection: {error}"))?;
    if (batch_count, history_count, projection_count) != (1, 1, 1) {
        return Err(format!(
            "replay created duplicate publication rows: batches={batch_count} history={history_count} projection={projection_count}"
        ));
    }

    let persisted: (Uuid, String, chrono::DateTime<Utc>) = sqlx::query_as(
        "SELECT source_batch_id, content_sha256, retrieved_at
           FROM data_batches
          WHERE provider='KRX' AND market='KR' AND kind='CALENDAR'
            AND source_batch_id=$1 AND source_file_name='calendar.json'",
    )
    .bind(canonical_batch_id)
    .fetch_one(&db.supervisor)
    .await
    .map_err(|error| format!("read published calendar batch: {error}"))?;
    if persisted.0 != canonical_batch_id || persisted.1.len() != 64 || persisted.2 != retrieved_at {
        return Err("published calendar batch lost canonical evidence metadata".to_owned());
    }

    // This is the same join and freshness contract used by the intraday
    // calendar resolver. It proves the production sink's rows are readable as
    // current immutable calendar proof, not merely that inserts succeeded.
    let resolver_rows: Vec<(chrono::NaiveDate, Uuid, String, chrono::DateTime<Utc>)> =
        sqlx::query_as(
            "WITH observed AS MATERIALIZED (
                    SELECT pg_catalog.clock_timestamp() AS observed_at
             )
             SELECT calendar.session_date,
                    calendar.source_batch_id,
                    calendar.content_sha256,
                    observed.observed_at
               FROM public.trading_calendars AS calendar
               JOIN public.trading_calendar_versions AS version
                 ON version.exchange = calendar.exchange
                AND version.session_date = calendar.session_date
                AND version.session_type = calendar.session_type
                AND version.source_version = calendar.source_version
                AND version.source = $1
                AND version.timezone = 'Asia/Seoul'
                AND version.source_batch_id = calendar.source_batch_id
                AND version.content_sha256 = calendar.content_sha256
               JOIN public.data_batches AS batch
                 ON batch.source_batch_id = calendar.source_batch_id
                AND batch.provider = 'KRX'
                AND batch.market = 'KR'
                AND batch.kind = 'CALENDAR'
                AND batch.batch_date = calendar.session_date
                AND batch.content_sha256 = calendar.content_sha256
                AND batch.source_file_name = 'calendar.json'
                AND batch.fetch_mode = 'credentialed'
             CROSS JOIN observed
              WHERE calendar.exchange = 'KRX'
                AND calendar.session_date =
                    (observed.observed_at AT TIME ZONE 'Asia/Seoul')::date
                AND calendar.session_type IN ('TRADING', 'CLOSED')
                AND calendar.timezone = 'Asia/Seoul'
                AND calendar.source = $1
                AND calendar.source_version = $2
                AND calendar.source_batch_id IS NOT NULL
                AND calendar.source_batch_id <> '00000000-0000-0000-0000-000000000000'::uuid
                AND calendar.content_sha256 ~ '^[0-9a-f]{64}$'
                AND calendar.retrieved_at <= observed.observed_at
                AND calendar.retrieved_at >= observed.observed_at - INTERVAL '36 hours'
                AND version.retrieved_at <= observed.observed_at
                AND version.retrieved_at >= observed.observed_at - INTERVAL '36 hours'
                AND batch.retrieved_at <= observed.observed_at
                AND batch.retrieved_at >= observed.observed_at - INTERVAL '36 hours'
              ORDER BY calendar.session_date, calendar.source_batch_id,
                       calendar.content_sha256",
        )
        .bind("kis")
        .bind(KIS_CALENDAR_SOURCE_VERSION)
        .fetch_all(&db.supervisor)
        .await
        .map_err(|error| format!("read intraday calendar contract: {error}"))?;
    if resolver_rows.len() != 1
        || resolver_rows[0].0 != date.as_naive_date()
        || resolver_rows[0].1 != canonical_batch_id
        || resolver_rows[0].2 != persisted.1
    {
        return Err(format!(
            "intraday resolver contract did not return the published proof: {resolver_rows:?}"
        ));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calendar_bootstrap_eod_first_actual_postgres_sink_replay_preserves_evidence() {
    let db = ScratchDb::create()
        .await
        .expect("DATABASE_URL must point to the disposable local PostgreSQL container");
    let result = exercise_actual_sink(&db).await;
    db.drop_db().await;
    result.expect("actual standalone calendar sink verification");
}
