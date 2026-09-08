//! Dedicated disposable PostgreSQL harness for the WP-3B1 contract.
//!
//! This harness intentionally does not use `tests/common`: that helper may
//! skip when `DATABASE_URL` is absent and its URL is configurable.  Intraday
//! acceptance is required to use the coordinator-owned QA cluster only.

use chrono::{Days, NaiveDate};
use job_queue::owner_equity_v2::{IntradaySessionProof, OwnerIntradayQuoteRepository};
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

pub const QA_DATABASE_URL: &str = "postgres://postgres:lagrange@127.0.0.1:55438/postgres";
pub const KIS_CALENDAR_SOURCE_VERSION: &str = "kis-chk-holiday-v1:schema-1";

static DATABASE_COUNTER: AtomicU64 = AtomicU64::new(0);
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

const ROLE_BOOTSTRAP_SQL: &str = r#"
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'migration_owner') THEN
    CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'app') THEN
    CREATE ROLE app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'worker') THEN
    CREATE ROLE worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN
    CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, admin,
    audit_writer, research_writer;
GRANT CREATE ON SCHEMA public TO migration_owner;
"#;

#[derive(Debug, Clone)]
pub struct MembershipFixture {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
}

#[derive(Clone)]
pub struct IntradayTestDb {
    pub database_name: String,
    pub superuser: PgPool,
    pub migration_owner: PgPool,
    pub app: PgPool,
    pub worker: PgPool,
    pub admin: PgPool,
    pub audit_writer: PgPool,
    pub research_writer: PgPool,
    pub session_date: NaiveDate,
    pub calendar_source_batch_id: Uuid,
    pub calendar_content_sha256: String,
    pub window_contract_sha256: String,
    root: PgPool,
}

impl IntradayTestDb {
    pub async fn create() -> Result<Self, String> {
        let configured = std::env::var("DATABASE_URL").map_err(|_| {
            format!("DATABASE_URL must equal the fixed intraday QA URL {QA_DATABASE_URL}")
        })?;
        if configured != QA_DATABASE_URL {
            return Err(format!(
                "intraday DB tests refuse non-QA DATABASE_URL; expected fixed loopback URL {QA_DATABASE_URL}"
            ));
        }

        let root = connect(QA_DATABASE_URL).await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
            .execute(&root)
            .await
            .map_err(|_| "could not bootstrap existing disposable test roles".to_owned())?;

        let n = DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let database_name = format!("lagrange_intraday_{}_{}", std::process::id(), n);
        assert!(safe_database_name(&database_name));
        sqlx::query(ddl_for(&database_name, "CREATE DATABASE {db}"))
            .execute(&root)
            .await
            .map_err(|_| "could not create generated intraday scratch database".to_owned())?;

        let database_url = database_url_for("postgres", &database_name);
        let superuser = connect(&database_url).await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
            .execute(&superuser)
            .await
            .map_err(|_| "could not prepare roles in intraday scratch database".to_owned())?;
        sqlx::raw_sql(ddl_for(
            &database_name,
            "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, admin, audit_writer, research_writer",
        ))
        .execute(&root)
        .await
        .map_err(|_| "could not grant scratch-database CONNECT to existing roles".to_owned())?;

        let migration_owner = connect(&database_url_for("migration_owner", &database_name)).await?;
        MIGRATOR
            .run(&migration_owner)
            .await
            .map_err(|_| "0054 migration set did not apply to scratch database".to_owned())?;

        let app = connect(&database_url_for("app", &database_name)).await?;
        let worker = connect(&database_url_for("worker", &database_name)).await?;
        let admin = connect(&database_url_for("admin", &database_name)).await?;
        let audit_writer = connect(&database_url_for("audit_writer", &database_name)).await?;
        let research_writer = connect(&database_url_for("research_writer", &database_name)).await?;

        let session_date: NaiveDate =
            sqlx::query_scalar("SELECT (pg_catalog.now() AT TIME ZONE 'Asia/Seoul')::date")
                .fetch_one(&superuser)
                .await
                .map_err(|_| "could not determine the QA database KST date".to_owned())?;
        let calendar_source_batch_id = Uuid::new_v4();
        let calendar_content_sha256 = "a".repeat(64);
        let window_contract_sha256 = format!("sha256:{}", "b".repeat(64));
        install_calendar(
            &superuser,
            session_date,
            calendar_source_batch_id,
            &calendar_content_sha256,
        )
        .await?;

        Ok(Self {
            database_name,
            superuser: superuser.clone(),
            migration_owner,
            app,
            worker,
            admin,
            audit_writer,
            research_writer,
            session_date,
            calendar_source_batch_id,
            calendar_content_sha256,
            window_contract_sha256,
            root,
        })
    }

    pub fn repository_as_app(&self) -> OwnerIntradayQuoteRepository {
        OwnerIntradayQuoteRepository::new(self.app.clone())
    }

    pub fn repository_as_worker(&self) -> OwnerIntradayQuoteRepository {
        OwnerIntradayQuoteRepository::new(self.worker.clone())
    }

    pub fn session_proof(&self) -> IntradaySessionProof {
        IntradaySessionProof::new(
            self.session_date,
            self.calendar_source_batch_id,
            self.calendar_content_sha256.clone(),
            self.window_contract_sha256.clone(),
        )
        .expect("fixture session proof is canonical")
    }

    pub async fn seed_ready_membership(
        &self,
        owner_user_id: Uuid,
        instrument_id: &str,
    ) -> Result<MembershipFixture, String> {
        if !instrument_id.ends_with(".KRX")
            || instrument_id.len() != 10
            || !instrument_id.as_bytes()[..6]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        {
            return Err("test instrument must be a six-digit .KRX identity".to_owned());
        }
        let membership_id = Uuid::new_v4();
        let generation_id = Uuid::new_v4();
        let code_commit = "c".repeat(40);
        let entitlement = format!("sha256:{}", "d".repeat(64));
        sqlx::query(
            "INSERT INTO public.owner_equity_memberships
                (id, owner_user_id, instrument_id, state,
                 transition_actor_user_id, transition_code_commit,
                 transition_entitlement_sha256)
             VALUES ($1, $2, $3, 'REQUESTED', $2, $4, $5)",
        )
        .bind(membership_id)
        .bind(owner_user_id)
        .bind(instrument_id)
        .bind(&code_commit)
        .bind(&entitlement)
        .execute(&self.superuser)
        .await
        .map_err(|_| "could not insert requested membership fixture".to_owned())?;
        transition_membership(&self.superuser, membership_id, "VALIDATING").await?;
        transition_membership(&self.superuser, membership_id, "BACKFILLING").await?;

        let first_session = self
            .session_date
            .checked_sub_days(Days::new(120))
            .ok_or_else(|| "fixture session date underflowed".to_owned())?;
        sqlx::query(
            "INSERT INTO public.owner_equity_instrument_generations
                (id, membership_id, owner_user_id, instrument_id, generation,
                 target_observed_sessions, minimum_observed_sessions,
                 observed_sessions, first_session, last_session)
             VALUES ($1, $2, $3, $4, 1, 261, 121, 121, $5, $6)",
        )
        .bind(generation_id)
        .bind(membership_id)
        .bind(owner_user_id)
        .bind(instrument_id)
        .bind(first_session)
        .bind(self.session_date)
        .execute(&self.superuser)
        .await
        .map_err(|_| "could not insert generation fixture".to_owned())?;
        sqlx::query(
            "INSERT INTO public.owner_equity_generation_admissions
                (generation_id, owner_user_id, membership_id, instrument_id,
                 generation, raw_manifest_sha256, artifact_manifest_sha256,
                 entitlement_sha256, capture_code_commit, materializer_code_commit)
             VALUES ($1, $2, $3, $4, 1, $5, $6, $7, $8, $8)",
        )
        .bind(generation_id)
        .bind(owner_user_id)
        .bind(membership_id)
        .bind(instrument_id)
        .bind(format!("sha256:{}", "e".repeat(64)))
        .bind(format!("sha256:{}", "f".repeat(64)))
        .bind(&entitlement)
        .bind(&code_commit)
        .execute(&self.superuser)
        .await
        .map_err(|_| "could not insert admission fixture".to_owned())?;
        transition_membership(&self.superuser, membership_id, "MATERIALIZING").await?;
        transition_membership(&self.superuser, membership_id, "READY").await?;
        Ok(MembershipFixture {
            owner_user_id,
            membership_id,
            generation_id,
            instrument_id: instrument_id.to_owned(),
            generation: 1,
        })
    }

    pub async fn seed_owner(&self, suffix: &str) -> Result<Uuid, String> {
        let user_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.users (issuer, subject, email)
             VALUES ('intraday-test', $1, $2) RETURNING id",
        )
        .bind(format!("owner-{suffix}-{}", Uuid::new_v4()))
        .bind(format!("owner-{suffix}-{}@example.test", Uuid::new_v4()))
        .fetch_one(&self.superuser)
        .await
        .map_err(|_| "could not insert owner fixture user".to_owned())?;
        sqlx::query("INSERT INTO public.roles (id, description) VALUES ('owner', 'fixture') ON CONFLICT (id) DO NOTHING")
            .execute(&self.superuser)
            .await
            .map_err(|_| "could not ensure owner role fixture".to_owned())?;
        sqlx::query("INSERT INTO public.user_roles (user_id, role_id) VALUES ($1, 'owner')")
            .bind(user_id)
            .execute(&self.superuser)
            .await
            .map_err(|_| "could not grant owner role fixture".to_owned())?;
        sqlx::query(
            "INSERT INTO public.owner_equity_universe_policies
                (owner_user_id, max_active_instruments,
                 target_observed_sessions, minimum_observed_sessions)
             VALUES ($1, 100, 261, 121)
             ON CONFLICT (owner_user_id) DO NOTHING",
        )
        .bind(user_id)
        .execute(&self.superuser)
        .await
        .map_err(|_| "could not install owner policy fixture".to_owned())?;
        Ok(user_id)
    }

    pub async fn drop_database(self) -> Result<(), String> {
        self.migration_owner.close().await;
        self.app.close().await;
        self.worker.close().await;
        self.admin.close().await;
        self.audit_writer.close().await;
        self.research_writer.close().await;
        self.superuser.close().await;
        sqlx::query(ddl_for(
            &self.database_name,
            "DROP DATABASE {db} WITH (FORCE)",
        ))
        .execute(&self.root)
        .await
        .map_err(|_| "could not drop this generated intraday scratch database".to_owned())?;
        self.root.close().await;
        Ok(())
    }
}

pub async fn run_body<F, Fut>(body: F)
where
    F: FnOnce(IntradayTestDb) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let db = IntradayTestDb::create()
        .await
        .unwrap_or_else(|error| panic!("intraday DB acceptance setup failed: {error}"));
    let result = body(db.clone()).await;
    let cleanup = db.drop_database().await;
    if let Err(error) = result {
        panic!("intraday DB acceptance body failed: {error}");
    }
    if let Err(error) = cleanup {
        panic!("intraday DB acceptance cleanup failed: {error}");
    }
}

async fn connect(url: &str) -> Result<PgPool, String> {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(url)
        .await
        .map_err(|_| "fixed loopback PostgreSQL connection failed".to_owned())?;
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
        .map_err(|_| "fixed loopback PostgreSQL did not answer SELECT 1".to_owned())?;
    Ok(pool)
}

async fn install_calendar(
    pool: &PgPool,
    session_date: NaiveDate,
    source_batch_id: Uuid,
    content_sha256: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO public.data_batches
            (id, provider, market, batch_date, kind, storage_path,
             content_sha256, bytes_size, retrieved_at, source_batch_id,
             source_file_name, fetch_mode)
         VALUES ($1, 'KIS', 'KR', $2, 'CALENDAR',
                 'fixture/intraday-calendar', $3, 1, pg_catalog.now(), $4,
                 'intraday-calendar.json', 'synthetic')",
    )
    .bind(source_batch_id)
    .bind(session_date)
    .bind(content_sha256)
    .bind(source_batch_id)
    .execute(pool)
    .await
    .map_err(|error| {
        format!(
            "could not install calendar source batch fixture ({})",
            database_error_code(&error)
        )
    })?;
    sqlx::query(
        "INSERT INTO public.trading_calendar_versions
            (exchange, session_date, session_type, timezone, source,
             source_version, source_batch_id, content_sha256, retrieved_at)
         VALUES ('KRX', $1, 'TRADING', 'Asia/Seoul', 'kis', $2, $3, $4,
                 pg_catalog.now())",
    )
    .bind(session_date)
    .bind(KIS_CALENDAR_SOURCE_VERSION)
    .bind(source_batch_id)
    .bind(content_sha256)
    .execute(pool)
    .await
    .map_err(|error| {
        format!(
            "could not install calendar version fixture ({})",
            database_error_code(&error)
        )
    })?;
    sqlx::query(
        "INSERT INTO public.trading_calendars
            (exchange, session_date, session_type, timezone, source,
             source_version, source_batch_id, content_sha256, retrieved_at)
         VALUES ('KRX', $1, 'TRADING', 'Asia/Seoul', 'kis', $2, $3, $4,
                 pg_catalog.now())",
    )
    .bind(session_date)
    .bind(KIS_CALENDAR_SOURCE_VERSION)
    .bind(source_batch_id)
    .bind(content_sha256)
    .execute(pool)
    .await
    .map_err(|error| {
        format!(
            "could not install current calendar projection fixture ({})",
            database_error_code(&error)
        )
    })?;
    Ok(())
}

async fn transition_membership(
    pool: &PgPool,
    membership_id: Uuid,
    state: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state = $2, updated_at = pg_catalog.now()
          WHERE id = $1",
    )
    .bind(membership_id)
    .bind(state)
    .execute(pool)
    .await
    .map_err(|_| "could not advance membership fixture".to_owned())?;
    Ok(())
}

fn safe_database_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn ddl_for(database_name: &str, statement: &str) -> AssertSqlSafe<String> {
    AssertSqlSafe(statement.replace("{db}", database_name))
}

fn database_url_for(role: &str, database_name: &str) -> String {
    format!("postgres://{role}:lagrange@127.0.0.1:55438/{database_name}")
}

fn database_error_code(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(database) => format!(
            "sqlstate={};constraint={}",
            database
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            database.constraint().unwrap_or("none")
        ),
        sqlx::Error::PoolTimedOut => "pool-timeout".to_owned(),
        _ => "non-database-error".to_owned(),
    }
}
