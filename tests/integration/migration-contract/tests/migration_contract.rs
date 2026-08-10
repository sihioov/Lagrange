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

const SOURCE_INDEX_UP_SQL: &str =
    include_str!("../../../../migrations/0024_research_publication_source_index.up.sql");
const SOURCE_INDEX_DOWN_SQL: &str =
    include_str!("../../../../migrations/0024_research_publication_source_index.down.sql");
const CALENDAR_VERSION_INDEX_UP_SQL: &str =
    include_str!("../../../../migrations/0025_research_calendar_version_lookup.up.sql");
const CALENDAR_VERSION_INDEX_DOWN_SQL: &str =
    include_str!("../../../../migrations/0025_research_calendar_version_lookup.down.sql");
const RESEARCH_PUBLICATION_DOWN_SQL: &str =
    include_str!("../../../../migrations/0022_research_publication.down.sql");
const RESEARCH_SCHEMA_GATE_SQL: &str =
    include_str!("../../../../deploy/compose/research-schema-check.sql");

#[test]
fn tracked_research_schema_gate_is_fail_closed_and_migrations_bound_locks() {
    for token in [
        "version BETWEEN 22 AND 25",
        "max(version)",
        "convalidated",
        "pg_get_constraintdef",
        "format_type",
        "attnotnull",
        "attidentity",
        "pg_get_expr",
        "storage_path",
        "EXCEPT",
        "indisunique",
        "indisvalid",
        "indisready",
        "indislive",
        "relrowsecurity",
        "rolcanlogin",
        "rolsuper",
        "rolbypassrls",
        "rolcreatedb",
        "rolcreaterole",
        "pg_auth_members",
        "polcmd",
        "polpermissive",
        "tgenabled",
        "tgtype",
        "prosecdef",
        "pg_get_functiondef",
        "regexp_replace",
        "actual_function",
        "expected_function",
        "role_table_grants",
        "has_schema_privilege",
        "has_table_privilege",
        "has_sequence_privilege",
    ] {
        assert!(
            RESEARCH_SCHEMA_GATE_SQL.contains(token),
            "tracked research schema gate is missing {token}"
        );
    }
    for migration in [SOURCE_INDEX_UP_SQL, CALENDAR_VERSION_INDEX_UP_SQL] {
        assert!(migration.contains("PGOPTIONS='-c lock_timeout=5s' sqlx migrate run"));
        assert!(migration.contains("CONCURRENTLY"));
    }
}

fn executable_sql(sql: &str) -> String {
    sql.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Database-local grants executed as supervisor on each fresh scratch database.
const BOOTSTRAP_SQL: &str = include_str!("../bootstrap.sql");
/// Cluster-global roles serialized in the supervisor database before scratch creation.
const ROLE_BOOTSTRAP_SQL: &str = include_str!("../role-bootstrap.sql");

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

#[test]
fn cluster_role_bootstrap_cannot_grant_scratch_schema_privileges() {
    let executable = executable_sql(ROLE_BOOTSTRAP_SQL).to_ascii_uppercase();
    assert!(!executable.contains("GRANT "));
    assert!(!executable.contains("SCHEMA "));
    let scratch_executable = executable_sql(BOOTSTRAP_SQL).to_ascii_uppercase();
    assert!(!scratch_executable.contains("CREATE ROLE"));
    assert!(!scratch_executable.contains("ALTER ROLE"));
}

/// Rewrite a supervisor URL (`postgres://user[:pw]@host:port/db`) for the
/// legacy serving-role checks. Migration and research writer pools retain the
/// supplied supervisor identity and assume their fixed effective role in
/// `after_connect` instead.
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

fn supervisor_db_url(super_url: &str, db: &str) -> String {
    let (head, _) = super_url
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    format!("{head}/{db}")
}

async fn effective_role_pool(
    super_url: &str,
    db: &str,
    role: &'static str,
    actor_user_id: Option<&str>,
    max_connections: u32,
) -> Result<PgPool, Box<dyn Error>> {
    let setup = match role {
        "migration_owner" => "SET ROLE migration_owner",
        "research_writer" => "SET ROLE research_writer",
        _ => return Err(format!("unsupported effective role {role}").into()),
    };
    let mut options: sqlx::postgres::PgConnectOptions = supervisor_db_url(super_url, db)
        .parse()
        .map_err(Box::<dyn Error>::from)?;
    if let Some(user_id) = actor_user_id {
        options = options.options([("app.actor_user_id", user_id.to_owned())]);
    }
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                sqlx::raw_sql(setup).execute(&mut *connection).await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .map_err(Box::<dyn Error>::from)?;
    let identities: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&pool)
        .await?;
    assert_eq!(identities.0, role);
    assert_ne!(identities.0, identities.1);
    Ok(pool)
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
    if role == "migration_owner" {
        return effective_role_pool(super_url, db, "migration_owner", Some(user_id), 3).await;
    }
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
    let mut role_bootstrap = admin.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('lagrange-test-role-bootstrap'))")
        .execute(&mut *role_bootstrap)
        .await?;
    sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
        .execute(&mut *role_bootstrap)
        .await?;
    role_bootstrap.commit().await?;
    sqlx::query(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    sqlx::query(ddl_for(&db, "CREATE DATABASE {db}"))
        .execute(&admin)
        .await?;
    drop(admin);

    let super_new = admin_pool(&supervisor_db_url(super_url, &db)).await?;
    sqlx::raw_sql(BOOTSTRAP_SQL).execute(&super_new).await?;
    sqlx::raw_sql(ddl_for(
        &db,
        "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer, research_writer, admin",
    ))
    .execute(&super_new)
    .await?;
    drop(super_new);

    let owner = effective_role_pool(super_url, &db, "migration_owner", None, 3).await?;
    let roles: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&owner)
        .await?;
    assert_eq!(roles.0, "migration_owner");
    assert_ne!(roles.0, roles.1);
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
    if role == "research_writer" {
        return effective_role_pool(super_url, db, "research_writer", None, 2).await;
    }
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
    assert!(
        RESEARCH_PUBLICATION_DOWN_SQL.contains("SET LOCAL lock_timeout = '5s';"),
        "0022 down must bound blocking rollback DDL with a transactional lock_timeout"
    );
    for (name, sql, expected_statement) in [
        (
            "0024 up",
            SOURCE_INDEX_UP_SQL,
            "CREATE UNIQUE INDEX CONCURRENTLY data_batches_source_file_uq\nON data_batches (provider, market, source_batch_id, source_file_name)\nWHERE source_batch_id IS NOT NULL;",
        ),
        (
            "0024 down",
            SOURCE_INDEX_DOWN_SQL,
            "DROP INDEX CONCURRENTLY IF EXISTS data_batches_source_file_uq;",
        ),
        (
            "0025 up",
            CALENDAR_VERSION_INDEX_UP_SQL,
            "CREATE INDEX CONCURRENTLY trading_calendar_versions_source_lookup_idx\nON trading_calendar_versions (exchange, source_version)\nINCLUDE (source, timezone, content_sha256);",
        ),
        (
            "0025 down",
            CALENDAR_VERSION_INDEX_DOWN_SQL,
            "DROP INDEX CONCURRENTLY IF EXISTS trading_calendar_versions_source_lookup_idx;",
        ),
    ] {
        assert!(
            sql.starts_with("-- no-transaction"),
            "{name} must begin with SQLx's no-transaction directive"
        );
        assert!(
            sql.contains("externally") && sql.contains("lock_timeout"),
            "{name} must document externally supplied finite lock_timeout"
        );
        assert_eq!(
            executable_sql(sql),
            expected_statement,
            "{name} must contain only the concurrent DDL statement"
        );
    }
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

    let source_index_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 24 && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("0024 source-lineage index migration must exist");
    assert!(
        source_index_migration.no_tx,
        "0024 must opt out of a transaction so PostgreSQL can build its index concurrently"
    );
    let source_index_down_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 24 && migration.migration_type == MigrationType::ReversibleDown
        })
        .expect("0024 source-lineage index down migration must exist");
    assert!(
        source_index_down_migration.no_tx,
        "0024 down must opt out of a transaction so PostgreSQL can drop its index concurrently"
    );
    let calendar_index_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 25 && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("0025 calendar source-version lookup migration must exist");
    assert!(
        calendar_index_migration.no_tx,
        "0025 must opt out of a transaction so PostgreSQL can build its index concurrently"
    );
    let calendar_index_down_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 25 && migration.migration_type == MigrationType::ReversibleDown
        })
        .expect("0025 calendar source-version lookup down migration must exist");
    assert!(
        calendar_index_down_migration.no_tx,
        "0025 down must opt out of a transaction so PostgreSQL can drop its index concurrently"
    );
    let calendar_index_shape: (i32, i32, bool, bool) = sqlx::query_as(
        "SELECT indnkeyatts::integer, indnatts::integer, indisunique, indisvalid \
         FROM pg_index WHERE indexrelid = \
         'public.trading_calendar_versions_source_lookup_idx'::regclass",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(calendar_index_shape, (2, 5, false, true));
    let calendar_index_definition: String = sqlx::query_scalar(
        "SELECT pg_get_indexdef('public.trading_calendar_versions_source_lookup_idx'::regclass)",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_index_definition.contains(
            "USING btree (exchange, source_version) INCLUDE (source, timezone, content_sha256)"
        ),
        "unexpected 0025 index definition: {calendar_index_definition}"
    );

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
    let publication_constraints: Vec<(String, bool)> = sqlx::query_as(
        "SELECT conname, convalidated FROM pg_constraint WHERE conname IN ( \
         'data_batches_fetch_mode_check', 'data_batches_provenance_all_or_none_check', \
         'trading_calendars_content_sha256_check', 'trading_calendars_provenance_all_or_none_check') \
         ORDER BY conname",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        publication_constraints,
        vec![
            ("data_batches_fetch_mode_check".into(), true),
            ("data_batches_provenance_all_or_none_check".into(), true),
            ("trading_calendars_content_sha256_check".into(), true),
            (
                "trading_calendars_provenance_all_or_none_check".into(),
                true
            ),
        ],
        "publication CHECK constraints must finish validated"
    );
    let source_lineage_index: (bool, bool) = sqlx::query_as(
        "SELECT i.indisunique, i.indpred IS NOT NULL \
         FROM pg_index i WHERE i.indexrelid = 'public.data_batches_source_file_uq'::regclass",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        source_lineage_index,
        (true, true),
        "source lineage index must be unique and partial"
    );

    let calendar_identity: (String, String, String) = sqlx::query_as(
        "SELECT data_type, is_identity, identity_generation \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'trading_calendar_versions' \
         AND column_name = 'id'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_identity,
        ("bigint".into(), "YES".into(), "ALWAYS".into()),
        "trading_calendar_versions.id must be bigint GENERATED ALWAYS AS IDENTITY"
    );
    let calendar_history_rls: bool = sqlx::query_scalar(
        "SELECT relrowsecurity FROM pg_class WHERE oid = 'public.trading_calendar_versions'::regclass",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_history_rls, "calendar history must enable RLS");

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
    let invalid_session_type = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-04', 'OPEN', 'Asia/Seoul', 'KRX', 'bad-session', $1, $2, now())",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&invalid_session_type).as_deref(),
        Some("23514"),
        "calendar history must reject only an invalid session_type"
    );
    let invalid_timezone = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-05', 'TRADING', 'UTC', 'KRX', 'bad-timezone', $1, $2, now())",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&invalid_timezone).as_deref(),
        Some("23514"),
        "calendar history must reject only an invalid timezone"
    );
    let invalid_calendar_hash = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-06', 'TRADING', 'Asia/Seoul', 'KRX', 'bad-hash', $1, 'bad', now())",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&invalid_calendar_hash).as_deref(),
        Some("23514"),
        "calendar history must reject only an invalid content hash"
    );
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
    for (table, expected) in [
        (
            "data_batches",
            (true, true, false, false, false, false, false, false),
        ),
        (
            "trading_calendar_versions",
            (true, true, false, false, false, false, false, false),
        ),
        (
            "trading_calendars",
            (true, true, true, false, false, false, false, false),
        ),
    ] {
        let actual: (bool, bool, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT has_table_privilege('research_writer', $1, 'SELECT'), \
                    has_table_privilege('research_writer', $1, 'INSERT'), \
                    has_table_privilege('research_writer', $1, 'UPDATE'), \
                    has_table_privilege('research_writer', $1, 'DELETE'), \
                    has_table_privilege('research_writer', $1, 'TRUNCATE'), \
                    has_table_privilege('research_writer', $1, 'REFERENCES'), \
                    has_table_privilege('research_writer', $1, 'TRIGGER'), \
                    has_table_privilege('research_writer', $1, 'MAINTAIN')",
        )
        .bind(table)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            actual, expected,
            "research_writer ACL must be exact for {table}"
        );
    }
    let writer_policies: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT tablename, policyname, cmd FROM pg_policies \
         WHERE schemaname = 'public' AND 'research_writer' = ANY(roles) \
         ORDER BY tablename, policyname",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        writer_policies,
        vec![
            (
                "data_batches".into(),
                "data_batches_insert_research_writer".into(),
                "INSERT".into()
            ),
            (
                "data_batches".into(),
                "data_batches_select_research_writer".into(),
                "SELECT".into()
            ),
            (
                "trading_calendar_versions".into(),
                "trading_calendar_versions_insert_research_writer".into(),
                "INSERT".into()
            ),
            (
                "trading_calendar_versions".into(),
                "trading_calendar_versions_select_research_writer".into(),
                "SELECT".into()
            ),
            (
                "trading_calendars".into(),
                "trading_calendars_insert_research_writer".into(),
                "INSERT".into()
            ),
            (
                "trading_calendars".into(),
                "trading_calendars_select_research_writer".into(),
                "SELECT".into()
            ),
            (
                "trading_calendars".into(),
                "trading_calendars_update_research_writer".into(),
                "UPDATE".into()
            ),
        ],
        "research_writer must have only the requested publication RLS policies"
    );
    let sequence_privileges: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_sequence_privilege('research_writer', 'public.trading_calendar_versions_id_seq', 'USAGE'), \
                has_sequence_privilege('research_writer', 'public.trading_calendar_versions_id_seq', 'SELECT'), \
                has_sequence_privilege('research_writer', 'public.trading_calendar_versions_id_seq', 'UPDATE')",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        sequence_privileges == (false, false, false),
        "research_writer must not have direct identity-sequence privileges"
    );
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
    let direct_sequence = sqlx::query("SELECT nextval('public.trading_calendar_versions_id_seq')")
        .execute(&rw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&direct_sequence).as_deref(),
        Some("42501"),
        "research_writer must not advance the calendar identity sequence directly"
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
        "TRUNCATE TABLE data_batches",
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

/// The online-migration sequence has observable safe boundaries: 0022 adds
/// NOT VALID checks under brief metadata locks, 0023 validates them after that
/// transaction commits, and 0024 builds the populated-table index concurrently.
#[tokio::test]
async fn research_publication_migration_boundaries() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = research_publication_boundaries_body(&owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("research-publication migration boundaries FAILED: {e}");
    }
}

async fn research_publication_boundaries_body(owner: &PgPool) -> Result<(), Box<dyn Error>> {
    const PUBLICATION_CHECKS: [&str; 4] = [
        "data_batches_fetch_mode_check",
        "data_batches_provenance_all_or_none_check",
        "trading_calendars_content_sha256_check",
        "trading_calendars_provenance_all_or_none_check",
    ];

    let validation_state = |expected: bool| async move {
        let checks: Vec<(String, bool)> = sqlx::query_as(
            "SELECT conname, convalidated FROM pg_constraint WHERE conname = ANY($1) ORDER BY conname",
        )
        .bind(PUBLICATION_CHECKS.as_slice())
        .fetch_all(owner)
        .await?;
        let expected_checks = PUBLICATION_CHECKS
            .iter()
            .map(|name| (name.to_string(), expected))
            .collect::<Vec<_>>();
        Ok::<_, sqlx::Error>((checks, expected_checks))
    };

    MIGRATOR.run_to(22, owner).await?;
    assert_eq!(applied_count(owner).await?, 22, "0022 must apply alone");
    let (checks_after_0022, expected_unvalidated) = validation_state(false).await?;
    assert_eq!(
        checks_after_0022, expected_unvalidated,
        "0022 must leave populated-table checks present but NOT VALID"
    );
    let source_batch_id = Uuid::parse_str("00000000-0000-0000-0000-000000000026").unwrap();
    let invalid_writes = [
        (
            "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
             VALUES ('KRX', 'KR', '2026-03-01', 'REFERENCE', 'data/raw/boundary/fetch', $1, 1, now(), $2, 'source.csv', 'INVALID')",
            true,
        ),
        (
            "INSERT INTO trading_calendars (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '2026-03-01', 'TRADING', 'Asia/Seoul', 'KRX', 'boundary', $1, 'invalid', now())",
            false,
        ),
    ];
    for (statement, has_hash_parameter) in invalid_writes {
        let mut query = sqlx::query(statement);
        if has_hash_parameter {
            query = query.bind("a".repeat(64));
        }
        let invalid = query
            .bind(source_batch_id)
            .execute(owner)
            .await
            .unwrap_err();
        assert_eq!(
            pg_code(&invalid).as_deref(),
            Some("23514"),
            "NOT VALID checks must still reject invalid new publication writes"
        );
    }

    MIGRATOR.run_to(23, owner).await?;
    assert_eq!(applied_count(owner).await?, 23, "0023 must validate checks");
    let (checks_after_0023, expected_validated) = validation_state(true).await?;
    assert_eq!(
        checks_after_0023, expected_validated,
        "0023 must finish publication checks validated"
    );

    MIGRATOR.run_to(24, owner).await?;
    assert_eq!(
        applied_count(owner).await?,
        24,
        "0024 must add the concurrent source index"
    );
    let source_index: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(source_index.as_deref(), Some("data_batches_source_file_uq"));
    let calendar_lookup_before_0025: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_lookup_before_0025.is_none());

    let expected = up_migration_count() as i64;
    MIGRATOR.run(owner).await?;
    assert_eq!(applied_count(owner).await?, expected);
    let calendar_lookup: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
    );

    MIGRATOR.undo(owner, 24).await?;
    assert_eq!(applied_count(owner).await?, 24, "0025 down must run first");
    let calendar_lookup_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_lookup_gone.is_none());
    let source_index_retained: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        source_index_retained.as_deref(),
        Some("data_batches_source_file_uq")
    );

    MIGRATOR.undo(owner, 23).await?;
    assert_eq!(applied_count(owner).await?, 23, "0024 down must run second");
    let source_index_gone: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        source_index_gone.is_none(),
        "0024 down must remove the index"
    );
    let (checks_after_0024_down, expected_still_validated) = validation_state(true).await?;
    assert_eq!(checks_after_0024_down, expected_still_validated);

    MIGRATOR.undo(owner, 22).await?;
    assert_eq!(applied_count(owner).await?, 22, "0023 down must run third");
    let (checks_after_0023_down, expected_restored_unvalidated) = validation_state(false).await?;
    assert_eq!(
        checks_after_0023_down, expected_restored_unvalidated,
        "0023 down must restore 0022's NOT VALID boundary"
    );

    MIGRATOR.undo(owner, 21).await?;
    assert_eq!(applied_count(owner).await?, 21, "0022 down must run last");
    let history_gone: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.trading_calendar_versions')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        history_gone.is_none(),
        "0022 down must remove calendar history"
    );

    MIGRATOR.run(owner).await?;
    assert_eq!(
        applied_count(owner).await?,
        expected,
        "full reapply must restore all publication migration boundaries"
    );
    Ok(())
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

    // Revert 0025, 0024, then 0023 before 0022 while all earlier tables remain.
    // This proves each down migration restores its own boundary rather than
    // relying on 0003.down to hide omitted objects in a full teardown.
    MIGRATOR.undo(owner, 24).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        expected - 1,
        "undo to 0024 must revert only 0025"
    );
    let calendar_lookup_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_lookup_gone.is_none(),
        "0025 down must remove the concurrent calendar source-version index"
    );
    let source_index_retained: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        source_index_retained.as_deref(),
        Some("data_batches_source_file_uq")
    );

    MIGRATOR.undo(owner, 23).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        expected - 2,
        "undo to 0023 must revert only 0024"
    );
    let source_index_gone: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        source_index_gone.is_none(),
        "0024 down must remove the concurrent source-lineage index"
    );
    MIGRATOR.undo(owner, 22).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        expected - 3,
        "undo to 0022 must revert only 0023"
    );
    sqlx::query(
        "CREATE UNIQUE INDEX data_batches_source_file_uq \
         ON data_batches (provider, market, source_batch_id, source_file_name) \
         WHERE source_batch_id IS NOT NULL",
    )
    .execute(owner)
    .await?;
    MIGRATOR.undo(owner, 21).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        expected - 4,
        "undo to 0021 must revert only 0022"
    );
    for object in [
        "public.trading_calendar_versions",
        "public.data_batches_source_file_uq",
        "public.trading_calendar_versions_source_lookup_idx",
    ] {
        let gone: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(object)
            .fetch_one(owner)
            .await?;
        assert!(gone.is_none(), "0022 down must remove {object}");
    }
    let remaining_0022_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = 'public' \
         AND ((table_name = 'data_batches' AND column_name IN ('source_batch_id', 'source_file_name', 'fetch_mode')) \
           OR (table_name = 'trading_calendars' AND column_name IN ('source_batch_id', 'content_sha256', 'retrieved_at')))",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        remaining_0022_columns, 0,
        "0022 down must remove added columns"
    );
    let remaining_0022_constraints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE conname IN ( \
         'data_batches_fetch_mode_check', 'data_batches_provenance_all_or_none_check', \
         'trading_calendars_content_sha256_check', 'trading_calendars_provenance_all_or_none_check')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        remaining_0022_constraints, 0,
        "0022 down must remove added constraints"
    );
    let function_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure('public.trading_calendar_versions_reject_mutation()')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        function_gone.is_none(),
        "0022 down must remove its trigger function"
    );
    let remaining_0022_policies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_policies WHERE policyname IN ( \
         'data_batches_select_research_writer', 'data_batches_insert_research_writer', \
         'trading_calendars_select_research_writer', 'trading_calendars_insert_research_writer', \
         'trading_calendars_update_research_writer', 'trading_calendar_versions_select_readers', \
         'trading_calendar_versions_select_research_writer', 'trading_calendar_versions_insert_research_writer')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        remaining_0022_policies, 0,
        "0022 down must remove its RLS policies"
    );
    for (table, privilege) in [("data_batches", "INSERT"), ("trading_calendars", "UPDATE")] {
        let retained: bool =
            sqlx::query_scalar("SELECT has_table_privilege('research_writer', $1, $2)")
                .bind(table)
                .bind(privilege)
                .fetch_one(owner)
                .await?;
        assert!(
            !retained,
            "0022 down must revoke research_writer {privilege} on {table}"
        );
    }
    let research_writer_survives: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'research_writer')",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        research_writer_survives,
        "0022 down must retain the externally created research_writer role"
    );

    MIGRATOR.run(owner).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        expected,
        "re-applying after 0022-only undo must restore 0022"
    );
    let calendar_history_restored: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.trading_calendar_versions')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        calendar_history_restored.as_deref(),
        Some("trading_calendar_versions"),
        "0022 up must restore calendar history after its standalone down"
    );
    let calendar_lookup_restored: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup_restored.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
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
    let calendar_lookup_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_lookup_gone.is_none());

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
    let calendar_lookup_back: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup_back.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
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
