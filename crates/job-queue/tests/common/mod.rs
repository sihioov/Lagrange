//! Disposable-database setup for `backtest_runner.rs`.
//!
//! `queue_contract.rs` keeps its own, richer harness — per-role connections, a
//! `run_test` wrapper, audit helpers — that the runner tests do not need. The
//! two are deliberately NOT merged: this is the small subset, and forcing the
//! contract suite through it would mean weakening it to fit.
//!
//! The duplication that would actually be dangerous is a schema one — a suite
//! passing against a schema the product does not have. That cannot happen
//! here: both harnesses embed the same `migrations/` directory at COMPILE time
//! via `sqlx::migrate!`, and both assert the applied count equals the embedded
//! count. What is duplicated is the role-bootstrap SQL, which creates
//! cluster-wide roles idempotently and is inert if it drifts.
//!
//! Each test gets its OWN database. Sharing one would make the queue tests
//! interfere by construction — a claim test and a sweep test racing over the
//! same `jobs` rows would fail in ways that look like queue bugs.

use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, PgPool};
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Role bootstrap reused as superuser on the supervisor and fresh databases.
///
/// Role definitions are cluster-wide; the existing schema grants are legacy
/// behavior. Keep new admitter privileges in the separate target-database
/// bootstrap below, which runs only after a scratch database connection opens.
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
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer') THEN
    CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'daily_input_admitter') THEN
    CREATE ROLE daily_input_admitter LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOREPLICATION NOBYPASSRLS PASSWORD 'lagrange';
  END IF;
END $role$;
DO $role_preflight$ BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_roles
    WHERE rolname = 'daily_input_admitter'
      AND rolcanlogin
      AND NOT rolinherit
      AND NOT rolsuper
      AND NOT rolcreatedb
      AND NOT rolcreaterole
      AND NOT rolreplication
      AND NOT rolbypassrls
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter role attributes mismatch';
  END IF;

  IF EXISTS (
    SELECT 1
    FROM pg_auth_members
    WHERE member = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
       OR roleid = (SELECT oid FROM pg_roles WHERE rolname = 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'daily_input_admitter has unexpected role membership';
  END IF;

  IF EXISTS (
    WITH RECURSIVE membership_closure(start_role, reachable_role) AS (
      SELECT oid, oid
      FROM pg_roles
      WHERE rolname IN ('app', 'worker')
      UNION
      SELECT closure.start_role, membership.roleid
      FROM membership_closure AS closure
      JOIN pg_auth_members AS membership ON membership.member = closure.reachable_role
    )
    SELECT 1
    FROM membership_closure AS closure
    JOIN pg_roles AS start_role ON start_role.oid = closure.start_role
    JOIN pg_roles AS reachable_role ON reachable_role.oid = closure.reachable_role
    WHERE start_role.rolname IN ('app', 'worker')
      AND reachable_role.rolname IN ('migration_owner', 'daily_input_admitter')
  ) THEN
    RAISE EXCEPTION 'app or worker reaches a protected migration role';
  END IF;
END $role_preflight$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, research_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
"#;

/// Per-scratch-database privileges for the isolated daily-input test role.
const DAILY_INPUT_ADMITTER_DB_BOOTSTRAP_SQL: &str = r#"
DO $database_privilege$ BEGIN
  EXECUTE format('GRANT CONNECT ON DATABASE %I TO daily_input_admitter', current_database());
END $database_privilege$;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO daily_input_admitter;
DO $privilege_preflight$ BEGIN
  IF NOT has_database_privilege('daily_input_admitter', current_database(), 'CONNECT') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks CONNECT on the scratch database';
  END IF;
  IF NOT has_schema_privilege('daily_input_admitter', 'public', 'USAGE') THEN
    RAISE EXCEPTION 'daily_input_admitter lacks USAGE on scratch public schema';
  END IF;
  IF has_database_privilege('daily_input_admitter', current_database(), 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has CREATE on the scratch database';
  END IF;
  IF has_schema_privilege('daily_input_admitter', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'daily_input_admitter has CREATE on scratch public schema';
  END IF;
  IF NOT has_schema_privilege('migration_owner', 'public', 'CREATE') THEN
    RAISE EXCEPTION 'migration_owner lacks CREATE on scratch public schema';
  END IF;
END $privilege_preflight$;
"#;

/// SQLx 0.9's SQL audit requires dynamic DDL (a database name cannot be a bind
/// parameter) to be wrapped. The only interpolated value is a generated name
/// asserted safe below.
fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

fn fresh_db_name() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("lagrange_jq_{}_{}", std::process::id(), n)
}

fn conn_url(base: &str, user: &str, db: &str) -> String {
    // The base URL is the superuser one; only the database part changes.
    let head = base.rsplit_once('/').map(|(h, _)| h).unwrap_or(base);
    let _ = user;
    format!("{head}/{db}")
}

async fn connect_with_retry(url: &str, attempts: u32) -> Result<PgPool, Box<dyn Error>> {
    let mut last: Option<sqlx::Error> = None;
    for _ in 0..attempts {
        match PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await
        {
            Ok(p) => return Ok(p),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(Box::new(last.expect("at least one attempt")))
}

async fn connect_as_migration_owner(
    super_url: &str,
    db: &str,
    attempts: u32,
) -> Result<PgPool, Box<dyn Error>> {
    let options: sqlx::postgres::PgConnectOptions = conn_url(super_url, "postgres", db).parse()?;
    let mut last: Option<sqlx::Error> = None;
    for _ in 0..attempts {
        match PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    sqlx::raw_sql("SET ROLE migration_owner")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options.clone())
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(error) => {
                last = Some(error);
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(Box::new(last.expect("at least one attempt")))
}

fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|m| {
            m.migration_type == MigrationType::Simple
                || m.migration_type == MigrationType::ReversibleUp
        })
        .count()
}

/// A disposable database with every migration applied.
///
/// Dropped by [`ScratchDb::drop_db`]; not on `Drop`, because an async drop is
/// not available and a blocking one inside a test runtime deadlocks.
pub struct ScratchDb {
    pub pool: PgPool,
    name: String,
    super_url: String,
}

impl ScratchDb {
    /// `None` when `DATABASE_URL` is unset, so DB-gated tests skip cleanly
    /// rather than failing on a machine without a database.
    pub async fn create() -> Option<ScratchDb> {
        let super_url = std::env::var("DATABASE_URL").ok()?;
        match Self::build(&super_url).await {
            Ok(db) => Some(db),
            Err(e) => panic!("scratch database could not be created: {e}"),
        }
    }

    async fn build(super_url: &str) -> Result<ScratchDb, Box<dyn Error>> {
        let db = fresh_db_name();
        assert!(
            db.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "generated database name must be a safe identifier"
        );

        let admin = connect_with_retry(super_url, 3).await?;
        // Roles are cluster-global. All disposable test databases coordinate
        // their idempotent bootstrap through the same supervisor database,
        // matching the migration-contract harness and avoiding CREATE ROLE
        // races between parallel test bodies.
        let mut role_bootstrap = admin.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('lagrange-test-role-bootstrap'))")
            .execute(&mut *role_bootstrap)
            .await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
            .execute(&mut *role_bootstrap)
            .await?;
        role_bootstrap.commit().await?;
        admin
            .execute(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
            .await?;
        admin.execute(ddl_for(&db, "CREATE DATABASE {db}")).await?;
        drop(admin);

        let fresh = connect_with_retry(&conn_url(super_url, "postgres", &db), 6).await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL).execute(&fresh).await?;
        sqlx::raw_sql(ddl_for(
            &db,
            "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer, research_writer, admin",
        ))
        .execute(&fresh)
        .await?;
        sqlx::raw_sql(DAILY_INPUT_ADMITTER_DB_BOOTSTRAP_SQL)
            .execute(&fresh)
            .await?;

        // Production migrations run as migration_owner. Running them as the
        // supervisor makes SECURITY DEFINER functions owned by
        // migration_owner unable to read the supervisor-owned tables, a false
        // failure that also hides ownership bugs from worker-role tests.
        let migration_owner = connect_as_migration_owner(super_url, &db, 6).await?;
        let identities: (String, String) = sqlx::query_as("SELECT current_user, session_user")
            .fetch_one(&migration_owner)
            .await?;
        assert_eq!(identities.0, "migration_owner");
        assert_ne!(identities.0, identities.1);
        MIGRATOR.run(&migration_owner).await?;
        let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&migration_owner)
            .await?;
        assert_eq!(
            applied as usize,
            up_migration_count(),
            "every migration must apply, or the test runs against a schema the product does not have"
        );
        let incorrect_owners: Vec<String> = sqlx::query_scalar(
            "SELECT c.relname::text \
             FROM pg_class AS c \
             WHERE c.relname IN ( \
                 'user_strategy_configs', 'recommendation_runs', 'jobs', 'dataset_versions') \
               AND pg_get_userbyid(c.relowner) <> 'migration_owner' \
             ORDER BY c.relname",
        )
        .fetch_all(&migration_owner)
        .await?;
        assert!(
            incorrect_owners.is_empty(),
            "attestation tables must be migration_owner-owned: {incorrect_owners:?}"
        );
        let scheduler_owner: String = sqlx::query_scalar(
            "SELECT pg_get_userbyid(p.proowner) \
             FROM pg_proc AS p \
             WHERE p.oid = 'public.schedule_recommendation_run( \
                 uuid, uuid, date, uuid, text, integer, text)'::regprocedure",
        )
        .fetch_one(&migration_owner)
        .await?;
        assert_eq!(scheduler_owner, "migration_owner");
        migration_owner.close().await;

        Ok(ScratchDb {
            pool: fresh,
            name: db,
            super_url: super_url.to_string(),
        })
    }

    /// A connection URL for this database as one of the production roles.
    ///
    /// Tests otherwise connect as superuser, where every GRANT is irrelevant
    /// and a role that is missing a privilege looks perfectly healthy. What
    /// that hides is specific: the failure appears only in production, on a
    /// path a green suite just declared working.
    pub fn role_url(&self, role: &str) -> String {
        let (scheme, rest) = self
            .super_url
            .split_once("://")
            .expect("DATABASE_URL has a scheme");
        // Everything before the last '@' is the credentials; a password may
        // itself contain '@', so the host is taken from the RIGHT.
        let host = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
        let host = host.rsplit_once('/').map(|(h, _)| h).unwrap_or(host);
        format!("{scheme}://{role}:lagrange@{host}/{}", self.name)
    }

    /// A direct-login pool for the isolated daily-input admission role.
    ///
    /// The caller owns the returned pool and must close it before dropping
    /// this scratch database. It carries no SET ROLE or actor GUC setup.
    pub async fn daily_input_admitter_pool(&self) -> PgPool {
        let options: PgConnectOptions = self
            .role_url("daily_input_admitter")
            .parse()
            .unwrap_or_else(|_| panic!("daily input admitter connection options parse"));
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .unwrap_or_else(|_| panic!("daily input admitter test pool connects"));
        let identities: (String, String, Option<String>) = sqlx::query_as(
            "SELECT current_user, session_user, current_setting('app.actor_user_id', true)",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|_| panic!("daily input admitter identity check runs"));
        assert_eq!(identities.0, "daily_input_admitter");
        assert_eq!(identities.1, "daily_input_admitter");
        assert!(
            identities.2.is_none(),
            "admitter pool must not carry actor GUC"
        );
        pool
    }

    pub async fn drop_db(self) {
        let ScratchDb {
            pool,
            name,
            super_url,
        } = self;
        pool.close().await;
        if let Ok(admin) = connect_with_retry(&super_url, 2).await {
            let _ = admin
                .execute(ddl_for(&name, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
                .await;
        }
    }
}
