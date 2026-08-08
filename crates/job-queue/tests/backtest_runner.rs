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
use job_queue::queue::{JobQueue, QueueConfig};
use job_queue::runner::{Outcome, ResolvedStrategy, RunnerPaths, StrategyResolver, run_once};
use job_queue::types::{JobStatus, SubmitJob};
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

/// Resolves every config id to the phase-0 golden strategy.
///
/// A stand-in for the real registry lookup, and deliberately still a LOOKUP:
/// it ignores what the payload says and returns a fixed entry. A resolver that
/// echoed a caller-supplied module path back would turn a backtest submission
/// into arbitrary code execution, so the double must not model that shape even
/// for convenience.
struct GoldenResolver;

impl StrategyResolver for GoldenResolver {
    fn resolve(&self, _strategy_config_id: &str) -> Result<ResolvedStrategy, String> {
        Ok(ResolvedStrategy {
            strategy_path: "ma200_trend:MA200Trend".into(),
            strategy_id: "ma200-trend".into(),
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

/// A resolver that always refuses, for the "the runner could not run it" path.
struct BrokenResolver;

impl StrategyResolver for BrokenResolver {
    fn resolve(&self, _id: &str) -> Result<ResolvedStrategy, String> {
        Err("strategy registry is unavailable".into())
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

/// Submits a job shaped exactly as `POST /api/v1/backtests` writes it.
async fn submit_backtest(queue: &JobQueue, owner: Uuid) -> Result<Uuid, QueueError> {
    let job = queue
        .submit(SubmitJob {
            owner_user_id: owner,
            job_type: "backtest".into(),
            payload: serde_json::json!({
                "kind": "backtest",
                "run_id": Uuid::new_v4(),
                "strategy_config_id": Uuid::new_v4(),
                "dataset_version_id": "kr-etf-daily-phase0-v1",
                "start_date": "2020-01-01",
                "end_date": "2020-12-31",
                "initial_cash": { "currency": "KRW", "amount": "100000000" },
                "benchmark": "069500.KRX",
                "cost_profile_id": "krx-etf-default",
                "execution_profile": "next_open",
            }),
            priority: 10,
            idempotency_key: Some(Uuid::new_v4().to_string()),
            max_attempts: 3,
            available_at: None,
        })
        .await?;
    Ok(job.id)
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

    db.drop_db().await;
}

#[tokio::test]
async fn an_empty_queue_is_idle_rather_than_an_error() {
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
async fn a_claimed_job_never_stays_running() {
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
