use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use collectors::service_status::{
    ResearchAttemptCompletion, ResearchAttemptFence, ResearchExecutionStatusRepository,
    ResearchFailureCode, ResearchStatusWriteError,
};
use collectors::{
    RecoveryObserver, ResearchBackend, ResearchWorker, ResearchWorkerConfig, WaitOutcome,
    WorkerControl, WorkerError, WorkerRunOutcome,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgConnection, PgPool, Row};
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

// Reuse the real migration fixture contracts. Role creation is cluster-global;
// schema privileges belong only to the newly created disposable database.
const ROLE_BOOTSTRAP: &str =
    include_str!("../../../tests/integration/migration-contract/role-bootstrap.sql");
const DATABASE_BOOTSTRAP: &str =
    include_str!("../../../tests/integration/migration-contract/bootstrap.sql");

fn fixture_setup_error(stage: &'static str, error: &sqlx::Error) -> &'static str {
    let code = error
        .as_database_error()
        .and_then(|error| error.code())
        .filter(|code| {
            code.len() == 5
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        });
    if let Some(code) = code {
        eprintln!("SERVICE_STATUS_FIXTURE:{stage}:SQLSTATE:{code}");
    } else {
        let kind = match error {
            sqlx::Error::PoolTimedOut => "POOL_TIMEOUT",
            sqlx::Error::PoolClosed => "POOL_CLOSED",
            sqlx::Error::Io(_) => "IO",
            _ => "OTHER",
        };
        eprintln!("SERVICE_STATUS_FIXTURE:{stage}:ERROR_KIND:{kind}");
    }
    "service-status fixture setup failed; see sanitized stage diagnostic"
}

async fn bootstrap_roles_in_transaction(connection: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::raw_sql("SET LOCAL lock_timeout='10s'; SET LOCAL statement_timeout='30s'")
        .execute(&mut *connection)
        .await?;
    // Match the migration/job-queue fixture lock, then the older API fixture
    // lock. Transaction ownership releases both on success or failure.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('lagrange-test-role-bootstrap'))")
        .execute(&mut *connection)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(424242001)")
        .execute(&mut *connection)
        .await?;
    sqlx::raw_sql(ROLE_BOOTSTRAP)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

async fn bootstrap_roles(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let result = bootstrap_roles_in_transaction(&mut transaction).await;
    match result {
        Ok(()) => transaction.commit().await,
        Err(error) => {
            transaction.rollback().await?;
            Err(error)
        }
    }
}

async fn remove_failed_setup(pool: &PgPool, name: &str) -> Result<(), &'static str> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
    )))
    .execute(pool)
    .await
    .map_err(|error| fixture_setup_error("SETUP_CLEANUP", &error))?;
    let absent: bool =
        sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=$1)")
            .bind(name)
            .fetch_one(pool)
            .await
            .map_err(|error| fixture_setup_error("SETUP_CLEANUP_CHECK", &error))?;
    check(absent, "failed fixture database still exists")
}

struct ScratchDb {
    name: String,
    base_options: PgConnectOptions,
    owner: PgPool,
    writer: PgPool,
    admin: PgPool,
    app: PgPool,
    worker: PgPool,
    audit: PgPool,
}

impl ScratchDb {
    async fn create() -> Result<Self, &'static str> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| "service_status requires DATABASE_URL; database suite cannot skip")?;
        let base_options = database_url
            .parse::<PgConnectOptions>()
            .map_err(|_| "test database URL is not parseable")?;
        let super_pool = connect(base_options.clone()).await?;
        if let Err(error) = bootstrap_roles(&super_pool).await {
            super_pool.close().await;
            return Err(fixture_setup_error("ROLE_BOOTSTRAP", &error));
        }
        let name = format!("status_{}_{}", std::process::id(), Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE \"{name}\" OWNER migration_owner"
        )))
        .execute(&super_pool)
        .await
        .map_err(|_| "isolated service-status database could not be created")?;
        let target_setup = connect(base_options.clone().database(&name)).await;
        let target_result = match target_setup {
            Ok(target) => {
                let result = sqlx::raw_sql(DATABASE_BOOTSTRAP)
                    .execute(&target)
                    .await
                    .map_err(|error| fixture_setup_error("TARGET_GRANTS", &error));
                target.close().await;
                result.map(|_| ())
            }
            Err(error) => Err(error),
        };
        if let Err(error) = target_result {
            let cleanup = remove_failed_setup(&super_pool, &name).await;
            super_pool.close().await;
            cleanup?;
            return Err(error);
        }
        super_pool.close().await;

        let owner = connect(role_options(&base_options, "migration_owner", &name)).await?;
        let writer = connect(role_options(&base_options, "research_writer", &name)).await?;
        let admin = connect(role_options(&base_options, "admin", &name)).await?;
        let app = connect(role_options(&base_options, "app", &name)).await?;
        let worker = connect(role_options(&base_options, "worker", &name)).await?;
        let audit = connect(role_options(&base_options, "audit_writer", &name)).await?;
        let db = Self {
            name,
            base_options,
            owner,
            writer,
            admin,
            app,
            worker,
            audit,
        };
        if MIGRATOR.run(&db.owner).await.is_err() {
            db.teardown().await;
            return Err("real workspace migrations did not apply to isolated database");
        }
        Ok(db)
    }

    async fn teardown(self) {
        let Self {
            name,
            base_options,
            owner,
            writer,
            admin,
            app,
            worker,
            audit,
        } = self;
        drop(owner);
        drop(writer);
        drop(admin);
        drop(app);
        drop(worker);
        drop(audit);
        if let Ok(super_pool) = connect(base_options).await {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
            )))
            .execute(&super_pool)
            .await;
        }
    }
}

#[tokio::test]
async fn role_bootstrap_lock_is_held_until_commit_or_rollback() {
    let database_url = std::env::var("DATABASE_URL")
        .expect("service_status requires DATABASE_URL; database suite cannot skip");
    let options = database_url
        .parse::<PgConnectOptions>()
        .expect("test database URL is not parseable");
    let pool = connect(options)
        .await
        .expect("isolated test database connection failed");
    for commit in [true, false] {
        let mut transaction = pool.begin().await.expect("bootstrap transaction required");
        let backend: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await
            .expect("bootstrap backend required");
        bootstrap_roles_in_transaction(&mut transaction)
            .await
            .expect("fixture role bootstrap required");
        let lock_query = "SELECT EXISTS (
            SELECT 1 FROM pg_catalog.pg_locks
             WHERE locktype = 'advisory' AND pid = $1
               AND classid = 0 AND objid = 424242001 AND objsubid = 1 AND granted
        )";
        let held: bool = sqlx::query_scalar(lock_query)
            .bind(backend)
            .fetch_one(&pool)
            .await
            .expect("bootstrap lock observation required");
        assert!(held, "bootstrap unlocked before transaction completion");
        if commit {
            transaction
                .commit()
                .await
                .expect("bootstrap commit required");
        } else {
            transaction
                .rollback()
                .await
                .expect("bootstrap rollback required");
        }
        let held: bool = sqlx::query_scalar(lock_query)
            .bind(backend)
            .fetch_one(&pool)
            .await
            .expect("released bootstrap lock observation required");
        assert!(!held, "bootstrap lock survived transaction completion");
    }
    pool.close().await;
}

async fn connect(options: PgConnectOptions) -> Result<PgPool, &'static str> {
    let options = options.disable_statement_logging();
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect_with(options)
        .await
        .map_err(|_| "isolated test database connection failed")
}

fn role_options(base: &PgConnectOptions, role: &str, database: &str) -> PgConnectOptions {
    base.clone()
        .username(role)
        .password("lagrange")
        .database(database)
}

fn check(condition: bool, reason: &'static str) -> Result<(), &'static str> {
    condition.then_some(()).ok_or(reason)
}

async fn status_row(
    pool: &PgPool,
) -> Result<
    (
        String,
        Option<DateTime<Utc>>,
        Option<NaiveDate>,
        Option<DateTime<Utc>>,
    ),
    &'static str,
> {
    sqlx::query_as(
        "SELECT attempt_state, last_success_at, published_session, heartbeat_at
           FROM public.research_execution_status WHERE id = 'kis-eod'",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "admin could not read the fixed status row")
}

struct Counts {
    recover: AtomicUsize,
    has_eod: AtomicUsize,
    ingest: AtomicUsize,
    revoke_status_update: Option<PgPool>,
}

#[async_trait]
impl ResearchBackend for Counts {
    async fn recover(
        &self,
        _control: &dyn WorkerControl,
        _observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        self.recover.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn has_eod(&self, _date: TradingDate) -> Result<bool, WorkerError> {
        self.has_eod.fetch_add(1, Ordering::SeqCst);
        if let Some(owner) = &self.revoke_status_update {
            sqlx::raw_sql(
                "REVOKE UPDATE ON TABLE public.research_execution_status FROM research_writer",
            )
            .execute(owner)
            .await
            .map_err(|_| WorkerError::ProviderNotConfigured)?;
        }
        Ok(true)
    }

    async fn ingest(
        &self,
        _date: TradingDate,
        _now: UtcTimestamp,
        _control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError> {
        self.ingest.fetch_add(1, Ordering::SeqCst);
        Err(WorkerError::ProviderNotConfigured)
    }
}

struct FixedControl(DateTime<Utc>);

#[async_trait]
impl WorkerControl for FixedControl {
    fn now_utc(&self) -> DateTime<Utc> {
        self.0
    }

    async fn wait(&self, _duration: Option<Duration>) -> WaitOutcome {
        if _duration.is_none() {
            std::future::pending().await
        } else {
            WaitOutcome::Shutdown
        }
    }
}

fn fixture_worker_config() -> Result<ResearchWorkerConfig, &'static str> {
    let values = HashMap::from([
        ("APP_ENV".to_owned(), "development".to_owned()),
        ("RESEARCH_FETCH_MODE".to_owned(), "synthetic".to_owned()),
        ("RESEARCH_RAW_ROOT".to_owned(), "/tmp/status-raw".to_owned()),
        (
            "RESEARCH_CURATED_ROOT".to_owned(),
            "/tmp/status-curated".to_owned(),
        ),
        (
            "RESEARCH_ENTITLEMENT_REFERENCE".to_owned(),
            "fixture:service-status".to_owned(),
        ),
        ("DB_HOST".to_owned(), "127.0.0.1".to_owned()),
        ("DB_PORT".to_owned(), "5432".to_owned()),
        ("DB_NAME".to_owned(), "fixture-only".to_owned()),
        ("DB_USER".to_owned(), "fixture-only".to_owned()),
        (
            "DB_PASSWORD_FILE".to_owned(),
            "/fixture/db-password".to_owned(),
        ),
    ]);
    ResearchWorkerConfig::from_map_with_reader(&values, |_| Ok("fixture".to_owned()))
        .map_err(|_| "provider-free ResearchWorker fixture config was rejected")
}

async fn seed_curated_publication(
    db: &ScratchDb,
    target: NaiveDate,
) -> Result<DateTime<Utc>, &'static str> {
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO public.users (issuer, subject, email)
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("fixture-status-{}", Uuid::new_v4()))
    .bind("research-status")
    .bind(format!("status-{}@fixture.invalid", Uuid::new_v4()))
    .fetch_one(&db.owner)
    .await
    .map_err(|_| "synthetic publication principal could not be seeded")?;

    let entitlement_id = Uuid::new_v4();
    let contract_reference = format!("fixture:curated:{}", Uuid::new_v4());
    let contract_sha = "d".repeat(64);
    sqlx::query(
        "INSERT INTO public.data_entitlements (
             id, contract_document_sha256, contract_reference, status,
             covered_datasets, covered_uses, effective_from, effective_until, managed_by
         ) VALUES (
             $1, $2, $3, 'ACTIVE', '[\"krx_eod_bars\"]'::jsonb,
             '[\"candidate\",\"dataset\",\"recommendation\",\"backtest\",\"paper_view\"]'::jsonb,
             $4, $4, $5
         )",
    )
    .bind(entitlement_id)
    .bind(contract_sha)
    .bind(&contract_reference)
    .bind(target)
    .bind(user_id)
    .execute(&db.owner)
    .await
    .map_err(|_| "synthetic exact publication rights could not be seeded")?;

    for symbol in market_data::providers::kis::KR_ETF_CORE_SYMBOLS {
        let instrument_id = format!("{symbol}.KRX");
        sqlx::query(
            "INSERT INTO public.instruments (
                 id, symbol, venue, currency, name, asset_class, status
             ) VALUES ($1, $2, 'KRX', 'KRW', 'Fixture ETF', 'ETF', 'ACTIVE')
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(instrument_id)
        .bind(symbol)
        .execute(&db.owner)
        .await
        .map_err(|_| "fixed ETF11 synthetic instrument could not be seeded")?;
    }

    let session = target.format("%Y-%m-%d").to_string();
    let coverage = market_data::providers::kis::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            serde_json::json!({
                "instrument_id": format!("{symbol}.KRX"),
                "first_session": session,
                "last_session": session,
                "session_count": 1,
                "sessions": [session],
            })
        })
        .collect::<Vec<_>>();
    let coverage = serde_json::to_string(&coverage)
        .map_err(|_| "synthetic ETF11 coverage could not be serialized")?;
    let raw_batch_id = Uuid::new_v4();
    let manifest_sha = "a".repeat(64);
    let raw_sha = "b".repeat(64);
    let published_at: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&db.writer)
        .await
        .map_err(|_| "synthetic publication timestamp could not be captured")?;
    let _: (Uuid, bool) = sqlx::query_as(
        "SELECT * FROM public.publish_candidate_price_publication(
             $1, $2, $3, 1, $4, $4, $5::jsonb, 'fixture-kis', $6, $7,
             $8, $9, $10, 'credentialed', $4, $11, $11
         )",
    )
    .bind(format!("fixture-status-{}", Uuid::new_v4()))
    .bind(manifest_sha)
    .bind("fixture/service-status/curated")
    .bind(target)
    .bind(coverage)
    .bind(entitlement_id)
    .bind(contract_reference)
    .bind(raw_batch_id.to_string())
    .bind(raw_batch_id)
    .bind(raw_sha)
    .bind(published_at)
    .fetch_one(&db.writer)
    .await
    .map_err(|_| "production candidate publication boundary rejected synthetic fixture")?;

    let published_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT published_at FROM public.candidate_raw_batch_publications
          WHERE batch_id = $1 AND surface = 'price'",
    )
    .bind(raw_batch_id)
    .fetch_one(&db.writer)
    .await
    .map_err(|_| "synthetic curated raw binding was not published")?;
    Ok(published_at)
}

async fn check_database_contract(db: &ScratchDb) -> Result<(), &'static str> {
    let empty_rows: i64 =
        sqlx::query_scalar("SELECT pg_catalog.count(*) FROM public.research_execution_status")
            .fetch_one(&db.admin)
            .await
            .map_err(|_| "admin status read failed before first attempt")?;
    check(
        empty_rows == 0,
        "migration fabricated an initial status row",
    )?;

    let researcher_can_write: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
             current_user, 'public.research_execution_status', 'INSERT,UPDATE'
         )",
    )
    .fetch_one(&db.writer)
    .await
    .map_err(|_| "research_writer privilege check failed")?;
    check(
        researcher_can_write,
        "research_writer lost status write capability",
    )?;
    let (rls_enabled, force_rls): (bool, bool) = sqlx::query_as(
        "SELECT relation.relrowsecurity, relation.relforcerowsecurity
           FROM pg_catalog.pg_class AS relation
          WHERE relation.oid = 'public.research_execution_status'::regclass",
    )
    .fetch_one(&db.admin)
    .await
    .map_err(|_| "status RLS metadata could not be read")?;
    check(
        rls_enabled && force_rls,
        "research status table does not enable and force row security",
    )?;
    let admin_can_mutate: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_table_privilege(
             current_user, 'public.research_execution_status',
             'INSERT,UPDATE,DELETE,TRUNCATE'
         )",
    )
    .fetch_one(&db.admin)
    .await
    .map_err(|_| "admin status mutation privilege check failed")?;
    check(!admin_can_mutate, "admin received a status mutation grant")?;
    for pool in [&db.app, &db.worker, &db.audit] {
        if sqlx::query("SELECT 1 FROM public.research_execution_status")
            .fetch_optional(pool)
            .await
            .is_ok()
        {
            return Err("a denied serving role can read research execution status");
        }
    }
    if sqlx::query("INSERT INTO public.research_execution_status DEFAULT VALUES")
        .execute(&db.admin)
        .await
        .is_ok()
        || sqlx::query(
            "UPDATE public.research_execution_status
                SET updated_at = pg_catalog.clock_timestamp()",
        )
        .execute(&db.admin)
        .await
        .is_ok()
        || sqlx::query("DELETE FROM public.research_execution_status")
            .execute(&db.writer)
            .await
            .is_ok()
    {
        return Err("status ACL allowed admin insert or research_writer delete");
    }

    let target = NaiveDate::from_ymd_opt(2026, 10, 8).ok_or("test target date is invalid")?;
    let counts = Arc::new(Counts {
        recover: AtomicUsize::new(0),
        has_eod: AtomicUsize::new(0),
        ingest: AtomicUsize::new(0),
        revoke_status_update: None,
    });
    let worker = ResearchWorker::new(fixture_worker_config()?, counts.clone())
        .with_service_status_repository(ResearchExecutionStatusRepository::new(db.admin.clone()));
    let date = TradingDate::new(target.year(), target.month(), target.day())
        .map_err(|_| "provider-free test target could not be constructed")?;
    let control = FixedControl(Utc::now());
    let start_failure_result = worker
        .run_once(date, &control)
        .await
        .map_err(|_| "worker lost its pipeline result when status start failed")?;
    check(
        start_failure_result == WorkerRunOutcome::AlreadyPublished,
        "status start failure replaced the worker's no-work outcome",
    )?;
    check(
        counts.recover.load(Ordering::SeqCst) == 1
            && counts.has_eod.load(Ordering::SeqCst) == 1
            && counts.ingest.load(Ordering::SeqCst) == 0,
        "status start failure changed worker call counts",
    )?;
    let still_empty: i64 =
        sqlx::query_scalar("SELECT pg_catalog.count(*) FROM public.research_execution_status")
            .fetch_one(&db.admin)
            .await
            .map_err(|_| "admin could not verify rejected start stayed absent")?;
    check(
        still_empty == 0,
        "rejected first status start fabricated a row",
    )?;

    let repository = ResearchExecutionStatusRepository::new(db.writer.clone());
    let (first, second) = tokio::join!(repository.start(target), repository.start(target));
    let first = first.map_err(|_| "first concurrent status fence write failed")?;
    let second = second.map_err(|_| "second concurrent status fence write failed")?;
    check(
        first.is_some() ^ second.is_some(),
        "concurrent attempts did not produce exactly one status fence",
    )?;
    let fence = first
        .or(second)
        .ok_or("confirmed status fence is missing")?;
    check(
        repository
            .start(target)
            .await
            .map_err(|_| "active fence check failed")?
            .is_none(),
        "second attempt overwrote an active singleton fence",
    )?;

    check(
        repository
            .heartbeat(fence)
            .await
            .map_err(|_| "initial research heartbeat failed")?,
        "current attempt rejected its own heartbeat fence",
    )?;
    let before_rejected_heartbeat = status_row(&db.admin).await?.3;
    sqlx::raw_sql("REVOKE UPDATE ON TABLE public.research_execution_status FROM research_writer")
        .execute(&db.owner)
        .await
        .map_err(|_| "heartbeat failure injection could not revoke UPDATE")?;
    check(
        repository.heartbeat(fence).await.is_err(),
        "injected heartbeat failure was reported as persisted",
    )?;
    let after_rejected_heartbeat = status_row(&db.admin).await?.3;
    check(
        before_rejected_heartbeat == after_rejected_heartbeat,
        "rejected heartbeat advanced persisted observability",
    )?;
    sqlx::raw_sql(
        "GRANT SELECT, INSERT, UPDATE ON TABLE public.research_execution_status TO research_writer",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "heartbeat failure injection could not restore UPDATE")?;

    sqlx::raw_sql("REVOKE UPDATE ON TABLE public.research_execution_status FROM research_writer")
        .execute(&db.owner)
        .await
        .map_err(|_| "finish failure injection could not revoke UPDATE")?;
    check(
        repository
            .finish(fence, ResearchAttemptCompletion::NoWork)
            .await
            .is_err(),
        "injected finish failure was reported as persisted",
    )?;
    check(
        status_row(&db.admin).await?.0 == "RUNNING",
        "rejected finish changed the persisted attempt state",
    )?;
    sqlx::raw_sql(
        "GRANT SELECT, INSERT, UPDATE ON TABLE public.research_execution_status TO research_writer",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "finish failure injection could not restore UPDATE")?;
    check(
        repository
            .finish(fence, ResearchAttemptCompletion::NoWork)
            .await
            .map_err(|_| "confirmed no-work finish failed")?,
        "confirmed no-work finish did not update the current fence",
    )?;
    check(
        status_row(&db.admin).await?.0 == "NO_WORK",
        "no-work attempt did not persist its terminal state",
    )?;

    let prior_success_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE public.research_execution_status
            SET last_success_at = pg_catalog.clock_timestamp() - INTERVAL '1 hour',
                published_session = $1
          WHERE id = 'kis-eod'
          RETURNING last_success_at",
    )
    .bind(
        target
            .pred_opt()
            .ok_or("prior synthetic target underflowed")?,
    )
    .fetch_one(&db.owner)
    .await
    .map_err(|_| "prior success fixture could not be seeded")?;
    sqlx::raw_sql(
        "REVOKE INSERT, UPDATE ON TABLE public.research_execution_status FROM research_writer",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "prior-row start failure injection could not revoke writes")?;
    check(
        repository.start(target).await.is_err(),
        "rejected start with prior row was reported as confirmed",
    )?;
    sqlx::raw_sql(
        "GRANT SELECT, INSERT, UPDATE ON TABLE public.research_execution_status TO research_writer",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "prior-row start failure injection could not restore writes")?;
    let prior_row = status_row(&db.admin).await?;
    check(
        prior_row.0 == "NO_WORK" && prior_row.1 == Some(prior_success_at),
        "rejected start overwrote a prior terminal row or success timestamp",
    )?;

    let no_work_fence = repository
        .start(target)
        .await
        .map_err(|_| "second no-work fence could not be started")?
        .ok_or("terminal row refused a new attempt")?;
    repository
        .finish(no_work_fence, ResearchAttemptCompletion::NoWork)
        .await
        .map_err(|_| "second no-work completion write failed")?;
    let preserved = status_row(&db.admin).await?;
    check(
        preserved.1 == Some(prior_success_at) && preserved.2 == Some(target.pred_opt().unwrap()),
        "NO_WORK erased or replaced the previous Curated success",
    )?;

    let publication_fence = repository
        .start(target)
        .await
        .map_err(|_| "publication verification attempt could not start")?
        .ok_or("publication verification attempt had no fence")?;
    check(
        repository
            .finish(
                publication_fence,
                ResearchAttemptCompletion::Published {
                    target_session: target,
                },
            )
            .await
            .map_err(|_| "missing-publication verification write failed")?,
        "missing Curated publication failed to finish status attempt",
    )?;
    let rejected_publication = status_row(&db.admin).await?;
    check(
        rejected_publication.0 == "FAILED"
            && rejected_publication.1 == Some(prior_success_at)
            && rejected_publication.2 == Some(target.pred_opt().unwrap()),
        "Raw-only or missing Curated publication advanced last success",
    )?;
    let failure_code: Option<String> = sqlx::query_scalar(
        "SELECT failure_code FROM public.research_execution_status WHERE id = 'kis-eod'",
    )
    .fetch_one(&db.admin)
    .await
    .map_err(|_| "publication failure code could not be read")?;
    check(
        failure_code.as_deref() == Some("PUBLICATION_FAILED"),
        "unverified Curated publication did not use its fixed failure code",
    )?;

    let publisher_target: NaiveDate =
        sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&db.owner)
            .await
            .map_err(|_| "current synthetic Curated target session could not be read")?;
    let published_at = seed_curated_publication(db, publisher_target).await?;
    let success_fence = repository
        .start(publisher_target)
        .await
        .map_err(|_| "verified publication attempt could not start")?
        .ok_or("verified publication attempt had no fence")?;
    check(
        repository
            .finish(
                success_fence,
                ResearchAttemptCompletion::Published {
                    target_session: publisher_target,
                },
            )
            .await
            .map_err(|_| "verified publication completion failed")?,
        "verified Curated publication was not accepted",
    )?;
    let successful = status_row(&db.admin).await?;
    check(
        successful.0 == "SUCCEEDED"
            && successful.1 == Some(published_at)
            && successful.2 == Some(publisher_target),
        "SUCCEEDED did not bind to exact published Curated source time/session",
    )?;

    let current_fence = repository
        .start(publisher_target)
        .await
        .map_err(|_| "current fenced attempt could not start")?
        .ok_or("current fenced attempt had no fence")?;
    check(
        !repository
            .finish(success_fence, ResearchAttemptCompletion::NoWork)
            .await
            .map_err(|_| "stale fence completion query failed")?,
        "stale producer fence updated the newer RUNNING attempt",
    )?;
    check(
        status_row(&db.admin).await?.0 == "RUNNING",
        "stale fence changed the current attempt state",
    )?;
    repository
        .finish(current_fence, ResearchAttemptCompletion::NoWork)
        .await
        .map_err(|_| "current fenced attempt could not finish")?;

    let ambiguous_fence = repository
        .start(publisher_target)
        .await
        .map_err(|_| "ambiguous-timeout attempt could not start")?
        .ok_or("ambiguous-timeout attempt had no fence")?;
    sqlx::raw_sql(
        "CREATE FUNCTION public.service_status_test_delay_finish()
         RETURNS trigger LANGUAGE plpgsql AS $delay$
         BEGIN
             IF NEW.attempt_state = 'NO_WORK' THEN
                 PERFORM pg_catalog.pg_sleep(3);
             END IF;
             RETURN NEW;
         END
         $delay$;
         CREATE TRIGGER service_status_test_delay_finish
         BEFORE UPDATE OF attempt_state ON public.research_execution_status
         FOR EACH ROW EXECUTE FUNCTION public.service_status_test_delay_finish()",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "synthetic delayed-finish trigger could not be installed")?;
    let ambiguous_finish = repository
        .finish(ambiguous_fence, ResearchAttemptCompletion::NoWork)
        .await;
    check(
        matches!(ambiguous_finish, Err(ResearchStatusWriteError::Timeout)),
        "delayed completion did not surface as a typed write timeout",
    )?;
    sqlx::raw_sql(
        "DROP TRIGGER service_status_test_delay_finish
             ON public.research_execution_status;
         DROP FUNCTION public.service_status_test_delay_finish()",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "synthetic delayed-finish trigger could not be removed")?;
    let observed_after_timeout = sqlx::query(
        "SELECT attempt_state, attempt_started_at, attempt_finished_at,
                last_success_at, published_session
           FROM public.research_execution_status
          WHERE id = 'kis-eod'",
    )
    .fetch_one(&db.admin)
    .await
    .map_err(|_| "timed-out attempt state could not be observed")?;
    let observed_state: String = observed_after_timeout
        .try_get("attempt_state")
        .map_err(|_| "timed-out attempt state was malformed")?;
    let observed_start: DateTime<Utc> = observed_after_timeout
        .try_get("attempt_started_at")
        .map_err(|_| "timed-out attempt fence was malformed")?;
    let observed_finish: Option<DateTime<Utc>> = observed_after_timeout
        .try_get("attempt_finished_at")
        .map_err(|_| "timed-out attempt completion was malformed")?;
    let observed_success: Option<DateTime<Utc>> = observed_after_timeout
        .try_get("last_success_at")
        .map_err(|_| "timed-out attempt success time was malformed")?;
    let observed_session: Option<NaiveDate> =
        observed_after_timeout
            .try_get("published_session")
            .map_err(|_| "timed-out attempt published session was malformed")?;
    check(
        (observed_state == "RUNNING" && observed_finish.is_none())
            || (observed_state == "NO_WORK"
                && observed_finish.is_some_and(|finished_at| finished_at >= observed_start)),
        "timed-out completion was projected without a valid persisted row",
    )?;
    check(
        observed_success == Some(published_at) && observed_session == Some(publisher_target),
        "ambiguous completion changed the last verified Curated success",
    )?;
    if observed_state == "RUNNING" {
        check(
            repository
                .finish(ambiguous_fence, ResearchAttemptCompletion::NoWork)
                .await
                .map_err(|_| "observed RUNNING timeout fence could not be cleaned up")?,
            "observed RUNNING timeout fence was no longer current",
        )?;
    }

    let finish_failure_counts = Arc::new(Counts {
        recover: AtomicUsize::new(0),
        has_eod: AtomicUsize::new(0),
        ingest: AtomicUsize::new(0),
        revoke_status_update: Some(db.owner.clone()),
    });
    let worker = ResearchWorker::new(fixture_worker_config()?, finish_failure_counts.clone())
        .with_service_status_repository(repository.clone());
    let result = worker
        .run_once(
            TradingDate::new(
                publisher_target.year(),
                publisher_target.month(),
                publisher_target.day(),
            )
            .map_err(|_| "current status target could not be constructed")?,
            &control,
        )
        .await
        .map_err(|_| "finish telemetry failure changed worker pipeline outcome")?;
    check(
        result == WorkerRunOutcome::AlreadyPublished,
        "finish telemetry failure replaced the original pipeline result",
    )?;
    check(
        finish_failure_counts.recover.load(Ordering::SeqCst) == 1
            && finish_failure_counts.has_eod.load(Ordering::SeqCst) == 1
            && finish_failure_counts.ingest.load(Ordering::SeqCst) == 0,
        "finish telemetry failure retried or changed pipeline call counts",
    )?;
    check(
        status_row(&db.admin).await?.0 == "RUNNING",
        "rejected worker finish fabricated terminal status evidence",
    )?;
    sqlx::raw_sql(
        "GRANT SELECT, INSERT, UPDATE ON TABLE public.research_execution_status TO research_writer",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "worker finish failure injection could not restore UPDATE")?;

    let down_sql = include_str!("../../../migrations/0059_research_execution_status.down.sql");
    let mut down_tx = db
        .owner
        .begin()
        .await
        .map_err(|_| "status down-migration transaction could not start")?;
    let nonempty_down = sqlx::raw_sql(down_sql).execute(&mut *down_tx).await;
    check(
        nonempty_down.is_err(),
        "0059 down migration silently removed a nonempty status row",
    )?;
    down_tx
        .rollback()
        .await
        .map_err(|_| "refused status down migration could not roll back")?;

    sqlx::query("DELETE FROM public.research_execution_status WHERE id = 'kis-eod'")
        .execute(&db.owner)
        .await
        .map_err(|_| "synthetic status fixture cleanup failed")?;
    let mut down_tx = db
        .owner
        .begin()
        .await
        .map_err(|_| "empty status down-migration transaction could not start")?;
    sqlx::raw_sql(down_sql)
        .execute(&mut *down_tx)
        .await
        .map_err(|_| "0059 down migration did not drop an empty fixture table")?;
    down_tx
        .commit()
        .await
        .map_err(|_| "empty status down migration did not commit")?;
    Ok(())
}

#[tokio::test]
async fn status_repository_is_fenced_best_effort_and_publication_bound() {
    assert!(
        std::env::var_os("DATABASE_URL").is_some(),
        "service_status requires DATABASE_URL; database suite cannot skip"
    );
    let db = ScratchDb::create()
        .await
        .expect("isolated service-status database with actual migrations is required");
    let result = check_database_contract(&db).await;
    db.teardown().await;
    assert_eq!(
        result,
        Ok(()),
        "service-status repository DB assertion failed"
    );
}

type AttemptSnapshot = (
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    String,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<NaiveDate>,
    Option<NaiveDate>,
    DateTime<Utc>,
);

async fn attempt_snapshot(db: &ScratchDb) -> Result<AttemptSnapshot, &'static str> {
    sqlx::query_as(
        "SELECT attempt_started_at, heartbeat_at, attempt_finished_at, attempt_state,
                last_success_at, last_failure_at, failure_code, target_session,
                published_session, updated_at
           FROM public.research_execution_status WHERE id = 'kis-eod'",
    )
    .fetch_one(&db.admin)
    .await
    .map_err(|_| "recorded attempt snapshot could not be read")
}

async fn age_running_fixture(db: &ScratchDb) -> Result<ResearchAttemptFence, &'static str> {
    let attempt_started_at = sqlx::query_scalar(
        "WITH fixture_clock AS MATERIALIZED (SELECT pg_catalog.clock_timestamp() AS now)
         UPDATE public.research_execution_status AS status
            SET attempt_started_at = fixture_clock.now - INTERVAL '120 seconds',
                heartbeat_at = fixture_clock.now - INTERVAL '91 seconds'
           FROM fixture_clock
          WHERE status.id = 'kis-eod' AND status.attempt_state = 'RUNNING'
          RETURNING status.attempt_started_at",
    )
    .fetch_one(&db.owner)
    .await
    .map_err(|_| "synthetic interrupted attempt could not be aged")?;
    Ok(ResearchAttemptFence { attempt_started_at })
}

async fn check_stale_takeover(db: &ScratchDb) -> Result<(), &'static str> {
    let target: NaiveDate =
        sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&db.owner)
            .await
            .map_err(|_| "takeover fixture session could not be read")?;
    let published_at = seed_curated_publication(db, target).await?;
    let repository = ResearchExecutionStatusRepository::new(db.writer.clone());
    let success_fence = repository
        .start(target)
        .await
        .map_err(|_| "takeover prior-success start failed")?
        .ok_or("takeover prior-success fence absent")?;
    check(
        repository
            .finish(
                success_fence,
                ResearchAttemptCompletion::Published {
                    target_session: target,
                },
            )
            .await
            .map_err(|_| "takeover prior-success finish failed")?,
        "takeover prior-success finish lost its fence",
    )?;
    let failed_fence = repository
        .start(target)
        .await
        .map_err(|_| "takeover prior-failure start failed")?
        .ok_or("takeover prior-failure fence absent")?;
    check(
        repository
            .finish(
                failed_fence,
                ResearchAttemptCompletion::Failed(ResearchFailureCode::PipelineFailed),
            )
            .await
            .map_err(|_| "takeover prior-failure finish failed")?,
        "takeover prior-failure finish lost its fence",
    )?;
    let active_fence = repository
        .start(target)
        .await
        .map_err(|_| "takeover active start failed")?
        .ok_or("takeover active fence absent")?;
    let active = attempt_snapshot(db).await?;
    check(
        repository
            .start(target)
            .await
            .map_err(|_| "takeover active-denial query failed")?
            .is_none(),
        "fresh RUNNING attempt was replaced",
    )?;
    check(
        attempt_snapshot(db).await? == active,
        "active start denial changed persisted evidence",
    )?;
    check(
        repository
            .heartbeat(active_fence)
            .await
            .map_err(|_| "active fence heartbeat failed")?,
        "active denial revoked the current fence",
    )?;

    let interrupted_fence = age_running_fixture(db).await?;
    let interrupted = attempt_snapshot(db).await?;
    let takeover = repository
        .start(target)
        .await
        .map_err(|_| "stale RUNNING takeover query failed")?
        .ok_or("stale RUNNING row blocked a new actual attempt")?;
    let current = attempt_snapshot(db).await?;
    check(
        takeover.attempt_started_at == current.0
            && current.0 > interrupted.0
            && current.1 == current.0
            && current.2.is_none()
            && current.3 == "RUNNING"
            && current.4 == Some(published_at)
            && current.4 == interrupted.4
            && current.5.is_some()
            && current.5 == interrupted.5
            && current.6.is_none()
            && current.7 == Some(target)
            && current.8 == interrupted.8
            && current.9 == current.0,
        "stale takeover fabricated terminal evidence or erased verified history",
    )?;
    check(
        !repository
            .heartbeat(interrupted_fence)
            .await
            .map_err(|_| "old takeover fence heartbeat query failed")?,
        "old takeover fence advanced the new heartbeat",
    )?;
    for completion in [
        ResearchAttemptCompletion::NoWork,
        ResearchAttemptCompletion::Failed(ResearchFailureCode::ExecutionInterrupted),
        ResearchAttemptCompletion::Published {
            target_session: target,
        },
    ] {
        check(
            !repository
                .finish(interrupted_fence, completion)
                .await
                .map_err(|_| "old takeover fence finish query failed")?,
            "old takeover fence completed the new attempt",
        )?;
    }
    check(
        attempt_snapshot(db).await? == current,
        "old-fence operations changed the newer persisted row",
    )?;

    let concurrent_old_fence = age_running_fixture(db).await?;
    let before_concurrent = attempt_snapshot(db).await?;
    let (first, second) = tokio::join!(repository.start(target), repository.start(target));
    let first = first.map_err(|_| "first concurrent stale takeover failed")?;
    let second = second.map_err(|_| "second concurrent stale takeover failed")?;
    check(
        first.is_some() ^ second.is_some(),
        "concurrent stale takeover did not have exactly one winner",
    )?;
    let winner = first
        .or(second)
        .ok_or("concurrent takeover winner absent")?;
    let after_concurrent = attempt_snapshot(db).await?;
    check(
        after_concurrent.0 == winner.attempt_started_at
            && after_concurrent.4 == before_concurrent.4
            && after_concurrent.5 == before_concurrent.5
            && after_concurrent.8 == before_concurrent.8,
        "concurrent takeover lost its fence or verified history",
    )?;
    check(
        !repository
            .heartbeat(concurrent_old_fence)
            .await
            .map_err(|_| "concurrent old-fence heartbeat query failed")?
            && !repository
                .finish(concurrent_old_fence, ResearchAttemptCompletion::NoWork)
                .await
                .map_err(|_| "concurrent old-fence finish query failed")?,
        "pre-takeover fence changed the concurrent winner",
    )?;
    check(
        attempt_snapshot(db).await? == after_concurrent,
        "concurrent old-fence denial changed stored timestamps",
    )?;

    // Test-only BEFORE INSERT fixture pins NEW to the persisted synthetic DB
    // heartbeat + 90s (then + 1us). It never updates the conflicting row in the
    // same INSERT. The real repository SQL executes both exact comparisons;
    // there is no caller clock argument, SQL mirror, or wall-clock sleep.
    age_running_fixture(db).await?;
    let exact_prior = attempt_snapshot(db).await?;
    sqlx::raw_sql(
        "CREATE TABLE public.service_status_test_start_clock (
             attempted_at timestamptz NOT NULL, prior_heartbeat_at timestamptz NOT NULL
         );
         CREATE FUNCTION public.service_status_test_start_boundary()
         RETURNS trigger LANGUAGE plpgsql AS $boundary$
         DECLARE prior_heartbeat timestamptz;
         BEGIN
             SELECT heartbeat_at INTO STRICT prior_heartbeat
               FROM public.research_execution_status WHERE id = 'kis-eod';
             NEW.attempt_started_at := prior_heartbeat + INTERVAL '90 seconds' + TG_ARGV[0]::interval;
             NEW.heartbeat_at := NEW.attempt_started_at;
             NEW.updated_at := NEW.attempt_started_at;
             INSERT INTO public.service_status_test_start_clock
                 VALUES (NEW.attempt_started_at, prior_heartbeat);
             RETURN NEW;
         END
         $boundary$;
         CREATE TRIGGER service_status_test_start_boundary
         BEFORE INSERT ON public.research_execution_status
         FOR EACH ROW EXECUTE FUNCTION public.service_status_test_start_boundary('0 seconds');
         GRANT INSERT ON public.service_status_test_start_clock TO research_writer",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "exact-age isolated SQL fixture could not be installed")?;
    check(
        repository
            .start(target)
            .await
            .map_err(|_| "exact90 repository SQL execution failed")?
            .is_none(),
        "exactly 90s heartbeat was treated as stale",
    )?;
    let exact_age: bool = sqlx::query_scalar(
        "SELECT status.heartbeat_at = fixture.attempted_at - INTERVAL '90 seconds'
                AND fixture.prior_heartbeat_at = status.heartbeat_at
           FROM public.research_execution_status AS status
           CROSS JOIN public.service_status_test_start_clock AS fixture",
    )
    .fetch_one(&db.owner)
    .await
    .map_err(|_| "exact90 SQL fixture comparison failed")?;
    check(
        exact_age,
        "exact90 SQL fixture did not have the exact persisted boundary",
    )?;
    check(
        attempt_snapshot(db).await? == exact_prior,
        "exact90 start denial changed the prior recorded attempt",
    )?;
    sqlx::raw_sql(
        "DROP TRIGGER service_status_test_start_boundary ON public.research_execution_status;
         TRUNCATE public.service_status_test_start_clock;
         CREATE TRIGGER service_status_test_start_boundary
         BEFORE INSERT ON public.research_execution_status
         FOR EACH ROW EXECUTE FUNCTION public.service_status_test_start_boundary('1 microsecond')",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "exact90plus1us isolated fixture could not be installed")?;
    let strict_takeover = repository
        .start(target)
        .await
        .map_err(|_| "exact90plus1us repository SQL execution failed")?
        .ok_or("90s plus one microsecond did not permit takeover")?;
    let strict_age: bool = sqlx::query_scalar(
        "SELECT attempted_at = $1
                AND prior_heartbeat_at = attempted_at - INTERVAL '90 seconds'
                                                   - INTERVAL '1 microsecond'
           FROM public.service_status_test_start_clock",
    )
    .bind(strict_takeover.attempt_started_at)
    .fetch_one(&db.owner)
    .await
    .map_err(|_| "strict-age SQL fixture comparison failed")?;
    check(
        strict_age,
        "strict-age takeover did not execute at the exact SQL boundary",
    )?;
    sqlx::raw_sql(
        "DROP TRIGGER service_status_test_start_boundary ON public.research_execution_status;
         DROP FUNCTION public.service_status_test_start_boundary();
         DROP TABLE public.service_status_test_start_clock",
    )
    .execute(&db.owner)
    .await
    .map_err(|_| "exact-age fixture cleanup failed")?;
    Ok(())
}

#[tokio::test]
async fn actual_attempt_takes_over_only_a_strictly_stale_running_fence() {
    let db = ScratchDb::create()
        .await
        .expect("isolated service-status database with actual migrations is required");
    let result = check_stale_takeover(&db).await;
    db.teardown().await;
    assert_eq!(result, Ok(()), "stale takeover DB assertion failed");
}

async fn check_down_first_insert_race(db: &ScratchDb) -> Result<(), &'static str> {
    let mut writer_tx = db
        .writer
        .begin()
        .await
        .map_err(|_| "first-insert writer transaction could not start")?;
    sqlx::query(
        "INSERT INTO public.research_execution_status (
             id, attempt_started_at, heartbeat_at, attempt_state, updated_at
         ) SELECT 'kis-eod', now, now, 'RUNNING', now
             FROM (SELECT pg_catalog.clock_timestamp() AS now) AS fixture_clock",
    )
    .execute(&mut *writer_tx)
    .await
    .map_err(|_| "uncommitted first research_writer INSERT failed")?;
    let mut down_tx = db
        .owner
        .begin()
        .await
        .map_err(|_| "concurrent down transaction could not start")?;
    let down_pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&mut *down_tx)
        .await
        .map_err(|_| "concurrent down backend could not be identified")?;
    let down_job = tokio::spawn(async move {
        let outcome = sqlx::raw_sql(include_str!(
            "../../../migrations/0059_research_execution_status.down.sql"
        ))
        .execute(&mut *down_tx)
        .await;
        let guard_refused = outcome.err().is_some_and(|error| {
            error
                .as_database_error()
                .and_then(|db_error| db_error.code())
                .is_some_and(|code| code == "P0001")
        });
        down_tx
            .rollback()
            .await
            .map_err(|_| "concurrent refused down did not roll back")?;
        Ok::<_, &'static str>(guard_refused)
    });
    // Observe an actual ungranted table lock, rather than guessing from elapsed
    // time. The existing migration's 5s lock_timeout bounds any broken harness.
    let blocked = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_catalog.pg_locks
                      WHERE pid = $1 AND locktype = 'relation'
                        AND relation = 'public.research_execution_status'::regclass
                        AND mode = 'AccessExclusiveLock' AND NOT granted
                 )",
            )
            .bind(down_pid)
            .fetch_one(&db.owner)
            .await
            .map_err(|_| "concurrent down lock observation failed")?;
            if waiting {
                return Ok::<_, &'static str>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    writer_tx
        .commit()
        .await
        .map_err(|_| "first research_writer INSERT did not commit")?;
    let guard_refused = down_job
        .await
        .map_err(|_| "concurrent down task failed")??;
    check(
        matches!(blocked, Ok(Ok(()))),
        "down did not block on the uncommitted first INSERT before inspecting emptiness",
    )?;
    check(
        guard_refused,
        "down lost the first row or failed for a reason other than the nonempty guard",
    )?;
    let rows: i64 =
        sqlx::query_scalar("SELECT pg_catalog.count(*) FROM public.research_execution_status")
            .fetch_one(&db.admin)
            .await
            .map_err(|_| "committed first row did not survive refused down")?;
    check(
        rows == 1,
        "committed first row disappeared during concurrent down",
    )?;
    sqlx::query("DELETE FROM public.research_execution_status WHERE id = 'kis-eod'")
        .execute(&db.owner)
        .await
        .map_err(|_| "first-insert race fixture cleanup failed")?;
    let mut empty_tx = db
        .owner
        .begin()
        .await
        .map_err(|_| "empty down transaction failed")?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0059_research_execution_status.down.sql"
    ))
    .execute(&mut *empty_tx)
    .await
    .map_err(|_| "empty down failed after first-insert race")?;
    empty_tx
        .commit()
        .await
        .map_err(|_| "empty down commit failed after first-insert race")?;
    Ok(())
}

#[tokio::test]
async fn down_locks_before_testing_a_concurrent_first_insert() {
    let db = ScratchDb::create()
        .await
        .expect("isolated service-status database with actual migrations is required");
    let result = check_down_first_insert_race(&db).await;
    db.teardown().await;
    assert_eq!(result, Ok(()), "first-insert/down race DB assertion failed");
}
