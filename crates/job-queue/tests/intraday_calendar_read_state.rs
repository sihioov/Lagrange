#[allow(dead_code)]
mod intraday_quotes_support;

use chrono::{DateTime, Days, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use collectors::{
    PostgresPublicationSink, PublicationSink, PublishOutcome, ensure_kis_calendar_source,
    ingest_normalize_publish_kis_with_calendar_source,
};
use domain::{TradingDate, UtcTimestamp};
use intraday_quotes_support::{IntradayTestDb, MembershipFixture, run_body};
use job_queue::owner_equity_v2::{
    DemandMutationKind, IntradayCalendarDisposition, IntradayStorageError,
};
use kis_client::{KisError, MarketDataReply};
use market_data::contract::{MARKET_KR, PROVIDER_KIS_CALENDAR_NORMALIZED};
use market_data::ingest::IngestRequest;
use market_data::normalize::normalize_kis_calendar_batch;
use market_data::providers::kis::{KR_ETF_CORE_SYMBOLS, KisProvider, KisRead};
use market_data::publication::CalendarPublicationBundle;
use market_data::storage::RawStore;
use serde_json::Value;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

const KIS_SOURCE_VERSION: &str = "kis-chk-holiday-v1:schema-1";

#[derive(Debug, Clone, Default)]
struct AcceptanceKisRead {
    total_calls: Arc<AtomicUsize>,
    calendar_calls: Arc<AtomicUsize>,
}

impl AcceptanceKisRead {
    fn query_value<'a>(query: &'a [(String, String)], key: &str) -> &'a str {
        query
            .iter()
            .find(|(query_key, _)| query_key == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
    }
}

impl KisRead for AcceptanceKisRead {
    async fn get(
        &self,
        path: &str,
        _tr_id: &str,
        query: &[(String, String)],
        _continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        self.total_calls.fetch_add(1, Ordering::SeqCst);
        let date = Self::query_value(query, "BASS_DT");
        let date = if date.is_empty() {
            Self::query_value(query, "FID_INPUT_DATE_1")
        } else {
            date
        };
        let symbol = Self::query_value(query, "FID_INPUT_ISCD");
        let body = if path.ends_with("inquire-daily-itemchartprice") {
            json!({
                "rt_cd": "0",
                "output1": {
                    "hts_kor_isnm": format!("ETF {symbol}"),
                    "stck_shrn_iscd": symbol
                },
                "output2": [{
                    "stck_bsop_date": date,
                    "stck_oprc": "100.00",
                    "stck_hgpr": "102.00",
                    "stck_lwpr": "99.00",
                    "stck_clpr": "101.00",
                    "acml_vol": "1300",
                    "acml_tr_pbmn": "131300"
                }]
            })
        } else if path.ends_with("inquire-price") {
            json!({
                "rt_cd": "0",
                "output": {"stck_shrn_iscd": symbol}
            })
        } else if path.ends_with("chk-holiday") {
            self.calendar_calls.fetch_add(1, Ordering::SeqCst);
            json!({
                "rt_cd": "0",
                "output": [{"bass_dt": date, "opnd_yn": "Y"}]
            })
        } else {
            json!({"rt_cd": "0", "output1": []})
        };
        Ok(MarketDataReply {
            body: serde_json::to_vec(&body).map_err(|_| KisError::SchemaDrift {
                endpoint: path.to_owned(),
                detail: "acceptance fixture serialization failed".to_owned(),
            })?,
            continuation: None,
        })
    }
}

#[derive(Debug, Clone)]
struct CalendarSeed {
    calendar_session_date: NaiveDate,
    version_session_date: NaiveDate,
    batch_date: NaiveDate,
    calendar_session_type: String,
    version_session_type: String,
    calendar_timezone: String,
    version_timezone: String,
    calendar_source: String,
    version_source: String,
    calendar_source_version: String,
    version_source_version: String,
    calendar_source_batch_id: Uuid,
    version_source_batch_id: Uuid,
    batch_id: Option<Uuid>,
    batch_source_batch_id: Uuid,
    calendar_hash: String,
    version_hash: String,
    batch_hash: String,
    batch_provider: String,
    batch_market: String,
    batch_source_file_name: String,
    batch_fetch_mode: String,
    batch_kind: String,
    calendar_retrieved_at: DateTime<Utc>,
    version_retrieved_at: DateTime<Utc>,
    batch_retrieved_at: DateTime<Utc>,
}

impl CalendarSeed {
    fn valid(session_date: NaiveDate, disposition: &str, observed_at: DateTime<Utc>) -> Self {
        let batch_id = Uuid::new_v4();
        let source_batch_id = Uuid::new_v4();
        let retrieved_at = observed_at - Duration::hours(1);
        Self {
            calendar_session_date: session_date,
            version_session_date: session_date,
            batch_date: session_date,
            calendar_session_type: disposition.to_owned(),
            version_session_type: disposition.to_owned(),
            calendar_timezone: "Asia/Seoul".to_owned(),
            version_timezone: "Asia/Seoul".to_owned(),
            calendar_source: "kis".to_owned(),
            version_source: "kis".to_owned(),
            calendar_source_version: KIS_SOURCE_VERSION.to_owned(),
            version_source_version: KIS_SOURCE_VERSION.to_owned(),
            calendar_source_batch_id: source_batch_id,
            version_source_batch_id: source_batch_id,
            batch_id: Some(batch_id),
            batch_source_batch_id: source_batch_id,
            calendar_hash: "a".repeat(64),
            version_hash: "a".repeat(64),
            batch_hash: "a".repeat(64),
            batch_provider: "KRX".to_owned(),
            batch_market: "KR".to_owned(),
            batch_source_file_name: "calendar.json".to_owned(),
            batch_fetch_mode: "credentialed".to_owned(),
            batch_kind: "CALENDAR".to_owned(),
            calendar_retrieved_at: retrieved_at,
            version_retrieved_at: retrieved_at,
            batch_retrieved_at: retrieved_at,
        }
    }
}

async fn run_without_calendar<F, Fut>(body: F)
where
    F: FnOnce(IntradayTestDb) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let db = IntradayTestDb::create_without_calendar()
        .await
        .unwrap_or_else(|error| panic!("intraday calendar DB setup failed: {error}"));
    let result = body(db.clone()).await;
    let cleanup = db.drop_database().await;
    if let Err(error) = result {
        panic!("intraday calendar test failed: {error}");
    }
    if let Err(error) = cleanup {
        panic!("intraday calendar DB cleanup failed: {error}");
    }
}

async fn database_now(db: &IntradayTestDb) -> Result<DateTime<Utc>, String> {
    sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not read the QA database clock".to_owned())
}

async fn seed_calendar_lineage(
    db: &IntradayTestDb,
    seed: &CalendarSeed,
    include_calendar: bool,
    include_version: bool,
) -> Result<(), String> {
    if let Some(batch_id) = seed.batch_id {
        sqlx::query(
            "INSERT INTO public.data_batches
                (id, provider, market, batch_date, kind, storage_path,
                 content_sha256, bytes_size, retrieved_at, source_batch_id,
                 source_file_name, fetch_mode)
             VALUES ($1, $2, $3, $4, $5, 'fixture/intraday-calendar',
                     $6, 1, $7, $8, $9, $10)",
        )
        .bind(batch_id)
        .bind(&seed.batch_provider)
        .bind(&seed.batch_market)
        .bind(seed.batch_date)
        .bind(&seed.batch_kind)
        .bind(&seed.batch_hash)
        .bind(seed.batch_retrieved_at)
        .bind(seed.batch_source_batch_id)
        .bind(&seed.batch_source_file_name)
        .bind(&seed.batch_fetch_mode)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not seed calendar data batch".to_owned())?;
    }
    if include_version {
        sqlx::query(
            "INSERT INTO public.trading_calendar_versions
                (exchange, session_date, session_type, timezone, source,
                 source_version, source_batch_id, content_sha256, retrieved_at)
             VALUES ('KRX', $1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(seed.version_session_date)
        .bind(&seed.version_session_type)
        .bind(&seed.version_timezone)
        .bind(&seed.version_source)
        .bind(&seed.version_source_version)
        .bind(seed.version_source_batch_id)
        .bind(&seed.version_hash)
        .bind(seed.version_retrieved_at)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not seed immutable calendar version".to_owned())?;
    }
    if include_calendar {
        sqlx::query(
            "INSERT INTO public.trading_calendars
                (exchange, session_date, session_type, timezone, source,
                 source_version, source_batch_id, content_sha256, retrieved_at)
             VALUES ('KRX', $1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(seed.calendar_session_date)
        .bind(&seed.calendar_session_type)
        .bind(&seed.calendar_timezone)
        .bind(&seed.calendar_source)
        .bind(&seed.calendar_source_version)
        .bind(seed.calendar_source_batch_id)
        .bind(&seed.calendar_hash)
        .bind(seed.calendar_retrieved_at)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not seed current calendar projection".to_owned())?;
    }
    Ok(())
}

fn assert_state_fields(
    state: &job_queue::owner_equity_v2::IntradayCalendarReadState,
    seed: &CalendarSeed,
    expected_disposition: IntradayCalendarDisposition,
) -> Result<(), String> {
    if state.session_date != seed.calendar_session_date
        || state.disposition != expected_disposition
        || state.calendar_source_batch_id != seed.calendar_source_batch_id
        || state.calendar_content_sha256 != seed.calendar_hash
        || state.calendar_source() != "kis"
        || state.calendar_source_version() != KIS_SOURCE_VERSION
    {
        return Err("calendar read returned the wrong disposition or lineage".to_owned());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum InvalidLineage {
    MissingProjection,
    MissingVersion,
    MissingBatch,
    WrongLineage,
    WrongType,
    WrongDate,
    WrongTimezone,
    WrongSource,
    WrongSourceVersion,
    WrongHash,
    WrongBatchHash,
    WrongBatchMetadata,
    WrongProvider,
    WrongSourceFileName,
    WrongFetchMode,
    WrongRawSourceId,
    NilBatch,
}

fn configure_invalid_case(seed: &mut CalendarSeed, case: InvalidLineage) -> (bool, bool) {
    match case {
        InvalidLineage::MissingProjection => (false, true),
        InvalidLineage::MissingVersion => (true, false),
        InvalidLineage::MissingBatch => {
            seed.batch_id = None;
            (true, true)
        }
        InvalidLineage::WrongLineage => {
            seed.version_source_batch_id = Uuid::new_v4();
            (true, true)
        }
        InvalidLineage::WrongType => {
            seed.calendar_session_type = "SETTLEMENT".to_owned();
            seed.version_session_type = "TRADING".to_owned();
            (true, true)
        }
        InvalidLineage::WrongDate => {
            let wrong_date = seed
                .calendar_session_date
                .checked_sub_days(Days::new(1))
                .expect("QA fixture date has a prior date");
            seed.calendar_session_date = wrong_date;
            seed.version_session_date = wrong_date;
            seed.batch_date = wrong_date;
            (true, true)
        }
        InvalidLineage::WrongTimezone => {
            seed.calendar_timezone = "UTC".to_owned();
            (true, true)
        }
        InvalidLineage::WrongSource => {
            seed.calendar_source = "other".to_owned();
            seed.version_source = "other".to_owned();
            (true, true)
        }
        InvalidLineage::WrongSourceVersion => {
            seed.calendar_source_version = "other-calendar-v1".to_owned();
            seed.version_source_version = "other-calendar-v1".to_owned();
            (true, true)
        }
        InvalidLineage::WrongHash => {
            seed.version_hash = "b".repeat(64);
            (true, true)
        }
        InvalidLineage::WrongBatchHash => {
            seed.batch_hash = "b".repeat(64);
            (true, true)
        }
        InvalidLineage::WrongBatchMetadata => {
            seed.batch_kind = "REFERENCE".to_owned();
            (true, true)
        }
        InvalidLineage::WrongProvider => {
            seed.batch_provider = "KIS".to_owned();
            (true, true)
        }
        InvalidLineage::WrongSourceFileName => {
            seed.batch_source_file_name = "intraday-calendar.json".to_owned();
            (true, true)
        }
        InvalidLineage::WrongFetchMode => {
            seed.batch_fetch_mode = "synthetic".to_owned();
            (true, true)
        }
        InvalidLineage::WrongRawSourceId => {
            seed.batch_source_batch_id = Uuid::new_v4();
            (true, true)
        }
        InvalidLineage::NilBatch => {
            seed.calendar_source_batch_id = Uuid::nil();
            seed.version_source_batch_id = Uuid::nil();
            seed.batch_id = Some(Uuid::nil());
            (true, true)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RetrievalFailure {
    CalendarStale,
    CalendarFuture,
    VersionStale,
    VersionFuture,
    BatchStale,
    BatchFuture,
}

fn configure_retrieval_failure(
    seed: &mut CalendarSeed,
    failure: RetrievalFailure,
    now: DateTime<Utc>,
) {
    let stale = now - Duration::hours(37);
    let future = now + Duration::hours(1);
    match failure {
        RetrievalFailure::CalendarStale => seed.calendar_retrieved_at = stale,
        RetrievalFailure::CalendarFuture => seed.calendar_retrieved_at = future,
        RetrievalFailure::VersionStale => seed.version_retrieved_at = stale,
        RetrievalFailure::VersionFuture => seed.version_retrieved_at = future,
        RetrievalFailure::BatchStale => seed.batch_retrieved_at = stale,
        RetrievalFailure::BatchFuture => seed.batch_retrieved_at = future,
    }
}

async fn fingerprint_rows(pool: &PgPool, owner_user_id: Uuid) -> Result<[Value; 8], String> {
    let demands: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(demand) ORDER BY demand.id),
                    '[]'::jsonb
                )
           FROM public.owner_intraday_quote_demands AS demand
          WHERE demand.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint demand rows".to_owned())?;
    let cache: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(cache) ORDER BY cache.membership_id),
                    '[]'::jsonb
                )
           FROM public.owner_intraday_quote_cache AS cache
          WHERE cache.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint cache rows".to_owned())?;
    let producers: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(producer) ORDER BY producer.owner_user_id),
                    '[]'::jsonb
                )
           FROM public.owner_intraday_quote_producers AS producer
          WHERE producer.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint producer rows".to_owned())?;
    let memberships: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(pg_catalog.to_jsonb(membership) ORDER BY membership.id),
                    '[]'::jsonb
                )
           FROM public.owner_equity_memberships AS membership
          WHERE membership.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint membership rows".to_owned())?;
    let admissions: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(
                        pg_catalog.to_jsonb(admission)
                        ORDER BY admission.generation_id
                    ),
                    '[]'::jsonb
                )
           FROM public.owner_equity_generation_admissions AS admission
          WHERE admission.owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint admission rows".to_owned())?;
    let calendars: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(
                        pg_catalog.to_jsonb(calendar)
                        ORDER BY calendar.exchange, calendar.session_date
                    ),
                    '[]'::jsonb
                )
           FROM public.trading_calendars AS calendar",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint calendar projection rows".to_owned())?;
    let versions: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(
                        pg_catalog.to_jsonb(version)
                        ORDER BY version.exchange, version.session_date,
                                 version.source_version
                    ),
                    '[]'::jsonb
                )
           FROM public.trading_calendar_versions AS version",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint calendar version rows".to_owned())?;
    let batches: Value = sqlx::query_scalar(
        "SELECT COALESCE(
                    jsonb_agg(
                        pg_catalog.to_jsonb(batch) ORDER BY batch.id
                    ),
                    '[]'::jsonb
                )
           FROM public.data_batches AS batch",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "could not fingerprint calendar batch rows".to_owned())?;
    Ok([
        demands,
        cache,
        producers,
        memberships,
        admissions,
        calendars,
        versions,
        batches,
    ])
}

async fn seed_populated_operational_rows(
    db: &IntradayTestDb,
    fixture: &MembershipFixture,
    seed: &CalendarSeed,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let received_at = now - Duration::minutes(10);
    sqlx::query(
        "INSERT INTO public.owner_intraday_quote_cache
            (owner_user_id, membership_id, generation_id, instrument_id,
             generation, session_date, calendar_source, calendar_source_version,
             calendar_source_batch_id, calendar_content_sha256,
             window_contract_sha256, price, base_price, change_amount,
             change_percent, direction, halted, received_at, last_success_at,
             quote_version, last_attempt_at, producer_fence)
         VALUES ($1, $2, $3, $4, $5, $6, 'kis', $7, $8, $9,
                 $10, 72500, 71000, 1500, 2.11, 'UP', FALSE, $11, $11,
                 7, $12, 4)",
    )
    .bind(fixture.owner_user_id)
    .bind(fixture.membership_id)
    .bind(fixture.generation_id)
    .bind(&fixture.instrument_id)
    .bind(i64::try_from(fixture.generation).expect("fixture generation fits bigint"))
    .bind(seed.calendar_session_date)
    .bind(KIS_SOURCE_VERSION)
    .bind(seed.calendar_source_batch_id)
    .bind(&seed.calendar_hash)
    .bind(format!("sha256:{}", "c".repeat(64)))
    .bind(received_at)
    .bind(now - Duration::minutes(1))
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed populated cache row".to_owned())?;

    sqlx::query(
        "INSERT INTO public.owner_intraday_quote_producers
            (owner_user_id, holder_id, fencing_token,
             lease_expires_at, heartbeat_at, updated_at)
         VALUES ($1, $2, 4, $3, $4, $4)",
    )
    .bind(fixture.owner_user_id)
    .bind(Uuid::new_v4())
    .bind(now + Duration::seconds(20))
    .bind(now)
    .execute(&db.superuser)
    .await
    .map_err(|_| "could not seed populated producer row".to_owned())?;
    Ok(())
}

async fn assert_app_actor_and_producer_boundary(
    db: &IntradayTestDb,
    owner_user_id: Uuid,
    demand_id: Uuid,
) -> Result<(), String> {
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&db.app)
        .await
        .map_err(|_| "could not inspect app current_user".to_owned())?;
    if current_user != "app" {
        return Err(format!("expected app role, observed {current_user}"));
    }
    let actor_setting: String = sqlx::query_scalar(
        "SELECT COALESCE(
                    pg_catalog.current_setting('app.actor_user_id', true),
                    ''
                )",
    )
    .fetch_one(&db.app)
    .await
    .map_err(|_| "could not inspect app actor setting".to_owned())?;
    if !actor_setting.is_empty() {
        return Err("actor GUC leaked outside the repository transaction".to_owned());
    }

    let without_actor: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_one(&db.app)
    .await
    .map_err(|_| "actor-less app tenant SELECT failed".to_owned())?;
    if without_actor != 0 {
        return Err("actor-less app tenant SELECT exposed owner rows".to_owned());
    }

    let mut tx = db
        .app
        .begin()
        .await
        .map_err(|_| "could not begin actor-scope assertion".to_owned())?;
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_user_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|_| "could not set actor-scope assertion".to_owned())?;
    let with_actor: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1 AND id = $2",
    )
    .bind(owner_user_id)
    .bind(demand_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "actor-scoped app tenant SELECT failed".to_owned())?;
    tx.commit()
        .await
        .map_err(|_| "could not commit actor-scope assertion".to_owned())?;
    if with_actor != 1 {
        return Err("actor-scoped app SELECT did not expose its owner row".to_owned());
    }

    let error = sqlx::query(
        "SELECT owner_user_id
           FROM public.owner_intraday_quote_producers
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .execute(&db.app)
    .await
    .expect_err("app producer SELECT unexpectedly succeeded");
    if !matches!(
        error,
        sqlx::Error::Database(database) if database.code().as_deref() == Some("42501")
    ) {
        return Err("app producer SELECT failed for a reason other than 42501".to_owned());
    }
    Ok(())
}

#[tokio::test]
async fn calendar_read_returns_current_trading_and_closed_without_quote_prerequisites() {
    for (label, disposition, expected) in [
        ("trading", "TRADING", IntradayCalendarDisposition::Trading),
        ("closed", "CLOSED", IntradayCalendarDisposition::Closed),
    ] {
        run_without_calendar(|db| async move {
            let now = database_now(&db).await?;
            let seed = CalendarSeed::valid(db.session_date, disposition, now);
            seed_calendar_lineage(&db, &seed, true, true).await?;
            let owner = db.seed_owner(&format!("calendar-{label}")).await?;
            let app = db.repository_as_app();
            let before = database_now(&db).await?;
            let state = app
                .read_current_calendar_disposition(owner)
                .await
                .map_err(|error| format!("{label} calendar read failed: {error}"))?
                .ok_or_else(|| format!("{label} calendar evidence was not readable"))?;
            let after = database_now(&db).await?;
            assert_state_fields(&state, &seed, expected)?;
            if state.observed_at < before || state.observed_at > after {
                return Err(format!("{label} observed_at was not one DB-clock snapshot"));
            }

            let other_owner = Uuid::new_v4();
            let other_state = app
                .read_current_calendar_disposition(other_owner)
                .await
                .map_err(|error| format!("{label} shared calendar read failed: {error}"))?
                .ok_or_else(|| format!("{label} shared calendar evidence was not readable"))?;
            assert_state_fields(&other_state, &seed, expected)?;
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
async fn actual_sink_reuses_morning_calendar_for_evening_eod_and_intraday_resolution() {
    run_without_calendar(|db| async move {
        let target_date = TradingDate::parse(&db.session_date.to_string())
            .map_err(|_| "could not construct the current QA trading date".to_owned())?;
        let raw_root = tempfile::tempdir().map_err(|_| "could not create Raw tempdir".to_owned())?;
        let store = RawStore::new(raw_root.path().join("data"));
        std::fs::create_dir_all(store.root().join("raw"))
            .map_err(|_| "could not create Raw root".to_owned())?;
        let reader = AcceptanceKisRead::default();
        let provider = KisProvider::kr_etf_core(reader.clone());
        let db_clock = database_now(&db).await?;
        let kst = FixedOffset::east_opt(9 * 60 * 60)
            .ok_or_else(|| "could not construct KST offset".to_owned())?;
        let day_start = kst
            .from_local_datetime(
                &db.session_date
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| "could not construct session-day start".to_owned())?,
            )
            .single()
            .ok_or_else(|| "session-day start was not a unique instant".to_owned())?
            .with_timezone(&Utc);
        let (morning_at, evening_at) = if db_clock - day_start >= Duration::seconds(120) {
            (db_clock - Duration::seconds(90), db_clock - Duration::seconds(30))
        } else {
            (day_start, db_clock)
        };
        if morning_at == evening_at {
            return Err("DB clock did not provide two distinct same-day timestamps".to_owned());
        }
        if morning_at >= evening_at {
            return Err("DB-derived morning timestamp was not before evening timestamp".to_owned());
        }
        if morning_at.with_timezone(&kst).date_naive() != db.session_date
            || evening_at.with_timezone(&kst).date_naive() != db.session_date
        {
            return Err("DB-derived timestamps crossed the KST session-day boundary".to_owned());
        }
        let morning_request = IngestRequest::new(
            MARKET_KR.to_owned(),
            target_date,
            UtcTimestamp::from_datetime(morning_at),
        );
        let evening_request = IngestRequest::new(
            MARKET_KR.to_owned(),
            target_date,
            UtcTimestamp::from_datetime(evening_at),
        );
        if morning_request.now == evening_request.now {
            return Err("calendar and EOD requests unexpectedly share retrieval time".to_owned());
        }
        let entitlement = Some("entitlement://wp1-actual");
        let calendar_source = ensure_kis_calendar_source(
            &store,
            &provider,
            &morning_request,
            entitlement,
        )
        .await
        .map_err(|error| format!("shared calendar acquisition failed: {error}"))?
        .ok_or_else(|| "shared calendar acquisition did not return a source".to_owned())?;

        let normalized_calendar = normalize_kis_calendar_batch(&store, &calendar_source)
            .map_err(|error| format!("morning calendar normalization failed: {error}"))?;
        let morning = CalendarPublicationBundle::from_raw(&store, &normalized_calendar.entry)
            .map_err(|error| format!("morning calendar evidence failed: {error}"))?;
        let sink = PostgresPublicationSink::new(db.research_writer.clone());
        if sink
            .publish_calendar(&morning)
            .await
            .map_err(|error| format!("morning calendar publication failed: {error}"))?
            != PublishOutcome::Published
        {
            return Err("morning calendar was not newly published".to_owned());
        }
        let morning_evidence = morning.calendar_evidence().clone();
        let morning_fact = morning.calendar_facts()[0].clone();
        let morning_counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM data_batches),
                    (SELECT count(*) FROM trading_calendar_versions),
                    (SELECT count(*) FROM trading_calendars)",
        )
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not inspect morning publication counts".to_owned())?;

        let evening = ingest_normalize_publish_kis_with_calendar_source(
            &store,
            &provider,
            &evening_request,
            entitlement,
            &sink,
            Some(&calendar_source),
        )
        .await
        .map_err(|error| format!("evening EOD acquisition failed: {error}"))?;
        if evening.manifest.files.len() != 4 {
            return Err(format!(
                "evening normalized EOD had {} files instead of four",
                evening.manifest.files.len()
            ));
        }
        if evening.published != PublishOutcome::Published {
            return Err("evening EOD was not newly published".to_owned());
        }
        if reader.calendar_calls.load(Ordering::SeqCst) != 1 {
            return Err(format!(
                "expected one calendar broker call, got {}",
                reader.calendar_calls.load(Ordering::SeqCst)
            ));
        }
        if reader.total_calls.load(Ordering::SeqCst)
            != KR_ETF_CORE_SYMBOLS.len() * 2 + 7 + 1
        {
            return Err(format!(
                "unexpected fake broker call count: {}",
                reader.total_calls.load(Ordering::SeqCst)
            ));
        }

        let evening_bundle = market_data::publication::PublicationBundle::from_raw(
            &store,
            &evening.manifest,
        )
        .map_err(|error| format!("evening publication evidence failed: {error}"))?;
        if evening_bundle.calendar_evidence.as_ref() != Some(&morning_evidence) {
            return Err("evening EOD changed the morning calendar evidence".to_owned());
        }
        let copied_calendar = evening
            .manifest
            .files
            .iter()
            .find(|file| file.file_name == "calendar.json")
            .and_then(|file| file.copied_from.as_ref())
            .ok_or_else(|| "evening EOD calendar file lost copied-source lineage".to_owned())?;
        if copied_calendar.provider != PROVIDER_KIS_CALENDAR_NORMALIZED
            || copied_calendar.batch_id != normalized_calendar.entry.batch_id
            || copied_calendar.content_hash != normalized_calendar.entry.files[0].content_hash
            || copied_calendar.retrieved_at != normalized_calendar.entry.retrieved_at
        {
            return Err("evening EOD copied-source identity differs from the morning source".to_owned());
        }
        let canonical_files = store
            .read_batch_bytes(
                PROVIDER_KIS_CALENDAR_NORMALIZED,
                MARKET_KR,
                &normalized_calendar.entry,
            )
            .map_err(|_| "could not read normalized morning calendar bytes".to_owned())?;
        let evening_files = store
            .read_batch_bytes(
                &evening.manifest.provider,
                &evening.manifest.market,
                &evening.manifest,
            )
            .map_err(|_| "could not read normalized evening EOD bytes".to_owned())?;
        let canonical_calendar = canonical_files
            .iter()
            .find(|file| file.file_name == "calendar.json")
            .ok_or_else(|| "normalized morning calendar bytes are missing".to_owned())?;
        let evening_calendar = evening_files
            .iter()
            .find(|file| file.file_name == "calendar.json")
            .ok_or_else(|| "normalized evening calendar bytes are missing".to_owned())?;
        if canonical_calendar.bytes != evening_calendar.bytes {
            return Err("evening normalized calendar bytes changed".to_owned());
        }

        let evening_counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM data_batches),
                    (SELECT count(*) FROM trading_calendar_versions),
                    (SELECT count(*) FROM trading_calendars)",
        )
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not inspect evening publication counts".to_owned())?;
        if evening_counts != (morning_counts.0 + 4, morning_counts.1, morning_counts.2) {
            return Err(format!(
                "evening publication changed calendar history/projection unexpectedly: {evening_counts:?}"
            ));
        }

        let replay = sink
            .publish(&evening_bundle)
            .await
            .map_err(|error| format!("evening EOD replay failed: {error}"))?;
        if replay != PublishOutcome::AlreadyPublished {
            return Err(format!("evening EOD replay returned {replay:?}"));
        }
        let calendar_replay = sink
            .publish_calendar(&morning)
            .await
            .map_err(|error| format!("morning calendar replay failed: {error}"))?;
        if calendar_replay != PublishOutcome::AlreadyPublished {
            return Err(format!("morning calendar replay returned {calendar_replay:?}"));
        }
        let replay_counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM data_batches),
                    (SELECT count(*) FROM trading_calendar_versions),
                    (SELECT count(*) FROM trading_calendars)",
        )
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not inspect replay counts".to_owned())?;
        if replay_counts != evening_counts {
            return Err("idempotent replay changed publication counts".to_owned());
        }

        let persisted: (String, Uuid, String, DateTime<Utc>) = sqlx::query_as(
            "SELECT source, source_batch_id, content_sha256, retrieved_at
               FROM trading_calendar_versions
              WHERE exchange = 'KRX' AND session_date = $1",
        )
        .bind(db.session_date)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not inspect persisted calendar evidence".to_owned())?;
        if persisted.0 != morning_fact.source
            || persisted.1 != morning_evidence.source_batch_id.as_uuid()
            || persisted.2 != morning_evidence.content_sha256
            || persisted.3 != morning_evidence.retrieved_at.as_datetime()
        {
            return Err("persisted calendar evidence does not match the morning source".to_owned());
        }

        let owner = db.seed_owner("actual-calendar-reuse").await?;
        let repository = db.repository_as_app();
        let state = repository
            .read_current_calendar_disposition(owner)
            .await
            .map_err(|error| format!("job-queue calendar resolver failed: {error}"))?
            .ok_or_else(|| "job-queue resolver could not read the published calendar".to_owned())?;
        if state.session_date != db.session_date
            || state.disposition != IntradayCalendarDisposition::Trading
            || state.calendar_source_batch_id != morning_evidence.source_batch_id.as_uuid()
            || state.calendar_content_sha256 != morning_evidence.content_sha256
        {
            return Err("job-queue resolver returned different calendar lineage".to_owned());
        }
        let proof = repository
            .resolve_current_session_proof(owner, &db.window_contract_sha256)
            .await
            .map_err(|error| format!("job-queue session proof resolver failed: {error}"))?
            .ok_or_else(|| "job-queue resolver could not read the session proof".to_owned())?;
        if proof.session_date != db.session_date
            || proof.calendar_source_batch_id != morning_evidence.source_batch_id.as_uuid()
            || proof.calendar_content_sha256 != morning_evidence.content_sha256
        {
            return Err("job-queue session proof returned different calendar lineage".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn credentialed_sink_calendar_is_accepted_by_disposition_and_session_reads() {
    run_body(|db| async move {
        let owner = db.seed_owner("sink-calendar-lineage").await?;
        let app = db.repository_as_app();
        let state = app
            .read_current_calendar_disposition(owner)
            .await
            .map_err(|error| format!("sink calendar disposition read failed: {error}"))?
            .ok_or_else(|| "sink-published calendar disposition was not readable".to_owned())?;
        if state.session_date != db.session_date
            || state.disposition != IntradayCalendarDisposition::Trading
            || state.calendar_source_batch_id != db.calendar_source_batch_id
            || state.calendar_content_sha256 != db.calendar_content_sha256
        {
            return Err("sink-published calendar returned the wrong current lineage".to_owned());
        }

        let proof = app
            .resolve_current_session_proof(owner, &db.window_contract_sha256)
            .await
            .map_err(|error| format!("sink calendar session proof read failed: {error}"))?
            .ok_or_else(|| "sink-published current session proof was not readable".to_owned())?;
        if proof.session_date != db.session_date
            || proof.calendar_source_batch_id != db.calendar_source_batch_id
            || proof.calendar_content_sha256 != db.calendar_content_sha256
        {
            return Err("sink-published session proof returned the wrong lineage".to_owned());
        }

        let persisted: (Uuid, String, String, String, String, String, Uuid) = sqlx::query_as(
            "SELECT id, provider, market, kind, source_file_name, fetch_mode, source_batch_id
               FROM public.data_batches
              WHERE source_batch_id = $1 AND source_file_name = 'calendar.json'",
        )
        .bind(db.calendar_source_batch_id)
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not inspect sink calendar batch lineage".to_owned())?;
        if persisted.0 == persisted.6
            || persisted.1 != "KRX"
            || persisted.2 != "KR"
            || persisted.3 != "CALENDAR"
            || persisted.4 != "calendar.json"
            || persisted.5 != "credentialed"
        {
            return Err(
                "sink calendar fixture did not preserve the DB-row/Raw-id contract".to_owned(),
            );
        }

        sqlx::query(
            "UPDATE public.data_batches
                SET retrieved_at = pg_catalog.now() - INTERVAL '37 hours'
              WHERE source_batch_id = $1 AND source_file_name = 'calendar.json'",
        )
        .bind(db.calendar_source_batch_id)
        .execute(&db.superuser)
        .await
        .map_err(|_| "could not age the sink calendar batch proof".to_owned())?;
        let stale = app
            .resolve_current_session_proof(owner, &db.window_contract_sha256)
            .await
            .map_err(|error| format!("stale sink proof read failed: {error}"))?;
        if stale.is_some() {
            return Err("stale sink calendar proof remained readable".to_owned());
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn calendar_read_returns_none_for_missing_or_invalid_lineage() {
    for case in [
        InvalidLineage::MissingProjection,
        InvalidLineage::MissingVersion,
        InvalidLineage::MissingBatch,
        InvalidLineage::WrongLineage,
        InvalidLineage::WrongType,
        InvalidLineage::WrongDate,
        InvalidLineage::WrongTimezone,
        InvalidLineage::WrongSource,
        InvalidLineage::WrongSourceVersion,
        InvalidLineage::WrongHash,
        InvalidLineage::WrongBatchHash,
        InvalidLineage::WrongBatchMetadata,
        InvalidLineage::WrongProvider,
        InvalidLineage::WrongSourceFileName,
        InvalidLineage::WrongFetchMode,
        InvalidLineage::WrongRawSourceId,
        InvalidLineage::NilBatch,
    ] {
        run_without_calendar(|db| async move {
            let now = database_now(&db).await?;
            let mut seed = CalendarSeed::valid(db.session_date, "TRADING", now);
            let (include_calendar, include_version) = configure_invalid_case(&mut seed, case);
            seed_calendar_lineage(&db, &seed, include_calendar, include_version).await?;
            let result = db
                .repository_as_app()
                .read_current_calendar_disposition(Uuid::new_v4())
                .await
                .map_err(|error| format!("invalid-lineage read returned typed error: {error}"))?;
            if result.is_some() {
                return Err(format!("invalid calendar case {case:?} produced a state"));
            }
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
async fn calendar_read_returns_none_when_any_lineage_timestamp_is_stale_or_future() {
    for failure in [
        RetrievalFailure::CalendarStale,
        RetrievalFailure::CalendarFuture,
        RetrievalFailure::VersionStale,
        RetrievalFailure::VersionFuture,
        RetrievalFailure::BatchStale,
        RetrievalFailure::BatchFuture,
    ] {
        run_without_calendar(|db| async move {
            let now = database_now(&db).await?;
            let mut seed = CalendarSeed::valid(db.session_date, "TRADING", now);
            configure_retrieval_failure(&mut seed, failure, now);
            seed_calendar_lineage(&db, &seed, true, true).await?;
            let result = db
                .repository_as_app()
                .read_current_calendar_disposition(Uuid::new_v4())
                .await
                .map_err(|error| format!("timestamp read returned typed error: {error}"))?;
            if result.is_some() {
                return Err(format!(
                    "invalid retrieval case {failure:?} produced a state"
                ));
            }
            Ok(())
        })
        .await;
    }
}

#[tokio::test]
async fn nil_owner_is_invalid_and_existing_uniqueness_guards_duplicate_results() {
    run_without_calendar(|db| async move {
        let nil_owner = db
            .repository_as_app()
            .read_current_calendar_disposition(Uuid::nil())
            .await;
        if nil_owner != Err(IntradayStorageError::InvalidInput) {
            return Err("nil owner did not produce InvalidInput".to_owned());
        }

        for (table, constraint) in [
            (
                "public.trading_calendars",
                "trading_calendars_exchange_date_key",
            ),
            (
                "public.trading_calendar_versions",
                "trading_calendar_versions_exchange_date_source_version_key",
            ),
        ] {
            let present: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                         SELECT 1
                           FROM pg_catalog.pg_constraint
                          WHERE conrelid = $1::regclass
                            AND contype = 'u'
                            AND conname = $2
                       )",
            )
            .bind(table)
            .bind(constraint)
            .fetch_one(&db.superuser)
            .await
            .map_err(|_| format!("could not inspect {constraint}"))?;
            if !present {
                return Err(format!(
                    "required duplicate-preventing constraint {constraint} missing"
                ));
            }
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn calendar_reads_are_independent_and_do_not_write_any_populated_state() {
    run_without_calendar(|db| async move {
        let now = database_now(&db).await?;
        let seed = CalendarSeed::valid(db.session_date, "TRADING", now);
        seed_calendar_lineage(&db, &seed, true, true).await?;
        let owner = db.seed_owner("calendar-no-write").await?;
        let fixture = db.seed_ready_membership(owner, "005930.KRX").await?;
        let app = db.repository_as_app();
        let demand_request = job_queue::owner_equity_v2::IntradayQuoteDemandRequest::new(
            Uuid::new_v4(),
            fixture.membership_id,
            fixture.generation,
            0,
            "calendar-no-write-demand".to_owned(),
        )
        .map_err(|error| format!("could not build demand fixture: {error}"))?;
        let demand = app
            .create_or_renew_demand(owner, &demand_request)
            .await
            .map_err(|error| format!("could not seed active demand: {error}"))?;
        if demand.kind != DemandMutationKind::Created {
            return Err("populated demand fixture was not created".to_owned());
        }
        seed_populated_operational_rows(&db, &fixture, &seed, now).await?;
        assert_app_actor_and_producer_boundary(&db, owner, demand.lease.demand_id).await?;

        let before = fingerprint_rows(&db.superuser, owner).await?;
        for count in [1_usize, 10, 100] {
            for _ in 0..count {
                let state = app
                    .read_current_calendar_disposition(owner)
                    .await
                    .map_err(|error| format!("repeated calendar read failed: {error}"))?
                    .ok_or_else(|| "repeated calendar read returned None".to_owned())?;
                assert_state_fields(&state, &seed, IntradayCalendarDisposition::Trading)?;
            }
        }
        let after = fingerprint_rows(&db.superuser, owner).await?;
        if before != after {
            return Err(
                "calendar reads changed an UPDATE-sensitive full-row fingerprint".to_owned(),
            );
        }
        Ok(())
    })
    .await;
}
