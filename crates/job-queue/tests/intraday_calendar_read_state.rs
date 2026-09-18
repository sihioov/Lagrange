#[allow(dead_code)]
mod intraday_quotes_support;

use chrono::{DateTime, Days, Duration, NaiveDate, Utc};
use intraday_quotes_support::{IntradayTestDb, MembershipFixture, run_body};
use job_queue::owner_equity_v2::{
    DemandMutationKind, IntradayCalendarDisposition, IntradayStorageError,
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const KIS_SOURCE_VERSION: &str = "kis-chk-holiday-v1:schema-1";

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
