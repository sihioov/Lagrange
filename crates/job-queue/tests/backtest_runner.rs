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
use job_queue::runner::{
    Outcome, ResolveError, ResolvedStrategy, RunnerPaths, StrategyResolver, run_once,
};
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
    async fn resolve(&self, _id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        Ok(ResolvedStrategy {
            strategy_path: "ma200_trend:MA200Trend".into(),
            config_path: "ma200_trend:MA200TrendConfig".into(),
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
        .env("DATABASE_URL", db.role_url("worker"))
        .env("LAGRANGE_REPO_ROOT", &root)
        .env("LAGRANGE_DATASET_ROOT", root.join("data/phase0"))
        .env("LAGRANGE_ARTIFACTS_ROOT", scratch.path())
        .env("LAGRANGE_UV_BIN", uv_bin())
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
}

impl StrategyResolver for BaselineResolver {
    async fn resolve(&self, _id: &str, _owner: Uuid) -> Result<ResolvedStrategy, ResolveError> {
        Ok(ResolvedStrategy {
            strategy_path: self.strategy_path.into(),
            config_path: self.config_path.into(),
            strategy_id: self.strategy_id.into(),
            strategy_version: "1.0.0".into(),
            config: serde_json::json!({
                "instrument_ids": ["069500.KRX"],
                "slippage_bps": 10,
                "lot_size": 100,
                "initial_cash": "100000000",
                "strategy_version": "1.0.0",
            }),
        })
    }
}

/// Rows the worker reported for an artifact type, from its own status file.
///
/// The worker publishes an exact `row_count` per artifact, so the assertion
/// reads that rather than inspecting parquet or guessing from file size. A
/// size threshold would be a heuristic standing in for the number the worker
/// already states, and the whole point of these tests is that "it produced a
/// file" is not the same claim as "it produced results".
fn reported_rows(scratch: &std::path::Path, artifact_type: &str) -> Option<u64> {
    let status_path = walk(scratch)
        .into_iter()
        .find(|p| p.ends_with("status.json"))?;
    let raw = std::fs::read_to_string(&status_path).ok()?;
    let status: serde_json::Value = serde_json::from_str(&raw).ok()?;
    status
        .get("artifacts")?
        .as_array()?
        .iter()
        .find(|a| a.get("artifact_type").and_then(|t| t.as_str()) == Some(artifact_type))?
        .get("row_count")?
        .as_u64()
}

#[tokio::test]
async fn a_baseline_backtest_produces_orders_rather_than_an_empty_success() {
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
    submit_backtest(&queue, owner).await.expect("submit");

    let resolver = BaselineResolver {
        strategy_id: "buy_and_hold",
        strategy_path: "strategies.buy_and_hold.adapter:BuyAndHoldAdapter",
        config_path: "strategies.buy_and_hold.adapter:BuyAndHoldConfig",
    };
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
async fn a_strategy_whose_factors_are_missing_fails_instead_of_reporting_success() {
    // The other half. Four baselines still need a factor series nobody
    // supplies, and the honest outcome is a FAILED run naming what is
    // missing. Before this, the run completed with zero orders and reported
    // SUCCEEDED -- the same empty artifacts as above, with no signal at all
    // that anything had gone wrong.
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
    };
    let outcome = run_once(&queue, "test-runner", &paths(&scratch), &resolver)
        .await
        .expect("runner");
    let Outcome::Failed { reason, .. } = &outcome else {
        panic!("a strategy that cannot compute a target must fail: {outcome:?}");
    };
    assert!(
        reason.contains("MISSING_FACTOR_SUPPLY"),
        "the failure must name what is missing, got {reason:?}"
    );

    // Settled permanently: rerunning it produces the identical answer until
    // somebody supplies factors, so the attempts are not worth spending.
    let after = queue.get_by_id(job_id).await.expect("get");
    assert_eq!(after.status, JobStatus::Failed);

    db.drop_db().await;
}

#[tokio::test]
async fn a_config_that_does_not_exist_fails_now_rather_than_three_times() {
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
