//! Todo 23 tenancy gate: Row-Level Security ownership isolation, append-only
//! audit, and the table-owner / BYPASSRLS hazards, proven against a disposable
//! PostgreSQL 18 database with the real migrations (0001..0010).
//!
//! Every test function is named `tenancy_*` because the plan acceptance
//! command is `cargo test -p auth -p api-server tenancy` (libtest filters on
//! test function names).
//!
//! Harness conventions (inherited from Todos 3b/19): DATABASE_URL is required
//! (tests skip cleanly without it); each test gets a FRESH scratch database
//! with roles bootstrapped cluster-wide and all migrations applied; pools are
//! built with the `app.actor_user_id` connection option to emulate "connected
//! as user X"; whole-test retries ride through the documented WSL relay
//! latency spikes.

use api_server::TenancyError;
use api_server::actor_tx::ACTOR_GUC;
use api_server::repos::accounts::{AccountRepo, NewAccount};
use api_server::repos::admin::AdminRepo;
use api_server::repos::artifacts::ArtifactRepo;
use api_server::repos::audit::AuditWriter;
use api_server::repos::backtest_runs::{BacktestRunRepo, NewBacktestRun};
use api_server::repos::shared::SharedDataRepo;
use api_server::repos::strategy_configs::{NewStrategyConfig, StrategyConfigRepo};
use auth::entitlement::Actor;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Roles bootstrapped cluster-wide by the harness (migrations grant to them).
/// `admin` is Todo 23's dedicated read-only admin role (no BYPASSRLS).
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

/// The 21 tenant tables (contract list from migration-contract) that must have
/// RLS enabled AND forced with row-local policies.
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
    "robustness_suites",
    "robustness_children",
    "account_strategy_bindings",
    "pending_targets",
];

/// The sqlx Migrator embedding the real workspace migrations.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

fn fresh_db_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis();
    format!(
        "tenancy_{}_{}_{}",
        std::process::id(),
        ts,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (_scheme, rest) = super_url
        .split_once("://")
        .expect("DATABASE_URL must start with a scheme");
    let (auth, hostport_db) = rest.split_once('@').expect("DATABASE_URL must contain @");
    let (_, pw) = match auth.split_once(':') {
        Some((_, p)) => ((), Some(p)),
        None => ((), None),
    };
    let (hostport, _old_db) = hostport_db
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    }
}

/// Dynamic DDL (database names cannot be bind parameters); the generated name
/// is validated `[a-z0-9_]` in `create_scratch_db` before any statement runs.
fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

/// Connect with retries (documented WSL relay latency spikes; pool answers a
/// real `SELECT 1` before counting).
async fn connect_with_retry(
    url: &str,
    max_conns: u32,
) -> Result<PgPool, Box<dyn std::error::Error>> {
    connect_with_options(url.parse::<PgConnectOptions>()?, max_conns).await
}

/// Connect with retries plus extra per-connection options (e.g. the actor GUC).
async fn connect_with_options(
    opts: PgConnectOptions,
    max_conns: u32,
) -> Result<PgPool, Box<dyn std::error::Error>> {
    let mut last: Option<sqlx::Error> = None;
    for attempt in 0..6u32 {
        let connect = sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_conns)
            .acquire_timeout(Duration::from_secs(8))
            .connect_with(opts.clone())
            .await;
        let pool = match connect {
            Ok(pool) => pool,
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(1200 * (attempt as u64 + 1))).await;
                continue;
            }
        };
        match sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&pool)
            .await
        {
            Ok(_) => return Ok(pool),
            Err(e) => {
                last = Some(e);
                pool.close().await;
                tokio::time::sleep(Duration::from_millis(1200 * (attempt as u64 + 1))).await;
            }
        }
    }
    Err(last
        .map(Into::into)
        .unwrap_or_else(|| "connect_with_options exhausted attempts".into()))
}

/// A pool whose connections carry the actor GUC (`app.actor_user_id = $uid`).
async fn actor_pool(
    super_url: &str,
    db: &str,
    role: &str,
    user_id: &str,
) -> Result<PgPool, Box<dyn std::error::Error>> {
    let opts = conn_url(super_url, role, db).parse::<PgConnectOptions>()?;
    connect_with_options(opts.options([(ACTOR_GUC, user_id.to_string())]), 4).await
}

async fn admin_pool(url: &str) -> Result<PgPool, Box<dyn std::error::Error>> {
    connect_with_retry(url, 4).await
}

/// Create a fresh scratch database, bootstrap roles, apply every migration,
/// and insert a shared strategy row. Returns (db_name, superuser_pool).
async fn create_scratch_db(
    super_url: &str,
) -> Result<(String, PgPool), Box<dyn std::error::Error>> {
    let db = fresh_db_name();
    assert!(
        db.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "generated database name must be a safe identifier"
    );
    let super_pool = admin_pool(super_url).await?;
    sqlx::query(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&super_pool)
        .await?;
    sqlx::query(ddl_for(&db, "CREATE DATABASE {db}"))
        .execute(&super_pool)
        .await?;
    drop(super_pool);

    let new_super = connect_with_retry(&conn_url(super_url, "postgres", &db), 4).await?;
    sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
        .execute(&new_super)
        .await?;
    sqlx::raw_sql(ddl_for(
        &db,
        "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer, admin",
    ))
    .execute(&new_super)
    .await?;
    // Migrations run as migration_owner (the schema owner), matching the
    // production deployment; the ownership assertions depend on it.
    let owner = connect_with_retry(&conn_url(super_url, "migration_owner", &db), 4).await?;
    MIGRATOR.run(&owner).await?;
    sqlx::query("INSERT INTO strategies (id, display_name) VALUES ('ma200-trend', 'MA200 Trend')")
        .execute(&new_super)
        .await?;
    Ok((db, new_super))
}

async fn drop_scratch_db(super_url: &str, db: &str) -> Result<(), Box<dyn std::error::Error>> {
    let super_pool = admin_pool(super_url).await?;
    sqlx::query(ddl_for(db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&super_pool)
        .await?;
    Ok(())
}

async fn insert_test_user(
    pool: &PgPool,
    subject: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind(subject)
    .bind(format!("{subject}@example.test"))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Run a test body on a fresh scratch database, retrying up to 3 times with a
/// NEW database per attempt (documented relay spikes / operator restarts).
type BoxedTenancyFuture<'a, T> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<T, Box<dyn std::error::Error>>> + 'a>,
>;

async fn run_tenancy<F, T>(label: &str, super_url: &str, body: F) -> T
where
    F: for<'a> Fn(&'a str, &'a str, &'a PgPool) -> BoxedTenancyFuture<'a, T>,
{
    let mut last: Option<String> = None;
    for attempt in 0..3u32 {
        let scratch = match create_scratch_db(super_url).await {
            Ok(v) => v,
            Err(e) => {
                last = Some(format!("setup: {e}"));
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        let result = body(super_url, &scratch.0, &scratch.1).await;
        let _ = drop_scratch_db(super_url, &scratch.0).await;
        match result {
            Ok(v) => return v,
            Err(e) => {
                last = Some(format!("{e}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        eprintln!(
            "{label}: attempt {} failed ({last:?}); retrying with a fresh DB",
            attempt + 1
        );
    }
    panic!(
        "{label} FAILED after 3 attempts: {}",
        last.unwrap_or_default()
    );
}

fn require_db_url() -> Result<String, Box<dyn std::error::Error>> {
    match std::env::var("DATABASE_URL") {
        Ok(u) if !u.is_empty() => Ok(u),
        _ => Err("DATABASE_URL is not set; skipping DB-gated tenancy suite".into()),
    }
}

fn pg_code(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(db) => db.code().map(|c| c.into_owned()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 1. RLS is enabled AND forced on every tenant table, with row-local policies.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_rls_enabled_and_forced_on_every_tenant_table() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("rls matrix", &super_url, |_s, _d, pool| {
        Box::pin(async move {
            for t in TENANT_TABLES {
                let row: (bool, bool) = sqlx::query_as(
                    "SELECT relrowsecurity, relforcerowsecurity \
                     FROM pg_class WHERE oid = $1::regclass",
                )
                .bind(t)
                .fetch_one(pool)
                .await
                .map_err(Box::<dyn std::error::Error>::from)?;
                assert!(row.0, "table {t} must have RLS enabled");
                assert!(row.1, "table {t} must have RLS FORCED");
                let policies: Vec<String> = sqlx::query_scalar(
                    "SELECT policyname FROM pg_policies WHERE schemaname = 'public' AND tablename = $1 ORDER BY policyname",
                )
                .bind(t)
                .fetch_all(pool)
                .await
                .map_err(Box::<dyn std::error::Error>::from)?;
                assert!(
                    policies.iter().any(|p| p.contains("app")),
                    "table {t} must have an app policy, got {policies:?}"
                );
                assert!(
                    policies.iter().any(|p| p.contains("worker")),
                    "table {t} must have a worker policy, got {policies:?}"
                );
            }
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 2. Member A cannot read Member B resources (repo + raw SQL, zero foreign
//    fields).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_member_a_cannot_read_member_b_strategy_configs() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("A cannot read B configs", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-read-a").await?;
            let b_id = insert_test_user(pool, "ten-read-b").await?;
            let actor_a = Actor::member(a_id.to_string());
            let actor_b = Actor::member(b_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;
            let repo_a = StrategyConfigRepo::new(pool_a.clone());
            let repo_b = StrategyConfigRepo::new(pool_b.clone());

            let created = repo_a
                .create(
                    &actor_a,
                    NewStrategyConfig {
                        strategy_id: "ma200-trend".into(),
                        strategy_version: "1.0.0".into(),
                        config_json: serde_json::json!({"fast": 20}),
                        is_active: true,
                    },
                )
                .await?;
            assert_eq!(created.owner_user_id, a_id, "row must be owned by A");

            // Repo surface: B gets NotFound, not a row.
            match repo_b.get(&actor_b, created.id).await {
                Err(TenancyError::NotFound) => {}
                other => panic!("B must not read A's config, got {other:?}"),
            }
            // B's list is empty: zero foreign fields.
            let b_list = repo_b.list(&actor_b).await?;
            assert!(b_list.is_empty(), "B's list must be empty, got {b_list:?}");

            // Raw SQL probe (GUC=B): direct id guess yields zero rows.
            let raw: i64 =
                sqlx::query_scalar("SELECT count(*) FROM user_strategy_configs WHERE id = $1")
                    .bind(created.id)
                    .fetch_one(&pool_b)
                    .await
                    .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(raw, 0, "raw SELECT as B must not see A's row");
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 3. Member A cannot update or delete Member B resources.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_member_a_cannot_update_or_delete_member_b() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("A cannot update/delete B", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-mut-a").await?;
            let b_id = insert_test_user(pool, "ten-mut-b").await?;
            let actor_a = Actor::member(a_id.to_string());
            let actor_b = Actor::member(b_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;
            let repo_a = StrategyConfigRepo::new(pool_a.clone());
            let repo_b = StrategyConfigRepo::new(pool_b.clone());

            let created = repo_a
                .create(
                    &actor_a,
                    NewStrategyConfig {
                        strategy_id: "ma200-trend".into(),
                        strategy_version: "1.0.0".into(),
                        config_json: serde_json::json!({"fast": 20}),
                        is_active: true,
                    },
                )
                .await?;

            match repo_b
                .update(&actor_b, created.id, serde_json::json!({"fast": 999}), true)
                .await
            {
                Err(TenancyError::NotFound) => {}
                other => panic!("B must not update A's config, got {other:?}"),
            }
            match repo_b.delete(&actor_b, created.id).await {
                Err(TenancyError::NotFound) => {}
                other => panic!("B must not delete A's config, got {other:?}"),
            }

            // Raw SQL: UPDATE/DELETE as B affect zero rows.
            let upd =
                sqlx::query("UPDATE user_strategy_configs SET is_active = false WHERE id = $1")
                    .bind(created.id)
                    .execute(&pool_b)
                    .await
                    .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(upd.rows_affected(), 0, "raw UPDATE as B must touch 0 rows");
            let del = sqlx::query("DELETE FROM user_strategy_configs WHERE id = $1")
                .bind(created.id)
                .execute(&pool_b)
                .await
                .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(del.rows_affected(), 0, "raw DELETE as B must touch 0 rows");

            // A still owns the untouched row.
            let still = repo_a.get(&actor_a, created.id).await?;
            assert!(still.is_active, "A's row must be untouched by B");
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 4. Crafted owner id / direct id guess: typed denial, never a leak.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_crafted_owner_id_insert_is_denied() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("crafted owner id", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-craft-a").await?;
            let b_id = insert_test_user(pool, "ten-craft-b").await?;
            let actor_b = Actor::member(b_id.to_string());
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;

            // B connects with actor context B and tries to INSERT a row owned
            // by A: RLS WITH CHECK denies with SQLSTATE 42501.
            let crafted = sqlx::query(
                "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
                 VALUES ($1, 'PAPER', 'stolen', 'KRW')",
            )
            .bind(a_id)
            .execute(&pool_b)
            .await
            .unwrap_err();
            assert_eq!(
                pg_code(&crafted).as_deref(),
                Some("42501"),
                "crafted owner insert must be denied by RLS, got {crafted}"
            );

            // The repository ignores a caller-supplied owner entirely: B's
            // create binds B as owner.
            let repo_b = AccountRepo::new(pool_b.clone());
            let row = repo_b
                .create(
                    &actor_b,
                    NewAccount {
                        account_type: "PAPER".into(),
                        name: "b-own".into(),
                        currency: "KRW".into(),
                        initial_cash: Some("10000000".into()),
                        cost_profile_id: "KRX_ETF_DEFAULT".into(),
                        cost_profile_version: 1,
                    },
                )
                .await?;
            assert_eq!(row.owner_user_id, b_id, "repo must derive owner from actor");
            let owned: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM accounts WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(row.id)
            .bind(a_id)
            .fetch_one(&pool_b)
            .await
            .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(
                owned, 0,
                "no row may exist under A's ownership created by B"
            );
            Ok(())
        })
    })
    .await;
}

#[tokio::test]
async fn tenancy_direct_id_guess_leaks_no_foreign_fields() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("id guess zero foreign fields", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-guess-a").await?;
            let b_id = insert_test_user(pool, "ten-guess-b").await?;
            let actor_a = Actor::member(a_id.to_string());
            let actor_b = Actor::member(b_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;
            let repo_a = AccountRepo::new(pool_a.clone());
            let repo_b = AccountRepo::new(pool_b.clone());

            let acc = repo_a
                .create(
                    &actor_a,
                    NewAccount {
                        account_type: "PAPER".into(),
                        name: "a-secret".into(),
                        currency: "KRW".into(),
                        initial_cash: Some("10000000".into()),
                        cost_profile_id: "KRX_ETF_DEFAULT".into(),
                        cost_profile_version: 1,
                    },
                )
                .await?;

            // B guesses A's account id through the repo: NotFound, no fields.
            match repo_b.get(&actor_b, acc.id).await {
                Err(TenancyError::NotFound) => {}
                other => panic!("id guess must be NotFound, got {other:?}"),
            }
            // Raw SQL as B: zero rows, zero foreign fields.
            let raw: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE id = $1")
                .bind(acc.id)
                .fetch_one(&pool_b)
                .await
                .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(raw, 0, "direct id guess must return zero rows");
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 5. Table-owner / BYPASSRLS hazard: connecting AS the table owner cannot
//    bypass RLS (FORCE RLS, no owner policies).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_table_owner_connection_cannot_bypass_rls() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("table owner cannot bypass RLS", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-owner-haz-a").await?;
            let actor_a = Actor::member(a_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let repo_a = AccountRepo::new(pool_a.clone());
            repo_a
                .create(
                    &actor_a,
                    NewAccount {
                        account_type: "PAPER".into(),
                        name: "a-account".into(),
                        currency: "KRW".into(),
                        initial_cash: Some("10000000".into()),
                        cost_profile_id: "KRX_ETF_DEFAULT".into(),
                        cost_profile_version: 1,
                    },
                )
                .await?;

            // Connect as migration_owner (the table owner) WITHOUT any actor
            // context: FORCE RLS + no owner policies => tenant rows invisible.
            let owner = connect_with_retry(&conn_url(s, "migration_owner", d), 4).await?;
            let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
                .fetch_one(&owner)
                .await
                .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(
                visible, 0,
                "table owner without actor context must see 0 tenant rows"
            );

            // INSERT as the table owner without actor context: 42501.
            let insert = sqlx::query(
                "INSERT INTO accounts (owner_user_id, account_type, name) \
                 VALUES ($1, 'PAPER', 'owner-bypass')",
            )
            .bind(a_id)
            .execute(&owner)
            .await
            .unwrap_err();
            assert_eq!(
                pg_code(&insert).as_deref(),
                Some("42501"),
                "table owner INSERT must be denied by forced RLS, got {insert}"
            );

            // Serving roles never hold BYPASSRLS.
            for role in ["app", "worker", "audit_writer", "admin"] {
                let bypass: bool =
                    sqlx::query_scalar("SELECT rolbypassrls FROM pg_roles WHERE rolname = $1")
                        .bind(role)
                        .fetch_one(pool)
                        .await
                        .map_err(Box::<dyn std::error::Error>::from)?;
                assert!(!bypass, "serving role {role} must not have BYPASSRLS");
            }
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 6. Migration/table owner is never the serving role.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_migration_owner_is_not_the_serving_role() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("owner != serving role", &super_url, |_s, _d, pool| {
        Box::pin(async move {
            let owners: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT tableowner FROM pg_tables WHERE schemaname = 'public'",
            )
            .fetch_all(pool)
            .await
            .map_err(Box::<dyn std::error::Error>::from)?;
            assert!(
                owners.iter().all(|o| o == "migration_owner"),
                "every table must be owned by migration_owner, got {owners:?}"
            );
            for role in ["app", "worker", "audit_writer", "admin"] {
                assert!(
                    !owners.contains(&role.to_string()),
                    "serving role {role} must never own a table"
                );
            }
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 7. Shared dataset/factor data is read-only for the app role.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_shared_data_is_read_only() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("shared data read-only", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-shared-a").await?;
            sqlx::query(
                "INSERT INTO instruments (id, symbol, venue, currency) \
                 VALUES ('069500.KRX', '069500', 'KRX', 'KRW')",
            )
            .execute(pool)
            .await
            .map_err(Box::<dyn std::error::Error>::from)?;
            let actor_a = Actor::member(a_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let shared = SharedDataRepo::new(pool_a.clone());

            // Read path works for every member.
            let inst = shared.get_instrument(&actor_a, "069500.KRX").await?;
            assert_eq!(inst.currency, "KRW");
            assert!(
                shared.list_factor_manifests(&actor_a).await?.is_empty(),
                "no manifests seeded"
            );

            // Mutations are denied by grants + RLS (no DML policies).
            for (sql, label) in [
                (
                    "INSERT INTO instruments (id, symbol, venue, currency) \
                     VALUES ('111111.KRX', '111111', 'KRX', 'KRW')",
                    "INSERT instruments",
                ),
                (
                    "UPDATE instruments SET status = 'DELISTED' WHERE id = '069500.KRX'",
                    "UPDATE instruments",
                ),
                (
                    "DELETE FROM instruments WHERE id = '069500.KRX'",
                    "DELETE instruments",
                ),
                (
                    "INSERT INTO dataset_versions (dataset_id, version, storage_path) \
                     VALUES ('krx_eod_bars', '1.0.0', 'data/curated/x')",
                    "INSERT dataset_versions",
                ),
                (
                    "UPDATE dataset_versions SET status = 'BLOCKED'",
                    "UPDATE dataset_versions",
                ),
                ("DELETE FROM dataset_versions", "DELETE dataset_versions"),
            ] {
                let err = sqlx::query(sql)
                    .execute(&pool_a)
                    .await
                    .expect_err(&format!("{label} must be denied for app"));
                assert_eq!(
                    pg_code(&err).as_deref(),
                    Some("42501"),
                    "{label} must fail with 42501, got {err}"
                );
            }
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 8. audit_logs is append-only: UPDATE/DELETE/TRUNCATE fail for every serving
//    role; only audit_writer can INSERT.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_audit_is_append_only() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("audit append-only", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-audit-a").await?;
            let aw = connect_with_retry(&conn_url(s, "audit_writer", d), 4).await?;
            let inserted = sqlx::query(
                "INSERT INTO audit_logs (action, actor_role, actor_user_id, reason) \
                 VALUES ('qa.seed', 'system', $1, 'append-only probe')",
            )
            .bind(a_id)
            .execute(&aw)
            .await
            .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(inserted.rows_affected(), 1, "audit_writer must append");

            for role in ["app", "worker", "audit_writer"] {
                let pool_r = connect_with_retry(&conn_url(s, role, d), 4).await?;
                let upd = sqlx::query("UPDATE audit_logs SET reason = 'tampered'")
                    .execute(&pool_r)
                    .await
                    .unwrap_err();
                assert_eq!(
                    pg_code(&upd).as_deref(),
                    Some("42501"),
                    "{role} UPDATE audit must be denied, got {upd}"
                );
                let del = sqlx::query("DELETE FROM audit_logs")
                    .execute(&pool_r)
                    .await
                    .unwrap_err();
                assert_eq!(
                    pg_code(&del).as_deref(),
                    Some("42501"),
                    "{role} DELETE audit must be denied, got {del}"
                );
                let tr = sqlx::query("TRUNCATE TABLE audit_logs")
                    .execute(&pool_r)
                    .await
                    .unwrap_err();
                assert_eq!(
                    pg_code(&tr).as_deref(),
                    Some("42501"),
                    "{role} TRUNCATE audit must be denied, got {tr}"
                );
                if role != "audit_writer" {
                    let ins = sqlx::query("INSERT INTO audit_logs (action) VALUES ('probe')")
                        .execute(&pool_r)
                        .await
                        .unwrap_err();
                    assert_eq!(
                        pg_code(&ins).as_deref(),
                        Some("42501"),
                        "{role} INSERT audit must be denied, got {ins}"
                    );
                }
            }
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 9. Admin pathway: explicit Owner-only, cross-user, audited (success AND
//    denial), and Members are denied.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_admin_pathway_is_explicit_and_audited() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("admin pathway audited", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-admin-a").await?;
            let b_id = insert_test_user(pool, "ten-admin-b").await?;
            let owner_id = insert_test_user(pool, "ten-admin-owner").await?;
            let actor_owner = Actor::owner(owner_id.to_string());
            let actor_a = Actor::member(a_id.to_string());

            // A and B each submit a job (queue path, app role, own rows).
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;
            for (p, owner) in [(&pool_a, a_id), (&pool_b, b_id)] {
                sqlx::query(
                    "INSERT INTO jobs (owner_user_id, job_type, status, max_attempts) \
                     VALUES ($1, 'backtest', 'QUEUED', 3)",
                )
                .bind(owner)
                .execute(p)
                .await
                .map_err(Box::<dyn std::error::Error>::from)?;
            }

            let admin_pool = connect_with_retry(&conn_url(s, "admin", d), 4).await?;
            let audit_pool = connect_with_retry(&conn_url(s, "audit_writer", d), 4).await?;
            let admin = AdminRepo::new(admin_pool.clone(), AuditWriter::new(audit_pool.clone()));

            // Member is denied; the denial is itself audited.
            match admin.list_all_jobs(&actor_a, "corr-member-denied").await {
                Err(TenancyError::Forbidden) => {}
                other => panic!("Member admin call must be Forbidden, got {other:?}"),
            }
            // Owner sees BOTH users' jobs (cross-user queue view).
            let jobs = admin.list_all_jobs(&actor_owner, "corr-owner-view").await?;
            assert_eq!(jobs.len(), 2, "Owner must see both jobs, got {jobs:?}");
            let owners: Vec<Uuid> = jobs.iter().map(|j| j.owner_user_id).collect();
            assert!(owners.contains(&a_id) && owners.contains(&b_id));

            // Audit rows: exactly two admin.jobs.list_all entries, the Owner
            // success and the Member denial, with actor/time/target/reason/
            // correlation captured.
            type AuditRow = (
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                String,
            );
            let rows: Vec<AuditRow> = sqlx::query_as(
                "SELECT action, actor_role, target_type, target_id, reason, \
                            correlation_id, created_at::text \
                     FROM audit_logs WHERE action = 'admin.jobs.list_all' ORDER BY created_at",
            )
            .fetch_all(pool)
            .await
            .map_err(Box::<dyn std::error::Error>::from)?;
            assert_eq!(
                rows.len(),
                2,
                "success + denial must both be audited: {rows:?}"
            );
            assert_eq!(rows[0].1, "member", "denial audited with member role");
            assert_eq!(
                rows[0].4.as_deref(),
                Some("FORBIDDEN_MEMBER"),
                "denial must carry the reason"
            );
            assert_eq!(rows[0].5.as_deref(), Some("corr-member-denied"));
            assert_eq!(rows[1].1, "owner", "success audited with owner role");
            assert_eq!(rows[1].2, "job");
            assert_eq!(rows[1].3, "all");
            assert_eq!(rows[1].5.as_deref(), Some("corr-owner-view"));
            assert!(
                !rows[0].6.is_empty() && !rows[1].6.is_empty(),
                "created_at must be captured for both rows"
            );
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 10. QA happy path: two users independently create configs / runs / accounts
//     and retrieve ONLY their own rows.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenancy_two_users_isolated_happy_path() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("two users isolated", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-happy-a").await?;
            let b_id = insert_test_user(pool, "ten-happy-b").await?;
            let actor_a = Actor::member(a_id.to_string());
            let actor_b = Actor::member(b_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;

            let cfg = StrategyConfigRepo::new(pool_a.clone());
            let cfg_b = StrategyConfigRepo::new(pool_b.clone());
            let acc = AccountRepo::new(pool_a.clone());
            let acc_b = AccountRepo::new(pool_b.clone());
            let runs = BacktestRunRepo::new(pool_a.clone());
            let runs_b = BacktestRunRepo::new(pool_b.clone());

            for (i, (actor, cfg_r, acc_r, runs_r)) in [
                (&actor_a, &cfg, &acc, &runs),
                (&actor_b, &cfg_b, &acc_b, &runs_b),
            ]
            .iter()
            .enumerate()
            {
                cfg_r
                    .create(
                        actor,
                        NewStrategyConfig {
                            strategy_id: "ma200-trend".into(),
                            strategy_version: "1.0.0".into(),
                            config_json: serde_json::json!({"user": i}),
                            is_active: true,
                        },
                    )
                    .await?;
                acc_r
                    .create(
                        actor,
                        NewAccount {
                            account_type: "PAPER".into(),
                            name: format!("acc-{i}"),
                            currency: "KRW".into(),
                            initial_cash: Some("10000000".into()),
                            cost_profile_id: "KRX_ETF_DEFAULT".into(),
                            cost_profile_version: 1,
                        },
                    )
                    .await?;
                runs_r
                    .create(
                        actor,
                        NewBacktestRun {
                            strategy_id: "ma200-trend".into(),
                            strategy_version: "1.0.0".into(),
                            dataset_version: "d1".into(),
                            engine_version: "1.231.0".into(),
                            config_sha256: "a".repeat(64),
                            code_commit: "0123456789abcdef0123456789abcdef01234567".into(),
                            random_seed: Some(42),
                            timezone: "Asia/Seoul".into(),
                            summary_json: serde_json::json!({}),
                        },
                    )
                    .await?;
            }

            assert_eq!(
                cfg.list(&actor_a).await?.len(),
                1,
                "A sees exactly 1 config"
            );
            assert_eq!(
                cfg_b.list(&actor_b).await?.len(),
                1,
                "B sees exactly 1 config"
            );
            assert_eq!(
                acc.list(&actor_a).await?.len(),
                1,
                "A sees exactly 1 account"
            );
            assert_eq!(
                acc_b.list(&actor_b).await?.len(),
                1,
                "B sees exactly 1 account"
            );
            assert_eq!(runs.list(&actor_a).await?.len(), 1, "A sees exactly 1 run");
            assert_eq!(
                runs_b.list(&actor_b).await?.len(),
                1,
                "B sees exactly 1 run"
            );

            // Cross-check: every listed row belongs to the acting user only.
            for row in cfg.list(&actor_a).await? {
                assert_eq!(row.owner_user_id, a_id);
            }
            for row in acc_b.list(&actor_b).await? {
                assert_eq!(row.owner_user_id, b_id);
            }
            Ok(())
        })
    })
    .await;
}

// ---------------------------------------------------------------------------
// 11. Artifact ownership: worker-inserted artifacts are reachable only
//     through an owned parent run (replay / id guess denied).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn tenancy_artifact_ownership_check() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_tenancy("artifact ownership", &super_url, |s, d, pool| {
        Box::pin(async move {
            let a_id = insert_test_user(pool, "ten-art-a").await?;
            let b_id = insert_test_user(pool, "ten-art-b").await?;
            let actor_a = Actor::member(a_id.to_string());
            let actor_b = Actor::member(b_id.to_string());
            let pool_a = actor_pool(s, d, "app", &a_id.to_string()).await?;
            let pool_b = actor_pool(s, d, "app", &b_id.to_string()).await?;
            let runs = BacktestRunRepo::new(pool_a.clone());

            // A creates a run; the WORKER stores its artifact (owner bound to
            // the run's owner, per manifest writer).
            let run = runs
                .create(
                    &actor_a,
                    NewBacktestRun {
                        strategy_id: "ma200-trend".into(),
                        strategy_version: "1.0.0".into(),
                        dataset_version: "d1".into(),
                        engine_version: "1.231.0".into(),
                        config_sha256: "a".repeat(64),
                        code_commit: "0123456789abcdef0123456789abcdef01234567".into(),
                        random_seed: None,
                        timezone: "Asia/Seoul".into(),
                        summary_json: serde_json::json!({}),
                    },
                )
                .await?;
            let worker_pool = connect_with_retry(&conn_url(s, "worker", d), 4).await?;
            let artifact_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO result_artifacts \
                 (backtest_run_id, owner_user_id, artifact_type, parquet_path, \
                  row_count, sha256, size_bytes) \
                 VALUES ($1, $2, 'EQUITY_CURVE', 'data/artifacts/a.parquet', 10, $3, 42) \
                 RETURNING id",
            )
            .bind(run.id)
            .bind(a_id)
            .bind("b".repeat(64))
            .fetch_one(&worker_pool)
            .await
            .map_err(Box::<dyn std::error::Error>::from)?;

            let artifacts = ArtifactRepo::new(pool_a.clone());
            let artifacts_b = ArtifactRepo::new(pool_b.clone());
            // Owner A can read the artifact through the run ownership.
            let seen = artifacts.get_owned(&actor_a, artifact_id).await?;
            assert_eq!(seen.sha256, "b".repeat(64));
            // Member B (replay/id guess) cannot.
            match artifacts_b.get_owned(&actor_b, artifact_id).await {
                Err(TenancyError::NotFound) => {}
                other => panic!("B must not read A's artifact, got {other:?}"),
            }
            Ok(())
        })
    })
    .await;
}
