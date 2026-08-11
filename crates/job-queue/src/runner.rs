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
    let Some(claim) = queue.claim_next_for(worker_id, "backtest").await? else {
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
    // Same shape as `factor_series` above, and for the same reason: a payload
    // without a window must produce the request it produced before this field
    // existed, byte for byte. Writing `"start_date": null` unconditionally
    // would change phase-0's approved request.json without changing anything
    // about phase-0.
    if let Some((start, end)) = window_for(&payload)? {
        request["start_date"] = serde_json::Value::String(start);
        request["end_date"] = serde_json::Value::String(end);
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
}
