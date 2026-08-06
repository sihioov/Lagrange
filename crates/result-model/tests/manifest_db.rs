//! Red tests (Todo 20): the T3 backtest-result DB manifest writer.
//!
//! Proves, against a disposable PostgreSQL 18 cluster (migration 0006
//! `backtest_runs` / `backtest_metrics` / `backtest_warnings` /
//! `result_artifacts`), that the worker's manifest lands as rows, that writes
//! are idempotent (a retried job never duplicates), that non-finite metrics
//! and malformed artifacts are rejected, and that the written run can be read
//! back. The harness follows the Todo 19 conventions: per-test scratch
//! database, role bootstrap, embedded migrations, whole-test retries; hosts
//! without `DATABASE_URL` skip cleanly.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use result_model::manifest::{ArtifactManifest, ArtifactType, BacktestManifest, ManifestWriter, RunManifest};
use result_model::{Warning, WarningSeverity};
use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::types::Uuid;

/// Migrations embedded at compile time from the workspace `migrations/` dir.
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
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'audit_writer') THEN
    CREATE ROLE audit_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD 'lagrange';
  END IF;
END $role$;
GRANT USAGE ON SCHEMA public TO migration_owner, app, worker, audit_writer;
GRANT CREATE ON SCHEMA public TO migration_owner;
"#;

fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

fn fresh_db_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis();
    format!("btk_{}_{}_{}", std::process::id(), ts, SEQ.fetch_add(1, Ordering::Relaxed))
}

fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (_scheme, rest) = super_url.split_once("://").expect("DATABASE_URL must start with a scheme");
    let (auth, hostport_db) = rest.split_once('@').expect("DATABASE_URL must contain @");
    let pw = auth.split_once(':').map(|(_, p)| p);
    let (hostport, _old_db) = hostport_db.rsplit_once('/').expect("DATABASE_URL must contain a database path");
    match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    }
}

fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.migration_type != MigrationType::ReversibleDown)
        .count()
}

async fn connect_with_retry(url: &str, max_conns: u32) -> Result<PgPool, Box<dyn Error>> {
    let mut opts: sqlx::postgres::PgConnectOptions = url.parse()?;
    opts = opts.options([("statement_timeout", "20s")]);
    let mut last: Option<sqlx::Error> = None;
    for attempt in 0..6u32 {
        let connect = PgPoolOptions::new()
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
        match sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&pool).await {
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
        .unwrap_or_else(|| "connect_with_retry exhausted attempts".into()))
}

async fn create_scratch_db(super_url: &str) -> Result<(String, PgPool), Box<dyn Error>> {
    let db = fresh_db_name();
    let admin = connect_with_retry(super_url, 3).await?;
    sqlx::query(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    sqlx::query(ddl_for(&db, "CREATE DATABASE {db}"))
        .execute(&admin)
        .await?;
    drop(admin);

    let super_new = connect_with_retry(&conn_url(super_url, "postgres", &db), 3).await?;
    sqlx::raw_sql(ROLE_BOOTSTRAP_SQL).execute(&super_new).await?;
    sqlx::raw_sql(ddl_for(
        &db,
        "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer",
    ))
    .execute(&super_new)
    .await?;
    drop(super_new);

    let pool = connect_with_retry(&conn_url(super_url, "postgres", &db), 6).await?;
    MIGRATOR.run(&pool).await?;
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(applied as usize, up_migration_count(), "all migrations applied");
    Ok((db, pool))
}

async fn drop_scratch_db(super_url: &str, db: &str) -> Result<(), Box<dyn Error>> {
    let admin = connect_with_retry(super_url, 3).await?;
    sqlx::query(ddl_for(db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    Ok(())
}

async fn insert_test_user(pool: &PgPool, subject: &str) -> Result<Uuid, Box<dyn Error>> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) VALUES ('test-issuer', $1, $2) RETURNING id",
    )
    .bind(subject)
    .bind(format!("{subject}@example.com"))
    .fetch_one(pool)
    .await?;
    Ok(id)
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

async fn run_test<F>(label: &str, super_url: &str, body: F)
where
    F: for<'a> Fn(&'a str, &'a str, &'a PgPool) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Box<dyn Error>>> + 'a>>,
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
            Ok(()) => return,
            Err(e) => {
                last = Some(format!("{e}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        eprintln!("{label}: attempt {} failed ({last:?}); retrying with a fresh DB", attempt + 1);
    }
    panic!("{label} FAILED after 3 attempts: {}", last.unwrap_or_default());
}

fn sample_manifest(run_id: Uuid, owner: Uuid, job_id: Uuid, seed: u64) -> BacktestManifest {
    let mut metrics = BTreeMap::new();
    metrics.insert("total_return".to_owned(), domain::ReportedStat::from_f64(0.010_609_6).unwrap());
    metrics.insert("max_drawdown".to_owned(), domain::ReportedStat::from_f64(0.0).unwrap());
    BacktestManifest {
        run: RunManifest {
            id: run_id,
            owner_user_id: owner,
            job_id: Some(job_id),
            strategy_id: "ma200-trend".to_owned(),
            strategy_version: "1.0.0".to_owned(),
            dataset_version: "kr-etf-daily-phase0-v1".to_owned(),
            engine: "nautilustrader".to_owned(),
            engine_version: "1.231.0".to_owned(),
            config_sha256: "a".repeat(64),
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            random_seed: Some(seed as i64),
            timezone: "Asia/Seoul".to_owned(),
            status: "SUCCEEDED".to_owned(),
            summary_json: serde_json::json!({"initial_equity": "100000000.0000"}),
        },
        metrics,
        warnings: vec![Warning::new(
            "example_warning",
            "synthetic fixture",
            WarningSeverity::Warning,
        )],
        artifacts: vec![
            ArtifactManifest {
                artifact_type: ArtifactType::EquityCurve,
                parquet_path: "equity.parquet".to_owned(),
                row_count: 260,
                sha256: "b".repeat(64),
                size_bytes: 4096,
                summary_json: serde_json::json!({}),
            },
            ArtifactManifest {
                artifact_type: ArtifactType::Orders,
                parquet_path: "orders.parquet".to_owned(),
                row_count: 6,
                sha256: "c".repeat(64),
                size_bytes: 512,
                summary_json: serde_json::json!({}),
            },
        ],
    }
}

#[tokio::test]
async fn manifest_write_persists_rows_across_all_tables() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("persist", &super_url, |super_url, db, pool| {
        Box::pin(async move {
            let owner = insert_test_user(pool, "persist-owner").await?;
            let job: Uuid = Uuid::new_v4();
            let run_id: Uuid = Uuid::new_v4();
            let manifest = sample_manifest(run_id, owner, job, 42);
            let worker_pool = connect_with_retry(&conn_url(super_url, "worker", db), 4).await?;
            let writer = ManifestWriter::new(worker_pool.clone());
            let report = writer.write(&manifest).await?;
            assert!(report.inserted, "first write must insert");

            let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(runs, 1);
            let status: String = sqlx::query_scalar("SELECT status FROM backtest_runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(status, "SUCCEEDED");
            let strategy: String = sqlx::query_scalar("SELECT strategy_id FROM backtest_runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(strategy, "ma200-trend");

            let metric_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_metrics WHERE backtest_run_id = $1")
                .bind(run_id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(metric_rows, 2);
            let metric_value: f64 = sqlx::query_scalar(
                "SELECT metric_value::float8 FROM backtest_metrics WHERE backtest_run_id = $1 AND metric_key = 'total_return'",
            )
            .bind(run_id)
            .fetch_one(&worker_pool)
            .await?;
            assert!((metric_value - 0.010_609_6).abs() < 1e-9, "metric value round-trips: {metric_value}");

            let warning_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_warnings WHERE backtest_run_id = $1")
                .bind(run_id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(warning_rows, 1);

            let artifact_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM result_artifacts WHERE backtest_run_id = $1")
                .bind(run_id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(artifact_rows, 2);
            let sha: String = sqlx::query_scalar(
                "SELECT sha256 FROM result_artifacts WHERE backtest_run_id = $1 AND artifact_type = 'EQUITY_CURVE'",
            )
            .bind(run_id)
            .fetch_one(&worker_pool)
            .await?;
            assert_eq!(sha, "b".repeat(64));
            Ok(())
        })
    })
    .await;
}

#[tokio::test]
async fn manifest_write_is_idempotent() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("idempotent", &super_url, |super_url, db, pool| {
        Box::pin(async move {
            let owner = insert_test_user(pool, "idem-owner").await?;
            let manifest = sample_manifest(Uuid::new_v4(), owner, Uuid::new_v4(), 42);
            let worker_pool = connect_with_retry(&conn_url(super_url, "worker", db), 4).await?;
            let writer = ManifestWriter::new(worker_pool.clone());

            let first = writer.write(&manifest).await?;
            assert!(first.inserted);
            let second = writer.write(&manifest).await?;
            assert!(!second.inserted, "a retried job must not duplicate the manifest");

            let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM backtest_runs WHERE id = $1")
                .bind(manifest.run.id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(runs, 1);
            let artifacts: i64 = sqlx::query_scalar("SELECT count(*) FROM result_artifacts WHERE backtest_run_id = $1")
                .bind(manifest.run.id)
                .fetch_one(&worker_pool)
                .await?;
            assert_eq!(artifacts, 2);
            Ok(())
        })
    })
    .await;
}

#[tokio::test]
async fn manifest_write_rejects_non_finite_metric_at_the_json_boundary() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("nonfinite", &super_url, |_super_url, _db, _pool| {
        Box::pin(async move {
            let json = r#"{"run":{"id":"123e4567-e89b-12d3-a456-426614174000","owner_user_id":"123e4567-e89b-12d3-a456-426614174001","job_id":null,"strategy_id":"ma200-trend","strategy_version":"1.0.0","dataset_version":"d","engine":"nautilustrader","engine_version":"1.231.0","config_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","code_commit":"0123456789abcdef0123456789abcdef01234567","random_seed":42,"timezone":"Asia/Seoul","status":"SUCCEEDED","summary_json":{}},"metrics":{"total_return":1e999},"warnings":[],"artifacts":[]}"#;
            let err = serde_json::from_str::<BacktestManifest>(json);
            assert!(err.is_err(), "1e999 metric must be rejected at the manifest boundary");
            Ok(())
        })
    })
    .await;
}

#[tokio::test]
async fn manifest_write_rejects_malformed_artifacts_without_touching_the_db() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    run_test("malformed", &super_url, |_super_url, _db, pool| {
        Box::pin(async move {
            let owner = insert_test_user(pool, "mal-owner").await?;
            let mut manifest = sample_manifest(Uuid::new_v4(), owner, Uuid::new_v4(), 42);
            manifest.artifacts[0].sha256 = "not-hex".to_owned();
            let writer = ManifestWriter::new(pool.clone());
            let err = writer.write(&manifest).await;
            assert!(err.is_err(), "an invalid artifact sha256 must be rejected before insert");
            Ok(())
        })
    })
    .await;
}
