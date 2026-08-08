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
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Role bootstrap executed as superuser on each fresh database.
///
/// Roles are cluster-wide and created idempotently; per-database grants land
/// via migration 0009.
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
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'admin') THEN
    CREATE ROLE admin LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer, admin;
GRANT CREATE ON SCHEMA public TO migration_owner;
"#;

/// SQLx 0.9's SQL audit requires dynamic DDL (a database name cannot be a bind
/// parameter) to be wrapped. The only interpolated value is a generated name
/// asserted safe below.
fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

fn fresh_db_name() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "lagrange_jq_{}_{}",
        std::process::id(),
        n
    )
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

fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.migration_type == MigrationType::Simple || m.migration_type == MigrationType::ReversibleUp)
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
        admin.execute(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)")).await?;
        admin.execute(ddl_for(&db, "CREATE DATABASE {db}")).await?;
        drop(admin);

        let fresh = connect_with_retry(&conn_url(super_url, "postgres", &db), 6).await?;
        sqlx::raw_sql(ROLE_BOOTSTRAP_SQL).execute(&fresh).await?;
        sqlx::raw_sql(ddl_for(
            &db,
            "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer",
        ))
        .execute(&fresh)
        .await?;

        MIGRATOR.run(&fresh).await?;
        let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&fresh)
            .await?;
        assert_eq!(
            applied as usize,
            up_migration_count(),
            "every migration must apply, or the test runs against a schema the product does not have"
        );

        Ok(ScratchDb {
            pool: fresh,
            name: db,
            super_url: super_url.to_string(),
        })
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
