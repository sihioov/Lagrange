//! The test that was missing.
//!
//! Every component on the backtest path had its own passing suite while the
//! path itself was broken, because no test ever asked the only question a user
//! asks: **I submitted a backtest — did it run?**
//!
//! That is the question here. A job goes into the queue the way the HTTP route
//! puts it there, the runner is invoked the way a daemon would invoke it, and
//! the assertions are about the job's final state and the artifacts on disk —
//! never about the runner's internals. A future refactor that keeps every unit
//! test green and breaks the wiring again fails HERE.

mod common;

use common::ScratchDb;
use job_queue::error::QueueError;
use job_queue::phase0::DATASET_ID;
use job_queue::queue::{JobQueue, QueueConfig};
use job_queue::runner::{
    Outcome, ResolveError, ResolvedStrategy, RunnerPaths, StrategyResolver, run_once,
};
use job_queue::types::{JobStatus, SubmitJob};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

// Every test below provisions cluster-global roles and several of them launch
// a memory-heavy NautilusTrader child. Running this one integration binary at
// the default test-harness parallelism races role bootstrap on a fresh
// PostgreSQL cluster and can exhaust a small CI runner before a child writes
// its terminal status. Product workers remain concurrent; only these
// disposable end-to-end environments are serialized.
static BACKTEST_RUNNER_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn serial_backtest_environment() -> tokio::sync::MutexGuard<'static, ()> {
    BACKTEST_RUNNER_TEST.lock().await
}

/// Resolves every config id to the phase-0 golden strategy.
///
/// A stand-in for the real registry lookup, and deliberately still a LOOKUP:
/// it ignores what the payload says and returns a fixed entry. A resolver that
/// echoed a caller-supplied module path back would turn a backtest submission
/// into arbitrary code execution, so the double must not model that shape even
/// for convenience.
struct GoldenResolver;

impl StrategyResolver for GoldenResolver {
    async fn resolve(&self, _id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        Ok(ResolvedStrategy {
            strategy_path: "ma200_trend:MA200Trend".into(),
            config_path: "ma200_trend:MA200TrendConfig".into(),
            strategy_id: "ma200_trend".into(),
            strategy_version: "1.0.0".into(),
            config: serde_json::json!({
                "ma_period": 200,
                "slippage_bps": 10,
                "lot_size": 100,
                "initial_cash": "100000000",
                "strategy_version": "1.0.0",
                "probe_future_fields": false,
            }),
        })
    }
}

/// A resolver whose registry is DOWN, for the "the runner could not run it"
/// path. Distinct from one that says no such config: an outage must requeue.
struct BrokenResolver;

impl StrategyResolver for BrokenResolver {
    async fn resolve(&self, _id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        Err(ResolveError::Unavailable(
            "strategy registry is unavailable".into(),
        ))
    }
}

/// Holds the runner in pre-child strategy resolution long enough that an
/// unmonitored short lease would expire and a second worker could claim it.
struct SlowResolver;

impl StrategyResolver for SlowResolver {
    async fn resolve(&self, _id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Err(ResolveError::Unavailable(
            "slow strategy registry is unavailable".into(),
        ))
    }
}

/// A resolver that answers, definitively, that the config does not exist.
struct MissingConfigResolver;

impl StrategyResolver for MissingConfigResolver {
    async fn resolve(&self, id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        Err(ResolveError::NotFound(format!("no strategy config {id}")))
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

/// Locates `uv` for the test process.
///
/// The test binary does not inherit a shell's PATH, so `uv` alone resolves to
/// "program not found" -- which the runner reports honestly and which would
/// otherwise look like a broken worker rather than a missing tool.
fn uv_bin() -> PathBuf {
    if let Some(explicit) = std::env::var_os("LAGRANGE_UV_BIN") {
        return PathBuf::from(explicit);
    }
    let candidates = [
        dirs_home().join("AppData/Local/Programs/Python/Python311/Scripts/uv.exe"),
        dirs_home().join(".cargo/bin/uv.exe"),
        dirs_home().join(".local/bin/uv"),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("uv"))
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn paths(scratch: &tempfile::TempDir) -> RunnerPaths {
    let root = repo_root();
    RunnerPaths {
        dataset_root: root.join("data/phase0"),
        repo_root: root,
        artifacts_root: scratch.path().to_path_buf(),
        uv_bin: uv_bin(),
        code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
    }
}

/// Seeds an owner the `jobs` foreign key will accept.
async fn seed_owner(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (id, issuer, subject, email) \
         VALUES (gen_random_uuid(), 'runner-iss', $1, $2) RETURNING id",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(format!("runner-{}@lagrange.test", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("seed owner")
}

/// Submits a job shaped exactly as `POST /api/v1/backtests` writes it, naming
/// a config id the caller controls.
async fn submit_backtest_for(
    queue: &JobQueue,
    owner: Uuid,
    config_id: Uuid,
) -> Result<Uuid, QueueError> {
    let job = queue
        .submit(SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".into(),
            payload: serde_json::json!({
                "kind": "backtest",
                "run_id": Uuid::new_v4(),
                "strategy_config_id": config_id,
                "dataset_version_id": DATASET_ID,
                "start_date": "2020-01-01",
                "end_date": "2020-12-31",
                "initial_cash": { "currency": "KRW", "amount": "100000000" },
                "benchmark": "069500.KRX",
                "cost_profile_id": "KRX_ETF_DEFAULT",
                "execution_profile": "next_open",
            }),
            priority: 10,
            idempotency_key: Some(Uuid::new_v4().to_string()),
            max_attempts: 3,
            available_at: None,
        })
        .await?;
    seed_run_for_job(
        &queue.pool(),
        job.id,
        owner,
        "ma200_trend",
        serde_json::json!({
            "ma_period": 200,
            "slippage_bps": 10,
            "lot_size": 100,
            "initial_cash": "100000000",
            "strategy_version": "1.0.0",
            "probe_future_fields": false,
        }),
    )
    .await;
    Ok(job.id)
}

async fn seed_run_for_job(
    pool: &PgPool,
    job_id: Uuid,
    owner: Uuid,
    strategy_id: &str,
    config: serde_json::Value,
) -> Uuid {
    sqlx::query(
        "INSERT INTO strategies (id, display_name) VALUES ($1, $1)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(strategy_id)
    .execute(pool)
    .await
    .expect("seed strategy");
    let run_id: Uuid =
        sqlx::query_scalar("SELECT (payload_json->>'run_id')::uuid FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(pool)
            .await
            .expect("run id in payload");
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(config.to_string().as_bytes());
    let config_sha256 = format!("{:x}", digest.finalize());
    sqlx::query(
        "INSERT INTO backtest_runs
            (id, owner_user_id, job_id, strategy_id, strategy_version,
             dataset_version, engine, engine_version, config_sha256, code_commit,
             random_seed, timezone, status, summary_json)
         VALUES ($1, $2, $3, $4, '1.0.0',
                 'kr-etf-daily-phase0-v2@2026-01', 'nautilustrader', '1.231.0',
                 $5, '0123456789abcdef0123456789abcdef01234567', 42,
                 'Asia/Seoul', 'PENDING', '{}')
         ON CONFLICT (id) DO UPDATE SET
             owner_user_id = EXCLUDED.owner_user_id,
             job_id = EXCLUDED.job_id,
             strategy_id = EXCLUDED.strategy_id,
             strategy_version = EXCLUDED.strategy_version,
             config_sha256 = EXCLUDED.config_sha256,
             status = 'PENDING',
             summary_json = '{}'::jsonb,
             started_at = NULL,
             finished_at = NULL",
    )
    .bind(run_id)
    .bind(owner)
    .bind(job_id)
    .bind(strategy_id)
    .bind(config_sha256)
    .execute(pool)
    .await
    .expect("seed backtest run");
    run_id
}

/// The common case: the config id does not matter because the test's resolver
/// ignores it.
async fn submit_backtest(queue: &JobQueue, owner: Uuid) -> Result<Uuid, QueueError> {
    submit_backtest_for(queue, owner, Uuid::new_v4()).await
}

fn walk(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

#[tokio::test]
async fn a_submitted_backtest_actually_runs_and_finishes() {
    let _serial = serial_backtest_environment().await;
    // THE test. Submit the way the route does; run the way a daemon does;
    // assert on the job's own final state and the artifacts on disk.
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    // Before the runner: QUEUED and untouched. This is the state the system
    // was stuck in -- a job nobody would ever pick up.
    assert_eq!(
        queue.get_by_id(job_id).await.expect("get").status,
        JobStatus::Queued
    );

    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &GoldenResolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Succeeded { .. }),
        "the backtest must actually run: {outcome:?}"
    );

    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(after.status, JobStatus::Succeeded);
    assert!(after.finished_at.is_some(), "a finished job records when");

    // Artifacts on disk, not merely a status flag. A runner that flipped the
    // row to SUCCEEDED without producing anything would pass every assertion
    // above and none of these.
    let produced = walk(scratch.path());
    for expected in ["equity.parquet", "orders.parquet", "fills.parquet"] {
        assert!(
            produced.iter().any(|p| p.ends_with(expected)),
            "{expected} missing from {produced:?}"
        );
    }

    let run_id: Uuid =
        sqlx::query_scalar("SELECT (payload_json->>'run_id')::uuid FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&db.pool)
            .await
            .expect("run id");
    let run_dir = scratch.path().join(run_id.to_string());
    let generation_root = run_dir.join("generations");
    assert!(
        generation_root.is_dir(),
        "immutable generation root must exist"
    );
    assert!(
        walk(&generation_root)
            .iter()
            .any(|path| path.ends_with("artifacts/manifest.json")),
        "validated manifest must be promoted to an immutable generation"
    );
    assert!(
        !walk(scratch.path())
            .iter()
            .any(|path| path.ends_with("status.json")),
        "attempt status must be cleaned rather than left as a canonical fallback"
    );
    let published_paths: Vec<String> =
        sqlx::query_scalar(
            "SELECT parquet_path FROM result_artifacts WHERE backtest_run_id = $1 ORDER BY parquet_path",
        )
            .bind(run_id)
            .fetch_all(&db.pool)
            .await
            .expect("published artifact paths");
    assert!(!published_paths.is_empty());
    let prefix = format!("backtest/runs/{run_id}/generations/");
    assert!(
        published_paths
            .iter()
            .all(|path| { path.starts_with(&prefix) && path.matches('/').count() == 7 })
    );
    assert!(
        published_paths.iter().all(|path| {
            let relative = path
                .strip_prefix(&format!("backtest/runs/{run_id}/"))
                .expect("path prefix");
            run_dir.join(relative).is_file()
        }),
        "every DB path must resolve to the immutable bytes"
    );

    // Compatibility fence: the result transaction has committed, but a
    // pre-isolation replica may still be alive and repeatedly write the old
    // canonical names. Model that writer after settlement and prove the DB
    // path, bytes, and hash it references remain unchanged.
    let published_path = run_dir.join(
        published_paths[0]
            .strip_prefix(&format!("backtest/runs/{run_id}/"))
            .expect("published path prefix"),
    );
    let before_bytes = std::fs::read(&published_path).expect("published bytes");
    let before_hash = format!("{:x}", Sha256::digest(&before_bytes));
    let legacy_root = run_dir.clone();
    let legacy_writer = std::thread::spawn(move || {
        let canonical = legacy_root.join("artifacts");
        std::fs::create_dir_all(&canonical).expect("legacy canonical artifacts");
        for index in 0..128 {
            std::fs::write(
                canonical.join("equity.parquet"),
                format!("legacy overwrite {index}"),
            )
            .expect("legacy artifact write");
            std::fs::write(
                legacy_root.join("status.json"),
                format!(r#"{{"state":"SUCCEEDED","legacy_write":{index}}}"#),
            )
            .expect("legacy status write");
        }
    });
    legacy_writer.join().expect("legacy writer");
    let after_paths: Vec<String> = sqlx::query_scalar(
        "SELECT parquet_path FROM result_artifacts WHERE backtest_run_id = $1 ORDER BY parquet_path",
    )
    .bind(run_id)
    .fetch_all(&db.pool)
    .await
    .expect("published artifact paths after legacy writer");
    assert_eq!(after_paths, published_paths);
    let after_bytes = std::fs::read(&published_path).expect("immutable bytes after legacy writer");
    assert_eq!(after_bytes, before_bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&after_bytes)), before_hash);

    db.drop_db().await;
}

#[tokio::test]
async fn runner_leaves_higher_priority_recommendations_for_their_worker() {
    let _serial = serial_backtest_environment().await;
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;

    let recommendation = queue
        .submit(SubmitJob {
            owner_user_id: owner,
            job_type: "recommendation".into(),
            payload: serde_json::json!({ "kind": "recommendation" }),
            priority: 11,
            idempotency_key: Some(Uuid::new_v4().to_string()),
            max_attempts: 3,
            available_at: None,
        })
        .await
        .expect("submit recommendation");
    let backtest_id = submit_backtest(&queue, owner)
        .await
        .expect("submit backtest");

    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &GoldenResolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Succeeded { ref job_id } if job_id == &backtest_id.to_string()),
        "the runner must process the backtest, not the higher-priority recommendation: {outcome:?}"
    );

    let recommendation_after = queue
        .get_by_id(recommendation.id)
        .await
        .expect("get recommendation");
    assert_eq!(recommendation_after.status, JobStatus::Queued);
    assert_eq!(recommendation_after.attempt_count, 0);
    assert_eq!(
        queue
            .get_by_id(backtest_id)
            .await
            .expect("get backtest")
            .status,
        JobStatus::Succeeded
    );

    db.drop_db().await;
}

#[tokio::test]
async fn an_empty_queue_is_idle_rather_than_an_error() {
    let _serial = serial_backtest_environment().await;
    // The normal case most of the time. A runner that treated "nothing to do"
    // as a failure would fill its logs and its retry counters with noise.
    //
    // Its own database, so "empty" means empty rather than "whatever another
    // test happened to leave".
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());

    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &GoldenResolver)
        .await
        .expect("runner");
    assert_eq!(outcome, Outcome::Idle);

    db.drop_db().await;
}

#[tokio::test]
async fn a_runner_fault_requeues_the_job_instead_of_discarding_it() {
    let _serial = serial_backtest_environment().await;
    // A resolver outage is the RUNNER's problem, not the user's. The job must
    // come back QUEUED so a later attempt can succeed -- discarding it would
    // lose work that was never given a fair chance.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &BrokenResolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Errored { .. }),
        "a resolver outage is a runner error: {outcome:?}"
    );

    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(
        after.status,
        JobStatus::Queued,
        "a transient fault must requeue, not discard"
    );
    assert_eq!(after.attempt_count, 1, "the attempt is still counted");

    db.drop_db().await;
}

#[tokio::test]
async fn lease_monitor_holds_claim_during_slow_preprocessing() {
    let _serial = serial_backtest_environment().await;
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(
        db.pool.clone(),
        None,
        QueueConfig {
            lease: std::time::Duration::from_millis(100),
            backoff_base: std::time::Duration::from_millis(1),
        },
    );
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");
    let first_queue = queue.clone();
    let first_paths = paths(&scratch);
    let first = tokio::spawn(async move {
        run_once(&first_queue, "slow-worker-a", &first_paths, &SlowResolver).await
    });

    // The resolver is still running after three lease periods. The monitor
    // must retain the exact claim, so a second worker sees no eligible job.
    tokio::time::sleep(std::time::Duration::from_millis(220)).await;
    let second = queue
        .claim_next_for("slow-worker-b", "backtest")
        .await
        .expect("second claim query");
    assert!(
        second.is_none(),
        "slow preprocessing must keep the lease alive"
    );

    let outcome = first
        .await
        .expect("first worker join")
        .expect("first worker");
    assert!(matches!(outcome, Outcome::Errored { .. }), "{outcome:?}");
    assert_eq!(
        queue.get_by_id(job_id).await.expect("job").status,
        JobStatus::Queued
    );
    db.drop_db().await;
}

/// Seeds a real `user_strategy_configs` row for the deployed strategy.
async fn seed_strategy_config(pool: &PgPool, owner: Uuid) -> Uuid {
    sqlx::query("INSERT INTO strategies (id, display_name) VALUES ('ma200_trend', 'MA200 Trend')")
        .execute(pool)
        .await
        .expect("seed strategy");
    sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
           (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'ma200_trend', '1.0.0', $2) RETURNING id",
    )
    .bind(owner)
    .bind(serde_json::json!({
        "ma_period": 200,
        "slippage_bps": 10,
        "lot_size": 100,
        "initial_cash": "100000000",
        "strategy_version": "1.0.0",
        "probe_future_fields": false,
    }))
    .fetch_one(pool)
    .await
    .expect("seed config")
}

#[tokio::test]
async fn the_daemon_drains_a_real_job_under_the_worker_role() {
    let _serial = serial_backtest_environment().await;
    // The end of the chain, and the only test here that uses none of the
    // doubles: the real binary, the real `DbStrategyResolver`, and a
    // connection as `worker` rather than as superuser.
    //
    // The role is the point. `worker` was granted everything a backtest
    // WRITES in 0009 and not the one table it must READ, and nothing noticed
    // because nothing consumed the queue. Every test above would still pass
    // with that grant missing -- they connect as superuser, where a GRANT is
    // irrelevant -- and the runner would resolve nothing in production. This
    // test is what makes migration 0021 provable rather than asserted.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let config_id = seed_strategy_config(&db.pool, owner).await;
    let job_id = submit_backtest_for(&queue, owner, config_id)
        .await
        .expect("submit");

    let root = repo_root();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_backtest-runner"))
        .arg("--once")
        .env("APP_ENV", "qa")
        .env("DATABASE_URL", db.role_url("worker"))
        .env("LAGRANGE_REPO_ROOT", &root)
        .env("LAGRANGE_DATASET_ROOT", root.join("data/phase0"))
        .env("LAGRANGE_ARTIFACTS_ROOT", scratch.path())
        .env("LAGRANGE_UV_BIN", uv_bin())
        .env(
            "LAGRANGE_CODE_COMMIT",
            "0123456789abcdef0123456789abcdef01234567",
        )
        .output()
        .expect("the daemon binary runs");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "the daemon exited {}: {stderr}",
        output.status
    );

    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(
        after.status,
        JobStatus::Succeeded,
        "the daemon must finish the job it claimed: {stderr}"
    );

    // `locked_by` is what an operator reads when a job looks stuck, so it has
    // to name a process rather than say "a worker".
    let attempt_worker: String = sqlx::query_scalar(
        "SELECT claimed_by FROM job_attempts WHERE job_id = $1 ORDER BY attempt_no DESC LIMIT 1",
    )
    .bind(job_id)
    .fetch_one(&db.pool)
    .await
    .expect("attempt recorded");
    assert!(
        attempt_worker.starts_with("backtest-runner@"),
        "the attempt must name the process that ran it, got {attempt_worker:?}"
    );

    db.drop_db().await;
}

/// A resolver for one of the five user-facing baseline packages.
struct BaselineResolver {
    strategy_id: &'static str,
    strategy_path: &'static str,
    config_path: &'static str,
    /// Overrides the package defaults when a test needs a specific branch of
    /// the target generator; `None` uses whatever the adapter config
    /// defaults to, which is what a real user config would carry.
    parameters: Option<serde_json::Value>,
}

impl StrategyResolver for BaselineResolver {
    async fn resolve(&self, _id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        Ok(ResolvedStrategy {
            strategy_path: self.strategy_path.into(),
            config_path: self.config_path.into(),
            strategy_id: self.strategy_id.into(),
            strategy_version: "1.0.0".into(),
            config: self.config_value(),
        })
    }
}

impl BaselineResolver {
    fn config_value(&self) -> serde_json::Value {
        let mut config = serde_json::json!({
            "slippage_bps": 10,
            "lot_size": 100,
            "initial_cash": "100000000",
            "strategy_version": "1.0.0",
        });
        // `instrument_ids` is deliberately absent: the worker overwrites it
        // with what it found in the dataset, and a resolver that set it would
        // describe a universe the strategy will not actually see.
        if let Some(params) = &self.parameters {
            config["parameters"] = params.clone();
        }
        config
    }
}

/// The worker publishes an exact `row_count` in its validated manifest, so
/// the assertion reads that rather than inspecting parquet or guessing from
/// file size. Attempt status is intentionally cleaned with the attempt
/// namespace after settlement; the manifest is the durable result artifact.
fn manifest_of(scratch: &std::path::Path) -> Option<serde_json::Value> {
    let manifest_path = walk(scratch)
        .into_iter()
        .find(|p| p.ends_with("manifest.json"))?;
    let raw = std::fs::read_to_string(&manifest_path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// The normalized result the worker published, which carries fee AMOUNTS.
///
/// A fees artifact holding one row of zero is what the uncharged version
/// produced -- so anything asserting costs has to read the values.
fn normalized_result(scratch: &std::path::Path) -> Option<serde_json::Value> {
    let path = walk(scratch)
        .into_iter()
        .find(|p| p.ends_with("result.json"))?;
    serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()
}

fn reported_rows(scratch: &std::path::Path, artifact_type: &str) -> Option<u64> {
    let manifest = manifest_of(scratch)?;
    manifest
        .get("artifacts")?
        .as_array()?
        .iter()
        .find(|a| a.get("artifact_type").and_then(|t| t.as_str()) == Some(artifact_type))?
        .get("row_count")?
        .as_u64()
}

#[tokio::test]
async fn a_baseline_backtest_produces_orders_rather_than_an_empty_success() {
    let _serial = serial_backtest_environment().await;
    // The defect this exists for: every baseline adapter recorded its
    // decisions in `order_intents` and submitted nothing, while the worker
    // collects with `getattr(strategy, "orders", [])`. The getattr default
    // turned that into an empty list rather than an error, so a baseline
    // backtest RAN, reported SUCCEEDED, and wrote artifacts holding zero
    // orders -- which a user cannot tell apart from a strategy that decided
    // not to trade.
    //
    // Asserting SUCCEEDED would therefore have passed the whole time it was
    // broken. The assertion that matters is that the orders and fills
    // artifacts are not EMPTY.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "buy_and_hold",
        strategy_path: "strategies.buy_and_hold.adapter:BuyAndHoldAdapter",
        config_path: "strategies.buy_and_hold.adapter:BuyAndHoldConfig",
        parameters: None,
    };
    seed_run_for_job(
        &db.pool,
        job_id,
        owner,
        resolver.strategy_id,
        resolver.config_value(),
    )
    .await;
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Succeeded { .. }),
        "buy_and_hold must run: {outcome:?}"
    );

    for artifact in ["ORDERS", "FILLS"] {
        let rows = reported_rows(scratch.path(), artifact)
            .unwrap_or_else(|| panic!("{artifact} was not reported at all"));
        assert!(
            rows > 0,
            "{artifact} holds {rows} rows -- the strategy traded nothing, which \
             is the silent failure this test exists to catch"
        );
    }

    db.drop_db().await;
}

#[tokio::test]
async fn a_fill_is_charged_the_profile_the_runner_resolved() {
    let _serial = serial_backtest_environment().await;
    // Backtests used to report zero fees on every fill, so an equity curve
    // was the one a strategy would have earned paying nothing. Nothing
    // complained: the normalizer, the cash ledger, the fees artifact and the
    // metrics all consumed the zeros.
    //
    // The FEES artifact is what proves it now. A non-empty one can only come
    // from a fill that was actually charged, and the cash ledger identity
    // (`cash == initial - fills - fees`) is checked by the normalizer on the
    // way out -- so a run that charged fees but did not pay them fails before
    // it is published.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "buy_and_hold",
        strategy_path: "strategies.buy_and_hold.adapter:BuyAndHoldAdapter",
        config_path: "strategies.buy_and_hold.adapter:BuyAndHoldConfig",
        parameters: None,
    };
    seed_run_for_job(
        &db.pool,
        job_id,
        owner,
        resolver.strategy_id,
        resolver.config_value(),
    )
    .await;
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Succeeded { .. }),
        "the run must survive the ledger check with fees charged: {outcome:?}"
    );

    // The row COUNT proves nothing: a fees artifact with one row and a zero
    // amount is exactly what the broken version produced. The AMOUNT is the
    // assertion, and it is pinned to arithmetic done by hand rather than to
    // whatever the code happens to emit:
    //
    //   notional   = 9700 x 102502400 / 10_000 = 99,427,328 KRW
    //   commission = max(99,427,328 x 0.00015, 1,000) = 14,914.0992 KRW
    //
    // A rate that drifts, a min_commission that stops applying, or a profile
    // that silently resolves to something else all fail here.
    let result = normalized_result(scratch.path()).expect("result.json");
    assert_eq!(result["fills"][0]["quantity"], "9700");
    assert_eq!(result["fills"][0]["price"], "10250.2400");
    let commission = result["fees"][0]["commission"]["amount"]
        .as_str()
        .expect("a fee entry with an amount");
    assert_eq!(
        commission, "14914.0992",
        "the fill was not charged what the resolved profile says it costs"
    );
    let total_cost = result["metrics"]["total_cost"]
        .as_f64()
        .expect("metrics report a total cost");
    assert!(
        total_cost > 0.0,
        "metrics report {total_cost} in costs -- a strategy's returns would be          the ones it earned paying nothing"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_factor_driven_strategy_trades_on_the_computed_series() {
    let _serial = serial_backtest_environment().await;
    // The end of the factor chain: the runner computes `vol_60` with the Rust
    // factor-engine, embeds the series in the worker request, the adapter
    // hands those values to the Python target generator, and the resulting
    // targets become real orders.
    //
    // Asserting SUCCEEDED would prove none of it -- an adapter that received
    // nothing also finishes SUCCEEDED. Only the row count separates a
    // strategy that traded from one that could not.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "inverse_volatility",
        strategy_path: "strategies.inverse_volatility.adapter:InverseVolatilityAdapter",
        config_path: "strategies.inverse_volatility.adapter:InverseVolatilityConfig",
        parameters: None,
    };
    seed_run_for_job(
        &db.pool,
        job_id,
        owner,
        resolver.strategy_id,
        resolver.config_value(),
    )
    .await;
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Succeeded { .. }),
        "inverse_volatility must run on the computed series: {outcome:?}"
    );
    for artifact in ["ORDERS", "FILLS"] {
        let rows = reported_rows(scratch.path(), artifact).unwrap_or_else(|| {
            panic!("{artifact} was not reported at all");
        });
        assert!(
            rows > 0,
            "{artifact} holds {rows} rows -- the factor series reached the \
             strategy but produced no trade"
        );
    }

    db.drop_db().await;
}

#[tokio::test]
async fn a_strategy_that_reads_two_factors_executes_the_invested_branch() {
    let _serial = serial_backtest_environment().await;
    // `trend_following` compares two factors rather than ranking one, so it
    // exercises a different path through the series into the generator.
    //
    // The parameters are swapped (fast 200 / slow 50) on purpose. With its
    // defaults this strategy legitimately holds CASH on both of phase-0's
    // rebalance dates -- trend_50 sits below trend_200 on each -- and a
    // correct run that places no orders is indistinguishable from a broken
    // one that cannot. Swapping the windows is schema-valid, reads the exact
    // same two factor values, and lands on the invested branch, so what the
    // assertion below proves is the plumbing rather than the market.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "trend_following",
        strategy_path: "strategies.trend_following.adapter:TrendFollowingAdapter",
        config_path: "strategies.trend_following.adapter:TrendFollowingConfig",
        parameters: Some(serde_json::json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 200,
            "slow_ma": 50,
        })),
    };
    seed_run_for_job(
        &db.pool,
        job_id,
        owner,
        resolver.strategy_id,
        resolver.config_value(),
    )
    .await;
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    assert!(
        matches!(outcome, Outcome::Succeeded { .. }),
        "trend_following must run: {outcome:?}"
    );
    let rows = reported_rows(scratch.path(), "ORDERS").expect("orders reported");
    assert!(
        rows > 0,
        "ORDERS holds {rows} rows -- the invested branch placed no order"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn parameters_that_ask_for_an_undeclared_factor_fail_loudly() {
    let _serial = serial_backtest_environment().await;
    // The silent empty success, reachable through a configuration the product
    // ALLOWS -- which is why the earlier guard was not enough on its own.
    //
    // The runner computes the factors a package STATICALLY declares
    // (`trend_50`, `trend_200`), while the generator looks them up from the
    // PARAMETERS. `fast_ma: 100` is inside the schema's 5..250 range, so it
    // passes validation, wants `trend_100`, and nothing had computed it. The
    // adapter's missing-factor guard saw only the static list, let it through,
    // and `generate_target` raised inside a handler NautilusTrader swallows --
    // leaving a run that finished SUCCEEDED with zero orders.
    //
    // Proven by observation before it was fixed: this exact config returned
    // `Succeeded` with ORDERS=0.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "trend_following",
        strategy_path: "strategies.trend_following.adapter:TrendFollowingAdapter",
        config_path: "strategies.trend_following.adapter:TrendFollowingConfig",
        parameters: Some(serde_json::json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 100,
            "slow_ma": 200,
        })),
    };
    seed_run_for_job(
        &db.pool,
        job_id,
        owner,
        resolver.strategy_id,
        resolver.config_value(),
    )
    .await;
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    let Outcome::Failed { reason, .. } = &outcome else {
        panic!("a target that cannot be computed must fail: {outcome:?}");
    };
    assert!(
        reason.contains("trend_100"),
        "the failure must name the factor nobody computed, got {reason:?}"
    );

    // Permanent: the same parameters produce the same answer on every attempt,
    // and it is the submitter's config to change.
    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(after.status, JobStatus::Failed);

    db.drop_db().await;
}

#[tokio::test]
async fn a_dataset_too_short_for_a_strategy_fails_permanently() {
    let _serial = serial_backtest_environment().await;
    // `dual_momentum` needs 252 sessions of history and phase-0 holds 260,
    // which leaves no month-end that is not also the final session. There is
    // no honest rebalance date, so the run must fail.
    //
    // PERMANENTLY. A dataset does not grow between attempts, so requeueing
    // would spend the job's two remaining attempts reaching the identical
    // answer while the message an operator needs -- this dataset is too short
    // for this strategy -- stayed hidden behind RETRYING until they ran out.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "dual_momentum",
        strategy_path: "strategies.dual_momentum.adapter:DualMomentumAdapter",
        config_path: "strategies.dual_momentum.adapter:DualMomentumConfig",
        parameters: None,
    };
    seed_run_for_job(
        &db.pool,
        job_id,
        owner,
        resolver.strategy_id,
        resolver.config_value(),
    )
    .await;
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    let Outcome::Failed { reason, .. } = &outcome else {
        panic!("a dataset that cannot support the strategy must fail: {outcome:?}");
    };
    assert!(
        reason.contains("252") && reason.contains("260"),
        "the failure must name both numbers so it is clearly the DATA that is \
         short, got {reason:?}"
    );

    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(after.status, JobStatus::Failed);

    db.drop_db().await;
}

#[tokio::test]
async fn a_config_that_does_not_exist_fails_now_rather_than_three_times() {
    let _serial = serial_backtest_environment().await;
    // The mirror of the test above, and the reason resolution has a typed
    // error at all. A config id that does not exist produces the identical
    // answer on every attempt, so requeueing it spends the job's remaining
    // attempts and a worker slot each time to tell the user nothing new --
    // while the failure they need to see stays hidden behind RETRYING.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let outcome = run_once(
        &queue,
        "test-runner",
        &paths(&scratch),
        &MissingConfigResolver,
    )
    .await
    .expect("runner");
    assert!(
        matches!(outcome, Outcome::Failed { .. }),
        "a missing config is the user's answer, not a runner fault: {outcome:?}"
    );

    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(
        after.status,
        JobStatus::Failed,
        "the job is settled, not waiting for two more identical attempts"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_claimed_job_never_stays_running() {
    let _serial = serial_backtest_environment().await;
    // Whatever happens, the row must not be left RUNNING. A job stuck there is
    // one the user sees as neither running nor failed, until a sweeper
    // eventually notices -- and "eventually" is what makes it a bad experience
    // rather than a safe one.
    let Some(db) = ScratchDb::create().await else {
        return;
    };
    let scratch = tempfile::tempdir().expect("scratch");
    let queue = JobQueue::new(db.pool.clone(), None, QueueConfig::default());
    let owner = seed_owner(&db.pool).await;
    let job_id = submit_backtest(&queue, owner).await.expect("submit");

    let _ = run_once(&queue, "test-runner", &paths(&scratch), &BrokenResolver).await;

    let after = queue.get_by_id(job_id).await.expect("get");
    assert_ne!(
        after.status,
        JobStatus::Running,
        "every path out of the runner settles the claim"
    );
    assert!(after.locked_by.is_none(), "the lease is released");

    db.drop_db().await;
}

#[tokio::test]
async fn a_second_worker_cannot_settle_an_expired_backtest_claim() {
    let _serial = serial_backtest_environment().await;
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let queue = JobQueue::new(
        db.pool.clone(),
        None,
        QueueConfig {
            lease: std::time::Duration::from_millis(25),
            backoff_base: std::time::Duration::from_millis(1),
        },
    );
    let owner = seed_owner(&db.pool).await;
    let job = queue
        .submit(SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".to_owned(),
            payload: serde_json::json!({"kind": "backtest"}),
            priority: 1,
            idempotency_key: Some(Uuid::new_v4().to_string()),
            max_attempts: 2,
            available_at: None,
        })
        .await
        .expect("submit");
    let first = queue
        .claim_next_for("backtest-worker-a", "backtest")
        .await
        .expect("first claim")
        .expect("first claim exists");
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    queue.sweep().await.expect("sweep expired lease");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let second = queue
        .claim_next_for("backtest-worker-b", "backtest")
        .await
        .expect("second claim")
        .expect("requeued claim exists");
    assert_ne!(first.attempt.id, second.attempt.id);
    assert!(matches!(
        queue.settle_success(&first).await,
        Err(QueueError::StaleClaim(id)) if id == job.id
    ));
    queue
        .settle_failure(
            &second,
            job_queue::types::ErrorClass::Input,
            "TEST_DONE",
            "lease ownership test",
        )
        .await
        .expect("second worker settles its own claim");
    db.drop_db().await;
}
