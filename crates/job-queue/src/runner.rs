//! The loop that turns a queued job into a finished backtest.
//!
//! Every piece of this path already existed and nothing called the next one.
//! `POST /api/v1/backtests` wrote a row to `jobs`; `python -m backtest_worker
//! run` produced artifacts; `claim_next`/`settle_*` moved rows between states.
//! What was missing was the process that reads a row, runs the worker, and
//! records the result — so a queued backtest sat in the table forever while
//! every component's tests passed.
//!
//! # What this must get right
//!
//! **A claimed job must always be settled.** A worker that claims and then
//! panics leaves a row RUNNING with a lease that only the sweeper reclaims,
//! and until it does the user sees a backtest that is neither running nor
//! failed. Every owned path out of [`run_once`] therefore settles; after the
//! lease is lost this process is forbidden from settling and the sweeper owns
//! recovery.
//!
//! **A crash mid-backtest must not lose the job.** It is claimed with a lease,
//! so if this process dies the sweeper requeues it. That is why the worker is
//! invoked as a CHILD process rather than in-process: a NautilusTrader
//! simulation that exhausts memory takes its process down, and the isolation
//! layer is built to contain exactly that.
//!
//! **A failed backtest is not the same as a broken runner.** A strategy that
//! errors is the user's result and must not be retried; a database blip is the
//! runner's problem and must be. [`ErrorClass`] carries that distinction, and
//! getting it backwards either retries a deterministic failure three times or
//! discards work that would have succeeded.

use crate::error::QueueError;
use crate::queue::JobQueue;
use crate::safety::ClaimGate;
use crate::types::{ClaimedJob, ErrorClass, HeartbeatStatus, SettleResult};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tokio::process::Child;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

/// Where a backtest run's inputs and outputs live.
#[derive(Debug, Clone)]
pub struct RunnerPaths {
    /// Repository root; the worker is invoked from here.
    pub repo_root: PathBuf,
    /// Dataset root the worker reads (`<root>/curated/bars/...`).
    pub dataset_root: PathBuf,
    /// Where each run's artifacts are written, under
    /// `<artifacts>/<run_id>/generations/<code_commit>/<publication_uuid>`.
    pub artifacts_root: PathBuf,
    /// The `uv` executable that launches the worker.
    ///
    /// Explicit rather than resolved from `PATH` at call time. A daemon in a
    /// container and a test runner on a developer's machine have different
    /// PATHs, and "program not found" from a process that inherited the wrong
    /// one is indistinguishable from a broken worker. Naming it here makes the
    /// dependency visible and overridable.
    pub uv_bin: PathBuf,
    /// The exact lowercase Git object name baked into the runner image.
    ///
    /// This is deliberately carried alongside the execution paths instead of
    /// being read by the worker from the job payload.  A payload is routing
    /// input; the image revision is an attested property of this process.
    pub code_commit: String,
}

impl RunnerPaths {
    /// Resolves `uv` the way a deployment should: an explicit override first,
    /// then the name, letting the OS search `PATH`.
    pub fn default_uv_bin() -> PathBuf {
        std::env::var_os("LAGRANGE_UV_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("uv"))
    }
}

/// What the worker reported.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WorkerStatus {
    pub state: String,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
    #[serde(default)]
    pub artifacts: Vec<serde_json::Value>,
}

/// Shared shutdown state for the daemon and the currently running child.
///
/// A signal stops new claims immediately, but does not abandon a claim: the
/// child is terminated with a bounded grace period and the current job is
/// settled as retryable before the daemon exits.
#[derive(Clone, Default)]
pub struct RunnerControl {
    shutdown: Arc<AtomicBool>,
    notify: Arc<Notify>,
    shutdown_started: Arc<OnceLock<std::time::Instant>>,
}

impl RunnerControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_shutdown(&self) {
        self.shutdown_started.get_or_init(std::time::Instant::now);
        self.shutdown.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Wait until the daemon has been asked to stop. The binary uses this to
    /// keep an outer shutdown deadline around an in-flight run; exposing the
    /// wait primitive also makes cancellation tests deterministic.
    pub async fn wait_shutdown(&self) {
        while !self.is_shutdown() {
            self.notify.notified().await;
        }
    }

    /// Return the portion of a daemon shutdown budget that has not elapsed
    /// since the first shutdown request. This keeps pool teardown from adding
    /// another full timeout after an in-flight run already used the grace.
    pub fn remaining_shutdown_budget(&self, budget: std::time::Duration) -> std::time::Duration {
        self.shutdown_started
            .get()
            .map(|started| budget.saturating_sub(started.elapsed()))
            .unwrap_or(budget)
    }
}

const LEASE_ACTIVE: u8 = 0;
const LEASE_LOST: u8 = 1;
const LEASE_CANCELED: u8 = 2;

/// Blocking work is deliberately bounded even though this daemon normally
/// has one active claim.  A cancellation can leave an already-started
/// `spawn_blocking` closure running until its next cooperative check, so an
/// unbounded stream of retries must not create an unbounded blocking pool.
const BLOCKING_WORKERS: usize = 2;

/// A cooperative blocking task gets a small chance to observe cancellation
/// before its JoinHandle is dropped.  `JoinHandle::abort` cannot stop a
/// `spawn_blocking` closure after it has started; dropping it is intentional
/// here, and the closure has no database handle with which it could publish.
const BLOCKING_CANCEL_JOIN_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

/// Manifest input is an untrusted worker output.  Keep parsing bounded so a
/// malformed/hostile file cannot occupy a blocking worker indefinitely.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STATUS_BYTES: u64 = 4 * 1024 * 1024;

static BLOCKING_WORK_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn blocking_work_semaphore() -> Arc<Semaphore> {
    BLOCKING_WORK_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(BLOCKING_WORKERS)))
        .clone()
}

/// Cancellation observed by a bounded blocking closure.
///
/// The flag is separate from [`RunnerControl`] and [`LeaseGuard`] because a
/// blocking closure must not hold references to async state.  It is checked
/// between filesystem chunks and before every publication-side operation.
#[derive(Clone, Default)]
struct BlockingCancellation(Arc<AtomicBool>);

impl BlockingCancellation {
    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_canceled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
enum BlockingWorkError<E> {
    Shutdown(String),
    Canceled(String),
    LeaseLost(String),
    Timeout(String),
    Join(String),
    Operation(E),
}

/// Run CPU/filesystem-heavy work without polling it on a Tokio worker.
///
/// The cancellation branches deliberately return after a bounded join grace;
/// they do not call `abort` and do not claim that dropping a started blocking
/// task stops it.  A closure that is still in one uninterruptible OS call may
/// finish later, but it only owns local inputs/outputs and can never reach the
/// queue transaction.  Cooperative closures normally observe the flag before
/// the grace expires, releasing their permit promptly.
async fn run_bounded_blocking<T, E, F>(
    control: &RunnerControl,
    guard: &LeaseGuard,
    deadline: std::time::Duration,
    label: &'static str,
    operation: F,
) -> Result<T, BlockingWorkError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce(BlockingCancellation) -> Result<T, E> + Send + 'static,
{
    let deadline_at = tokio::time::Instant::now() + deadline;
    let semaphore = blocking_work_semaphore();
    let permit = tokio::select! {
        biased;
        _ = control.wait_shutdown() => {
            return Err(BlockingWorkError::Shutdown(format!(
                "runner shutdown requested during {label}"
            )));
        }
        signal = guard.wait_signal() => {
            return Err(if signal == LEASE_CANCELED {
                BlockingWorkError::Canceled(format!(
                    "backtest cancellation requested during {label}"
                ))
            } else {
                BlockingWorkError::LeaseLost(format!(
                    "backtest lease was lost during {label}"
                ))
            });
        }
        result = tokio::time::timeout_at(deadline_at, semaphore.acquire_owned()) => {
            result
                .map_err(|_| BlockingWorkError::Timeout(format!("{label} exceeded {deadline:?}")))
                .and_then(|permit| permit.map_err(|error| {
                    BlockingWorkError::Join(format!("blocking work semaphore closed: {error}"))
                }))?
        }
    };

    let cancellation = BlockingCancellation::default();
    let task_cancellation = cancellation.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation(task_cancellation)
    });

    tokio::select! {
        biased;
        result = &mut task => match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(BlockingWorkError::Operation(error)),
            Err(error) => Err(BlockingWorkError::Join(format!("{label} task failed: {error}"))),
        },
        _ = control.wait_shutdown() => {
            cancellation.cancel();
            let _ = tokio::time::timeout(BLOCKING_CANCEL_JOIN_GRACE, &mut task).await;
            Err(BlockingWorkError::Shutdown(format!(
                "runner shutdown requested during {label}"
            )))
        }
        signal = guard.wait_signal() => {
            cancellation.cancel();
            let _ = tokio::time::timeout(BLOCKING_CANCEL_JOIN_GRACE, &mut task).await;
            Err(if signal == LEASE_CANCELED {
                BlockingWorkError::Canceled(format!(
                    "backtest cancellation requested during {label}"
                ))
            } else {
                BlockingWorkError::LeaseLost(format!(
                    "backtest lease was lost during {label}"
                ))
            })
        }
        _ = tokio::time::sleep_until(deadline_at) => {
            cancellation.cancel();
            let _ = tokio::time::timeout(BLOCKING_CANCEL_JOIN_GRACE, &mut task).await;
            Err(BlockingWorkError::Timeout(format!("{label} exceeded {deadline:?}")))
        }
    }
}

/// One lease heartbeat task owns the claim for the complete pipeline. It is
/// deliberately independent of child-process execution: resolver lookups,
/// factor computation, manifest hashing, and publication can all outlive a
/// short queue lease too.
#[derive(Clone)]
struct LeaseGuard {
    status: Arc<AtomicU8>,
    notify: Arc<Notify>,
}

struct LeaseMonitor {
    guard: LeaseGuard,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl LeaseMonitor {
    fn start(queue: &JobQueue, claim: &ClaimedJob) -> Self {
        let guard = LeaseGuard {
            status: Arc::new(AtomicU8::new(LEASE_ACTIVE)),
            notify: Arc::new(Notify::new()),
        };
        let task_guard = guard.clone();
        let queue = queue.clone();
        let claim = claim.clone();
        let every = std::cmp::max(
            queue.config().lease / 3,
            std::time::Duration::from_millis(10),
        );
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(every);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if task_guard.status.load(Ordering::Acquire) != LEASE_ACTIVE {
                    return;
                }
                match tokio::time::timeout(DB_OPERATION_DEADLINE, queue.heartbeat(&claim)).await {
                    Err(_) => {
                        task_guard.status.store(LEASE_LOST, Ordering::Release);
                        task_guard.notify.notify_waiters();
                        return;
                    }
                    Ok(Ok(HeartbeatStatus::Extended)) => {}
                    Ok(Ok(HeartbeatStatus::Canceled)) => {
                        task_guard.status.store(LEASE_CANCELED, Ordering::Release);
                        task_guard.notify.notify_waiters();
                        return;
                    }
                    Ok(Ok(HeartbeatStatus::LeaseLost)) | Ok(Err(_)) => {
                        task_guard.status.store(LEASE_LOST, Ordering::Release);
                        task_guard.notify.notify_waiters();
                        return;
                    }
                }
            }
        });
        Self {
            guard,
            task: Some(task),
        }
    }

    async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for LeaseMonitor {
    fn drop(&mut self) {
        // If a caller is itself canceled (including the daemon's outer
        // shutdown budget), do not leave a detached heartbeat task extending
        // a claim after the owner has gone away.
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl LeaseGuard {
    fn check(&self) -> Result<(), String> {
        if self.status.load(Ordering::Acquire) == LEASE_LOST {
            Err("backtest lease was lost".to_owned())
        } else {
            Ok(())
        }
    }

    fn is_canceled(&self) -> bool {
        self.status.load(Ordering::Acquire) == LEASE_CANCELED
    }

    async fn wait_signal(&self) -> u8 {
        loop {
            let status = self.status.load(Ordering::Acquire);
            if status != LEASE_ACTIVE {
                return status;
            }
            self.notify.notified().await;
        }
    }
}

/// Canonical provenance copied from the API-created `backtest_runs` row.
///
/// The queue payload is untrusted routing data.  A worker must never invent
/// engine, dataset, code, seed, or strategy identity while building a child
/// request, so every one of these fields is loaded from this row and checked
/// again at publication time.
#[derive(Debug, Clone, sqlx::FromRow)]
struct CanonicalRun {
    id: Uuid,
    owner_user_id: Uuid,
    job_id: Option<Uuid>,
    strategy_id: String,
    strategy_version: String,
    dataset_version: String,
    engine: String,
    engine_version: String,
    config_sha256: String,
    code_commit: String,
    random_seed: Option<i32>,
    timezone: String,
    status: String,
}

#[derive(Debug, Clone)]
struct WorkerExecution {
    status: WorkerStatus,
    run_dir: PathBuf,
    /// The namespace owned by this exact queue attempt.  Requests, worker
    /// status, and every materialized output remain here until publication
    /// promotes them under the queue transaction.
    attempt_dir: PathBuf,
    /// Whether the supervisor process exited successfully.  The worker writes
    /// a terminal status for user-level failures and exits non-zero; keeping
    /// the process result alongside that status prevents a stale/synthetic
    /// `SUCCEEDED` status from turning a failed process into a success.
    process_succeeded: bool,
    /// Durable publication identity reserved for this execution. It becomes
    /// one path component of the immutable generation only after the worker
    /// output has passed validation; it is never used as the mutable
    /// canonical run directory that an older runner may still write.
    publication_id: Uuid,
    /// The runner image revision that was validated before claiming the job.
    /// Keep this separate from the mutable DB snapshot so the durable path
    /// visibly carries the baked/attested commit used by this process.
    publication_code_commit: String,
}

/// Files owned by one queue attempt live below a namespace that no retry can
/// reuse.  This is deliberately separate from the canonical run directory:
/// a detached `spawn_blocking` closure may finish after its caller has
/// observed cancellation, but it must never be able to write the next
/// attempt's request or any path the API treats as published.
fn attempt_namespace(run_dir: &Path, attempt_id: Uuid) -> PathBuf {
    run_dir.join(".attempts").join(attempt_id.to_string())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// Remove one exact attempt namespace after its child/request work has
/// stopped.  The path is constructed from a UUID rather than user input and
/// is never the canonical run directory, so a stale attempt can be cleaned
/// without risking another attempt's request or published artifacts.
fn cleanup_attempt_namespace(run_dir: &Path, attempt_id: Uuid) {
    let namespace = attempt_namespace(run_dir, attempt_id);
    let _ = std::fs::remove_dir_all(namespace);
}

#[derive(Debug, Deserialize)]
struct PublishedManifest {
    run: PublishedRun,
    #[serde(default)]
    metrics: BTreeMap<String, domain::ReportedStat>,
    #[serde(default)]
    warnings: Vec<PublishedWarning>,
    #[serde(default)]
    artifacts: Vec<PublishedArtifact>,
}

#[derive(Debug, Deserialize)]
struct PublishedRun {
    id: Uuid,
    owner_user_id: Uuid,
    job_id: Option<Uuid>,
    strategy_id: String,
    strategy_version: String,
    dataset_version: String,
    engine: String,
    engine_version: String,
    config_sha256: String,
    code_commit: String,
    random_seed: Option<i32>,
    timezone: String,
    status: String,
    #[serde(default)]
    summary_json: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct PublishedWarning {
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct PublishedArtifact {
    artifact_type: String,
    parquet_path: String,
    row_count: i64,
    sha256: String,
    size_bytes: i64,
    #[serde(default)]
    summary_json: serde_json::Value,
}

const EXPECTED_ARTIFACT_TYPES: [&str; 9] = [
    "EQUITY_CURVE",
    "DRAWDOWN_CURVE",
    "MONTHLY_RETURNS",
    "ORDERS",
    "FILLS",
    "POSITIONS",
    "CASH_LEDGER",
    "FEES",
    "BENCHMARK",
];

/// Maximum time spent asking a child process to terminate after cancellation
/// or lease loss.  The daemon's outer shutdown budget includes this grace and
/// the bounded settlement budget below.
pub const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

/// Every queue/database operation in the runner is short.  A blocked query is
/// allowed to finish normally, but never holds daemon shutdown hostage past
/// this deadline; an expired claim is then recovered by the sweeper.
const DB_OPERATION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Strategy lookup and factor preprocessing are expected to be much shorter
/// than the child simulation.  This cap is a safety net for a stalled
/// registry/worker database or a pathological dataset read.
const PREPROCESSING_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// The binary uses this as its maximum post-SIGTERM drain budget.  It is
/// intentionally public so static/integration tests can assert the contract
/// without duplicating a magic number.
pub const DAEMON_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// Stable event code used when the publication transaction's COMMIT response
/// is ambiguous. Kept here as well as [`crate::error`] so runner diagnostics
/// do not need to infer a code from display text.
pub const BACKTEST_COMMIT_UNKNOWN_EVENT_CODE: &str = crate::error::BACKTEST_COMMIT_UNKNOWN_CODE;

/// Whether a value is an exact, non-zero lowercase Git commit object name.
///
/// A backtest result carries this value as part of its immutable provenance;
/// accepting a short hash, uppercase hex, or an all-zero placeholder would
/// make two different images indistinguishable in the result store.
pub fn is_exact_code_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && value.bytes().any(|byte| byte != b'0')
}

/// A backtest job's payload, as `POST /api/v1/backtests` writes it.
///
/// Only the fields this runner needs. `serde` ignores the rest rather than
/// failing, because the route may add fields the runner has no use for and a
/// strict parse would turn that into every job failing at once.
#[derive(Debug, Deserialize)]
pub struct BacktestPayload {
    pub kind: Option<String>,
    pub run_id: Option<String>,
    pub strategy_config_id: Option<String>,
    pub dataset_version_id: Option<String>,
    pub initial_cash: Option<serde_json::Value>,
    pub cost_profile_id: Option<String>,
    /// The period the user asked to simulate.
    ///
    /// `POST /api/v1/backtests` has always required both and has always put
    /// them in the payload; until now this struct did not name them, so serde
    /// dropped them and every run silently covered the whole dataset while the
    /// stored row still said 2020-2021. A wrong number reported as the
    /// requested one is worse than an error, because nothing looks wrong.
    ///
    /// `Option` rather than required: rows queued before this field existed,
    /// and the phase-0 payloads, carry no window. Absent means the whole
    /// dataset -- the previous behaviour, now the explicit one.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

/// How a single claimed job finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing was waiting.
    Idle,
    Succeeded {
        job_id: String,
    },
    /// The backtest itself failed. The user's result, not a runner fault.
    Failed {
        job_id: String,
        reason: String,
    },
    /// The runner could not do its job. Retryable.
    Errored {
        job_id: String,
        reason: String,
    },
}

/// Why a strategy could not be resolved.
///
/// The split is the whole point. A config id that does not exist produces the
/// same failure on every attempt, so retrying it burns the job's attempts and
/// tells the user nothing they did not already know; a registry that is
/// briefly unreachable is the runner's problem and a later attempt may well
/// work. Collapsing these into one error gets one of the two wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// No such config for this owner, or it is no longer active. PERMANENT.
    NotFound(String),
    /// The config is real but names a strategy this runner carries no code
    /// for. PERMANENT, and deliberately distinct from `NotFound`: one is the
    /// submitter's to fix, the other is a deployment that is behind.
    Unknown(String),
    /// The registry could not be consulted at all. RETRYABLE.
    Unavailable(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotFound(d) => write!(f, "{d}"),
            ResolveError::Unknown(d) => write!(f, "{d}"),
            ResolveError::Unavailable(d) => write!(f, "{d}"),
        }
    }
}

/// The strategy a config id resolves to.
///
/// Resolution is a LOOKUP, never a passthrough. If a caller could put a module
/// path in the payload and have it executed, submitting a backtest would be
/// remote code execution; the registry is a closed set for that reason, and
/// this signature is where that guarantee is kept.
///
/// `owner_user_id` is not decoration. It comes from the CLAIMED JOB, not from
/// the payload, and an implementation must filter by it: without that, a job
/// naming another tenant's config id would run that tenant's strategy and
/// parameters. The runner serves every tenant, so nothing above this call
/// scopes the lookup for it.
pub trait StrategyResolver {
    fn resolve(
        &self,
        strategy_config_id: &str,
        owner_user_id: Uuid,
    ) -> impl Future<Output = Result<ResolvedStrategy, ResolveError>> + Send;
}

/// A registry entry, expanded.
#[derive(Debug, Clone)]
pub struct ResolvedStrategy {
    /// `module:Class`, produced by the registry — never by a request.
    pub strategy_path: String,
    /// `module:Class` of the strategy's config type.
    ///
    /// Sent explicitly even though the worker would otherwise derive
    /// `{class}Config` from `strategy_path`. That derivation is a convention
    /// two codebases have to keep agreeing on, and when it breaks it breaks
    /// inside a child process at import time — far from the table that chose
    /// the name.
    pub config_path: String,
    pub strategy_id: String,
    pub strategy_version: String,
    pub config: serde_json::Value,
}

/// Claims at most one job and runs it to completion.
///
/// Returns `Idle` when the queue is empty, which is the normal case most of
/// the time and deliberately not an error.
pub async fn run_once<R: StrategyResolver>(
    queue: &JobQueue,
    worker_id: &str,
    paths: &RunnerPaths,
    resolver: &R,
) -> Result<Outcome, QueueError> {
    run_once_with_control(queue, worker_id, paths, resolver, &RunnerControl::new()).await
}

/// Claim and execute one backtest while observing shutdown and lease loss.
pub async fn run_once_with_control<R: StrategyResolver>(
    queue: &JobQueue,
    worker_id: &str,
    paths: &RunnerPaths,
    resolver: &R,
    control: &RunnerControl,
) -> Result<Outcome, QueueError> {
    run_once_with_control_and_gate(queue, worker_id, paths, resolver, control, None).await
}

/// Claim and execute one backtest while observing shutdown, lease loss, and an
/// optional fail-closed capacity gate. Existing library callers that do not
/// own a readiness loop may pass `None`; deployed daemons pass their shared
/// [`ClaimGate`].
pub async fn run_once_with_control_and_gate<R: StrategyResolver>(
    queue: &JobQueue,
    worker_id: &str,
    paths: &RunnerPaths,
    resolver: &R,
    control: &RunnerControl,
    gate: Option<&ClaimGate>,
) -> Result<Outcome, QueueError> {
    if !is_exact_code_commit(&paths.code_commit) {
        return Err(QueueError::InvalidInput(
            "LAGRANGE_CODE_COMMIT must be an exact lowercase 40-hex Git commit".to_owned(),
        ));
    }
    if control.is_shutdown() {
        return Ok(Outcome::Idle);
    }
    if gate.is_some_and(|gate| !gate.allows_claim()) {
        return Ok(Outcome::Idle);
    }
    let claim = match await_controlled(
        queue.claim_next_for(worker_id, "backtest"),
        control,
        DB_OPERATION_DEADLINE,
        "claiming a backtest job",
    )
    .await
    {
        Ok(Ok(claim)) => claim,
        Ok(Err(error)) => return Err(error),
        Err(ExecError::Shutdown(_)) => return Ok(Outcome::Idle),
        Err(error) => return Err(QueueError::Internal(error.reason())),
    };
    let Some(claim) = claim else {
        return Ok(Outcome::Idle);
    };
    let job_id = claim.job.id.to_string();
    // Start before any provenance read or state transition. The monitor is
    // intentionally stopped only after queue/result publication completes.
    let lease = LeaseMonitor::start(queue, &claim);
    let guard = lease.guard.clone();
    let result = async {
        // A job of another type is not this runner's, and claiming it was a
        // mistake this process must undo rather than fail: settling it as an
        // error would burn one of its attempts for no reason.
        if claim.job.job_type != "backtest" {
            guard.check().map_err(QueueError::Internal)?;
            await_settlement(
                queue.settle_failure(
                    &claim,
                    ErrorClass::Transient,
                    "WRONG_RUNNER",
                    &format!("job type {} is not backtest", claim.job.job_type),
                ),
                "wrong-runner settlement",
            )
            .await?;
            return Ok(Outcome::Errored {
                job_id,
                reason: "wrong runner".into(),
            });
        }

        let (payload, canonical) =
            match load_payload_and_run(queue, &claim, &guard, control, &paths.code_commit).await {
                Ok(value) => value,
                Err(error) => {
                    let reason = error.reason();
                    guard.check().map_err(QueueError::Internal)?;
                    await_settlement(
                        settle_without_run(queue, &claim, error.class(), error.code(), &reason),
                        "preprocessing failure settlement",
                    )
                    .await?;
                    return Ok(Outcome::Failed { job_id, reason });
                }
            };
        match mark_run_running(queue, &claim, &canonical, &guard, control).await {
            Ok(()) => {}
            Err(MarkRunError::StateMismatch(reason)) => {
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Integrity,
                        "BACKTEST_RUN_STATE",
                        &reason,
                    ),
                    "run-state failure settlement",
                )
                .await?;
                return Ok(Outcome::Failed { job_id, reason });
            }
            Err(MarkRunError::Shutdown(reason)) => {
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Transient,
                        "RUNNER_SHUTDOWN",
                        &reason,
                    ),
                    "run-state shutdown settlement",
                )
                .await?;
                return Ok(Outcome::Errored { job_id, reason });
            }
            Err(MarkRunError::Canceled(reason)) => {
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Input,
                        "BACKTEST_CANCELED",
                        &reason,
                    ),
                    "run-state cancellation settlement",
                )
                .await?;
                return Ok(Outcome::Failed { job_id, reason });
            }
            Err(MarkRunError::Database(error)) => {
                let reason = format!("cannot mark backtest run {} RUNNING: {error}", canonical.id);
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Transient,
                        "RUNNER_ERROR",
                        &reason,
                    ),
                    "run-state database failure settlement",
                )
                .await?;
                return Ok(Outcome::Errored { job_id, reason });
            }
        }

        let execution = execute(
            &claim, &payload, &canonical, paths, resolver, control, &guard,
        )
        .await;
        let outcome = match execution {
            Ok(execution)
                if execution.process_succeeded && execution.status.state == "SUCCEEDED" =>
            {
                match await_publish_success(queue, &claim, &canonical, &execution, &guard, control)
                    .await
                {
                    Ok(SettleResult::Committed(_)) => Ok(Outcome::Succeeded { job_id }),
                    Ok(SettleResult::Canceled(_)) => Ok(Outcome::Failed {
                        job_id,
                        reason: "backtest was canceled while publishing".to_owned(),
                    }),
                    Err(PublishError::Invalid(reason)) => {
                        guard.check().map_err(QueueError::Internal)?;
                        await_settlement(
                            settle_failure_with_run(
                                queue,
                                &claim,
                                &canonical,
                                ErrorClass::Integrity,
                                "RESULT_VALIDATION_FAILED",
                                &reason,
                            ),
                            "manifest validation settlement",
                        )
                        .await?;
                        Ok(Outcome::Failed { job_id, reason })
                    }
                    Err(PublishError::Shutdown(reason)) => {
                        guard.check().map_err(QueueError::Internal)?;
                        await_settlement(
                            settle_failure_with_run(
                                queue,
                                &claim,
                                &canonical,
                                ErrorClass::Transient,
                                "RUNNER_SHUTDOWN",
                                &reason,
                            ),
                            "publication shutdown settlement",
                        )
                        .await?;
                        Ok(Outcome::Errored { job_id, reason })
                    }
                    Err(PublishError::Canceled(reason)) => {
                        await_settlement(
                            settle_failure_with_run(
                                queue,
                                &claim,
                                &canonical,
                                ErrorClass::Input,
                                "BACKTEST_CANCELED",
                                &reason,
                            ),
                            "publication cancellation settlement",
                        )
                        .await?;
                        Ok(Outcome::Failed { job_id, reason })
                    }
                    Err(PublishError::LeaseLost(reason)) => Ok(Outcome::Errored { job_id, reason }),
                    Err(PublishError::Database(error) | PublishError::CommitUnknown(error)) => {
                        Err(error)
                    }
                }
            }
            Ok(execution) => {
                // The backtest ran and did not succeed. That is a RESULT, and
                // retrying it would produce the same result while telling the user
                // nothing new, so it is settled as permanent.
                let reason = execution
                    .status
                    .error
                    .as_ref()
                    .and_then(|e| e.get("detail"))
                    .and_then(|d| d.as_str())
                    .unwrap_or("the backtest did not succeed")
                    .to_string();
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        classify_backtest_failure(&reason),
                        "BACKTEST_FAILED",
                        &reason,
                    ),
                    "backtest failure settlement",
                )
                .await?;
                Ok(Outcome::Failed { job_id, reason })
            }
            Err(ExecError::Permanent {
                class,
                code,
                reason,
            }) => {
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(queue, &claim, &canonical, class, code, &reason),
                    "permanent failure settlement",
                )
                .await?;
                Ok(Outcome::Failed { job_id, reason })
            }
            Err(ExecError::Transient(reason)) => {
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Transient,
                        "RUNNER_ERROR",
                        &reason,
                    ),
                    "runner failure settlement",
                )
                .await?;
                Ok(Outcome::Errored { job_id, reason })
            }
            Err(ExecError::Shutdown(reason)) => {
                guard.check().map_err(QueueError::Internal)?;
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Transient,
                        "RUNNER_SHUTDOWN",
                        &reason,
                    ),
                    "shutdown settlement",
                )
                .await?;
                Ok(Outcome::Errored { job_id, reason })
            }
            Err(ExecError::Canceled(reason)) => {
                await_settlement(
                    settle_failure_with_run(
                        queue,
                        &claim,
                        &canonical,
                        ErrorClass::Input,
                        "BACKTEST_CANCELED",
                        &reason,
                    ),
                    "cancellation settlement",
                )
                .await?;
                Ok(Outcome::Failed { job_id, reason })
            }
            Err(ExecError::LeaseLost(reason)) => {
                // A stale worker must not publish or settle anything; the queue
                // sweeper owns requeue/failure of the attempt now.
                Ok(Outcome::Errored { job_id, reason })
            }
        };
        // Keep the attempt namespace alive until validation/publication has
        // consumed it.  Cleanup is scoped to this UUID only; a detached old
        // writer can never remove a retry's namespace or immutable generation.
        cleanup_attempt_namespace(
            &paths.artifacts_root.join(canonical.id.to_string()),
            claim.attempt.id,
        );
        outcome
    }
    .await;
    lease.stop().await;
    result
}

/// Why [`execute`] did not return a worker status.
///
/// Separated from `Outcome` because the distinction it carries is about
/// RETRYABILITY, which the queue acts on, rather than about what to report.
#[derive(Debug)]
enum ExecError {
    /// No attempt can succeed. Settled permanently.
    Permanent {
        class: ErrorClass,
        code: &'static str,
        reason: String,
    },
    /// The runner faltered; a later attempt may work.
    Transient(String),
    /// The daemon was asked to stop and drained the active child.
    Shutdown(String),
    /// The user canceled the claimed job.
    Canceled(String),
    /// The database lease was canceled, expired, or acquired by another
    /// worker. No result may be published after this point.
    LeaseLost(String),
}

impl ExecError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Permanent { class, .. } => *class,
            Self::Transient(_) | Self::Shutdown(_) | Self::LeaseLost(_) => ErrorClass::Transient,
            Self::Canceled(_) => ErrorClass::Input,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Permanent { code, .. } => code,
            Self::Transient(_) => "RUNNER_ERROR",
            Self::Shutdown(_) => "RUNNER_SHUTDOWN",
            Self::Canceled(_) => "BACKTEST_CANCELED",
            Self::LeaseLost(_) => "LEASE_LOST",
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::Permanent { reason, .. }
            | Self::Transient(reason)
            | Self::Shutdown(reason)
            | Self::Canceled(reason)
            | Self::LeaseLost(reason) => reason.clone(),
        }
    }
}

/// Await a preprocessing/queue operation while giving SIGTERM and the
/// operation deadline priority. Dropping a SQLx future is safe: PostgreSQL
/// receives cancellation when the connection future is dropped, and the
/// claim remains owned until its lease expires if no settlement can finish.
async fn await_controlled<F, T>(
    future: F,
    control: &RunnerControl,
    deadline: std::time::Duration,
    label: &'static str,
) -> Result<T, ExecError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = control.wait_shutdown() => Err(ExecError::Shutdown(format!("runner shutdown requested during {label}"))),
        result = tokio::time::timeout(deadline, future) => result
            .map_err(|_| ExecError::Transient(format!("{label} exceeded {:?}", deadline))),
    }
}

/// Await an operation owned by a claimed job while observing both daemon
/// shutdown and the lease monitor. Resolver/factor work can outlive a normal
/// queue operation, but a user cancellation must not wait for the entire
/// preprocessing deadline before the claim is settled.
async fn await_controlled_with_guard<F, T>(
    future: F,
    control: &RunnerControl,
    guard: &LeaseGuard,
    deadline: std::time::Duration,
    label: &'static str,
) -> Result<T, ExecError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = control.wait_shutdown() => Err(ExecError::Shutdown(format!(
            "runner shutdown requested during {label}"
        ))),
        signal = guard.wait_signal() => Err(if signal == LEASE_CANCELED {
            ExecError::Canceled(format!("backtest cancellation requested during {label}"))
        } else {
            ExecError::LeaseLost(format!("backtest lease was lost during {label}"))
        }),
        result = tokio::time::timeout(deadline, future) => result
            .map_err(|_| ExecError::Transient(format!("{label} exceeded {:?}", deadline))),
    }
}

/// A settlement is the one operation we still attempt after cancellation:
/// queue state and the backtest run must cross one transaction boundary. It
/// gets a finite deadline, after which the claim is intentionally left for the
/// sweeper rather than being partially updated or reported as committed.
async fn await_settlement<F, T>(future: F, label: &'static str) -> Result<T, QueueError>
where
    F: Future<Output = Result<T, QueueError>>,
{
    tokio::time::timeout(DB_OPERATION_DEADLINE, future)
        .await
        .map_err(|_| {
            QueueError::Internal(format!("{label} exceeded {:?}", DB_OPERATION_DEADLINE))
        })?
}

fn check_execution_guard(guard: &LeaseGuard, control: &RunnerControl) -> Result<(), ExecError> {
    guard.check().map_err(ExecError::LeaseLost)?;
    if control.is_shutdown() {
        return Err(ExecError::Shutdown("runner shutdown requested".to_owned()));
    }
    if guard.is_canceled() {
        return Err(ExecError::Canceled(
            "backtest cancellation requested".to_owned(),
        ));
    }
    Ok(())
}

impl From<ResolveError> for ExecError {
    fn from(e: ResolveError) -> ExecError {
        match e {
            ResolveError::NotFound(reason) => ExecError::Permanent {
                class: ErrorClass::Input,
                code: "STRATEGY_CONFIG_NOT_FOUND",
                reason,
            },
            ResolveError::Unknown(reason) => ExecError::Permanent {
                // Not `Input`: the submitter did nothing wrong, and an
                // operator reading this needs to see a deployment that is
                // missing code rather than a user who sent a bad parameter.
                class: ErrorClass::DataBlocked,
                code: "STRATEGY_NOT_DEPLOYED",
                reason,
            },
            ResolveError::Unavailable(reason) => ExecError::Transient(reason),
        }
    }
}

/// Which `ErrorClass` a failed backtest belongs to.
///
/// None of these retry, and that is the point: the job's inputs produced this
/// failure, so running the identical inputs again produces the identical
/// failure while consuming another attempt and telling the user nothing new.
///
/// The classes are still distinguished rather than collapsed, because they
/// route differently for an operator: a blocked dataset is someone's to
/// unblock, while a bad parameter is the submitter's to fix.
fn classify_backtest_failure(reason: &str) -> ErrorClass {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("dataset") || lower.contains("curated") || lower.contains("zone missing") {
        ErrorClass::DataBlocked
    } else if lower.contains("hash") || lower.contains("mismatch") || lower.contains("ledger") {
        ErrorClass::Integrity
    } else {
        ErrorClass::Input
    }
}

async fn load_payload_and_run(
    queue: &JobQueue,
    claim: &ClaimedJob,
    guard: &LeaseGuard,
    control: &RunnerControl,
    expected_code_commit: &str,
) -> Result<(BacktestPayload, CanonicalRun), ExecError> {
    check_execution_guard(guard, control)?;
    let payload: BacktestPayload =
        serde_json::from_value(claim.job.payload_json.clone()).map_err(|error| {
            ExecError::Permanent {
                class: ErrorClass::Input,
                code: "MALFORMED_PAYLOAD",
                reason: format!("payload is not a backtest request: {error}"),
            }
        })?;
    if payload.kind.as_deref() != Some("backtest") {
        return Err(ExecError::Permanent {
            class: ErrorClass::Input,
            code: "MALFORMED_PAYLOAD",
            reason: "payload kind is not backtest".to_owned(),
        });
    }
    let raw_run_id = payload
        .run_id
        .as_deref()
        .ok_or_else(|| ExecError::Permanent {
            class: ErrorClass::Integrity,
            code: "BACKTEST_RUN_ID_MISSING",
            reason: "backtest payload has no pre-created run id".to_owned(),
        })?;
    let run_id = Uuid::parse_str(raw_run_id).map_err(|_| ExecError::Permanent {
        class: ErrorClass::Integrity,
        code: "BACKTEST_RUN_ID_INVALID",
        reason: format!("backtest run id {raw_run_id:?} is not a UUID"),
    })?;
    let canonical = await_controlled_with_guard(
        sqlx::query_as::<_, CanonicalRun>(
            "SELECT id, owner_user_id, job_id, strategy_id, strategy_version,
                dataset_version, engine, engine_version, config_sha256,
                code_commit, random_seed, timezone, status
         FROM backtest_runs
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $3",
        )
        .bind(run_id)
        .bind(claim.job.owner_user_id)
        .bind(claim.job.id)
        .fetch_optional(&queue.pool()),
        control,
        guard,
        DB_OPERATION_DEADLINE,
        "loading backtest provenance",
    )
    .await?
    .map_err(|error| ExecError::Transient(format!("cannot load backtest provenance: {error}")))?
    .ok_or_else(|| ExecError::Permanent {
        class: ErrorClass::Integrity,
        code: "BACKTEST_RUN_IDENTITY_MISMATCH",
        reason: format!(
            "no pre-created backtest run {run_id} belongs to owner {} and job {}",
            claim.job.owner_user_id, claim.job.id
        ),
    })?;
    check_execution_guard(guard, control)?;
    if !matches!(canonical.status.as_str(), "PENDING" | "RUNNING") {
        return Err(ExecError::Permanent {
            class: ErrorClass::Integrity,
            code: "BACKTEST_RUN_NOT_CLAIMABLE",
            reason: format!(
                "backtest run {} is already {}",
                canonical.id, canonical.status
            ),
        });
    }
    if !is_sha256_hex(&canonical.config_sha256)
        || !canonical.config_sha256.bytes().any(|byte| byte != b'0')
        || !is_exact_code_commit(&canonical.code_commit)
        || canonical.random_seed.is_none()
        || canonical.random_seed == Some(0)
    {
        return Err(ExecError::Permanent {
            class: ErrorClass::Integrity,
            code: "BACKTEST_PROVENANCE_INVALID",
            reason: format!(
                "backtest run {} has invalid immutable provenance",
                canonical.id
            ),
        });
    }
    validate_attested_code_commit(canonical.id, &canonical.code_commit, expected_code_commit)?;
    Ok((payload, canonical))
}

fn validate_attested_code_commit(
    run_id: Uuid,
    asserted_code_commit: &str,
    runner_code_commit: &str,
) -> Result<(), ExecError> {
    if asserted_code_commit != runner_code_commit {
        return Err(ExecError::Permanent {
            class: ErrorClass::Integrity,
            code: "BACKTEST_CODE_COMMIT_MISMATCH",
            reason: format!(
                "backtest run {run_id} asserts code commit {asserted_code_commit}, but this runner is {runner_code_commit}"
            ),
        });
    }
    Ok(())
}

enum MarkRunError {
    StateMismatch(String),
    Shutdown(String),
    Canceled(String),
    Database(QueueError),
}

async fn mark_run_running(
    queue: &JobQueue,
    claim: &ClaimedJob,
    run: &CanonicalRun,
    guard: &LeaseGuard,
    control: &RunnerControl,
) -> Result<(), MarkRunError> {
    if let Err(error) = check_execution_guard(guard, control) {
        return Err(match error {
            ExecError::Shutdown(reason) => MarkRunError::Shutdown(reason),
            ExecError::Canceled(reason) => MarkRunError::Canceled(reason),
            ExecError::LeaseLost(reason) => MarkRunError::StateMismatch(reason),
            ExecError::Permanent { reason, .. } | ExecError::Transient(reason) => {
                MarkRunError::StateMismatch(reason)
            }
        });
    }
    let job_id = run.job_id.ok_or_else(|| {
        MarkRunError::StateMismatch("backtest run has no job identity".to_owned())
    })?;
    let rows = match await_controlled_with_guard(
        sqlx::query(
            "UPDATE backtest_runs
         SET status = 'RUNNING', started_at = COALESCE(started_at, now()),
             finished_at = NULL
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $3
           AND status IN ('PENDING', 'RUNNING')",
        )
        .bind(run.id)
        .bind(claim.job.owner_user_id)
        .bind(job_id)
        .execute(&queue.pool()),
        control,
        guard,
        DB_OPERATION_DEADLINE,
        "marking backtest run RUNNING",
    )
    .await
    {
        Ok(rows) => rows.map_err(|error| MarkRunError::Database(QueueError::Database(error)))?,
        Err(ExecError::Shutdown(reason)) => return Err(MarkRunError::Shutdown(reason)),
        Err(ExecError::Canceled(reason)) => return Err(MarkRunError::Canceled(reason)),
        Err(error) => return Err(MarkRunError::Database(QueueError::Internal(error.reason()))),
    };
    if rows.rows_affected() != 1 {
        return Err(MarkRunError::StateMismatch(format!(
            "backtest run {} changed state before execution",
            run.id
        )));
    }
    Ok(())
}

async fn settle_without_run(
    queue: &JobQueue,
    claim: &ClaimedJob,
    class: ErrorClass,
    code: &str,
    reason: &str,
) -> Result<SettleResult, QueueError> {
    let mut tx = queue.begin().await?;
    let result = queue
        .settle_failure_in(&mut tx, claim, class, code, reason)
        .await;
    match result {
        Ok(result) => {
            tx.commit().await?;
            Ok(result)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}

async fn settle_failure_with_run(
    queue: &JobQueue,
    claim: &ClaimedJob,
    run: &CanonicalRun,
    class: ErrorClass,
    code: &str,
    reason: &str,
) -> Result<SettleResult, QueueError> {
    let mut tx = queue.begin().await?;
    let result = queue
        .settle_failure_in(&mut tx, claim, class, code, reason)
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            tx.rollback().await?;
            return Err(error);
        }
    };
    let status = match &result {
        SettleResult::Canceled(_) => "CANCELED",
        SettleResult::Committed(job) if job.status.as_str() == "QUEUED" => "PENDING",
        SettleResult::Committed(_) => "FAILED",
    };
    let summary = serde_json::json!({
        "error_code": code,
        "error_message": reason,
    });
    let rows = sqlx::query(
        "UPDATE backtest_runs
         SET status = $4, summary_json = $5, finished_at =
             CASE WHEN $4 IN ('FAILED', 'CANCELED') THEN now() ELSE NULL END
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $3
           AND status IN ('PENDING', 'RUNNING')",
    )
    .bind(run.id)
    .bind(claim.job.owner_user_id)
    .bind(claim.job.id)
    .bind(status)
    .bind(summary)
    .execute(&mut *tx)
    .await?;
    if rows.rows_affected() != 1 {
        tx.rollback().await?;
        return Err(QueueError::Internal(format!(
            "backtest run {} could not be settled with job {}",
            run.id, claim.job.id
        )));
    }
    tx.commit().await?;
    Ok(result)
}

#[derive(Debug)]
enum PublishError {
    Invalid(String),
    Shutdown(String),
    Canceled(String),
    LeaseLost(String),
    Database(QueueError),
    /// PostgreSQL may have committed even when the client observed an error
    /// while waiting for COMMIT. Retain the immutable generation in this
    /// case: deleting it could leave a committed DB row pointing at missing
    /// bytes, and reconciliation can distinguish referenced from orphaned
    /// generations later.
    CommitUnknown(QueueError),
}

async fn await_publish_success(
    queue: &JobQueue,
    claim: &ClaimedJob,
    canonical: &CanonicalRun,
    execution: &WorkerExecution,
    guard: &LeaseGuard,
    control: &RunnerControl,
) -> Result<SettleResult, PublishError> {
    match tokio::time::timeout(
        DB_OPERATION_DEADLINE,
        publish_success(queue, claim, canonical, execution, guard, control),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            // `publish_success_transaction` disarms its cleanup guard exactly
            // before entering COMMIT. If the outer deadline fires while that
            // future is waiting, the generation remains on disk and the DB
            // outcome is ambiguous even though no SQLx error was returned.
            let generation = immutable_publication_artifacts(
                &execution.run_dir,
                &execution.publication_code_commit,
                execution.publication_id,
            )
            .map_err(|error| match error {
                PublishError::Invalid(reason) => PublishError::Database(QueueError::Internal(
                    format!("cannot identify timed-out publication: {reason}"),
                )),
                other => other,
            })?;
            if std::fs::symlink_metadata(&generation).is_ok() {
                let detail = format!(
                    "backtest publication exceeded {:?} after immutable promotion",
                    DB_OPERATION_DEADLINE
                );
                if let Err(marker_error) =
                    write_commit_unknown_marker(&generation, canonical.id, claim.job.id, &detail)
                {
                    eprintln!(
                        "event_code={} run_id={} job_id={} generation_path={} marker_error={}",
                        BACKTEST_COMMIT_UNKNOWN_EVENT_CODE,
                        canonical.id,
                        claim.job.id,
                        generation.display(),
                        marker_error
                    );
                }
                Err(PublishError::CommitUnknown(QueueError::CommitUnknown {
                    run_id: canonical.id,
                    job_id: claim.job.id,
                    generation_path: generation.display().to_string(),
                    detail,
                }))
            } else {
                Err(PublishError::Database(QueueError::Internal(format!(
                    "backtest publication exceeded {:?}",
                    DB_OPERATION_DEADLINE
                ))))
            }
        }
    }
}

fn check_publication_guard(
    guard: &LeaseGuard,
    control: &RunnerControl,
) -> Result<(), PublishError> {
    guard.check().map_err(PublishError::LeaseLost)?;
    if control.is_shutdown() {
        return Err(PublishError::Shutdown(
            "runner shutdown requested during publication".to_owned(),
        ));
    }
    if guard.is_canceled() {
        return Err(PublishError::Canceled(
            "backtest cancellation requested during publication".to_owned(),
        ));
    }
    Ok(())
}

fn canonical_provenance_matches(left: &CanonicalRun, right: &CanonicalRun) -> bool {
    left.id == right.id
        && left.owner_user_id == right.owner_user_id
        && left.job_id == right.job_id
        && left.strategy_id == right.strategy_id
        && left.strategy_version == right.strategy_version
        && left.dataset_version == right.dataset_version
        && left.engine == right.engine
        && left.engine_version == right.engine_version
        && left.config_sha256 == right.config_sha256
        && left.code_commit == right.code_commit
        && left.random_seed == right.random_seed
        && left.timezone == right.timezone
}

/// Re-read immutable run provenance after the child exits and while the
/// publication transaction owns the row.  The pre-child snapshot is useful for
/// request construction, but it is not enough to authorize promotion after a
/// lease/retry boundary or a concurrent administrative update.
async fn recheck_canonical_provenance(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    canonical: &CanonicalRun,
    claim: &ClaimedJob,
    guard: &LeaseGuard,
    control: &RunnerControl,
) -> Result<(), PublishError> {
    check_publication_guard(guard, control)?;
    let current = sqlx::query_as::<_, CanonicalRun>(
        "SELECT id, owner_user_id, job_id, strategy_id, strategy_version,
                dataset_version, engine, engine_version, config_sha256,
                code_commit, random_seed, timezone, status
         FROM backtest_runs
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $3
         FOR UPDATE",
    )
    .bind(canonical.id)
    .bind(claim.job.owner_user_id)
    .bind(claim.job.id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| PublishError::Database(QueueError::Database(error)))?
    .ok_or_else(|| {
        PublishError::Invalid(format!(
            "backtest run {} disappeared before publication",
            canonical.id
        ))
    })?;
    if current.status != "RUNNING" || !canonical_provenance_matches(&current, canonical) {
        return Err(PublishError::Invalid(
            "backtest provenance changed before publication".to_owned(),
        ));
    }
    check_publication_guard(guard, control)
}

/// Remove one exact publication path without following a symlink.  This is
/// used only for a generation that this invocation created before its DB
/// transaction committed; it never targets the legacy `<run>/artifacts` or
/// `<run>/status.json` names an older runner may still write.
fn remove_path_safely(path: &Path) -> Result<(), PublishError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        PublishError::Invalid(format!("publication target is unavailable: {error}"))
    })?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path).map_err(|error| {
            PublishError::Invalid(format!("cannot remove publication target: {error}"))
        })
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(path).map_err(|error| {
            PublishError::Invalid(format!("cannot remove publication directory: {error}"))
        })
    } else {
        Err(PublishError::Invalid(
            "publication target is not a regular path".to_owned(),
        ))
    }
}

/// Check or create one directory component without following a symlink.
/// `create_dir`, rather than `create_dir_all`, is intentional: every ancestor
/// is checked independently so an old writer cannot turn a path component
/// into a symlink escape between publication attempts.
fn ensure_directory_component(path: &Path, label: &str) -> Result<(), PublishError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PublishError::Invalid(format!(
                    "{label} must be a regular directory"
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map_err(|error| {
                PublishError::Invalid(format!("cannot create {label}: {error}"))
            })?;
            let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                PublishError::Invalid(format!("cannot inspect {label}: {error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PublishError::Invalid(format!(
                    "{label} must be a regular directory"
                )));
            }
            Ok(())
        }
        Err(error) => Err(PublishError::Invalid(format!(
            "cannot inspect {label}: {error}"
        ))),
    }
}

/// Validate every already-existing ancestor of a configured artifact path.
/// Checking only the leaf with `symlink_metadata` is insufficient: a
/// symlinked artifact-root or run parent would still be followed while
/// resolving the leaf. Missing ancestors are left for the component-by-
/// component creator below.
fn ensure_no_symlink_ancestors(path: &Path, label: &str) -> Result<(), PublishError> {
    let mut ancestors = Vec::new();
    let mut current = path;
    loop {
        if !current.as_os_str().is_empty() {
            ancestors.push(current.to_path_buf());
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }
    for ancestor in ancestors.into_iter().rev() {
        match std::fs::symlink_metadata(&ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PublishError::Invalid(format!(
                    "{label} contains a symlinked path component"
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(PublishError::Invalid(format!(
                    "{label} path component is not a directory"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(PublishError::Invalid(format!(
                    "cannot inspect {label} path component: {error}"
                )));
            }
        }
    }
    Ok(())
}

/// The API-root-relative prefix for one immutable publication.  The commit is
/// validated again at this boundary even though the run row was validated
/// before execution; this keeps path construction fail-closed if a future
/// caller bypasses that earlier check.
fn validate_immutable_publication_identity(
    code_commit: &str,
    publication_id: Uuid,
) -> Result<(), PublishError> {
    if !is_exact_code_commit(code_commit) {
        return Err(PublishError::Invalid(
            "immutable publication code commit is not an exact lowercase 40-hex Git commit"
                .to_owned(),
        ));
    }
    if publication_id.is_nil() {
        return Err(PublishError::Invalid(
            "immutable publication UUID must not be nil".to_owned(),
        ));
    }
    Ok(())
}

fn immutable_publication_prefix(
    run_id: Uuid,
    code_commit: &str,
    publication_id: Uuid,
) -> Result<String, PublishError> {
    validate_immutable_publication_identity(code_commit, publication_id)?;
    Ok(format!(
        "backtest/runs/{run_id}/generations/{code_commit}/{publication_id}/artifacts"
    ))
}

/// Filesystem destination for one immutable publication.  It is deliberately
/// nested below `generations/<validated commit>/<unique UUID>` rather than the
/// mutable `<run>/artifacts` directory used by pre-isolation runners.
fn immutable_publication_artifacts(
    run_dir: &Path,
    code_commit: &str,
    publication_id: Uuid,
) -> Result<PathBuf, PublishError> {
    validate_immutable_publication_identity(code_commit, publication_id)?;
    Ok(run_dir
        .join("generations")
        .join(code_commit)
        .join(publication_id.to_string())
        .join("artifacts"))
}

/// Atomically promote one validated attempt's artifact directory into a fresh
/// immutable generation. The destination is never removed or replaced: a
/// UUID collision, stale directory, or symlink is an integrity failure. The
/// caller creates this path *before* its DB transaction, and cleans exactly
/// this path if that transaction rolls back.
fn promote_attempt_artifacts(source: &Path, destination: &Path) -> Result<(), PublishError> {
    let source_metadata = std::fs::symlink_metadata(source).map_err(|error| {
        PublishError::Invalid(format!("attempt artifacts are unavailable: {error}"))
    })?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
        return Err(PublishError::Invalid(
            "attempt artifact root must be a regular directory".to_owned(),
        ));
    }
    let publication_parent = destination.parent().ok_or_else(|| {
        PublishError::Invalid("immutable publication has no parent directory".to_owned())
    })?;
    let publication_id = publication_parent
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            PublishError::Invalid("immutable publication parent is not a UUID directory".to_owned())
        })
        .and_then(|value| {
            Uuid::parse_str(value).map_err(|_| {
                PublishError::Invalid(
                    "immutable publication parent is not a UUID directory".to_owned(),
                )
            })
        })?;
    let commit_dir = publication_parent.parent().ok_or_else(|| {
        PublishError::Invalid("immutable publication has no commit directory".to_owned())
    })?;
    let generations_dir = commit_dir.parent().ok_or_else(|| {
        PublishError::Invalid("immutable publication has no generations directory".to_owned())
    })?;
    let run_dir = generations_dir.parent().ok_or_else(|| {
        PublishError::Invalid("immutable publication has no run directory".to_owned())
    })?;
    if generations_dir.file_name().and_then(|name| name.to_str()) != Some("generations") {
        return Err(PublishError::Invalid(
            "immutable publication is outside the generations directory".to_owned(),
        ));
    }
    let code_commit = commit_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            PublishError::Invalid(
                "immutable publication commit directory is not valid UTF-8".to_owned(),
            )
        })?;
    validate_immutable_publication_identity(code_commit, publication_id)?;
    ensure_no_symlink_ancestors(run_dir, "immutable publication")?;
    ensure_directory_component(run_dir, "run directory")?;
    ensure_directory_component(generations_dir, "generations directory")?;
    ensure_directory_component(commit_dir, "commit generation directory")?;
    // The publication directory itself is the no-reuse boundary. An existing
    // UUID directory is rejected even when empty; retries receive a new UUID
    // rather than reusing a prior durable name.
    match std::fs::symlink_metadata(publication_parent) {
        Ok(_) => {
            return Err(PublishError::Invalid(
                "immutable publication directory already exists".to_owned(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(publication_parent).map_err(|error| {
                PublishError::Invalid(format!("cannot create publication directory: {error}"))
            })?;
        }
        Err(error) => {
            return Err(PublishError::Invalid(format!(
                "cannot inspect publication directory: {error}"
            )));
        }
    }
    let metadata = match std::fs::symlink_metadata(publication_parent) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = std::fs::remove_dir(publication_parent);
            return Err(PublishError::Invalid(format!(
                "cannot inspect publication directory: {error}"
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        let _ = std::fs::remove_dir(publication_parent);
        return Err(PublishError::Invalid(
            "publication directory must be a regular directory".to_owned(),
        ));
    }
    if std::fs::symlink_metadata(destination).is_ok() {
        let _ = std::fs::remove_dir(publication_parent);
        return Err(PublishError::Invalid(
            "immutable publication artifact directory already exists".to_owned(),
        ));
    }
    // Both paths are below the configured artifact root, so rename is an
    // atomic same-filesystem directory move. Parquet bytes are not copied or
    // rewritten after their manifest hashes have been validated.
    match std::fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_dir(publication_parent);
            Err(PublishError::Invalid(format!(
                "cannot promote immutable artifacts: {error}"
            )))
        }
    }
}

/// Remove a failed immutable publication and its now-empty UUID directory.
/// Commit-generation and run directories are shared by other publications,
/// so they are never recursively removed.
fn cleanup_publication_artifacts(path: &Path) -> Result<(), PublishError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => remove_path_safely(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(PublishError::Invalid(format!(
                "cannot inspect failed publication: {error}"
            )));
        }
    }
    if let Some(parent) = path.parent() {
        match std::fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(PublishError::Invalid(
                    "publication parent is not a regular directory".to_owned(),
                ));
            }
            Ok(_) => match std::fs::remove_dir(parent) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                // A concurrent diagnostic file does not make the publication
                // bytes unsafe; leave the non-empty UUID directory for the
                // documented reconciler rather than deleting anything else.
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) => {
                    return Err(PublishError::Invalid(format!(
                        "cannot remove empty publication parent: {error}"
                    )));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(PublishError::Invalid(format!(
                    "cannot inspect publication parent: {error}"
                )));
            }
        }
    }
    Ok(())
}

/// Synchronous fallback for a timeout/cancellation that drops the async
/// publication future after filesystem promotion but before its explicit
/// rollback branch runs. A COMMIT error is marked disarmed by the caller,
/// because its outcome is ambiguous and the bytes must remain for
/// reconciliation.
struct PublicationCleanupGuard {
    path: Option<PathBuf>,
}

impl PublicationCleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }

    fn arm(&mut self, path: PathBuf) {
        self.path = Some(path);
    }
}

impl Drop for PublicationCleanupGuard {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Err(error) = cleanup_publication_artifacts(&path) {
            // There is no error channel from Drop. The runbook's reconciler
            // will inspect unreferenced generations; keep this diagnostic
            // explicit instead of silently turning a cleanup failure into an
            // apparent successful publication.
            eprintln!("backtest publication cleanup deferred: {error:?}");
        }
    }
}

/// Leave a durable diagnostic beside an ambiguous generation. The marker is
/// metadata, not part of `artifacts/`, so preserving it never rewrites or
/// invalidates the immutable result bytes. Failure to write it does not turn a
/// COMMIT-unknown outcome into a cleanup-eligible one; the reconciler still
/// retains the generation until its grace period expires.
fn write_commit_unknown_marker(
    publication_artifacts: &Path,
    run_id: Uuid,
    job_id: Uuid,
    detail: &str,
) -> Result<PathBuf, String> {
    let publication_dir = publication_artifacts
        .parent()
        .ok_or_else(|| "ambiguous publication has no generation parent".to_owned())?;
    let metadata = std::fs::symlink_metadata(publication_dir)
        .map_err(|error| format!("cannot inspect ambiguous generation: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("ambiguous generation parent is not a regular directory".to_owned());
    }
    let marker = publication_dir.join("COMMIT_UNKNOWN.json");
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("ambiguous generation marker is not a regular file".to_owned());
        }
        Ok(_) => return Ok(marker),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect ambiguous marker: {error}")),
    }
    let temporary = publication_dir.join("COMMIT_UNKNOWN.json.tmp");
    let body = serde_json::json!({
        "event_code": BACKTEST_COMMIT_UNKNOWN_EVENT_CODE,
        "run_id": run_id,
        "job_id": job_id,
        "generation_path": publication_artifacts,
        "detail": detail,
    });
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&body).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write ambiguous generation marker: {error}"))?;
    if let Err(error) = std::fs::rename(&temporary, &marker) {
        let _ = std::fs::remove_file(&temporary);
        // A concurrent marker is fine; it is the same durable diagnostic.
        if std::fs::symlink_metadata(&marker).is_err() {
            return Err(format!(
                "cannot publish ambiguous generation marker: {error}"
            ));
        }
    }
    eprintln!(
        "event_code={} run_id={} job_id={} generation_path={} marker_path={}",
        BACKTEST_COMMIT_UNKNOWN_EVENT_CODE,
        run_id,
        job_id,
        publication_artifacts.display(),
        marker.display()
    );
    Ok(marker)
}

async fn publish_success(
    queue: &JobQueue,
    claim: &ClaimedJob,
    canonical: &CanonicalRun,
    execution: &WorkerExecution,
    guard: &LeaseGuard,
    control: &RunnerControl,
) -> Result<SettleResult, PublishError> {
    check_publication_guard(guard, control)?;
    // The worker's result is read exclusively from this claim's private
    // namespace. The canonical run directory is never read; doing so would
    // allow an expired attempt or a legacy writer's stale manifest/parquet to
    // satisfy a retry.
    let manifest_path = execution.attempt_dir.join("artifacts/manifest.json");
    let artifact_root = execution.attempt_dir.join("artifacts");
    let manifest = run_bounded_blocking(
        control,
        guard,
        DB_OPERATION_DEADLINE,
        "reading and validating backtest manifest",
        {
            let canonical = canonical.clone();
            let claim = claim.clone();
            move |cancellation| {
                read_and_validate_manifest(
                    &manifest_path,
                    &artifact_root,
                    &canonical,
                    &claim,
                    cancellation,
                )
            }
        },
    )
    .await
    .map_err(|error| match error {
        BlockingWorkError::Operation(error) => check_publication_guard(guard, control)
            .err()
            .unwrap_or(error),
        BlockingWorkError::Shutdown(reason) => PublishError::Shutdown(reason),
        BlockingWorkError::Canceled(reason) => PublishError::Canceled(reason),
        BlockingWorkError::LeaseLost(reason) => PublishError::LeaseLost(reason),
        BlockingWorkError::Timeout(reason) | BlockingWorkError::Join(reason) => {
            PublishError::Database(QueueError::Internal(reason))
        }
    })?;
    check_publication_guard(guard, control)?;
    // Promotion happens before the DB transaction. The destination is a
    // fresh, commit- and UUID-qualified directory that no pre-isolation
    // runner knows how to address. A transaction failure therefore cannot
    // roll back the rename in place; the exact generation is removed below.
    let publication_artifacts = immutable_publication_artifacts(
        &execution.run_dir,
        &execution.publication_code_commit,
        execution.publication_id,
    )?;
    promote_attempt_artifacts(
        &execution.attempt_dir.join("artifacts"),
        &publication_artifacts,
    )?;
    let mut cleanup = PublicationCleanupGuard::new(publication_artifacts.clone());
    check_publication_guard(guard, control)?;
    let publication_prefix = immutable_publication_prefix(
        canonical.id,
        &execution.publication_code_commit,
        execution.publication_id,
    )?;
    let result = publish_success_transaction(
        queue,
        claim,
        canonical,
        &manifest,
        &publication_prefix,
        &mut cleanup,
        guard,
        control,
    )
    .await;
    match result {
        Ok(SettleResult::Committed(job)) => {
            cleanup.disarm();
            Ok(SettleResult::Committed(job))
        }
        Ok(SettleResult::Canceled(job)) => {
            cleanup.arm(publication_artifacts.clone());
            cleanup_publication_artifacts(&publication_artifacts)?;
            cleanup.disarm();
            Ok(SettleResult::Canceled(job))
        }
        Err(PublishError::CommitUnknown(error)) => {
            // A COMMIT response error is ambiguous. Retaining the bytes is
            // safer than deleting a generation a committed DB row may now
            // reference; reconciliation handles both outcomes.
            cleanup.disarm();
            let detail = error.to_string();
            if let Err(marker_error) = write_commit_unknown_marker(
                &publication_artifacts,
                canonical.id,
                claim.job.id,
                &detail,
            ) {
                eprintln!(
                    "event_code={} run_id={} job_id={} generation_path={} marker_error={}",
                    BACKTEST_COMMIT_UNKNOWN_EVENT_CODE,
                    canonical.id,
                    claim.job.id,
                    publication_artifacts.display(),
                    marker_error
                );
            }
            Err(PublishError::CommitUnknown(QueueError::CommitUnknown {
                run_id: canonical.id,
                job_id: claim.job.id,
                generation_path: publication_artifacts.display().to_string(),
                detail,
            }))
        }
        Err(error) => {
            match cleanup_publication_artifacts(&publication_artifacts) {
                Ok(()) => cleanup.disarm(),
                Err(cleanup_error) => {
                    return Err(PublishError::Database(QueueError::Internal(format!(
                        "publication failed ({error:?}) and cleanup failed ({cleanup_error:?})"
                    ))));
                }
            }
            Err(error)
        }
    }
}

/// Settle queue/run state and insert all manifest rows while the caller owns
/// one SQL transaction. The filesystem promotion has already happened into a
/// fresh immutable generation; every error before COMMIT rolls this
/// transaction back and lets the caller remove that generation.
#[allow(clippy::too_many_arguments)]
async fn publish_success_transaction(
    queue: &JobQueue,
    claim: &ClaimedJob,
    canonical: &CanonicalRun,
    manifest: &PublishedManifest,
    publication_prefix: &str,
    cleanup: &mut PublicationCleanupGuard,
    guard: &LeaseGuard,
    control: &RunnerControl,
) -> Result<SettleResult, PublishError> {
    check_publication_guard(guard, control)?;
    let mut tx = queue.begin().await.map_err(PublishError::Database)?;
    // Lock/settle the exact queue attempt before touching result rows.  A
    // cancel that wins here returns CANCELED and never exposes a successful
    // result; any later validation, promotion, or DB error rolls both changes
    // back.  Holding this row lock is what prevents an expired old attempt
    // from promoting while a retry publishes.
    check_publication_guard(guard, control)?;
    let settled = queue
        .settle_success_in(&mut tx, claim)
        .await
        .map_err(|error| match error {
            QueueError::StaleClaim(_) => {
                PublishError::LeaseLost("backtest lease was lost before publication".to_owned())
            }
            other => PublishError::Database(other),
        })?;
    let status = match &settled {
        SettleResult::Canceled(_) => "CANCELED",
        SettleResult::Committed(_) => "SUCCEEDED",
    };
    if status == "SUCCEEDED" {
        // The manifest was validated against the pre-child snapshot; this
        // second read is the authorization check immediately before DB
        // publication. It also locks the run row for this transaction.
        recheck_canonical_provenance(&mut tx, canonical, claim, guard, control).await?;
    }
    let summary = if status == "SUCCEEDED" {
        &manifest.run.summary_json
    } else {
        &serde_json::json!({"error_code": "CANCELED", "error_message": "backtest canceled"})
    };
    guard.check().map_err(PublishError::LeaseLost)?;
    let rows = sqlx::query(
        "UPDATE backtest_runs
         SET status = $4, summary_json = $5, finished_at = now()
         WHERE id = $1 AND owner_user_id = $2 AND job_id = $3 AND status = 'RUNNING'",
    )
    .bind(canonical.id)
    .bind(claim.job.owner_user_id)
    .bind(claim.job.id)
    .bind(status)
    .bind(summary)
    .execute(&mut *tx)
    .await
    .map_err(|error| PublishError::Database(QueueError::Database(error)))?;
    if rows.rows_affected() != 1 {
        return Err(PublishError::Invalid(format!(
            "backtest run {} was not RUNNING for publication",
            canonical.id
        )));
    }
    if status == "SUCCEEDED" {
        for (key, value) in &manifest.metrics {
            check_publication_guard(guard, control)?;
            sqlx::query(
                "INSERT INTO backtest_metrics
                    (backtest_run_id, owner_user_id, metric_key, metric_value)
                 VALUES ($1, $2, $3, $4::numeric)",
            )
            .bind(canonical.id)
            .bind(claim.job.owner_user_id)
            .bind(key)
            .bind(value.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|error| PublishError::Database(QueueError::Database(error)))?;
        }
        for warning in &manifest.warnings {
            check_publication_guard(guard, control)?;
            sqlx::query(
                "INSERT INTO backtest_warnings
                    (backtest_run_id, owner_user_id, warning_code, message)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(canonical.id)
            .bind(claim.job.owner_user_id)
            .bind(&warning.code)
            .bind(&warning.message)
            .execute(&mut *tx)
            .await
            .map_err(|error| PublishError::Database(QueueError::Database(error)))?;
        }
        for artifact in &manifest.artifacts {
            check_publication_guard(guard, control)?;
            sqlx::query(
                "INSERT INTO result_artifacts
                    (backtest_run_id, owner_user_id, artifact_type, parquet_path,
                     row_count, sha256, size_bytes, summary_json)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(canonical.id)
            .bind(claim.job.owner_user_id)
            .bind(&artifact.artifact_type)
            // The worker is only allowed to name a basename inside its
            // private artifact directory. Persist the API-root-relative path
            // to the immutable generation, never the worker's local path or
            // the mutable legacy `<run>/artifacts` namespace.
            .bind(format!("{publication_prefix}/{}", artifact.parquet_path))
            .bind(artifact.row_count)
            .bind(&artifact.sha256)
            .bind(artifact.size_bytes)
            .bind(&artifact.summary_json)
            .execute(&mut *tx)
            .await
            .map_err(|error| PublishError::Database(QueueError::Database(error)))?;
        }
    }
    check_publication_guard(guard, control)?;
    // From the instant COMMIT starts, cancellation can drop this future while
    // PostgreSQL's outcome is still unknown. Retain the generation across
    // that boundary; deleting it here could race a committed DB row. A
    // normal canceled settlement is explicitly cleaned by the caller after
    // COMMIT returns, and an interrupted/ambiguous one is reconciled later.
    cleanup.disarm();
    match tx.commit().await {
        Ok(()) => Ok(settled),
        Err(error) => {
            // COMMIT outcome is unknown; retaining bytes is the only safe
            // choice until reconciliation can inspect the DB.
            cleanup.disarm();
            Err(PublishError::CommitUnknown(QueueError::Database(error)))
        }
    }
}

fn read_and_validate_manifest(
    manifest_path: &Path,
    artifact_root: &Path,
    canonical: &CanonicalRun,
    claim: &ClaimedJob,
    cancellation: BlockingCancellation,
) -> Result<PublishedManifest, PublishError> {
    if cancellation.is_canceled() {
        return Err(PublishError::Canceled(canceled_blocking("manifest")));
    }
    let artifact_root_metadata = std::fs::symlink_metadata(artifact_root)
        .map_err(|error| PublishError::Invalid(format!("artifact root is unavailable: {error}")))?;
    if artifact_root_metadata.file_type().is_symlink() || !artifact_root_metadata.is_dir() {
        return Err(PublishError::Invalid(
            "artifact root must be a regular directory".to_owned(),
        ));
    }
    let manifest_metadata = std::fs::symlink_metadata(manifest_path).map_err(|error| {
        PublishError::Invalid(format!("manifest is missing or unreadable: {error}"))
    })?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return Err(PublishError::Invalid(
            "manifest must be a regular non-symlink file".to_owned(),
        ));
    }
    let root_canonical = artifact_root
        .canonicalize()
        .map_err(|error| PublishError::Invalid(format!("artifact root is unavailable: {error}")))?;
    let manifest_canonical = manifest_path
        .canonicalize()
        .map_err(|error| PublishError::Invalid(format!("manifest is unavailable: {error}")))?;
    if !manifest_canonical.starts_with(&root_canonical) {
        return Err(PublishError::Invalid(
            "manifest resolves outside the current attempt artifact root".to_owned(),
        ));
    }
    let raw = read_manifest_text(manifest_path, &cancellation)?;
    if cancellation.is_canceled() {
        return Err(PublishError::Canceled(canceled_blocking("manifest")));
    }
    let manifest: PublishedManifest = serde_json::from_str(&raw)
        .map_err(|error| PublishError::Invalid(format!("manifest is invalid JSON: {error}")))?;
    if cancellation.is_canceled() {
        return Err(PublishError::Canceled(canceled_blocking("manifest")));
    }
    validate_manifest_with_cancel(
        &manifest,
        canonical,
        claim,
        artifact_root,
        Some(&cancellation),
    )?;
    Ok(manifest)
}

fn read_manifest_text(
    path: &Path,
    cancellation: &BlockingCancellation,
) -> Result<String, PublishError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|error| {
        PublishError::Invalid(format!("manifest is missing or unreadable: {error}"))
    })?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_canceled() {
            return Err(PublishError::Canceled(canceled_blocking("manifest")));
        }
        let read = file.read(&mut buffer).map_err(|error| {
            PublishError::Invalid(format!("manifest is missing or unreadable: {error}"))
        })?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > MAX_MANIFEST_BYTES as usize {
            return Err(PublishError::Invalid(format!(
                "manifest is larger than the {} byte limit",
                MAX_MANIFEST_BYTES
            )));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes)
        .map_err(|error| PublishError::Invalid(format!("manifest is not valid UTF-8: {error}")))
}

#[cfg(test)]
fn validate_manifest(
    manifest: &PublishedManifest,
    canonical: &CanonicalRun,
    claim: &ClaimedJob,
    artifact_root: &Path,
) -> Result<(), PublishError> {
    validate_manifest_with_cancel(manifest, canonical, claim, artifact_root, None)
}

fn validate_manifest_with_cancel(
    manifest: &PublishedManifest,
    canonical: &CanonicalRun,
    claim: &ClaimedJob,
    artifact_root: &Path,
    cancellation: Option<&BlockingCancellation>,
) -> Result<(), PublishError> {
    check_manifest_cancellation(cancellation)?;
    let run = &manifest.run;
    let expected_job = canonical
        .job_id
        .ok_or_else(|| PublishError::Invalid("backtest run has no job identity".to_owned()))?;
    if run.status != "SUCCEEDED"
        || run.id != canonical.id
        || canonical.owner_user_id != claim.job.owner_user_id
        || run.owner_user_id != claim.job.owner_user_id
        || run.job_id != Some(expected_job)
        || run.strategy_id != canonical.strategy_id
        || run.strategy_version != canonical.strategy_version
        || run.dataset_version != canonical.dataset_version
        || run.engine != canonical.engine
        || run.engine_version != canonical.engine_version
        || run.config_sha256 != canonical.config_sha256
        || run.code_commit != canonical.code_commit
        || run.random_seed != canonical.random_seed
        || run.timezone != canonical.timezone
    {
        return Err(PublishError::Invalid(
            "manifest provenance or owner/job identity does not match the pre-created run"
                .to_owned(),
        ));
    }
    let mut seen = HashSet::new();
    for artifact in &manifest.artifacts {
        check_manifest_cancellation(cancellation)?;
        if !EXPECTED_ARTIFACT_TYPES.contains(&artifact.artifact_type.as_str())
            || !seen.insert(artifact.artifact_type.as_str())
        {
            return Err(PublishError::Invalid(format!(
                "manifest has missing or duplicate artifact type {:?}",
                artifact.artifact_type
            )));
        }
        if artifact.row_count < 0
            || artifact.size_bytes < 0
            || !is_sha256_hex(&artifact.sha256)
            || artifact.parquet_path.is_empty()
        {
            return Err(PublishError::Invalid(format!(
                "artifact {:?} has invalid metadata",
                artifact.artifact_type
            )));
        }
        if !is_worker_artifact_basename(&artifact.parquet_path) {
            return Err(PublishError::Invalid(format!(
                "artifact path {:?} must be a worker basename",
                artifact.parquet_path
            )));
        }
        let relative = Path::new(&artifact.parquet_path);
        let path = artifact_root.join(relative);
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            PublishError::Invalid(format!("artifact {:?} is unavailable: {error}", path))
        })?;
        check_manifest_cancellation(cancellation)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PublishError::Invalid(format!(
                "artifact {:?} must be a regular non-symlink file",
                artifact.parquet_path
            )));
        }
        let root_canonical = artifact_root.canonicalize().map_err(|error| {
            PublishError::Invalid(format!("artifact root is unavailable: {error}"))
        })?;
        let path_canonical = path.canonicalize().map_err(|error| {
            PublishError::Invalid(format!("artifact {:?} is unavailable: {error}", path))
        })?;
        check_manifest_cancellation(cancellation)?;
        if !path_canonical.starts_with(&root_canonical) {
            return Err(PublishError::Invalid(format!(
                "artifact {:?} resolves outside the artifact root",
                artifact.parquet_path
            )));
        }
        if metadata.len() != artifact.size_bytes as u64 {
            return Err(PublishError::Invalid(format!(
                "artifact {:?} size does not match manifest",
                artifact.parquet_path
            )));
        }
        let digest = match cancellation {
            Some(cancellation) => sha256_file_with_cancel(&path, cancellation)?,
            None => sha256_file(&path).map_err(|error| {
                PublishError::Invalid(format!("artifact {:?} cannot be hashed: {error}", path))
            })?,
        };
        check_manifest_cancellation(cancellation)?;
        if digest != artifact.sha256 {
            return Err(PublishError::Invalid(format!(
                "artifact {:?} sha256 does not match manifest",
                artifact.parquet_path
            )));
        }
    }
    check_manifest_cancellation(cancellation)?;
    if seen.len() != EXPECTED_ARTIFACT_TYPES.len() {
        return Err(PublishError::Invalid(format!(
            "manifest declares {} artifacts, expected {}",
            seen.len(),
            EXPECTED_ARTIFACT_TYPES.len()
        )));
    }
    if manifest
        .warnings
        .iter()
        .any(|warning| warning.code.trim().is_empty())
    {
        return Err(PublishError::Invalid(
            "manifest warning code must not be empty".to_owned(),
        ));
    }
    Ok(())
}

fn check_manifest_cancellation(
    cancellation: Option<&BlockingCancellation>,
) -> Result<(), PublishError> {
    if cancellation.is_some_and(BlockingCancellation::is_canceled) {
        Err(PublishError::Canceled(canceled_blocking("manifest")))
    } else {
        Ok(())
    }
}

/// Worker manifests name files relative to `<run>/artifacts`, not API-root
/// paths. Requiring one normal component also rejects Unix traversal, Windows
/// separators, absolute paths, and hidden path tricks before any filesystem
/// access or database publication occurs.
fn is_worker_artifact_basename(value: &str) -> bool {
    if value.is_empty() || value.contains(['/', '\\']) {
        return false;
    }
    let path = Path::new(value);
    matches!(
        (path.components().count(), path.components().next()),
        (1, Some(std::path::Component::Normal(_)))
    )
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_json(value: &serde_json::Value) -> String {
    let mut digest = Sha256::new();
    digest.update(value.to_string().as_bytes());
    format!("{:x}", digest.finalize())
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_file_with_cancel(
    path: &Path,
    cancellation: &BlockingCancellation,
) -> Result<String, PublishError> {
    use std::io::Read;
    if cancellation.is_canceled() {
        return Err(PublishError::Canceled(canceled_blocking("manifest")));
    }
    let mut file = std::fs::File::open(path).map_err(|error| {
        PublishError::Invalid(format!("artifact {:?} cannot be hashed: {error}", path))
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        if cancellation.is_canceled() {
            return Err(PublishError::Canceled(canceled_blocking("manifest")));
        }
        let read = file.read(&mut buffer).map_err(|error| {
            PublishError::Invalid(format!("artifact {:?} cannot be hashed: {error}", path))
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// The values needed to construct `request.json`.  Keeping this owned bundle
/// lets the whole JSON build/serialize/write sequence run in one bounded
/// blocking closure rather than serializing a potentially large factor series
/// while polling an async future.
struct WorkerRequestInput {
    run_id: String,
    owner_user_id: Uuid,
    job_id: Uuid,
    strategy_path: String,
    config_path: String,
    strategy_config: serde_json::Value,
    strategy_id: String,
    strategy_version: String,
    dataset_version: String,
    dataset_path: PathBuf,
    engine: String,
    engine_version: String,
    code_commit: String,
    random_seed: i32,
    timezone: String,
    currency: String,
    config_sha256: String,
    initial_cash: String,
    cost_profile: serde_json::Value,
    factor_series: Option<crate::factor_series::FactorSeries>,
    window: Option<(String, String)>,
}

fn canceled_blocking(label: &'static str) -> String {
    format!("{label} canceled")
}

/// Establish a fresh namespace for one claimed attempt before any child is
/// started.
///
/// This function is called from a cancelable `spawn_blocking` closure.  It is
/// therefore deliberately limited to the UUID-owned attempt namespace and
/// its private scratch directories.  In particular, it must never inspect,
/// remove, or otherwise mutate `<run>/artifacts` or a legacy `<run>/status.json`:
/// a closure that outlives its canceled caller could otherwise erase the
/// result that a retry has already promoted.  Legacy cleanup belongs to the
/// lease/transaction-serialized publication path below.
fn prepare_attempt_namespace(
    run_dir: &Path,
    attempt_dir: &Path,
    cancellation: &BlockingCancellation,
) -> Result<(), String> {
    if cancellation.is_canceled() {
        return Err(canceled_blocking("attempt preparation"));
    }
    let attempts_root = attempt_dir
        .parent()
        .ok_or_else(|| "attempt namespace has no parent".to_owned())?;
    let expected_attempts_root = run_dir.join(".attempts");
    if attempts_root != expected_attempts_root {
        return Err("attempt namespace is outside this run's attempt root".to_owned());
    }
    let attempt_name = attempt_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "attempt namespace is not a UUID directory".to_owned())?;
    Uuid::parse_str(attempt_name)
        .map_err(|_| "attempt namespace is not a UUID directory".to_owned())?;
    std::fs::create_dir_all(attempts_root)
        .map_err(|error| format!("cannot create attempt root: {error}"))?;

    if cancellation.is_canceled() {
        return Err(canceled_blocking("attempt preparation"));
    }
    // A claim UUID is never reused.  Refusing an existing namespace makes a
    // stale same-attempt directory fail closed instead of mixing old request,
    // status, or artifact files into a new child invocation.
    std::fs::create_dir(attempt_dir)
        .map_err(|error| format!("cannot create fresh attempt namespace: {error}"))?;
    for name in ["scratch", "tmp", "home", "uv-cache"] {
        std::fs::create_dir_all(attempt_dir.join(name))
            .map_err(|error| format!("cannot create attempt {name} directory: {error}"))?;
    }
    Ok(())
}

fn write_worker_request(
    attempt_dir: &Path,
    input: WorkerRequestInput,
    cancellation: BlockingCancellation,
) -> Result<(PathBuf, PathBuf), String> {
    write_worker_request_with_hook(attempt_dir, input, cancellation, || {})
}

/// Build a request in an attempt-private directory and hand it to the child
/// only after the complete file has been written.  The hook is a test seam for
/// the otherwise impossible-to-schedule check/rename window; production uses
/// the no-op wrapper above.
fn write_worker_request_with_hook<F: FnOnce()>(
    attempt_dir: &Path,
    input: WorkerRequestInput,
    cancellation: BlockingCancellation,
    before_publish: F,
) -> Result<(PathBuf, PathBuf), String> {
    if cancellation.is_canceled() {
        return Err(canceled_blocking("worker request"));
    }

    let mut request = serde_json::json!({
        "run_id": input.run_id,
        "owner_user_id": input.owner_user_id.to_string(),
        "job_id": input.job_id.to_string(),
        // From the REGISTRY, never from the request body.
        "strategy_path": input.strategy_path,
        "strategy_config_class": input.config_path,
        "strategy_config": input.strategy_config,
        "strategy_id": input.strategy_id,
        "strategy_version": input.strategy_version,
        // Every provenance value below comes from the pre-created run row.
        "dataset_version": input.dataset_version,
        "dataset_path": input.dataset_path.to_string_lossy(),
        "engine": input.engine,
        "engine_version": input.engine_version,
        "code_commit": input.code_commit,
        "random_seed": input.random_seed,
        "timezone": input.timezone,
        "currency": input.currency,
        "config_sha256": format!("sha256:{}", input.config_sha256),
        "slippage_bps": 10,
        "initial_cash": input.initial_cash,
        "limits": {
            "memory_bytes": 2_147_483_648_u64,
            "cpu_seconds": null,
            "wall_seconds": 300,
            "active_processes": 1,
            // The simulation reads local files and must reach nothing else. A
            // strategy that could open a socket could exfiltrate the dataset
            // this project has a written-rights obligation over.
            "network_disabled": true,
        },
        "readonly_mounts": [],
        // What a fill is charged. Resolved from the versioned profile rather
        // than assembled here, so a backtest's fees are the ones the paper
        // and live ledgers would charge for the same trade.
        "cost_profile": input.cost_profile,
    });

    // Attached only when the strategy needs it, so a request for a strategy
    // that computes its own signal is unchanged.
    if let Some(series) = input.factor_series {
        if cancellation.is_canceled() {
            return Err(canceled_blocking("worker request"));
        }
        request["factor_series"] = serde_json::to_value(series)
            .map_err(|error| format!("cannot serialise factors: {error}"))?;
    }
    // Same shape as `factor_series` above, and for the same reason: a payload
    // without a window must produce the request it produced before this field
    // existed, byte for byte. Writing `"start_date": null` unconditionally
    // would change phase-0's approved request.json without changing anything
    // about phase-0.
    if let Some((start, end)) = input.window {
        request["start_date"] = serde_json::Value::String(start);
        request["end_date"] = serde_json::Value::String(end);
    }

    if cancellation.is_canceled() {
        return Err(canceled_blocking("worker request"));
    }
    std::fs::create_dir_all(attempt_dir)
        .map_err(|error| format!("cannot create {attempt_dir:?}: {error}"))?;
    let request_path = attempt_dir.join("request.json");
    let temporary_path = attempt_dir.join("request.json.tmp");
    let bytes = serde_json::to_vec_pretty(&request)
        .map_err(|error| format!("cannot serialise request: {error}"))?;
    if cancellation.is_canceled() {
        return Err(canceled_blocking("worker request"));
    }
    // Rename only after the complete request is written.  If cancellation is
    // observed after the write, remove the temporary file and never expose a
    // partial request to a child that was not started.
    std::fs::write(&temporary_path, bytes)
        .map_err(|error| format!("cannot write request: {error}"))?;
    if cancellation.is_canceled() {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(canceled_blocking("worker request"));
    }

    // This is the final check before publication. The test hook deliberately
    // pauses immediately after it, making the check-to-rename race
    // deterministic without changing production timing.
    if cancellation.is_canceled() {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(canceled_blocking("worker request"));
    }
    // The destination is attempt-private. Even if cancellation wins while
    // this hook is paused (or while rename is in progress), a late task can
    // only touch this stale namespace; it cannot overwrite a retry's request
    // or a canonical published path.
    before_publish();
    std::fs::rename(&temporary_path, &request_path)
        .map_err(|error| format!("cannot publish request file: {error}"))?;

    // Cancellation can arrive immediately after the final pre-rename check.
    // Remove the attempt-private destination before returning in that case;
    // the caller will never start a child from a canceled request.
    if cancellation.is_canceled() {
        let _ = std::fs::remove_file(&request_path);
        return Err(canceled_blocking("worker request"));
    }
    // Status belongs to this attempt just like the request.  In particular,
    // never fall back to `<run>/status.json`: a previous expired attempt may
    // have left a canonical status there, and accepting it after a retry's
    // child exits non-zero would turn stale data into a successful run.
    let status_path = attempt_dir.join("status.json");
    Ok((request_path, status_path))
}

fn read_worker_status(
    status_path: &Path,
    cancellation: BlockingCancellation,
) -> Result<WorkerStatus, String> {
    if cancellation.is_canceled() {
        return Err(canceled_blocking("worker status"));
    }
    let raw = read_status_text(status_path, &cancellation)?;
    if cancellation.is_canceled() {
        return Err(canceled_blocking("worker status"));
    }
    serde_json::from_str(&raw).map_err(|error| format!("worker status is not JSON: {error}"))
}

fn validate_worker_exit_status(
    process_succeeded: bool,
    status: &WorkerStatus,
) -> Result<(), String> {
    if !process_succeeded && status.state == "SUCCEEDED" {
        return Err("backtest worker exited non-zero but reported SUCCEEDED".to_owned());
    }
    if process_succeeded && status.state != "SUCCEEDED" {
        return Err(format!(
            "backtest worker exited successfully but reported {}",
            status.state
        ));
    }
    Ok(())
}

fn read_status_text(path: &Path, cancellation: &BlockingCancellation) -> Result<String, String> {
    use std::io::Read;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("worker wrote no readable status: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("worker status must be a regular non-symlink file".to_owned());
    }
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("worker wrote no readable status: {error}"))?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_canceled() {
            return Err(canceled_blocking("worker status"));
        }
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("worker wrote no readable status: {error}"))?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > MAX_STATUS_BYTES as usize {
            return Err(format!(
                "worker status exceeds the {} byte limit",
                MAX_STATUS_BYTES
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes).map_err(|error| format!("worker status is not UTF-8: {error}"))
}

/// Builds the worker request and runs it, returning the worker's own status.
async fn execute<R: StrategyResolver>(
    claim: &ClaimedJob,
    payload: &BacktestPayload,
    canonical: &CanonicalRun,
    paths: &RunnerPaths,
    resolver: &R,
    control: &RunnerControl,
    guard: &LeaseGuard,
) -> Result<WorkerExecution, ExecError> {
    check_execution_guard(guard, control)?;
    let config_id = payload
        .strategy_config_id
        .as_deref()
        .ok_or_else(|| ExecError::Permanent {
            class: ErrorClass::Input,
            code: "MALFORMED_PAYLOAD",
            reason: "payload names no strategy_config_id".to_string(),
        })?;
    // The owner comes from the CLAIMED JOB, never from the payload: a payload
    // that could name its own owner would reach any tenant's config.
    let strategy = await_controlled_with_guard(
        resolver.resolve(config_id, claim.job.owner_user_id),
        control,
        guard,
        PREPROCESSING_DEADLINE,
        "strategy resolution",
    )
    .await??;
    check_execution_guard(guard, control)?;
    if strategy.strategy_id != canonical.strategy_id
        || strategy.strategy_version != canonical.strategy_version
    {
        return Err(ExecError::Permanent {
            class: ErrorClass::Integrity,
            code: "STRATEGY_PROVENANCE_MISMATCH",
            reason: format!(
                "resolved strategy {}@{} does not match run {}@{}",
                strategy.strategy_id,
                strategy.strategy_version,
                canonical.strategy_id,
                canonical.strategy_version
            ),
        });
    }
    let config_for_hash = strategy.config.clone();
    let resolved_hash = run_bounded_blocking(
        control,
        guard,
        PREPROCESSING_DEADLINE,
        "hashing strategy config",
        move |cancellation| {
            if cancellation.is_canceled() {
                return Err("strategy config hashing canceled".to_owned());
            }
            let hash = sha256_json(&config_for_hash);
            if cancellation.is_canceled() {
                return Err("strategy config hashing canceled".to_owned());
            }
            Ok::<_, String>(hash)
        },
    )
    .await
    .map_err(|error| match error {
        BlockingWorkError::Operation(reason) => match check_execution_guard(guard, control) {
            Ok(()) => ExecError::Transient(reason),
            Err(error) => error,
        },
        BlockingWorkError::Shutdown(reason) => ExecError::Shutdown(reason),
        BlockingWorkError::Canceled(reason) => ExecError::Canceled(reason),
        BlockingWorkError::LeaseLost(reason) => ExecError::LeaseLost(reason),
        BlockingWorkError::Timeout(reason) | BlockingWorkError::Join(reason) => {
            ExecError::Transient(reason)
        }
    })?;
    if resolved_hash != canonical.config_sha256 {
        return Err(ExecError::Permanent {
            class: ErrorClass::Integrity,
            code: "STRATEGY_CONFIG_HASH_MISMATCH",
            reason: format!(
                "resolved strategy config hash {resolved_hash} does not match run {}",
                canonical.config_sha256
            ),
        });
    }
    let factor_series =
        factor_series_for(&strategy.strategy_id, &paths.dataset_root, control, guard).await?;
    check_execution_guard(guard, control)?;
    let initial_cash = payload
        .initial_cash
        .as_ref()
        .and_then(|cash| cash.get("amount"))
        .and_then(|amount| amount.as_str())
        .ok_or_else(|| ExecError::Permanent {
            class: ErrorClass::Input,
            code: "MALFORMED_PAYLOAD",
            reason: "payload initial_cash.amount is missing".to_owned(),
        })?;
    if initial_cash.trim().is_empty() {
        return Err(ExecError::Permanent {
            class: ErrorClass::Input,
            code: "MALFORMED_PAYLOAD",
            reason: "payload initial_cash.amount is empty".to_owned(),
        });
    }
    let currency = payload
        .initial_cash
        .as_ref()
        .and_then(|cash| cash.get("currency"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ExecError::Permanent {
            class: ErrorClass::Input,
            code: "MALFORMED_PAYLOAD",
            reason: "payload initial_cash.currency is missing".to_owned(),
        })?;

    let run_id = canonical.id.to_string();
    let run_dir = absolute_path(&paths.artifacts_root).join(&run_id);
    let attempt_dir = attempt_namespace(&run_dir, claim.attempt.id);
    // A fresh UUID is part of the durable publication path. It is distinct
    // from the queue attempt UUID so a future retry can never reuse a prior
    // generation directory, even if an interrupted attempt left bytes behind.
    let publication_id = Uuid::new_v4();
    let random_seed = canonical.random_seed.ok_or_else(|| ExecError::Permanent {
        class: ErrorClass::Integrity,
        code: "BACKTEST_RUN_SEED_MISSING",
        reason: format!("backtest run {} has no random seed", canonical.id),
    })?;
    let cost_profile = cost_profile_for(payload)?;
    let window = window_for(payload)?;

    // Everything from here down is the RUNNER's environment -- a full disk, an
    // unwritable directory, a missing interpreter. None of it is the job's
    // fault, so all of it is transient. Request JSON construction, factor
    // serialization, directory creation, and the file write all happen in a
    // bounded blocking task. The task has no queue/database handle, and the
    // guard is checked again before the child is started.
    check_execution_guard(guard, control)?;
    let request_input = WorkerRequestInput {
        run_id,
        owner_user_id: claim.job.owner_user_id,
        job_id: claim.job.id,
        strategy_path: strategy.strategy_path,
        config_path: strategy.config_path,
        strategy_config: strategy.config,
        strategy_id: strategy.strategy_id,
        strategy_version: strategy.strategy_version,
        dataset_version: canonical.dataset_version.clone(),
        dataset_path: absolute_path(&paths.dataset_root),
        engine: canonical.engine.clone(),
        engine_version: canonical.engine_version.clone(),
        code_commit: canonical.code_commit.clone(),
        random_seed,
        timezone: canonical.timezone.clone(),
        currency: currency.to_owned(),
        config_sha256: canonical.config_sha256.clone(),
        initial_cash: initial_cash.to_owned(),
        cost_profile,
        factor_series,
        window,
    };
    let request_dir = attempt_dir.clone();
    let run_dir_for_prepare = run_dir.clone();
    let attempt_dir_for_prepare = attempt_dir.clone();
    let (request_path, status_path) = run_bounded_blocking(
        control,
        guard,
        PREPROCESSING_DEADLINE,
        "writing worker request",
        move |cancellation| {
            prepare_attempt_namespace(
                &run_dir_for_prepare,
                &attempt_dir_for_prepare,
                &cancellation,
            )?;
            write_worker_request(&request_dir, request_input, cancellation)
        },
    )
    .await
    .map_err(|error| match error {
        BlockingWorkError::Operation(reason) => match check_execution_guard(guard, control) {
            Ok(()) => ExecError::Transient(reason),
            Err(error) => error,
        },
        BlockingWorkError::Shutdown(reason) => ExecError::Shutdown(reason),
        BlockingWorkError::Canceled(reason) => ExecError::Canceled(reason),
        BlockingWorkError::LeaseLost(reason) => ExecError::LeaseLost(reason),
        BlockingWorkError::Timeout(reason) | BlockingWorkError::Join(reason) => {
            ExecError::Transient(reason)
        }
    })?;
    check_execution_guard(guard, control)?;

    let process_succeeded = run_worker(
        paths,
        &attempt_dir,
        &request_path,
        &attempt_dir.join("artifacts"),
        &status_path,
        control,
        guard,
    )
    .await
    .map_err(|error| match error {
        WorkerRunError::Transient(reason) => ExecError::Transient(reason),
        WorkerRunError::Shutdown(reason) => ExecError::Shutdown(reason),
        WorkerRunError::Canceled(reason) => ExecError::Canceled(reason),
        WorkerRunError::LeaseLost(reason) => ExecError::LeaseLost(reason),
    })?;

    // The child has exited. Re-check ownership before consuming any status or
    // artifacts; an expired attempt may have been swept while the process was
    // winding down, in which case the retry owns a different namespace.
    check_execution_guard(guard, control)?;

    let status = run_bounded_blocking(
        control,
        guard,
        PREPROCESSING_DEADLINE,
        "reading worker status",
        move |cancellation| read_worker_status(&status_path, cancellation),
    )
    .await
    .map_err(|error| match error {
        BlockingWorkError::Operation(reason) => match check_execution_guard(guard, control) {
            Ok(()) => ExecError::Transient(reason),
            Err(error) => error,
        },
        BlockingWorkError::Shutdown(reason) => ExecError::Shutdown(reason),
        BlockingWorkError::Canceled(reason) => ExecError::Canceled(reason),
        BlockingWorkError::LeaseLost(reason) => ExecError::LeaseLost(reason),
        BlockingWorkError::Timeout(reason) | BlockingWorkError::Join(reason) => {
            ExecError::Transient(reason)
        }
    })?;
    validate_worker_exit_status(process_succeeded, &status).map_err(ExecError::Transient)?;
    Ok(WorkerExecution {
        status,
        run_dir,
        attempt_dir,
        process_succeeded,
        publication_id,
        publication_code_commit: paths.code_commit.clone(),
    })
}

/// The cost settings a fill is charged under, resolved for the worker.
///
/// The RATES are not restated here and must never be. `cost.rs` is explicit
/// about why — 세율과 수수료는 변경 가능하므로 코드 상수로 고정하지 않고 설정
/// 버전으로 관리한다 — so a rate written into a second place is a rate that
/// changes in one of them. What crosses into Python is the resolved profile:
/// numbers with a version attached, never a formula's inputs invented locally.
///
/// The identity travels with them so a stored result can say which profile
/// produced it. A backtest whose fees cannot be traced to a profile version
/// is not reproducible, and reproducibility is the whole point of pinning the
/// dataset, the seed, and the engine version alongside it.
/// The simulation period, validated, or `None` for the whole dataset.
///
/// What the window bounds is what the SIMULATION SEES -- the bars fed to the
/// engine. It deliberately does not bound the factor series, which stays
/// full-history: a 200-day trend factor on the window's first day needs the
/// 200 days before it, and a factor series cut to the window would be null
/// exactly where the strategy first wants to trade.
///
/// The consequence is worth stating plainly, because it is a real limit and
/// not an oversight: a strategy that derives its own signal from bars instead
/// of from the factor series gets no warm-up either, and sees exactly the
/// requested window. Loading warm-up bars while suppressing pre-window trading
/// would need the fills, the equity curve's first point, and the benchmark's
/// base all policed against a second date -- much more machinery than the
/// honest simple rule earns.
///
/// Both halves must parse and be ordered. A payload row is written once at
/// submit time, so neither failure can be fixed by retrying it.
fn window_for(payload: &BacktestPayload) -> Result<Option<(String, String)>, ExecError> {
    let malformed = |reason: String| ExecError::Permanent {
        class: ErrorClass::Input,
        code: "MALFORMED_PAYLOAD",
        reason,
    };
    let parse = |label: &str, raw: &str| {
        chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|_| malformed(format!("{label} is not a YYYY-MM-DD date: {raw:?}")))
    };

    match (payload.start_date.as_deref(), payload.end_date.as_deref()) {
        (None, None) => Ok(None),
        // Half a window is not a window. Silently treating it as "from here to
        // the end of the dataset" would answer a question nobody asked.
        (Some(_), None) => Err(malformed("payload has start_date but no end_date".into())),
        (None, Some(_)) => Err(malformed("payload has end_date but no start_date".into())),
        (Some(start), Some(end)) => {
            let (s, e) = (parse("start_date", start)?, parse("end_date", end)?);
            if s > e {
                return Err(malformed(format!(
                    "start_date {start} is after end_date {end}"
                )));
            }
            // NORMALIZED, never echoed back as written. chrono accepts
            // `2020-1-1` for `%Y-%m-%d`, and the worker compares these against
            // parquet `date32` values read back zero-padded as `2020-01-20`.
            // `"2020-1-1" > "2020-01-20"` as strings -- so passing the raw
            // text through would drop all of January while looking like it had
            // honoured the request. Formatting from the parsed date makes the
            // zero-padding an invariant of this function instead of a property
            // the caller has to have got right.
            Ok(Some((
                s.format("%Y-%m-%d").to_string(),
                e.format("%Y-%m-%d").to_string(),
            )))
        }
    }
}

fn cost_profile_for(payload: &BacktestPayload) -> Result<serde_json::Value, ExecError> {
    use portfolio_model::cost::CostProfile;

    let id = payload
        .cost_profile_id
        .as_deref()
        .unwrap_or("KRX_ETF_DEFAULT");
    // Defense in depth, not the primary check.
    //
    // `POST /api/v1/backtests` now resolves this id at submission and answers
    // 400, so an unresolvable profile should never reach a claimed job. It
    // still might: rows queued before that validation existed, and any future
    // path that enqueues without going through the route. Charging the
    // default's fees under an id nobody could resolve would report costs the
    // submitter never chose, so this fails rather than substitutes.
    //
    // Until this commit the arms here also accepted `krx-etf-default` and
    // `krx-etf-default@2026-01`, spellings that resolved nowhere and that
    // every backtest test in the repository had settled on precisely because
    // nothing validated them. Both are gone; the route and this share one
    // resolver whose accepted names are `CostProfileId`'s own.
    let profile = match CostProfile::resolve(id) {
        Ok(p) => p,
        // Every rejection is PERMANENT. A payload row is written once at
        // submit time, so no retry changes the id, and retrying would burn the
        // job's attempts to reach the same answer three times.
        Err(e) => {
            return Err(ExecError::Permanent {
                class: ErrorClass::Input,
                code: "COST_PROFILE_INVALID",
                reason: format!("cost_profile_id {id:?}: {e}"),
            });
        }
    };

    Ok(serde_json::json!({
        "profile_id": profile.id_str(),
        "version": profile.version,
        // Decimal strings, not floats: a commission rate of 0.00015 has no
        // exact binary representation, and a fee that is off in the last digit
        // breaks the ledger identity the property suite asserts.
        "commission_rate": profile.commission_rate.to_string(),
        "min_commission": profile.min_commission.amount().to_string(),
        "sell_tax_rate": profile.sell_tax_rate.to_string(),
        "slippage_bps": profile.slippage_bps,
    }))
}

/// The factor values a strategy needs, or `None` when it needs none.
///
/// The metadata comes from `selector::baseline`, which is the registry the
/// rest of the system reads. Restating `required_factors` here would make a
/// fourth copy of it — after the Python package, the Rust registry, and the
/// database — and the copy that drifts is the one that decides what a
/// backtest computes.
async fn factor_series_for(
    strategy_id: &str,
    dataset_root: &Path,
    control: &RunnerControl,
    guard: &LeaseGuard,
) -> Result<Option<crate::factor_series::FactorSeries>, ExecError> {
    check_execution_guard(guard, control)?;
    let Some(package) = selector::baseline::baseline_packages()
        .into_iter()
        .find(|p| p.strategy_id == strategy_id)
    else {
        // Not a baseline: the phase-0 golden strategy computes its own signal
        // from the bars it is given and asks for nothing.
        return Ok(None);
    };
    if package.required_factors.is_empty() {
        return Ok(None);
    }

    let required: Vec<String> = package.required_factors.iter().cloned().collect();
    // `dataset_root` is what the WORKER is given, and the worker appends
    // `curated` itself. `CurateStore` is rooted one level in, at the zone
    // rather than at the dataset, so the two readers reach the same files
    // from different starting points.
    let curated_root = dataset_root.join("curated");
    let lookback = package.minimum_lookback_sessions;

    // Off the async runtime. Reading a year of parquet and collecting Polars
    // lazy frames is seconds of CPU-bound work with blocking file I/O inside
    // it, and leaving that on an executor thread starves every other task the
    // process is running -- the heartbeat that holds this job's own lease
    // among them. Polars makes the point loudly: its collect calls
    // `block_in_place`, which PANICS on a current-thread runtime. The bounded
    // helper also tells the task to stop before returning cancellation; if a
    // deep engine call cannot stop immediately, it cannot publish anything.
    let computed = run_bounded_blocking(
        control,
        guard,
        PREPROCESSING_DEADLINE,
        "factor preprocessing",
        move |cancellation| {
            if cancellation.is_canceled() {
                return Err(crate::factor_series::FactorSeriesError::Compute(
                    "factor preprocessing canceled".to_owned(),
                ));
            }
            let shape = crate::factor_series::dataset_shape(&curated_root)?;
            if cancellation.is_canceled() {
                return Err(crate::factor_series::FactorSeriesError::Compute(
                    "factor preprocessing canceled".to_owned(),
                ));
            }
            crate::factor_series::build(&curated_root, &shape, &required, lookback)
        },
    )
    .await;

    let computed = match computed {
        Ok(value) => value,
        Err(BlockingWorkError::Operation(error)) => {
            check_execution_guard(guard, control)?;
            return match error {
                error @ crate::factor_series::FactorSeriesError::InsufficientHistory { .. } => {
                    Err(ExecError::Permanent {
                        class: ErrorClass::DataBlocked,
                        code: "DATASET_TOO_SHORT",
                        reason: error.to_string(),
                    })
                }
                error => Err(ExecError::Transient(format!(
                    "factor series unavailable: {error}"
                ))),
            };
        }
        Err(BlockingWorkError::Shutdown(reason)) => return Err(ExecError::Shutdown(reason)),
        Err(BlockingWorkError::Canceled(reason)) => return Err(ExecError::Canceled(reason)),
        Err(BlockingWorkError::LeaseLost(reason)) => return Err(ExecError::LeaseLost(reason)),
        Err(BlockingWorkError::Timeout(reason) | BlockingWorkError::Join(reason)) => {
            return Err(ExecError::Transient(reason));
        }
    };
    check_execution_guard(guard, control)?;
    Ok(Some(computed))
}

/// Spawns the worker as a CHILD process.
///
/// Out of process on purpose: a NautilusTrader run that exhausts memory takes
/// its process down with it, and the worker's isolation layer exists to
/// contain that. In-process it would take the runner — and its claim — down
/// too, leaving the job RUNNING until the sweeper noticed.
#[derive(Debug)]
enum WorkerRunError {
    Transient(String),
    Shutdown(String),
    Canceled(String),
    LeaseLost(String),
}

fn worker_python_bin(paths: &RunnerPaths) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("LAGRANGE_PYTHON_BIN") {
        return Some(PathBuf::from(path));
    }
    let candidate = if cfg!(windows) {
        paths.repo_root.join("nt/.venv/Scripts/python.exe")
    } else {
        paths.repo_root.join("nt/.venv/bin/python")
    };
    candidate.is_file().then_some(candidate)
}

async fn run_worker(
    paths: &RunnerPaths,
    attempt_dir: &Path,
    request: &Path,
    artifacts: &Path,
    status: &Path,
    control: &RunnerControl,
    guard: &LeaseGuard,
) -> Result<bool, WorkerRunError> {
    let repo_root = absolute_path(&paths.repo_root);
    let nt_dir = repo_root.join("nt");
    let python_path = format!(
        "{}{}{}",
        repo_root.join("nt/backtest-worker").display(),
        if cfg!(windows) { ";" } else { ":" },
        repo_root.join("nt/strategies").display()
    );

    // The runner has database credentials in its own environment. A Python
    // worker is not allowed to inherit them (the worker can launch a second
    // process and its stderr is retained as an artifact), so this is an
    // explicit, minimal environment rather than a scrub-after-the-fact
    // filter. PATH is retained only so a bare configured executable remains
    // resolvable.  Production images contain the already-built `/opt/lagrange/
    // nt/.venv`; invoking it directly avoids uv's cache and all writes under a
    // read-only home.  The uv fallback is kept for development and uses only
    // the explicit writable temporary cache.
    let direct_python = worker_python_bin(paths).map(|path| absolute_path(&path));
    let mut command = if let Some(python) = direct_python {
        tokio::process::Command::new(python)
    } else {
        let mut command = tokio::process::Command::new(&paths.uv_bin);
        command
            .arg("run")
            .arg("--project")
            .arg(&nt_dir)
            .arg("--no-sync")
            .arg("python");
        command
    };
    command
        .arg("-m")
        .arg("backtest_worker")
        .arg("run")
        .arg("--request")
        .arg(request)
        .arg("--output-dir")
        .arg(artifacts)
        .arg("--status-path")
        .arg(status)
        // The supervisor, its isolated simulator, scratch directory, status,
        // and materialized artifacts all belong to this attempt namespace.
        // A retry therefore cannot observe or overwrite any of this attempt's
        // files, even while an expired worker is winding down.
        .arg("--scratch")
        .arg(attempt_dir.join("scratch"))
        .current_dir(attempt_dir)
        .env_clear()
        .env("PYTHONPATH", python_path)
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("HOME", attempt_dir.join("home"))
        .env("TMPDIR", attempt_dir.join("tmp"))
        .env("UV_NO_CONFIG", "1")
        .env("UV_NO_PROGRESS", "1")
        .env("UV_CACHE_DIR", attempt_dir.join("uv-cache"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A worker launches the simulator as a grandchild.  Put the uv/Python
        // supervisor in its own process group so lease loss and shutdown kill
        // the entire tree, not just the wrapper that happens to be awaited.
        unsafe {
            command.as_std_mut().pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|error| {
        WorkerRunError::Transient(format!("could not start the backtest worker: {error}"))
    })?;
    tokio::select! {
            result = child.wait() => {
                let exit = result.map_err(|error| WorkerRunError::Transient(format!("worker wait failed: {error}")))?;
                // A non-zero worker exit is a valid user-level FAILED result
                // only when this attempt wrote its own terminal status.  Do
                // not accept a canonical `<run>/status.json` (or any other
                // stale file) as evidence that this process succeeded.
                if !status.exists() {
                    Err(WorkerRunError::Transient(format!(
                        "worker exited {exit} without writing a status file"
                    )))
                } else {
                    Ok(exit.success())
                }
            }
            signal = guard.wait_signal() => {
                terminate_child(&mut child).await;
                Err(if signal == LEASE_CANCELED {
                    WorkerRunError::Canceled("backtest cancellation requested".to_owned())
                } else {
                    WorkerRunError::LeaseLost("backtest lease was lost".to_owned())
                })
            }
            _ = control.wait_shutdown() => {
                terminate_child(&mut child).await;
                Err(WorkerRunError::Shutdown("runner shutdown requested".to_owned()))
            }
    }
}

async fn terminate_child(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.start_kill();
    }
    let grace = tokio::time::sleep(SHUTDOWN_GRACE);
    tokio::pin!(grace);
    tokio::select! {
        _ = child.wait() => {}
        _ = &mut grace => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = child.start_kill();
            }
            let _ = child.wait().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(start: Option<&str>, end: Option<&str>) -> BacktestPayload {
        BacktestPayload {
            kind: None,
            run_id: None,
            strategy_config_id: None,
            dataset_version_id: None,
            initial_cash: None,
            cost_profile_id: None,
            start_date: start.map(str::to_string),
            end_date: end.map(str::to_string),
        }
    }

    fn rejection(start: Option<&str>, end: Option<&str>) -> String {
        match window_for(&payload(start, end)) {
            Err(ExecError::Permanent {
                class,
                code,
                reason,
            }) => {
                // The class drives the retry decision. A payload row is
                // written once at submit time, so a window the runner cannot
                // read is the submitter's to fix and retrying only burns the
                // job's attempts.
                assert_eq!(class, ErrorClass::Input);
                assert_eq!(code, "MALFORMED_PAYLOAD");
                reason
            }
            other => panic!("expected a permanent input error, got {other:?}"),
        }
    }

    #[test]
    fn a_window_is_carried_through_as_written() {
        let got = window_for(&payload(Some("2020-01-01"), Some("2020-12-31"))).unwrap();
        assert_eq!(
            got,
            Some(("2020-01-01".to_string(), "2020-12-31".to_string()))
        );
    }

    /// Absent is the whole dataset, and it must stay expressible.
    ///
    /// Jobs queued before this field existed carry no window, and neither do
    /// the phase-0 payloads. `None` here is what keeps their request.json
    /// byte-identical -- the approved phase-0 artifact hash depends on it.
    #[test]
    fn no_window_means_the_whole_dataset() {
        assert_eq!(window_for(&payload(None, None)).unwrap(), None);
    }

    #[test]
    fn a_single_day_window_is_legal() {
        let got = window_for(&payload(Some("2020-06-15"), Some("2020-06-15"))).unwrap();
        assert_eq!(
            got,
            Some(("2020-06-15".to_string(), "2020-06-15".to_string()))
        );
    }

    /// Half a window is rejected rather than completed.
    ///
    /// Reading `start` alone as "from here to the end of the data" would
    /// answer a question nobody asked, and the answer would look like a
    /// normal result.
    #[test]
    fn half_a_window_is_not_a_window() {
        assert!(rejection(Some("2020-01-01"), None).contains("no end_date"));
        assert!(rejection(None, Some("2020-12-31")).contains("no start_date"));
    }

    #[test]
    fn a_backwards_window_is_rejected() {
        let reason = rejection(Some("2020-12-31"), Some("2020-01-01"));
        assert!(reason.contains("after"), "unhelpful reason: {reason}");
    }

    /// Text that is not a date at all is rejected.
    ///
    /// The worker compares these against parquet `date32` values read back as
    /// `"2020-01-20"`. A bound that is not a date would still compare -- as
    /// strings, wrongly, and without ever failing.
    #[test]
    fn text_that_is_not_a_date_is_rejected() {
        for bad in ["01/01/2020", "2020-01-01T00:00:00Z", "yesterday", ""] {
            let reason = rejection(Some(bad), Some("2020-12-31"));
            assert!(
                reason.contains(bad),
                "reason should quote {bad:?}: {reason}"
            );
        }
    }

    /// A date that parses but is not zero-padded is normalized, not echoed.
    ///
    /// This one was found by the test rather than reasoned about, and it is
    /// the whole reason the function formats instead of passing text through.
    /// `chrono` accepts `2020-1-1` for `%Y-%m-%d`; the worker then compares
    /// that string against `2020-01-20`, and `"2020-1-1" > "2020-01-20"`, so
    /// the run would have silently skipped January while reporting the window
    /// the user asked for. Every bound leaving here is zero-padded.
    #[test]
    fn a_date_that_parses_but_is_not_padded_is_normalized() {
        let got = window_for(&payload(Some("2020-1-1"), Some("2020-6-5"))).unwrap();
        assert_eq!(
            got,
            Some(("2020-01-01".to_string(), "2020-06-05".to_string()))
        );

        // The property that matters, stated as the comparison the worker
        // actually performs.
        let (start, _) = got.unwrap();
        assert!(
            start.as_str() <= "2020-01-20",
            "a January bar must fall inside a window starting 2020-1-1"
        );
    }

    fn canonical_and_claim() -> (CanonicalRun, ClaimedJob) {
        use crate::types::{AttemptOutcome, Job, JobAttempt, JobStatus};
        let owner = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let canonical = CanonicalRun {
            id: Uuid::new_v4(),
            owner_user_id: owner,
            job_id: Some(job_id),
            strategy_id: "ma200_trend".to_owned(),
            strategy_version: "1.0.0".to_owned(),
            dataset_version: "kr-etf-daily-phase0-v2@2026-01".to_owned(),
            engine: "nautilustrader".to_owned(),
            engine_version: "1.231.0".to_owned(),
            config_sha256: "a".repeat(64),
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            random_seed: Some(42),
            timezone: "Asia/Seoul".to_owned(),
            status: "RUNNING".to_owned(),
        };
        let claim = ClaimedJob {
            job: Job {
                id: job_id,
                owner_user_id: owner,
                job_type: "backtest".to_owned(),
                status: JobStatus::Running,
                priority: 1,
                idempotency_key: None,
                payload_json: serde_json::json!({}),
                max_attempts: 3,
                attempt_count: 1,
                available_at: now,
                locked_by: Some("test".to_owned()),
                locked_at: Some(now),
                started_at: Some(now),
                finished_at: None,
                error_code: None,
                error_message: None,
                created_at: now,
                updated_at: now,
            },
            attempt: JobAttempt {
                id: attempt_id,
                job_id,
                attempt_no: 1,
                outcome: AttemptOutcome::Running,
                claimed_by: Some("test".to_owned()),
                error_code: None,
                error_message: None,
                started_at: Some(now),
                finished_at: None,
                created_at: now,
            },
            lease_expires_at: now + chrono::Duration::minutes(1),
            worker_id: "test".to_owned(),
        };
        (canonical, claim)
    }

    #[test]
    fn manifest_validation_rejects_missing_artifact_set() {
        let (canonical, claim) = canonical_and_claim();
        let manifest = PublishedManifest {
            run: PublishedRun {
                id: canonical.id,
                owner_user_id: canonical.owner_user_id,
                job_id: canonical.job_id,
                strategy_id: canonical.strategy_id.clone(),
                strategy_version: canonical.strategy_version.clone(),
                dataset_version: canonical.dataset_version.clone(),
                engine: canonical.engine.clone(),
                engine_version: canonical.engine_version.clone(),
                config_sha256: canonical.config_sha256.clone(),
                code_commit: canonical.code_commit.clone(),
                random_seed: canonical.random_seed,
                timezone: canonical.timezone.clone(),
                status: "SUCCEEDED".to_owned(),
                summary_json: serde_json::json!({}),
            },
            metrics: BTreeMap::new(),
            warnings: Vec::new(),
            artifacts: Vec::new(),
        };
        let root = tempfile::tempdir().unwrap();
        let error = validate_manifest(&manifest, &canonical, &claim, root.path()).unwrap_err();
        assert!(format!("{error:?}").contains("expected 9"));
    }

    #[test]
    fn worker_artifact_names_are_single_path_components() {
        assert!(is_worker_artifact_basename("equity.parquet"));
        for invalid in [
            "",
            "nested/equity.parquet",
            "../equity.parquet",
            "/tmp/equity.parquet",
            r"nested\equity.parquet",
            ".",
            "..",
        ] {
            assert!(
                !is_worker_artifact_basename(invalid),
                "accepted {invalid:?}"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_control_wakes_an_inflight_worker() {
        let control = RunnerControl::new();
        let waiter = control.clone();
        let task = tokio::spawn(async move {
            waiter.wait_shutdown().await;
            waiter.is_shutdown()
        });
        control.request_shutdown();
        assert!(task.await.unwrap());
    }

    #[tokio::test]
    async fn cancellation_interrupts_preprocessing_before_its_deadline() {
        let control = RunnerControl::new();
        let waiter = control.clone();
        let task = tokio::spawn(async move {
            await_controlled(
                std::future::pending::<()>(),
                &waiter,
                std::time::Duration::from_secs(30),
                "strategy resolution",
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        control.request_shutdown();
        let result = tokio::time::timeout(std::time::Duration::from_millis(250), task)
            .await
            .expect("preprocessing cancellation must be bounded")
            .expect("preprocessing task must join");
        assert!(matches!(result, Err(ExecError::Shutdown(_))));
    }

    #[tokio::test]
    async fn job_cancellation_interrupts_guarded_preprocessing() {
        let control = RunnerControl::new();
        let guard = LeaseGuard {
            status: Arc::new(AtomicU8::new(LEASE_ACTIVE)),
            notify: Arc::new(Notify::new()),
        };
        let task_guard = guard.clone();
        let task = tokio::spawn(async move {
            await_controlled_with_guard(
                std::future::pending::<()>(),
                &control,
                &task_guard,
                std::time::Duration::from_secs(30),
                "strategy resolution",
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        guard.status.store(LEASE_CANCELED, Ordering::Release);
        guard.notify.notify_waiters();
        let result = tokio::time::timeout(std::time::Duration::from_millis(250), task)
            .await
            .expect("job cancellation must interrupt preprocessing")
            .expect("preprocessing task must join");
        assert!(matches!(result, Err(ExecError::Canceled(_))));
    }

    #[test]
    fn asserted_commit_mismatch_is_a_permanent_integrity_error() {
        let run_id = Uuid::new_v4();
        let asserted = "0123456789abcdef0123456789abcdef01234567";
        let runner = "fedcba9876543210fedcba9876543210fedcba98";
        match validate_attested_code_commit(run_id, asserted, runner) {
            Err(ExecError::Permanent {
                class: ErrorClass::Integrity,
                code: "BACKTEST_CODE_COMMIT_MISMATCH",
                reason,
            }) => {
                assert!(reason.contains(asserted));
                assert!(reason.contains(runner));
            }
            other => panic!("expected a commit mismatch integrity error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stalled_preprocessing_hits_its_explicit_deadline() {
        let control = RunnerControl::new();
        let started = std::time::Instant::now();
        let result = await_controlled(
            std::future::pending::<()>(),
            &control,
            std::time::Duration::from_millis(20),
            "factor preprocessing",
        )
        .await;
        assert!(matches!(result, Err(ExecError::Transient(_))));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[tokio::test]
    async fn stalled_blocking_io_observes_shutdown_without_late_publication() {
        let control = RunnerControl::new();
        let guard = LeaseGuard {
            status: Arc::new(AtomicU8::new(LEASE_ACTIVE)),
            notify: Arc::new(Notify::new()),
        };
        let published = Arc::new(AtomicBool::new(false));
        let task_published = published.clone();
        let task_control = control.clone();
        let task_guard = guard.clone();
        let task = tokio::spawn(async move {
            run_bounded_blocking(
                &task_control,
                &task_guard,
                std::time::Duration::from_secs(30),
                "stalled test I/O",
                move |cancellation| {
                    // Simulate an I/O operation that is slow but cooperative
                    // between chunks. It must never cross into the publication
                    // side after its caller has been canceled.
                    while !cancellation.is_canceled() {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    if cancellation.is_canceled() {
                        return Err("stalled I/O canceled".to_owned());
                    }
                    task_published.store(true, Ordering::Release);
                    Ok::<_, String>(())
                },
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        control.request_shutdown();
        let result = tokio::time::timeout(std::time::Duration::from_millis(250), task)
            .await
            .expect("shutdown must interrupt blocking I/O")
            .expect("blocking I/O task must join");
        assert!(matches!(result, Err(BlockingWorkError::Shutdown(_))));
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(
            !published.load(Ordering::Acquire),
            "canceled blocking work must not publish late"
        );
    }

    #[tokio::test]
    async fn timed_out_blocking_io_is_bounded_and_cannot_settle_late() {
        let control = RunnerControl::new();
        let guard = LeaseGuard {
            status: Arc::new(AtomicU8::new(LEASE_ACTIVE)),
            notify: Arc::new(Notify::new()),
        };
        let settled = Arc::new(AtomicBool::new(false));
        let task_settled = settled.clone();
        let started = std::time::Instant::now();
        let result = run_bounded_blocking(
            &control,
            &guard,
            std::time::Duration::from_millis(20),
            "timed-out test I/O",
            move |cancellation| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                if !cancellation.is_canceled() {
                    task_settled.store(true, Ordering::Release);
                }
                Ok::<_, String>(())
            },
        )
        .await;
        assert!(matches!(result, Err(BlockingWorkError::Timeout(_))));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(250),
            "timeout must not wait for a stalled blocking task"
        );
        tokio::time::sleep(std::time::Duration::from_millis(325)).await;
        assert!(
            !settled.load(Ordering::Acquire),
            "a timed-out task must not settle after its caller returned"
        );
    }

    fn test_request_input(run_id: &str, job_id: Uuid) -> WorkerRequestInput {
        WorkerRequestInput {
            run_id: run_id.to_owned(),
            owner_user_id: Uuid::new_v4(),
            job_id,
            strategy_path: "test:Strategy".to_owned(),
            config_path: "test:Config".to_owned(),
            strategy_config: serde_json::json!({"period": 2}),
            strategy_id: "test".to_owned(),
            strategy_version: "1.0.0".to_owned(),
            dataset_version: "dataset@1".to_owned(),
            dataset_path: PathBuf::from("/tmp/dataset"),
            engine: "test-engine".to_owned(),
            engine_version: "1".to_owned(),
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            random_seed: 42,
            timezone: "UTC".to_owned(),
            currency: "KRW".to_owned(),
            config_sha256: "a".repeat(64),
            initial_cash: "100".to_owned(),
            cost_profile: serde_json::json!({}),
            factor_series: None,
            window: None,
        }
    }

    #[test]
    fn retry_status_and_artifacts_never_fall_back_to_canonical_stale_data() {
        let scratch = tempfile::tempdir().expect("scratch");
        let run_dir = scratch.path().join("run");
        let first_attempt = Uuid::new_v4();
        let retry_attempt = Uuid::new_v4();
        let first_dir = attempt_namespace(&run_dir, first_attempt);
        let retry_dir = attempt_namespace(&run_dir, retry_attempt);

        // Simulate an expired attempt that left a successful-looking status
        // and artifact behind. A legacy canonical status is deliberately
        // present too: the retry must never read it.
        std::fs::create_dir_all(first_dir.join("artifacts")).expect("first artifacts");
        std::fs::write(
            first_dir.join("status.json"),
            r#"{"state":"SUCCEEDED","artifacts":[{"artifact_type":"EQUITY_CURVE"}]}"#,
        )
        .expect("first status");
        std::fs::write(
            first_dir.join("artifacts/equity.parquet"),
            b"expired-attempt",
        )
        .expect("first artifact");
        std::fs::create_dir_all(&run_dir).expect("run directory");
        std::fs::write(
            run_dir.join("status.json"),
            r#"{"state":"SUCCEEDED","artifacts":[{"artifact_type":"EQUITY_CURVE"}]}"#,
        )
        .expect("legacy canonical status");

        let cancellation = BlockingCancellation::default();
        prepare_attempt_namespace(&run_dir, &retry_dir, &cancellation).expect("fresh retry");
        let retry_status_path = retry_dir.join("status.json");
        assert_ne!(retry_status_path, run_dir.join("status.json"));
        assert_ne!(
            retry_dir.join("artifacts/equity.parquet"),
            first_dir.join("artifacts/equity.parquet")
        );

        // The retry writes a terminal failure. Reading only its own status
        // proves that a failed/non-zero retry cannot succeed from the old
        // canonical or first-attempt data.
        std::fs::write(&retry_status_path, r#"{"state":"FAILED"}"#).expect("retry status");
        let retry_status = read_worker_status(&retry_status_path, cancellation.clone())
            .expect("retry status is readable");
        assert_eq!(retry_status.state, "FAILED");
        assert!(validate_worker_exit_status(false, &retry_status).is_ok());
        let stale_success = WorkerStatus {
            state: "SUCCEEDED".to_owned(),
            error: None,
            artifacts: Vec::new(),
        };
        assert!(validate_worker_exit_status(false, &stale_success).is_err());
        assert!(validate_worker_exit_status(true, &retry_status).is_err());
        // Preparation never touches legacy canonical status.  It remains
        // present but is not a source for the retry's status read.
        assert!(read_worker_status(&run_dir.join("status.json"), cancellation.clone()).is_ok());

        cleanup_attempt_namespace(&run_dir, first_attempt);
        cleanup_attempt_namespace(&run_dir, retry_attempt);
        assert!(run_dir.join("status.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn attempt_status_symlink_cannot_reuse_canonical_success() {
        let scratch = tempfile::tempdir().expect("scratch");
        let run_dir = scratch.path().join("run");
        let attempt_dir = attempt_namespace(&run_dir, Uuid::new_v4());
        std::fs::create_dir_all(&attempt_dir).expect("attempt directory");
        let canonical = run_dir.join("status.json");
        std::fs::create_dir_all(&run_dir).expect("run directory");
        std::fs::write(&canonical, r#"{"state":"SUCCEEDED"}"#).expect("canonical status");
        std::os::unix::fs::symlink(&canonical, attempt_dir.join("status.json"))
            .expect("status symlink");
        let error = read_worker_status(
            &attempt_dir.join("status.json"),
            BlockingCancellation::default(),
        )
        .expect_err("status symlink must fail closed");
        assert!(error.contains("regular non-symlink"));
    }

    #[test]
    fn expired_attempt_writer_cannot_contaminate_retry_promotion() {
        let scratch = tempfile::tempdir().expect("scratch");
        let run_dir = scratch.path().join("run");
        let first_attempt = Uuid::new_v4();
        let first_dir = attempt_namespace(&run_dir, first_attempt);
        let first_artifacts = first_dir.join("artifacts");
        std::fs::create_dir_all(&first_artifacts).expect("first artifacts");
        std::fs::write(first_artifacts.join("equity.parquet"), b"old-before-retry")
            .expect("old artifact");
        let retry_artifacts = attempt_namespace(&run_dir, Uuid::new_v4()).join("artifacts");
        std::fs::create_dir_all(&retry_artifacts).expect("retry artifacts");
        std::fs::write(retry_artifacts.join("equity.parquet"), b"retry-output")
            .expect("retry artifact");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let publication_id = Uuid::new_v4();
        let published = immutable_publication_artifacts(&run_dir, commit, publication_id)
            .expect("publication path");
        let legacy_artifacts = run_dir.join("artifacts");
        std::fs::create_dir_all(&legacy_artifacts).expect("legacy artifact directory");

        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let old_writer_artifacts = legacy_artifacts.clone();
        let old_writer_dir = run_dir.clone();
        let old_writer = std::thread::spawn(move || {
            release_rx.recv().expect("release expired writer");
            // This models the late write an old binary can perform after a
            // retry has already claimed the job: it knows only canonical
            // `<run>/artifacts` and `<run>/status.json`.
            std::fs::write(old_writer_artifacts.join("equity.parquet"), b"old-late")
                .expect("late old artifact");
            std::fs::write(
                old_writer_dir.join("status.json"),
                r#"{"state":"SUCCEEDED"}"#,
            )
            .expect("late old status");
        });

        promote_attempt_artifacts(&retry_artifacts, &published).expect("retry promotion");
        release_tx.send(()).expect("release expired writer");
        old_writer.join().expect("expired writer join");

        assert_eq!(
            std::fs::read(published.join("equity.parquet")).expect("published artifact"),
            b"retry-output"
        );
        assert_eq!(
            std::fs::read(run_dir.join("artifacts/equity.parquet"))
                .expect("legacy canonical artifact"),
            b"old-late"
        );
        assert!(run_dir.join("status.json").exists());

        cleanup_publication_artifacts(&published).expect("publication cleanup");
        cleanup_attempt_namespace(&run_dir, first_attempt);
    }

    #[test]
    fn immutable_publication_survives_continuous_legacy_writer_after_settlement() {
        let scratch = tempfile::tempdir().expect("scratch");
        let run_id = Uuid::new_v4();
        let run_dir = scratch.path().join(run_id.to_string());
        let attempt_dir = attempt_namespace(&run_dir, Uuid::new_v4());
        let attempt_artifacts = attempt_dir.join("artifacts");
        std::fs::create_dir_all(&attempt_artifacts).expect("attempt artifacts");
        let published_bytes = b"new immutable result bytes";
        std::fs::write(attempt_artifacts.join("equity.parquet"), published_bytes)
            .expect("attempt result");

        let code_commit = "0123456789abcdef0123456789abcdef01234567";
        let publication_id = Uuid::new_v4();
        let publication = immutable_publication_artifacts(&run_dir, code_commit, publication_id)
            .expect("immutable publication path");
        promote_attempt_artifacts(&attempt_artifacts, &publication).expect("publish generation");

        // Model the successful DB settlement boundary: from this point on,
        // the DB's result_artifacts row points only at `publication` and the
        // old binary is free to keep writing its canonical names.
        let db_path = format!(
            "{}/equity.parquet",
            immutable_publication_prefix(run_id, code_commit, publication_id)
                .expect("DB publication prefix")
        );
        assert_eq!(
            db_path,
            format!(
                "backtest/runs/{run_id}/generations/{code_commit}/{publication_id}/artifacts/equity.parquet"
            )
        );
        let expected_hash = sha256_file(&publication.join("equity.parquet")).expect("hash");
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let legacy_root = run_dir.clone();
        let legacy_started = started.clone();
        let legacy_writer = std::thread::spawn(move || {
            legacy_started.wait();
            let canonical_artifacts = legacy_root.join("artifacts");
            std::fs::create_dir_all(&canonical_artifacts).expect("legacy artifacts");
            for index in 0..128 {
                std::fs::write(
                    canonical_artifacts.join("equity.parquet"),
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
        started.wait();
        legacy_writer.join().expect("legacy writer");

        assert_eq!(
            std::fs::read(publication.join("equity.parquet")).expect("published bytes"),
            published_bytes
        );
        assert_eq!(
            sha256_file(&publication.join("equity.parquet")).expect("published hash"),
            expected_hash,
            "legacy canonical writes must not change the DB-referenced bytes"
        );
        assert_ne!(
            std::fs::read(run_dir.join("artifacts/equity.parquet")).expect("legacy bytes"),
            published_bytes
        );
        assert!(run_dir.join("status.json").is_file());

        cleanup_publication_artifacts(&publication).expect("publication cleanup");
    }

    #[tokio::test]
    async fn canceled_old_attempt_cleanup_cannot_erase_retry_promotion() {
        let control = RunnerControl::new();
        let guard = LeaseGuard {
            status: Arc::new(AtomicU8::new(LEASE_ACTIVE)),
            notify: Arc::new(Notify::new()),
        };
        let scratch = tempfile::tempdir().expect("scratch");
        let run_dir = scratch.path().join("run");
        let old_attempt = Uuid::new_v4();
        let retry_attempt = Uuid::new_v4();
        let old_dir = attempt_namespace(&run_dir, old_attempt);
        let retry_dir = attempt_namespace(&run_dir, retry_attempt);
        let retry_artifacts = retry_dir.join("artifacts");
        std::fs::create_dir_all(&run_dir).expect("run directory");
        std::fs::create_dir_all(run_dir.join("artifacts")).expect("published artifacts");
        std::fs::write(
            run_dir.join("artifacts/equity.parquet"),
            b"previous-published",
        )
        .expect("previous publication");
        std::fs::write(run_dir.join("status.json"), r#"{"state":"SUCCEEDED"}"#)
            .expect("legacy status");

        // Hold an old attempt's cleanup closure after its caller has entered
        // the cancelable blocking section.  The lease loss below detaches it;
        // the retry is then allowed to publish before the old cleanup runs.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let old_cleanup_dir = run_dir.clone();
        let old_prepare_dir = old_dir.clone();
        let old_artifacts = old_dir.join("artifacts");
        let old_cleanup = tokio::spawn({
            let task_control = control.clone();
            let task_guard = guard.clone();
            async move {
                run_bounded_blocking(
                    &task_control,
                    &task_guard,
                    std::time::Duration::from_secs(1),
                    "old attempt cleanup race test",
                    move |cancellation| {
                        prepare_attempt_namespace(
                            &old_cleanup_dir,
                            &old_prepare_dir,
                            &cancellation,
                        )?;
                        std::fs::create_dir_all(&old_artifacts)
                            .map_err(|error| format!("old artifacts: {error}"))?;
                        std::fs::write(old_artifacts.join("equity.parquet"), b"old-attempt")
                            .map_err(|error| format!("old artifact: {error}"))?;
                        entered_tx.send(()).expect("old cleanup entered");
                        release_rx.recv().expect("release old cleanup");
                        // Cleanup is constrained to the UUID-owned namespace;
                        // cancellation cannot turn this into a canonical run
                        // directory delete.
                        if cancellation.is_canceled() {
                            cleanup_attempt_namespace(&old_cleanup_dir, old_attempt);
                            done_tx.send(()).expect("old cleanup done");
                            return Err("old cleanup canceled".to_owned());
                        }
                        cleanup_attempt_namespace(&old_cleanup_dir, old_attempt);
                        done_tx.send(()).expect("old cleanup done");
                        Ok::<_, String>(())
                    },
                )
                .await
            }
        });
        tokio::task::spawn_blocking(move || entered_rx.recv())
            .await
            .expect("old cleanup waiter")
            .expect("old cleanup entered");
        assert_eq!(
            std::fs::read(run_dir.join("artifacts/equity.parquet"))
                .expect("previous publication remains"),
            b"previous-published"
        );
        assert!(
            run_dir.join("status.json").exists(),
            "old preparation must not remove legacy status"
        );

        guard.status.store(LEASE_LOST, Ordering::Release);
        guard.notify.notify_waiters();
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), old_cleanup)
            .await
            .expect("old cleanup caller must detach on lease loss")
            .expect("old cleanup wrapper");
        assert!(matches!(result, Err(BlockingWorkError::LeaseLost(_))));

        // Retry C owns a fresh UUID namespace and promotes while the old
        // cleanup is still blocked. The legacy canonical directory is never
        // part of this publication operation.
        let cancellation = BlockingCancellation::default();
        prepare_attempt_namespace(&run_dir, &retry_dir, &cancellation).expect("fresh retry");
        std::fs::create_dir_all(&retry_artifacts).expect("retry artifacts");
        std::fs::write(retry_artifacts.join("equity.parquet"), b"retry-c").expect("retry artifact");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let publication = immutable_publication_artifacts(&run_dir, commit, Uuid::new_v4())
            .expect("immutable publication path");
        promote_attempt_artifacts(&retry_artifacts, &publication).expect("retry promotion");

        // Release the detached old cleanup only after promotion.  Its late
        // removal may delete old-attempt data, but it has no path to the
        // immutable publication directory or the retry namespace.
        release_tx.send(()).expect("release old cleanup");
        tokio::task::spawn_blocking(move || done_rx.recv())
            .await
            .expect("old cleanup completion waiter")
            .expect("old cleanup completed");

        assert_eq!(
            std::fs::read(publication.join("equity.parquet")).expect("immutable retry artifact"),
            b"retry-c"
        );
        assert_eq!(
            std::fs::read(run_dir.join("artifacts/equity.parquet"))
                .expect("legacy publication remains"),
            b"previous-published"
        );
        assert!(run_dir.join("status.json").exists());
        assert!(!old_dir.exists(), "old cleanup only removes old namespace");
        assert!(
            retry_dir.exists(),
            "retry namespace is not old cleanup's target"
        );
        cleanup_publication_artifacts(&publication).expect("publication cleanup");
        cleanup_attempt_namespace(&run_dir, retry_attempt);
    }

    #[test]
    fn canceled_request_writer_cannot_publish_after_the_check_window() {
        let scratch = tempfile::tempdir().expect("scratch");
        let run_dir = scratch.path().join("run");
        let first_attempt = Uuid::new_v4();
        let first_dir = attempt_namespace(&run_dir, first_attempt);
        let writer_dir = first_dir.clone();
        let cancellation = BlockingCancellation::default();
        let writer_cancellation = cancellation.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            write_worker_request_with_hook(
                &writer_dir,
                test_request_input("first-attempt", Uuid::new_v4()),
                writer_cancellation,
                move || {
                    // The production final cancellation check has already
                    // passed. Hold the writer here to force the exact
                    // check-to-rename window under test.
                    entered_tx.send(()).expect("signal rename window");
                    release_rx.recv().expect("release rename window");
                },
            )
        });
        entered_rx
            .recv()
            .expect("writer reached final check-to-rename window");
        cancellation.cancel();
        release_tx.send(()).expect("release canceled writer");
        let result = writer.join().expect("writer thread");
        assert!(
            result.is_err(),
            "a writer canceled in the final window must not report success"
        );
        assert!(
            !first_dir.join("request.json").exists(),
            "the canceled attempt must not leave a published request"
        );
        assert!(
            !run_dir.join("request.json").exists(),
            "an old attempt must never publish into the canonical retry path"
        );

        // A retry gets a different namespace and remains uncontaminated even
        // after the canceled writer has returned.
        let retry_attempt = Uuid::new_v4();
        let retry_dir = attempt_namespace(&run_dir, retry_attempt);
        let (retry_request, _) = write_worker_request(
            &retry_dir,
            test_request_input("retry-attempt", Uuid::new_v4()),
            BlockingCancellation::default(),
        )
        .expect("retry request");
        let retry = std::fs::read_to_string(&retry_request).expect("retry request contents");
        assert!(retry.contains("retry-attempt"));
        assert!(!retry.contains("first-attempt"));
        cleanup_attempt_namespace(&run_dir, first_attempt);
        cleanup_attempt_namespace(&run_dir, retry_attempt);
    }

    #[tokio::test]
    async fn lease_loss_detaches_request_writer_without_retry_contamination() {
        let control = RunnerControl::new();
        let guard = LeaseGuard {
            status: Arc::new(AtomicU8::new(LEASE_ACTIVE)),
            notify: Arc::new(Notify::new()),
        };
        let scratch = tempfile::tempdir().expect("scratch");
        let run_dir = scratch.path().join("run");
        let attempt_id = Uuid::new_v4();
        let attempt_dir = attempt_namespace(&run_dir, attempt_id);
        let writer_dir = attempt_dir.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn({
            let task_control = control.clone();
            let task_guard = guard.clone();
            async move {
                run_bounded_blocking(
                    &task_control,
                    &task_guard,
                    std::time::Duration::from_secs(1),
                    "request race test",
                    move |cancellation| {
                        write_worker_request_with_hook(
                            &writer_dir,
                            test_request_input("lost-attempt", Uuid::new_v4()),
                            cancellation,
                            move || {
                                entered_tx.send(()).expect("signal detached writer");
                                // Hold the blocking closure past its bounded
                                // join grace so the caller really detaches.
                                release_rx.recv().expect("release detached writer");
                            },
                        )
                    },
                )
                .await
            }
        });
        tokio::task::spawn_blocking(move || entered_rx.recv())
            .await
            .expect("wait for request race hook")
            .expect("request race hook entered");

        guard.status.store(LEASE_LOST, Ordering::Release);
        guard.notify.notify_waiters();
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), task)
            .await
            .expect("lease loss must return while the writer is detached")
            .expect("blocking wrapper task");
        assert!(matches!(result, Err(BlockingWorkError::LeaseLost(_))));
        assert!(
            !attempt_dir.join("request.json").exists(),
            "lease loss must not publish before the detached writer is released"
        );
        assert!(!run_dir.join("request.json").exists());

        // Let the detached closure finish its late rename. Its cancellation
        // check removes the attempt-private destination, and the retry gets a
        // fresh namespace that the old closure cannot touch.
        release_tx.send(()).expect("release detached writer");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            !attempt_dir.join("request.json").exists(),
            "a detached late rename must not leave an attempt request"
        );

        let retry_attempt = Uuid::new_v4();
        let retry_dir = attempt_namespace(&run_dir, retry_attempt);
        let (retry_request, _) = write_worker_request(
            &retry_dir,
            test_request_input("retry-after-loss", Uuid::new_v4()),
            BlockingCancellation::default(),
        )
        .expect("retry request");
        let retry = std::fs::read_to_string(&retry_request).expect("retry request contents");
        assert!(retry.contains("retry-after-loss"));
        assert!(!retry.contains("lost-attempt"));
        cleanup_attempt_namespace(&run_dir, attempt_id);
        cleanup_attempt_namespace(&run_dir, retry_attempt);
    }

    #[test]
    fn commit_unknown_marker_retains_generation_and_exposes_stable_path() {
        let scratch = tempfile::tempdir().expect("scratch");
        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let generation = scratch
            .path()
            .join("run")
            .join("generations")
            .join("0123456789abcdef0123456789abcdef01234567")
            .join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(generation.join("artifacts")).expect("generation");
        std::fs::write(generation.join("artifacts/equity.parquet"), b"immutable")
            .expect("immutable bytes");
        let artifacts = generation.join("artifacts");
        let marker = write_commit_unknown_marker(&artifacts, run_id, job_id, "commit timed out")
            .expect("marker");
        assert!(artifacts.join("equity.parquet").is_file());
        assert!(marker.is_file());
        let marker_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&marker).expect("marker bytes"))
                .expect("marker JSON");
        assert_eq!(
            marker_json["event_code"],
            BACKTEST_COMMIT_UNKNOWN_EVENT_CODE
        );
        assert_eq!(marker_json["run_id"], run_id.to_string());
        assert_eq!(marker_json["job_id"], job_id.to_string());
        let error = QueueError::CommitUnknown {
            run_id,
            job_id,
            generation_path: artifacts.display().to_string(),
            detail: "commit timed out".to_owned(),
        };
        assert_eq!(error.event_code(), BACKTEST_COMMIT_UNKNOWN_EVENT_CODE);
        assert!(error.to_string().contains(&artifacts.display().to_string()));
    }
}
