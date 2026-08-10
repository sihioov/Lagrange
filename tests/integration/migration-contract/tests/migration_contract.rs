//! Todo 3 migration contract gate: schemas, roles, and immutable state
//! boundaries, proven against a disposable PostgreSQL 18 cluster.
//!
//! PLAN-COMMAND DEFECT (documented deviation): the plan's QA line for Todo 3 is
//! `cargo test -p api-server --test migration_contract`, but `apps/api-server`
//! is the Node/TypeScript application, NOT a Rust crate. This crate
//! (`migration-contract`, root workspace member) is the documented replacement:
//! it embeds `migrations/` via `sqlx::migrate!("../../../migrations")` (path is
//! relative to CARGO_MANIFEST_DIR = `tests/integration/migration-contract`) and
//! drives run/re-run/revert/denial assertions against a disposable database.
//!
//! Every test requires `DATABASE_URL` to point at a SUPERVISOR connection to a
//! DISPOSABLE PostgreSQL 18 cluster, e.g.
//! `postgres://postgres:lagrange@127.0.0.1:5432/postgres`. The tests run by
//! DEFAULT (un-gated since Todo 3's live gate passed): with `DATABASE_URL` set
//! they create/drop their own scratch databases and run the full contract; on
//! hosts with no database at all, `require_db_url()` below skips them cleanly
//! (reported as `ok`, zero assertions) instead of failing with a connection
//! error, so `cargo test --workspace` stays green everywhere.
//!
//! Covered contract (acceptance for plan Todo 3):
//!   - `sqlx migrate run` applies all migrations; a second run is a no-op.
//!   - `revert` (undo all) then `run` succeeds in a disposable DB.
//!   - every tenant table carries an ownership column (`owner_user_id`,
//!     `account_id`, `user_id`, or `created_by_user_id`).
//!   - every table is owned by `migration_owner`; `app`/`worker`/`audit_writer`
//!     have no table ownership and no BYPASSRLS.
//!   - public `jobs.status` has EXACTLY five values
//!     (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELED); a sixth (ORPHANED) is
//!     rejected by CHECK.
//!   - `job_attempts.outcome` includes `ORPHANED` (attempt-level only;
//!     CANCELED is rejected there).
//!   - app role denial: ALTER TABLE, TRUNCATE audit_logs, UPDATE/DELETE/INSERT
//!     audit_logs, INSERT into system-owned shared tables (cross-owner insert),
//!     sixth job status, CREATE TABLE.
//!   - audit_logs append-only: audit_writer may INSERT but never
//!     UPDATE/DELETE/TRUNCATE.
//!   - sha256-hash columns enforce `^[0-9a-f]{64}$`; web_sessions.session_hash
//!     is unique; data_entitlements lifecycle is CHECK-enforced.
//!   - large curves/orders/fills live in Parquet with DB manifests
//!     (result_artifacts: parquet_path + row_count + sha256 + summary_json).

use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::PgPool;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Uuid;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Migrations embedded at compile time from the workspace `migrations/` dir.
static MIGRATOR: Migrator = sqlx::migrate!("../../../migrations");

/// Role/schema bootstrap executed as superuser on each fresh database.
const BOOTSTRAP_SQL: &str = include_str!("../bootstrap.sql");

/// Tenant tables that MUST carry an ownership column (design §7.3).
const TENANT_TABLES: &[&str] = &[
    "user_strategy_configs",
    "recommendation_runs",
    "recommendation_items",
    "target_portfolios",
    "jobs",
    "backtest_runs",
    "backtest_metrics",
    "backtest_warnings",
    "result_artifacts",
    "accounts",
    "cash_ledger",
    "positions",
    "orders",
    "fills",
    "daily_equity",
    "broker_connections",
    "reconciliation_runs",
    "risk_events",
    "notifications",
    "web_sessions",
    "invitations",
];

const PUBLIC_JOB_STATUSES: [&str; 5] = ["QUEUED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"];

fn pg_code(err: &sqlx::Error) -> Option<String> {
    match err {
        // sqlx 0.9's `DatabaseError::code()` returns `Option<Cow<'_, str>>`;
        // materialize an owned String so the error code outlives `err`.
        sqlx::Error::Database(e) => e.code().map(|c| c.into_owned()),
        _ => None,
    }
}

/// Audit point for dynamic DDL. `CREATE/DROP DATABASE` and
/// `GRANT CONNECT ON DATABASE` take database *identifiers*, which PostgreSQL
/// cannot express as bind parameters, so these statements must be assembled
/// dynamically. Injection is impossible: the only interpolated value is the
/// database name produced by `fresh_db_name()` (`contract_{pid}_{ts}`) and
/// asserted to contain only `[a-z0-9_]` in `create_contract_db` before any
/// statement is built. `AssertSqlSafe` records that audit for sqlx 0.9's
/// compile-time SQL audit.
fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

/// Number of rows in sqlx's bookkeeping table — the count of applied
/// migrations. sqlx 0.9's `Migrator::run`/`undo` return `()` (0.8 returned a
/// count), so the contract asserts on this table instead.
async fn applied_count(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
}

/// Rewrite a superuser URL (`postgres://user[:pw]@host:port/db`) for a role
/// and database. Password is preserved if present (disposable cluster uses
/// trust auth, so it is normally empty).
fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (_scheme, rest) = super_url
        .split_once("://")
        .expect("DATABASE_URL must start with a scheme");
    let (auth, hostport_db) = rest.split_once('@').expect("DATABASE_URL must contain @");
    let (user, pw) = match auth.split_once(':') {
        Some((u, p)) => (u, Some(p)),
        None => (auth, None),
    };
    let (hostport, _old_db) = hostport_db
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    let _ = user; // role replaces the original user
    match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    }
}

/// A pool whose connections carry an explicit RLS actor context
/// (`app.actor_user_id` startup option). Since migration 0010 forces row-level
/// security on every tenant table, tenant reads (which are STRICT: the policy
/// requires `owner = current_setting('app.actor_user_id')`) only return rows
/// when the connection carries the actor GUC. The migration owner may touch
/// tenant rows while impersonating a user; without the GUC, FORCE RLS denies
/// it (hazard proven by the Todo 23 tenancy suite).
async fn actor_pool(
    super_url: &str,
    db: &str,
    role: &str,
    user_id: &str,
) -> Result<PgPool, Box<dyn Error>> {
    let opts: sqlx::postgres::PgConnectOptions = conn_url(super_url, role, db)
        .parse()
        .map_err(Box::<dyn Error>::from)?;
    PgPoolOptions::new()
        .max_connections(3)
        .connect_with(opts.options([("app.actor_user_id", user_id.to_string())]))
        .await
        .map_err(Box::<dyn Error>::from)
}

/// Scratch-database name, unique per call. Both tests share one process and
/// run in PARALLEL test threads, so pid+millis alone can collide (two
/// `CREATE DATABASE` of the same name -> duplicate key on
/// `pg_database_datname_index`); the monotonic counter guarantees uniqueness.
fn fresh_db_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis();
    format!(
        "contract_{}_{}_{}",
        std::process::id(),
        ts,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Number of migrations `Migrator::run` applies. sqlx 0.9 records each
/// `.up.sql` and `.down.sql` file as its OWN `Migration` entry
/// (`ReversibleUp`/`ReversibleDown`), so `migrations.len()` is 2x the real
/// count for the 9 up/down pairs in `migrations/`. `run` applies every
/// non-`ReversibleDown` entry; `undo` consumes the down side.
fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.migration_type != MigrationType::ReversibleDown)
        .count()
}

async fn admin_pool(url: &str) -> Result<PgPool, Box<dyn Error>> {
    Ok(PgPoolOptions::new().max_connections(3).connect(url).await?)
}

/// Create a brand-new database on the disposable cluster, bootstrap roles and
/// schema grants, and return `(db_name, migration_owner_pool)`.
async fn create_contract_db(super_url: &str) -> Result<(String, PgPool), Box<dyn Error>> {
    let db = fresh_db_name();
    assert!(
        db.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "generated database name must be a safe identifier"
    );
    let admin = admin_pool(super_url).await?;
    sqlx::query(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    sqlx::query(ddl_for(&db, "CREATE DATABASE {db}"))
        .execute(&admin)
        .await?;
    drop(admin);

    let super_new = admin_pool(&conn_url(super_url, "postgres", &db)).await?;
    sqlx::raw_sql(BOOTSTRAP_SQL).execute(&super_new).await?;
    sqlx::raw_sql(ddl_for(
        &db,
        "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer, research_writer, admin",
    ))
    .execute(&super_new)
    .await?;
    drop(super_new);

    let owner = PgPoolOptions::new()
        .max_connections(3)
        .connect(&conn_url(super_url, "migration_owner", &db))
        .await?;
    Ok((db, owner))
}

/// Drop the disposable database, terminating any remaining connections.
async fn drop_contract_db(super_url: &str, db: &str) -> Result<(), Box<dyn Error>> {
    let admin = admin_pool(super_url).await?;
    sqlx::query(ddl_for(db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    Ok(())
}

async fn role_pool(super_url: &str, db: &str, role: &str) -> Result<PgPool, Box<dyn Error>> {
    let opts: sqlx::postgres::PgConnectOptions = conn_url(super_url, role, db)
        .parse()
        .map_err(Box::<dyn Error>::from)?;
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await?)
}

fn require_db_url() -> Result<String, Box<dyn Error>> {
    match env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()) {
        Some(url) => Ok(url),
        None => {
            eprintln!("SKIP: DATABASE_URL not set - no disposable PostgreSQL cluster available");
            Err("DATABASE_URL not set".into())
        }
    }
}

/// Full contract: migrate run -> no-op re-run -> five-state CHECK -> ORPHANED
/// attempt -> ownership columns -> role invariants -> app-role denials ->
/// audit append-only -> worker capability -> hash/unique/check constraints.
///
/// Un-gated since Todo 3's live gate passed: runs with `cargo test -p
/// migration-contract` against `DATABASE_URL` (disposable scratch DB).
#[tokio::test]
async fn migration_contract_full() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };

    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = full_contract_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await; // best-effort cleanup
    if let Err(e) = result {
        panic!("migration contract FAILED: {e}");
    }
}

async fn full_contract_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    // ------------------------------------------------------------------
    // 1. `sqlx migrate run` applies every migration; a second run is a no-op.
    // ------------------------------------------------------------------
    let expected = up_migration_count();
    assert!(expected > 0, "migrator must embed at least one migration");
    MIGRATOR.run(owner).await?;
    let applied = applied_count(owner).await? as usize;
    assert_eq!(
        applied, expected,
        "first run must apply all {expected} migrations"
    );
    MIGRATOR.run(owner).await?;
    let applied_again = applied_count(owner).await? as usize;
    assert_eq!(applied_again, applied, "second run must be a no-op");

    // Tables exist.
    let jobs_class: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.jobs')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        jobs_class.as_deref(),
        Some("jobs"),
        "public.jobs must exist"
    );
    let attempts_class: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.job_attempts')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        attempts_class.as_deref(),
        Some("job_attempts"),
        "public.job_attempts must exist"
    );

    // ------------------------------------------------------------------
    // 2. Ownership columns on every tenant table (design §7.3).
    // ------------------------------------------------------------------
    for t in TENANT_TABLES {
        let cols: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = $1",
        )
        .bind(t)
        .fetch_all(owner)
        .await?;
        let has_ownership = cols.iter().any(|c| {
            matches!(
                c.as_str(),
                "owner_user_id" | "account_id" | "user_id" | "created_by_user_id"
            )
        });
        assert!(
            has_ownership,
            "tenant table `{t}` must carry an ownership column, got columns {cols:?}"
        );
    }

    // ------------------------------------------------------------------
    // 3. Role invariants: every table owned by migration_owner; serving
    //    roles have no BYPASSRLS.
    // ------------------------------------------------------------------
    let owners: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT tableowner FROM pg_tables WHERE schemaname = 'public'",
    )
    .fetch_all(owner)
    .await?
    .into_iter()
    .collect();
    assert!(!owners.is_empty(), "at least one table must exist");
    assert!(
        owners.iter().all(|o| o == "migration_owner"),
        "all tables must be owned by migration_owner, got {owners:?}"
    );
    for role in ["app", "worker", "audit_writer", "research_writer", "admin"] {
        let bypass: bool =
            sqlx::query_scalar::<_, bool>("SELECT rolbypassrls FROM pg_roles WHERE rolname = $1")
                .bind(role)
                .fetch_one(owner)
                .await?;
        assert!(!bypass, "role {role} must not have BYPASSRLS");
    }

    // ------------------------------------------------------------------
    // 4. jobs.status: EXACTLY five public values; ORPHANED never a sixth.
    // ------------------------------------------------------------------
    let uid: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (issuer, subject, email, display_name) \
         VALUES ('https://issuer.test','owner-subject','owner@example.test','Owner') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    // Tenant DML runs under an explicit actor context (RLS policies are
    // strict for writes too since migration 0010).
    let owner_actor = actor_pool(super_url, db, "migration_owner", &uid.to_string()).await?;
    for (i, status) in PUBLIC_JOB_STATUSES.iter().enumerate() {
        sqlx::query(
            "INSERT INTO jobs (owner_user_id, job_type, status, priority, payload_json, \
             max_attempts, idempotency_key) VALUES ($1, 'backtest', $2, 10, '{}'::jsonb, 3, $3)",
        )
        .bind(uid)
        .bind(status)
        .bind(format!("idem-{i}"))
        .execute(&owner_actor)
        .await?;
    }
    let sixth = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, status) VALUES ($1, 'backtest', 'ORPHANED')",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&sixth).as_deref(),
        Some("23514"),
        "a sixth public jobs.status (ORPHANED) must be rejected by CHECK"
    );
    let checkdef: String = sqlx::query_scalar::<_, String>(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conrelid = 'public.jobs'::regclass AND conname = 'jobs_status_check'",
    )
    .fetch_one(owner)
    .await?;
    for s in PUBLIC_JOB_STATUSES {
        assert!(
            checkdef.contains(s),
            "jobs_status_check must list {s}, got {checkdef}"
        );
    }
    assert!(
        !checkdef.contains("ORPHANED"),
        "ORPHANED must never appear in the public jobs.status CHECK, got {checkdef}"
    );

    // ------------------------------------------------------------------
    // 5. job_attempts.outcome includes ORPHANED (attempt-level only).
    // ------------------------------------------------------------------
    let job_id: Uuid =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM jobs ORDER BY created_at LIMIT 1")
            .fetch_one(&owner_actor)
            .await?;
    sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome, claimed_by) \
         VALUES ($1, 1, 'ORPHANED', 'worker-probe')",
    )
    .bind(job_id)
    .execute(owner)
    .await?;
    let canceled_attempt = sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome) VALUES ($1, 2, 'CANCELED')",
    )
    .bind(job_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&canceled_attempt).as_deref(),
        Some("23514"),
        "CANCELED is not an attempt-level outcome; ORPHANED covers worker death"
    );
    let dup_attempt = sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome) VALUES ($1, 1, 'RUNNING')",
    )
    .bind(job_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&dup_attempt).as_deref(),
        Some("23505"),
        "attempt_no must be unique per job"
    );

    // ------------------------------------------------------------------
    // 6. App role: functional on tenant data, denied everything else.
    // ------------------------------------------------------------------
    let app = role_pool(super_url, db, "app").await?;
    // Tenant writes that RETURN rows run under an explicit actor context
    // (Todo 23 RLS policies are strict when a GUC is present); the app pool
    // acts "as the owner" for the positive tenant assertions.
    let app_actor = actor_pool(super_url, db, "app", &uid.to_string()).await?;
    // Positive: app serves its owner's tenant data.
    let acc: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'PAPER', 'qa-paper', 'KRW') RETURNING id",
    )
    .bind(uid)
    .fetch_one(&app_actor)
    .await?;
    assert!(
        acc.as_bytes().len() == 16,
        "app must be able to insert tenant rows"
    );

    // Denial: ALTER TABLE (no ownership, no schema CREATE).
    let ddl = sqlx::query("ALTER TABLE jobs ADD COLUMN hacked_by_app text")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&ddl).as_deref(),
        Some("42501"),
        "app role must not ALTER TABLE"
    );
    let create_table = sqlx::query("CREATE TABLE app_hack (id integer)")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&create_table).as_deref(),
        Some("42501"),
        "app role must not CREATE TABLE"
    );

    // Denial: TRUNCATE audit_logs.
    let truncate = sqlx::query("TRUNCATE TABLE audit_logs")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&truncate).as_deref(),
        Some("42501"),
        "app role must not TRUNCATE audit_logs"
    );

    // Denial: audit rows are append-only for app (SELECT only).
    let upd = sqlx::query("UPDATE audit_logs SET reason = 'tampered'")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&upd).as_deref(),
        Some("42501"),
        "app role must not UPDATE audit_logs"
    );
    let del = sqlx::query("DELETE FROM audit_logs")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&del).as_deref(),
        Some("42501"),
        "app role must not DELETE audit_logs"
    );
    let app_audit_insert = sqlx::query("INSERT INTO audit_logs (action) VALUES ('probe')")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&app_audit_insert).as_deref(),
        Some("42501"),
        "audit_logs is write-restricted to audit_writer"
    );

    // Denial: cross-owner insert into system-owned shared metadata.
    let cross_owner = sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('069500.KRX', '069500', 'KRX', 'KRW')",
    )
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&cross_owner).as_deref(),
        Some("42501"),
        "app role must not INSERT into system-owned shared tables (cross-owner insert)"
    );

    // Denial: immutable shared datasets cannot be mutated either.
    let shared_update = sqlx::query("UPDATE dataset_versions SET status = 'READY'")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&shared_update).as_deref(),
        Some("42501"),
        "shared dataset rows are read-only"
    );

    // Denial: sixth status via app role too.
    let app_sixth = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, status) VALUES ($1, 'backtest', 'ORPHANED')",
    )
    .bind(uid)
    .execute(&app_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&app_sixth).as_deref(),
        Some("23514"),
        "sixth status denied for app too"
    );

    // ------------------------------------------------------------------
    // 7. audit_writer: append-only writer of audit_logs, nothing else.
    // ------------------------------------------------------------------
    let aw = role_pool(super_url, db, "audit_writer").await?;
    sqlx::query("INSERT INTO audit_logs (action, actor_role) VALUES ('qa.probe', 'audit_writer')")
        .execute(&aw)
        .await?;
    let aw_upd = sqlx::query("UPDATE audit_logs SET reason = 'tampered'")
        .execute(&aw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&aw_upd).as_deref(),
        Some("42501"),
        "audit_writer must not UPDATE audit rows"
    );
    let aw_del = sqlx::query("DELETE FROM audit_logs")
        .execute(&aw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&aw_del).as_deref(),
        Some("42501"),
        "audit_writer must not DELETE audit rows"
    );
    let aw_tr = sqlx::query("TRUNCATE TABLE audit_logs")
        .execute(&aw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&aw_tr).as_deref(),
        Some("42501"),
        "audit_writer must not TRUNCATE audit_logs"
    );
    let aw_tenant = sqlx::query(
        "INSERT INTO accounts (owner_user_id, account_type, name) \
                                 VALUES ($1, 'PAPER', 'x')",
    )
    .bind(uid)
    .execute(&aw)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&aw_tenant).as_deref(),
        Some("42501"),
        "audit_writer must not write tenant data (cross-owner insert denied)"
    );

    // ------------------------------------------------------------------
    // 8. Worker role: can claim/advance jobs and attempts, nothing else.
    // ------------------------------------------------------------------
    let wk = role_pool(super_url, db, "worker").await?;
    sqlx::query(
        "UPDATE jobs SET status = 'RUNNING', locked_by = 'worker-probe', locked_at = now() \
         WHERE id = $1 AND status = 'QUEUED'",
    )
    .bind(job_id)
    .execute(&wk)
    .await?;
    sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome, claimed_by) \
         VALUES ($1, 2, 'RUNNING', 'worker-probe')",
    )
    .bind(job_id)
    .execute(&wk)
    .await?;
    let wk_audit = sqlx::query("INSERT INTO audit_logs (action) VALUES ('worker-probe')")
        .execute(&wk)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&wk_audit).as_deref(),
        Some("42501"),
        "worker must not write audit_logs"
    );
    let wk_ddl = sqlx::query("ALTER TABLE jobs DROP COLUMN IF EXISTS priority")
        .execute(&wk)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&wk_ddl).as_deref(),
        Some("42501"),
        "worker must not DDL"
    );
    let wk_tenant = sqlx::query(
        "INSERT INTO accounts (owner_user_id, account_type, name) VALUES ($1, 'PAPER', 'y')",
    )
    .bind(uid)
    .execute(&wk)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&wk_tenant).as_deref(),
        Some("42501"),
        "worker must not write tenant data"
    );

    // ------------------------------------------------------------------
    // 9. Research publication: stable Raw lineage, immutable calendar
    //    history, and a narrowly-scoped publication writer.
    // ------------------------------------------------------------------
    let provenance_columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'data_batches' \
         AND column_name IN ('source_batch_id', 'source_file_name', 'fetch_mode') \
         ORDER BY column_name",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        provenance_columns,
        vec![
            ("fetch_mode".into(), "text".into(), "YES".into()),
            ("source_batch_id".into(), "uuid".into(), "YES".into()),
            ("source_file_name".into(), "text".into(), "YES".into()),
        ],
        "data_batches must expose nullable Raw provenance columns"
    );

    let legacy_batch: Uuid = sqlx::query_scalar(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2026-02-01', 'REFERENCE', 'data/raw/legacy/1', $1, 1, now()) \
         RETURNING id",
    )
    .bind("d".repeat(64))
    .fetch_one(owner)
    .await?;
    assert!(
        legacy_batch.as_bytes().len() == 16,
        "legacy rows need no provenance"
    );

    let source_batch_id = Uuid::parse_str("00000000-0000-0000-0000-000000000022").unwrap();
    let publication_batch_sql = "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
         VALUES ('KRX', 'KR', '2026-02-02', 'REFERENCE', $1, $2, 1, now(), $3, 'master.csv', $4)";
    sqlx::query(publication_batch_sql)
        .bind("data/raw/published/1")
        .bind("e".repeat(64))
        .bind(source_batch_id)
        .bind("credentialed")
        .execute(owner)
        .await?;
    let duplicate_provenance = sqlx::query(publication_batch_sql)
        .bind("data/raw/published/2")
        .bind("f".repeat(64))
        .bind(source_batch_id)
        .bind("credentialed")
        .execute(owner)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&duplicate_provenance).as_deref(),
        Some("23505"),
        "published Raw lineage must be unique per provider, market, batch, and file"
    );
    let incomplete_provenance = sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at, source_batch_id) \
         VALUES ('KRX', 'KR', '2026-02-02', 'REFERENCE', 'data/raw/published/incomplete', $1, 1, now(), $2)",
    )
    .bind("0".repeat(64))
    .bind(Uuid::parse_str("00000000-0000-0000-0000-000000000023").unwrap())
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&incomplete_provenance).as_deref(), Some("23514"));
    let invalid_fetch_mode = sqlx::query(publication_batch_sql)
        .bind("data/raw/published/invalid-mode")
        .bind("1".repeat(64))
        .bind(Uuid::parse_str("00000000-0000-0000-0000-000000000024").unwrap())
        .bind("CREDENTIALed")
        .execute(owner)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&invalid_fetch_mode).as_deref(), Some("23514"));

    let calendar_hash = "2".repeat(64);
    let calendar_version_id: i64 = sqlx::query_scalar(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-03', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1, $2, now()) \
         RETURNING id",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_version_id > 0,
        "calendar history must use an identity key"
    );
    let duplicate_calendar_version = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-03', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1, $2, now())",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&duplicate_calendar_version).as_deref(),
        Some("23505")
    );
    let invalid_calendar_hash = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-04', 'OPEN', 'UTC', 'KRX', 'v1', $1, 'bad', now())",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&invalid_calendar_hash).as_deref(), Some("23514"));
    for reader in ["app", "worker", "admin"] {
        let reader_pool = role_pool(super_url, db, reader).await?;
        let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
            .fetch_one(&reader_pool)
            .await?;
        assert!(
            visible >= 1,
            "{reader} must retain shared calendar history reads"
        );
    }
    for statement in [
        "UPDATE trading_calendar_versions SET source = 'tampered'",
        "DELETE FROM trading_calendar_versions",
    ] {
        let append_only = sqlx::query(statement).execute(owner).await.unwrap_err();
        assert_eq!(
            pg_code(&append_only).as_deref(),
            Some("55000"),
            "{statement}"
        );
    }

    let legacy_calendar: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_calendars (exchange, session_date, session_type, timezone, source, source_version) \
         VALUES ('KRX', '2026-02-05', 'CLOSED', 'Asia/Seoul', 'KRX', 'legacy') RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        legacy_calendar.as_bytes().len() == 16,
        "legacy calendar rows need no provenance"
    );
    let incomplete_projection = sqlx::query(
        "INSERT INTO trading_calendars (exchange, session_date, session_type, timezone, source, source_version, source_batch_id) \
         VALUES ('KRX', '2026-02-06', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1)",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&incomplete_projection).as_deref(), Some("23514"));
    let invalid_projection_hash = sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-06', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1, 'BAD', now())",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&invalid_projection_hash).as_deref(), Some("23514"));

    let rw = role_pool(super_url, db, "research_writer").await?;
    let writable_source_batch = Uuid::parse_str("00000000-0000-0000-0000-000000000025").unwrap();
    sqlx::query(publication_batch_sql)
        .bind("data/raw/research-writer/1")
        .bind("3".repeat(64))
        .bind(writable_source_batch)
        .bind("synthetic")
        .execute(&rw)
        .await?;
    let raw_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM data_batches")
        .fetch_one(&rw)
        .await?;
    assert!(
        raw_visible >= 1,
        "research_writer must read published Raw batches"
    );
    sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-07', 'TRADING', 'Asia/Seoul', 'KRX', 'writer-v1', $1, $2, now())",
    )
    .bind(writable_source_batch)
    .bind("4".repeat(64))
    .execute(&rw)
    .await?;
    let history_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
        .fetch_one(&rw)
        .await?;
    assert!(
        history_visible >= 1,
        "research_writer must read published calendar history"
    );
    let projection_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-08', 'TRADING', 'Asia/Seoul', 'KRX', 'writer-v1', $1, $2, now()) RETURNING id",
    )
    .bind(writable_source_batch)
    .bind("5".repeat(64))
    .fetch_one(&rw)
    .await?;
    sqlx::query("UPDATE trading_calendars SET source_version = 'writer-v2' WHERE id = $1")
        .bind(projection_id)
        .execute(&rw)
        .await?;
    for statement in [
        "UPDATE data_batches SET kind = 'tampered'",
        "DELETE FROM data_batches",
        "DELETE FROM trading_calendars",
        "DELETE FROM trading_calendar_versions",
        "UPDATE trading_calendar_versions SET source = 'tampered'",
        "SELECT * FROM orders",
        "SELECT * FROM jobs",
        "SELECT * FROM audit_logs",
        "CREATE TABLE research_writer_hack (id integer)",
    ] {
        let denied = sqlx::query(statement).execute(&rw).await.unwrap_err();
        assert_eq!(pg_code(&denied).as_deref(), Some("42501"), "{statement}");
    }

    // ------------------------------------------------------------------
    // 10. sha256-hash columns enforce `^[0-9a-f]{64}$` (immutable manifests).
    // ------------------------------------------------------------------
    let bad_hash = sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2026-01-05', 'EOD', 'data/raw/qa/1', 'not-a-hash', 1, now())",
    )
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&bad_hash).as_deref(),
        Some("23514"),
        "sha256 columns must reject non-hex hashes"
    );
    let good_hash = "a".repeat(64);
    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2026-01-05', 'EOD', 'data/raw/qa/2', $1, 1, now())",
    )
    .bind(&good_hash)
    .execute(owner)
    .await?;

    // Large curves/orders/fills live in Parquet with DB manifests.
    let bad_artifact_hash = sqlx::query(
        "INSERT INTO result_artifacts (backtest_run_id, owner_user_id, artifact_type, \
         parquet_path, row_count, sha256, size_bytes) \
         VALUES ('00000000-0000-0000-0000-000000000001', $1, 'EQUITY_CURVE', \
         'data/artifacts/qa/1.parquet', 0, 'zz', 0)",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&bad_artifact_hash).as_deref(),
        Some("23514"),
        "result_artifacts.sha256 must be 64 hex chars"
    );

    // ------------------------------------------------------------------
    // 10. web_sessions: opaque-hash identity, unique.
    // ------------------------------------------------------------------
    let sess_hash = "b".repeat(64);
    let csrf_hash = "c".repeat(64);
    sqlx::query(
        "INSERT INTO web_sessions (user_id, session_hash, csrf_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(uid)
    .bind(&sess_hash)
    .bind(&csrf_hash)
    .execute(&owner_actor)
    .await?;
    let dup_session = sqlx::query(
        "INSERT INTO web_sessions (user_id, session_hash, csrf_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(uid)
    .bind(&sess_hash)
    .bind(&csrf_hash)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&dup_session).as_deref(),
        Some("23505"),
        "web_sessions.session_hash must be unique"
    );

    // ------------------------------------------------------------------
    // 11. data_entitlements lifecycle CHECK (PENDING|ACTIVE|EXPIRED|REVOKED).
    // ------------------------------------------------------------------
    sqlx::query(
        "INSERT INTO data_entitlements (contract_document_sha256, contract_reference, status, \
         covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES ($1, 'ref/qa/1', 'PENDING', '[\"krx-eod\"]'::jsonb, '[\"backtest\"]'::jsonb, \
         '2026-01-01', '2026-12-31', $2)",
    )
    .bind(&good_hash)
    .bind(uid)
    .execute(owner)
    .await?;
    let bad_entitlement = sqlx::query(
        "INSERT INTO data_entitlements (contract_document_sha256, contract_reference, status, \
         covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES ($1, 'ref/qa/2', 'BOGUS', '[]'::jsonb, '[]'::jsonb, '2026-01-01', '2026-12-31', $2)",
    )
    .bind(&good_hash)
    .bind(uid)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&bad_entitlement).as_deref(),
        Some("23514"),
        "data_entitlements.status must be CHECK-enforced"
    );

    // ------------------------------------------------------------------
    // 12. Risk gateway (0018): one immutable decision per intent.
    //
    // Every assertion here corresponds to a claim the migration's comments
    // make. A gate decision that could be edited, duplicated, or written
    // half-populated would not be evidence of why an order was allowed, which
    // is the only reason the row exists.
    // ------------------------------------------------------------------
    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('contract-v1', 3000, 1000000, 5000000, 500000, 300)",
    )
    .execute(owner)
    .await?;

    // A limit set that would deny every order is a misconfiguration, refused
    // by CHECK exactly as the crate's constructor refuses it.
    let zero_limit = sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('contract-bad', 3000, 0, 5000000, 500000, 300)",
    )
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&zero_limit).as_deref(),
        Some("23514"),
        "a zero max_order_value must be CHECK-refused"
    );

    let gate_insert = "INSERT INTO risk_events (owner_user_id, event_type, severity, \
         intent_ref, correlation_id, limits_version, decision, denied_by_check, reason_code, \
         evaluated_at) VALUES ($1, 'LIVE_ORDER_GATE', $2, $3, 'corr-1', 'contract-v1', $4, $5, \
         $6, now())";

    sqlx::query(gate_insert)
        .bind(uid)
        .bind("INFO")
        .bind("intent-contract-1")
        .bind("APPROVED")
        .bind(Option::<String>::None)
        .bind("APPROVED")
        .execute(&owner_actor)
        .await?;

    // One decision per intent, enforced by the partial unique index.
    let duplicate = sqlx::query(gate_insert)
        .bind(uid)
        .bind("WARNING")
        .bind("intent-contract-1")
        .bind("DENIED")
        .bind(Some("KILL_SWITCH"))
        .bind("LIVE_KILL_SWITCH_ENGAGED")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&duplicate).as_deref(),
        Some("23505"),
        "an intent may carry exactly one gate decision"
    );

    // A half-populated decision is the shape a partial write would take.
    let incomplete = sqlx::query(
        "INSERT INTO risk_events (owner_user_id, event_type, intent_ref, decision) \
         VALUES ($1, 'LIVE_ORDER_GATE', 'intent-contract-2', 'APPROVED')",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&incomplete).as_deref(),
        Some("23514"),
        "a gate decision missing its limits version or correlation id must be refused"
    );

    // An approval that names a denying check is self-contradictory.
    let contradiction = sqlx::query(gate_insert)
        .bind(uid)
        .bind("INFO")
        .bind("intent-contract-3")
        .bind("APPROVED")
        .bind(Some("KILL_SWITCH"))
        .bind("APPROVED")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&contradiction).as_deref(),
        Some("23514"),
        "an APPROVED decision must not name a denying check"
    );

    // The constraint applies to gate decisions only: other risk events keep
    // 0007's shape and need none of these columns.
    sqlx::query("INSERT INTO risk_events (owner_user_id, event_type) VALUES ($1, 'RATE_LIMIT')")
        .bind(uid)
        .execute(&owner_actor)
        .await?;

    // Append-only, even for the migration owner: the trigger refuses both.
    let updated = sqlx::query("UPDATE risk_events SET decision = 'DENIED' WHERE intent_ref = $1")
        .bind("intent-contract-1")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&updated).as_deref(),
        Some("42501"),
        "a recorded risk decision must not be editable"
    );
    let deleted = sqlx::query("DELETE FROM risk_events WHERE intent_ref = $1")
        .bind("intent-contract-1")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&deleted).as_deref(),
        Some("42501"),
        "a recorded risk decision must not be deletable"
    );

    // And `app` -- the role the API actually runs as -- holds no grant to try.
    for statement in [
        "UPDATE risk_events SET decision = 'DENIED'",
        "DELETE FROM risk_events",
    ] {
        let denied = sqlx::query(statement).execute(&app).await.unwrap_err();
        assert_eq!(
            pg_code(&denied).as_deref(),
            Some("42501"),
            "app must hold no mutation grant on risk_events: {statement}"
        );
    }

    // ------------------------------------------------------------------
    // 13. Order intents (0019): the constraints the migration claims.
    //
    // The api-server suite proves the repository behaves; these prove the
    // SCHEMA refuses the shapes the repository is trusted not to write, so a
    // future writer -- or a psql session -- cannot create them either.
    // ------------------------------------------------------------------
    // 0019's FK requires the instrument to exist; this database seeds none.
    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency, name, asset_class, status)          VALUES ('069500.KRX', '069500', 'KRX', 'KRW', 'KODEX 200', 'ETF', 'ACTIVE')          ON CONFLICT (id) DO NOTHING",
    )
    .execute(owner)
    .await?;

    let account_id: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency)          VALUES ($1, 'LIVE', 'contract-live', 'KRW') RETURNING id",
    )
    .bind(uid)
    .fetch_one(&owner_actor)
    .await?;

    let intent_insert = "INSERT INTO order_intents          (intent_ref, owner_user_id, account_id, instrument_id, side, quantity, price,           correlation_id, state, broker_order_no, cumulative_filled)          VALUES ($1, $2, $3, '069500.KRX', $4, $5::numeric, 7250, 'corr', $6, $7, $8::numeric)";

    // A well-formed intent.
    sqlx::query(intent_insert)
        .bind("oi-contract-1")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("INTENT_CREATED")
        .bind(Option::<String>::None)
        .bind("0")
        .execute(&owner_actor)
        .await?;

    // A state that names a broker order must HAVE one: an ACCEPTED row with
    // no order number is a row nobody can reconcile against the broker.
    let unbound = sqlx::query(intent_insert)
        .bind("oi-contract-2")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("ACCEPTED")
        .bind(Option::<String>::None)
        .bind("0")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&unbound).as_deref(),
        Some("23514"),
        "ACCEPTED without a broker order number must be refused"
    );

    // A fill beyond the order quantity.
    let overfilled = sqlx::query(intent_insert)
        .bind("oi-contract-3")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("PARTIALLY_FILLED")
        .bind(Some("B-1"))
        .bind("11")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&overfilled).as_deref(),
        Some("23514"),
        "cumulative_filled above the order quantity must be refused"
    );

    // An unknown state string. The set is closed so a typo cannot create a
    // state the machine has never heard of.
    let bogus_state = sqlx::query(intent_insert)
        .bind("oi-contract-4")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("ALMOST_FILLED")
        .bind(Option::<String>::None)
        .bind("0")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&bogus_state).as_deref(), Some("23514"));

    // One broker order belongs to one intent, in both directions.
    for (r, no) in [("oi-contract-5", "B-UNIQ"), ("oi-contract-6", "B-UNIQ")] {
        let result = sqlx::query(intent_insert)
            .bind(r)
            .bind(uid)
            .bind(account_id)
            .bind("BUY")
            .bind("10")
            .bind("ACCEPTED")
            .bind(Some(no))
            .bind("0")
            .execute(&owner_actor)
            .await;
        if r == "oi-contract-6" {
            assert_eq!(
                pg_code(&result.unwrap_err()).as_deref(),
                Some("23505"),
                "two intents must not claim one broker order"
            );
        } else {
            result?;
        }
    }

    // The event log: gapless per intent, and append-only.
    let event_insert = "INSERT INTO order_intent_events          (intent_ref, owner_user_id, seq, event_type, resulting_state)          VALUES ($1, $2, $3, $4, $5)";
    sqlx::query(event_insert)
        .bind("oi-contract-1")
        .bind(uid)
        .bind(1_i32)
        .bind("RISK_APPROVED")
        .bind("RISK_APPROVED")
        .execute(&owner_actor)
        .await?;
    let duplicate_seq = sqlx::query(event_insert)
        .bind("oi-contract-1")
        .bind(uid)
        .bind(1_i32)
        .bind("SUBMISSION_STARTED")
        .bind("SUBMITTING")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&duplicate_seq).as_deref(),
        Some("23505"),
        "two events must not share a sequence number within one intent"
    );

    // History does not change, not even for the migration owner.
    for statement in [
        "UPDATE order_intent_events SET resulting_state = 'FILLED'",
        "DELETE FROM order_intent_events",
    ] {
        let err = sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
            .execute(&owner_actor)
            .await
            .unwrap_err();
        assert_eq!(pg_code(&err).as_deref(), Some("42501"), "{statement}");
    }

    // But the intent row ITSELF is mutable: its state legitimately moves, and
    // fencing it would have been copying the 0018 pattern without the reason.
    sqlx::query("UPDATE order_intents SET state = 'RISK_APPROVED' WHERE intent_ref = $1")
        .bind("oi-contract-1")
        .execute(&owner_actor)
        .await?;

    drop(app);
    drop(aw);
    drop(wk);
    Ok(())
}

/// Revert (undo all migrations) then run again in a disposable DB: the schema
/// must be fully removed by the down scripts and fully restored by re-run.
///
/// Un-gated since Todo 3's live gate passed: runs with `cargo test -p
/// migration-contract` against `DATABASE_URL` (disposable scratch DB).
#[tokio::test]
async fn revert_and_rerun_in_disposable_db() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = revert_and_rerun_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("revert-and-rerun FAILED: {e}");
    }
}

async fn revert_and_rerun_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    let expected = up_migration_count();
    MIGRATOR.run(owner).await?;
    let applied = applied_count(owner).await? as usize;
    assert_eq!(
        applied, expected,
        "fresh DB must apply all {expected} migrations"
    );

    // Undo every migration. sqlx 0.9's `undo` reverts migrations whose version
    // is > target; target 0 therefore reverts everything (the pre-fix code
    // passed `expected`, which would have reverted nothing).
    MIGRATOR.undo(owner, 0).await?;
    let remaining = applied_count(owner).await?;
    assert_eq!(remaining, 0, "undo must revert all {expected} migrations");

    // Schema objects are gone.
    let jobs_gone: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.jobs')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        jobs_gone.is_none(),
        "after revert, public.jobs must not exist"
    );
    let calendar_history_gone: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass('public.trading_calendar_versions')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_history_gone.is_none(),
        "after revert, public.trading_calendar_versions must not exist"
    );

    // Run again from scratch.
    MIGRATOR.run(owner).await?;
    let applied2 = applied_count(owner).await? as usize;
    assert_eq!(
        applied2, expected,
        "re-run after revert must re-apply everything"
    );
    let audit_back: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.audit_logs')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        audit_back.as_deref(),
        Some("audit_logs"),
        "audit_logs must exist after re-run"
    );
    let calendar_history_back: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass('public.trading_calendar_versions')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_history_back.as_deref(),
        Some("trading_calendar_versions"),
        "trading_calendar_versions must exist after re-run"
    );

    // Post-revert DB still enforces the five-state contract (deterministic).
    let uid: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test','revert-owner','revert@example.test') RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    // The RLS policy check precedes constraint evaluation (PG18), so the
    // ORPHANED probe runs under an actor context to reach the CHECK.
    let owner_actor = actor_pool(super_url, db, "migration_owner", &uid.to_string()).await?;
    let sixth = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, status) VALUES ($1, 'backtest', 'ORPHANED')",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&sixth).as_deref(),
        Some("23514"),
        "sixth status still rejected after revert+run"
    );

    let _ = super_url;
    Ok(())
}
