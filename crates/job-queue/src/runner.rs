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
//! failed. Every path out of [`run_once`] therefore settles.
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
use crate::types::{ClaimedJob, ErrorClass};
use serde::Deserialize;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use uuid::Uuid;

/// Where a backtest run's inputs and outputs live.
#[derive(Debug, Clone)]
pub struct RunnerPaths {
    /// Repository root; the worker is invoked from here.
    pub repo_root: PathBuf,
    /// Dataset root the worker reads (`<root>/curated/bars/...`).
    pub dataset_root: PathBuf,
    /// Where each run's artifacts are written, under `<artifacts>/<run_id>`.
    pub artifacts_root: PathBuf,
    /// The `uv` executable that launches the worker.
    ///
    /// Explicit rather than resolved from `PATH` at call time. A daemon in a
    /// container and a test runner on a developer's machine have different
    /// PATHs, and "program not found" from a process that inherited the wrong
    /// one is indistinguishable from a broken worker. Naming it here makes the
    /// dependency visible and overridable.
    pub uv_bin: PathBuf,
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
#[derive(Debug, Deserialize)]
pub struct WorkerStatus {
    pub state: String,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
    #[serde(default)]
    pub artifacts: Vec<serde_json::Value>,
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
    let Some(claim) = queue.claim_next(worker_id).await? else {
        return Ok(Outcome::Idle);
    };
    let job_id = claim.job.id.to_string();

    // A job of another type is not this runner's, and claiming it was a
    // mistake this process must undo rather than fail: settling it as an
    // error would burn one of its attempts for no reason.
    if claim.job.job_type != "backtest" {
        queue
            .settle_failure(
                &claim,
                ErrorClass::Transient,
                "WRONG_RUNNER",
                &format!("job type {} is not backtest", claim.job.job_type),
            )
            .await?;
        return Ok(Outcome::Errored {
            job_id,
            reason: "wrong runner".into(),
        });
    }

    match execute(&claim, paths, resolver).await {
        Ok(status) if status.state == "SUCCEEDED" => {
            queue.settle_success(&claim).await?;
            Ok(Outcome::Succeeded { job_id })
        }
        Ok(status) => {
            // The backtest ran and did not succeed. That is a RESULT, and
            // retrying it would produce the same result while telling the user
            // nothing new, so it is settled as permanent.
            let reason = status
                .error
                .as_ref()
                .and_then(|e| e.get("detail"))
                .and_then(|d| d.as_str())
                .unwrap_or("the backtest did not succeed")
                .to_string();
            queue
                .settle_failure(
                    &claim,
                    classify_backtest_failure(&reason),
                    "BACKTEST_FAILED",
                    &reason,
                )
                .await?;
            Ok(Outcome::Failed { job_id, reason })
        }
        Err(ExecError::Permanent {
            class,
            code,
            reason,
        }) => {
            // The job's own inputs cannot produce a run: an unknown config, a
            // strategy this deployment has no code for, a malformed payload.
            // Retrying spends the remaining attempts to reach the identical
            // answer, so the user is told now.
            queue.settle_failure(&claim, class, code, &reason).await?;
            Ok(Outcome::Failed { job_id, reason })
        }
        Err(ExecError::Transient(reason)) => {
            // The runner could not run it: a missing interpreter, an
            // unreadable status file, a registry outage. Retryable, because a
            // later attempt may well work.
            queue
                .settle_failure(&claim, ErrorClass::Transient, "RUNNER_ERROR", &reason)
                .await?;
            Ok(Outcome::Errored { job_id, reason })
        }
    }
}

/// Why [`execute`] did not return a worker status.
///
/// Separated from `Outcome` because the distinction it carries is about
/// RETRYABILITY, which the queue acts on, rather than about what to report.
enum ExecError {
    /// No attempt can succeed. Settled permanently.
    Permanent {
        class: ErrorClass,
        code: &'static str,
        reason: String,
    },
    /// The runner faltered; a later attempt may work.
    Transient(String),
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

/// Builds the worker request and runs it, returning the worker's own status.
async fn execute<R: StrategyResolver>(
    claim: &ClaimedJob,
    paths: &RunnerPaths,
    resolver: &R,
) -> Result<WorkerStatus, ExecError> {
    // A payload the runner cannot read is PERMANENT: the row is written once
    // at submit time and no retry will make it parse.
    let payload: BacktestPayload =
        serde_json::from_value(claim.job.payload_json.clone()).map_err(|e| {
            ExecError::Permanent {
                class: ErrorClass::Input,
                code: "MALFORMED_PAYLOAD",
                reason: format!("payload is not a backtest request: {e}"),
            }
        })?;

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
    let strategy = resolver.resolve(config_id, claim.job.owner_user_id).await?;
    let factor_series = factor_series_for(&strategy.strategy_id, &paths.dataset_root).await?;

    let run_id = payload
        .run_id
        .clone()
        .unwrap_or_else(|| claim.job.id.to_string());
    let run_dir = paths.artifacts_root.join(&run_id);
    // Everything from here down is the RUNNER's environment -- a full disk, an
    // unwritable directory, a missing interpreter. None of it is the job's
    // fault, so all of it is transient.
    std::fs::create_dir_all(&run_dir)
        .map_err(|e| ExecError::Transient(format!("cannot create {run_dir:?}: {e}")))?;

    let mut request = serde_json::json!({
        "run_id": run_id,
        "owner_user_id": claim.job.owner_user_id.to_string(),
        "job_id": claim.job.id.to_string(),
        // From the REGISTRY, never from the request body.
        "strategy_path": strategy.strategy_path,
        "strategy_config_class": strategy.config_path,
        "strategy_config": strategy.config,
        "strategy_id": strategy.strategy_id,
        "strategy_version": strategy.strategy_version,
        "dataset_version": payload.dataset_version_id.clone().unwrap_or_default(),
        "dataset_path": paths.dataset_root.to_string_lossy(),
        "engine_version": "1.231.0",
        "code_commit": "0".repeat(40),
        "random_seed": 42,
        "timezone": "Asia/Seoul",
        "currency": "KRW",
        "config_sha256": format!("sha256:{}", "0".repeat(64)),
        "slippage_bps": 10,
        "initial_cash": payload
            .initial_cash
            .as_ref()
            .and_then(|c| c.get("amount"))
            .and_then(|a| a.as_str())
            .unwrap_or("100000000"),
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
        "cost_profile": cost_profile_for(&payload)?,
    });
    // Attached only when the strategy needs it, so a request for a strategy
    // that computes its own signal is unchanged.
    if let Some(series) = factor_series {
        request["factor_series"] = serde_json::to_value(series)
            .map_err(|e| ExecError::Transient(format!("cannot serialise factors: {e}")))?;
    }

    let request_path = run_dir.join("request.json");
    let status_path = run_dir.join("status.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&request)
            .map_err(|e| ExecError::Transient(format!("cannot serialise request: {e}")))?,
    )
    .map_err(|e| ExecError::Transient(format!("cannot write request: {e}")))?;

    run_worker(
        paths,
        &request_path,
        &run_dir.join("artifacts"),
        &status_path,
    )
    .await
    .map_err(ExecError::Transient)?;

    let raw = std::fs::read_to_string(&status_path)
        .map_err(|e| ExecError::Transient(format!("worker wrote no readable status: {e}")))?;
    serde_json::from_str(&raw)
        .map_err(|e| ExecError::Transient(format!("worker status is not JSON: {e}")))
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
fn cost_profile_for(payload: &BacktestPayload) -> Result<serde_json::Value, ExecError> {
    use portfolio_model::cost::{CostProfile, CostProfileId};

    let id = payload
        .cost_profile_id
        .as_deref()
        .unwrap_or("KRX_ETF_DEFAULT");
    // Two spellings of one profile reach this point, and both are load-bearing
    // today.
    //
    // `paper.rs` resolves `KRX_ETF_DEFAULT`, which is the enum's own serde
    // name. `POST /api/v1/backtests` passes `cost_profile_id` into the job
    // payload WITHOUT validating it, and every backtest test in the repository
    // sends `krx-etf-default@2026-01` — a versioned spelling that resolves
    // nowhere and was never wired to anything.
    //
    // Accepting both is a compatibility shim, not the fix. The fix is for the
    // route to validate the id the way the paper route does, so a profile
    // nobody can resolve is a 400 at submission instead of a job that fails
    // minutes later; that is an API change and is called out rather than made
    // quietly here. What this does NOT do is accept anything else: an
    // unrecognised id fails, because charging the default's fees under
    // another profile's name would report costs the submitter never chose.
    let profile = match id {
        "KRX_ETF_DEFAULT" | "krx-etf-default" | "krx-etf-default@2026-01" => {
            CostProfile::krx_etf_default().map_err(|e| ExecError::Permanent {
                class: ErrorClass::Input,
                code: "COST_PROFILE_INVALID",
                reason: format!("the shipped cost profile is unusable: {e}"),
            })?
        }
        // A CUSTOM profile needs explicit rates the payload does not carry.
        // Substituting the default would charge a backtest fees the submitter
        // never asked for and report them as if they had.
        other => {
            return Err(ExecError::Permanent {
                class: ErrorClass::Input,
                code: "UNKNOWN_COST_PROFILE",
                reason: format!("cost profile {other:?} is not one this runner can resolve"),
            });
        }
    };

    Ok(serde_json::json!({
        "profile_id": match profile.profile_id {
            CostProfileId::KrxEtfDefault => "KRX_ETF_DEFAULT",
            CostProfileId::Custom => "CUSTOM",
        },
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
) -> Result<Option<crate::factor_series::FactorSeries>, ExecError> {
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
    // `block_in_place`, which PANICS on a current-thread runtime.
    let computed = tokio::task::spawn_blocking(move || {
        let shape = crate::factor_series::dataset_shape(&curated_root)?;
        crate::factor_series::build(&curated_root, &shape, &required, lookback)
    })
    .await
    .map_err(|e| ExecError::Transient(format!("factor computation did not finish: {e}")))?;

    match computed {
        Ok(series) => Ok(Some(series)),
        Err(e @ crate::factor_series::FactorSeriesError::InsufficientHistory { .. }) => {
            // PERMANENT. A dataset does not grow between attempts, so the two
            // remaining retries would spend a worker slot each to reach the
            // identical answer, and the message an operator needs -- this
            // dataset is too short for this strategy -- would stay hidden
            // behind RETRYING until they were used up.
            Err(ExecError::Permanent {
                class: ErrorClass::DataBlocked,
                code: "DATASET_TOO_SHORT",
                reason: e.to_string(),
            })
        }
        Err(e) => Err(ExecError::Transient(format!(
            "factor series unavailable: {e}"
        ))),
    }
}

/// Spawns the worker as a CHILD process.
///
/// Out of process on purpose: a NautilusTrader run that exhausts memory takes
/// its process down with it, and the worker's isolation layer exists to
/// contain that. In-process it would take the runner — and its claim — down
/// too, leaving the job RUNNING until the sweeper noticed.
async fn run_worker(
    paths: &RunnerPaths,
    request: &Path,
    artifacts: &Path,
    status: &Path,
) -> Result<(), String> {
    let nt_dir = paths.repo_root.join("nt");
    let python_path = format!(
        "{}{}{}",
        paths.repo_root.join("nt/backtest-worker").display(),
        if cfg!(windows) { ";" } else { ":" },
        paths.repo_root.join("nt/strategies").display()
    );

    let output = tokio::process::Command::new(&paths.uv_bin)
        .arg("run")
        .arg("--project")
        .arg(&nt_dir)
        .arg("python")
        .arg("-m")
        .arg("backtest_worker")
        .arg("run")
        .arg("--request")
        .arg(request)
        .arg("--output-dir")
        .arg(artifacts)
        .arg("--status-path")
        .arg(status)
        .current_dir(&paths.repo_root)
        .env("PYTHONPATH", python_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("could not start the backtest worker: {e}"))?;

    // A non-zero exit is NOT treated as a runner error here: the worker exits
    // 1 for a backtest that legitimately failed, and it still writes a status
    // file. The status file is the authority; the exit code only matters when
    // there is no status to read, which the caller detects.
    if !output.status.success() && !status.exists() {
        return Err(format!(
            "worker exited {} without writing a status file: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("no stderr")
        ));
    }
    Ok(())
}
