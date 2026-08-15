//! The process that drains the backtest queue.
//!
//! `run_once` is a library function, and a library function nobody runs leaves
//! the queue exactly as full as it was. This binary is the deployment unit:
//! one process, one job at a time, until it is asked to stop.
//!
//! # Two responsibilities, not one
//!
//! It runs jobs, and it SWEEPS. `runner.rs` says a crash mid-backtest is safe
//! because "the sweeper requeues it" — and until this binary existed, nothing
//! in the repository ever called [`JobQueue::sweep`]. The recovery story was
//! prose. A job whose runner died stayed RUNNING behind an expired lease with
//! no process left that would ever look at it, so the crash-safety the lease
//! design buys was never actually collected.
//!
//! Sweeping happens on the IDLE path. A busy runner is a runner whose leases
//! are being heartbeated; the moment worth spending on recovery is the moment
//! there is nothing else to do. Several runners sweeping concurrently is safe
//! — the sweep is a conditional UPDATE — and is the normal case with more than
//! one replica.
//!
//! # What it deliberately does not do
//!
//! **Cancellation and shutdown are bounded.** The child supervisor heartbeats
//! the exact claim while the run is in flight. A canceled claim or SIGTERM
//! terminates the whole child process group with a bounded grace period, then
//! settles the current job before the daemon leaves. A lease lost to a second
//! worker is different: that stale process is forbidden from settling or
//! publishing, and the queue sweeper owns recovery.
//!
//! **One job at a time.** A NautilusTrader run is given a 2 GiB limit; two in
//! one container is how a host that was sized for one runner starts swapping.
//! Concurrency here is `--replicas`, not threads.
//!
//! **No audit connection.** [`JobQueue`] takes an optional second pool for
//! `audit_logs`, and this process passes `None` — a decision rather than an
//! omission. `request_cancel` is the only operation that writes an audit row,
//! it is the API server's to call, and the daemon calls `claim_next_for`
//! for `backtest` jobs,
//! `settle_*` and `sweep` exclusively. `worker` is not granted INSERT on
//! `audit_logs` (the migration contract asserts it), so handing this process
//! an audit pool would mean giving the role a privilege it has no use for.
//!
//! **`--once` does not sweep.** The one-shot path exits on an empty queue
//! before reaching the recovery step, so it will not pick up an orphan left by
//! a previous crash. That is right for the gates it exists for, which start
//! from a fresh database; a long-lived deployment must run without `--once`.

use job_queue::queue::{JobQueue, QueueConfig};
use job_queue::reconciler::{
    BACKTEST_RECONCILE_DB_UNAVAILABLE_CODE, BACKTEST_RECONCILE_ERROR_CODE,
    BACKTEST_RECONCILE_EVENT_CODE, ReconcileError, ReconcilerConfig, reconcile_artifacts,
};
use job_queue::resolver::DbStrategyResolver;
use job_queue::runner::{
    DAEMON_SHUTDOWN_GRACE, Outcome, RunnerControl, RunnerPaths, is_exact_code_commit,
    run_once_with_control_and_gate,
};
use job_queue::safety::{
    BACKTEST_BACKPRESSURE_EVENT_CODE, BackpressureConfig, ClaimGate, DEFAULT_MAX_QUEUED_BACKTESTS,
    DEFAULT_MIN_FREE_BYTES, DEFAULT_RECONCILE_INTERVAL,
};
use sqlx::postgres::PgPoolOptions;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long to wait after finding nothing.
///
/// A backtest takes seconds to minutes, so polling faster buys latency nobody
/// perceives and costs a query per runner per interval. `LISTEN/NOTIFY` would
/// remove the poll entirely; it is not worth its failure modes until the idle
/// query shows up in a profile.
const IDLE_SLEEP: Duration = Duration::from_secs(2);

/// Backoff after a queue error, so a database outage does not become a hot
/// loop hammering a server that is already unwell.
const ERROR_SLEEP: Duration = Duration::from_secs(10);

struct Args {
    /// Claim at most one job, then exit. For scripts and gates, which need a
    /// deterministic end rather than a daemon they have to kill.
    once: bool,
    /// Probe the database and worker schema, then exit. This is intentionally
    /// separate from `--once`: an empty queue is a healthy worker, while a
    /// missing database/schema is not.
    healthcheck: bool,
    readiness: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppEnvironment {
    Development,
    Qa,
    Production,
}

impl AppEnvironment {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "development" => Ok(Self::Development),
            "qa" => Ok(Self::Qa),
            "production" => Ok(Self::Production),
            _ => Err("APP_ENV must be exactly development, qa, or production".to_owned()),
        }
    }

    const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

fn resolve_app_environment(value: Option<OsString>) -> Result<AppEnvironment, String> {
    let value = value.ok_or_else(|| "APP_ENV is required".to_owned())?;
    let value = value
        .into_string()
        .map_err(|_| "APP_ENV must be valid Unicode".to_owned())?;
    AppEnvironment::parse(&value)
}

fn positive_u64_setting(key: &str, production: bool, default: u64) -> Result<u64, String> {
    let Some(value) = std::env::var_os(key) else {
        if production {
            return Err(format!("{key} is required in production"));
        }
        return Ok(default);
    };
    let value = value
        .into_string()
        .map_err(|_| format!("{key} must be valid Unicode"))?;
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{key} must be a positive integer"))
}

fn positive_i64_setting(key: &str, production: bool, default: i64) -> Result<i64, String> {
    let Some(value) = std::env::var_os(key) else {
        if production {
            return Err(format!("{key} is required in production"));
        }
        return Ok(default);
    };
    let value = value
        .into_string()
        .map_err(|_| format!("{key} must be valid Unicode"))?;
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{key} must be a positive integer"))
}

fn safety_configuration(
    production: bool,
) -> Result<(BackpressureConfig, ReconcilerConfig, Duration), String> {
    let capacity = BackpressureConfig {
        min_free_bytes: positive_u64_setting(
            "BACKTEST_MIN_FREE_BYTES",
            production,
            DEFAULT_MIN_FREE_BYTES,
        )?,
        max_queued_backtests: positive_i64_setting(
            "BACKTEST_MAX_QUEUED_BACKTESTS",
            production,
            DEFAULT_MAX_QUEUED_BACKTESTS,
        )?,
    }
    .validate()?;
    let grace_seconds = positive_u64_setting("BACKTEST_RECONCILE_GRACE_SECS", production, 900)?;
    let interval_seconds = positive_u64_setting(
        "BACKTEST_RECONCILE_INTERVAL_SECS",
        production,
        DEFAULT_RECONCILE_INTERVAL.as_secs(),
    )?;
    let grace = ReconcilerConfig {
        safe_grace: Duration::from_secs(grace_seconds),
    }
    .validate()
    .map_err(|error| error.to_string())?;
    Ok((capacity, grace, Duration::from_secs(interval_seconds)))
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        once: false,
        healthcheck: false,
        readiness: false,
    };
    let mut values = std::env::args().skip(1).peekable();
    if matches!(
        values.peek().map(String::as_str),
        Some("healthcheck" | "readiness")
    ) {
        let probe = values.next().expect("peeked probe command");
        if values.next().is_some() {
            return Err("healthcheck/readiness does not accept additional arguments".to_owned());
        }
        args.healthcheck = probe == "healthcheck";
        args.readiness = probe == "readiness";
        return Ok(args);
    }
    for arg in values {
        match arg.as_str() {
            "--once" => args.once = true,
            "--help" | "-h" => {
                println!(
                    "backtest-runner [--once]\nbacktest-runner healthcheck|readiness\n\n\
                     Drains the `backtest` job queue.\n\n\
                     Environment:\n  \
                       DATABASE_URL(_FILE)    optional URL mode (manual/QA only)\n  \
                       DB_HOST/DB_PORT/DB_NAME/DB_USER/DB_PASSWORD_FILE\n  \
                                               required component mode for deployment\n  \
                       APP_ENV                development, qa, or production\n  \
                       LAGRANGE_REPO_ROOT     defaults to the current directory\n  \
                       LAGRANGE_DATASET_ROOT  defaults to <repo>/data/phase0\n  \
                       LAGRANGE_ARTIFACTS_ROOT defaults to <repo>/artifacts\n  \
                       LAGRANGE_UV_BIN        defaults to `uv` on PATH\n  \
                       BACKTEST_MIN_FREE_BYTES / BACKTEST_MAX_QUEUED_BACKTESTS\n  \
                                               production-required capacity gates\n  \
                       BACKTEST_RECONCILE_GRACE_SECS / _INTERVAL_SECS\n  \
                                               production-required retention settings\n"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unrecognised argument {other:?} (try --help)")),
        }
    }
    Ok(args)
}

/// Read a secret only from a mounted regular file. In particular, this does
/// not fall back to a `DB_PASSWORD` environment variable: putting a database
/// password in the process environment makes accidental logging and inherited
/// environment disclosure much easier.
fn read_secret_file(path: PathBuf, key: &str) -> Result<String, String> {
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|_| format!("{key} is inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{key} must be a non-symlink regular file"));
    }
    let value = std::fs::read_to_string(path).map_err(|_| format!("{key} is unreadable"))?;
    if value.contains(['\n', '\r']) {
        return Err(format!("{key} must contain one line"));
    }
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(format!("{key} is empty"))
    } else {
        Ok(value)
    }
}

fn read_database_options_from<F>(
    get: F,
    production: bool,
) -> Result<sqlx::postgres::PgConnectOptions, String>
where
    F: Fn(&str) -> Option<OsString>,
{
    let direct_value = get("DATABASE_URL");
    let direct_present = direct_value.is_some();
    let file = get("DATABASE_URL_FILE").map(PathBuf::from);
    let keys = [
        "DB_HOST",
        "DB_PORT",
        "DB_NAME",
        "DB_USER",
        "DB_PASSWORD_FILE",
    ];
    let components: Vec<(&str, Option<OsString>)> =
        keys.iter().map(|key| (*key, get(key))).collect();
    let any_component = components.iter().any(|(_, value)| value.is_some());
    if production && (direct_present || file.is_some()) {
        return Err(
            "production requires DB_HOST/DB_PORT/DB_NAME/DB_USER/DB_PASSWORD_FILE component mode"
                .to_owned(),
        );
    }
    if get("DB_PASSWORD").is_some() {
        return Err("DB_PASSWORD is forbidden; use DB_PASSWORD_FILE".to_owned());
    }
    if (direct_present || file.is_some()) && any_component {
        return Err(
            "DATABASE_URL(_FILE) and DB_* component modes are mutually exclusive".to_owned(),
        );
    }
    let direct = direct_value
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "DATABASE_URL must be valid Unicode".to_owned())
        })
        .transpose()?;
    match (direct, file, any_component) {
        (Some(_), Some(_), false) => {
            Err("set exactly one of DATABASE_URL or DATABASE_URL_FILE".to_owned())
        }
        (Some(url), None, false) if !url.trim().is_empty() => url
            .parse()
            .map_err(|_| "DATABASE_URL is invalid".to_owned()),
        (Some(_), None, false) => Err("DATABASE_URL must not be empty".to_owned()),
        (None, Some(path), false) => read_secret_file(path, "DATABASE_URL_FILE")?
            .parse()
            .map_err(|_| "DATABASE_URL_FILE is invalid".to_owned()),
        (None, None, true) => {
            let mut values = std::collections::HashMap::new();
            for (key, value) in components {
                let value = value
                    .ok_or_else(|| format!("{key} is required with component database mode"))?
                    .into_string()
                    .map_err(|_| format!("{key} must be valid Unicode"))?;
                if value.trim().is_empty() {
                    return Err(format!("{key} must not be empty"));
                }
                values.insert(key, value);
            }
            let port = values["DB_PORT"]
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| "DB_PORT must be a valid TCP port".to_owned())?;
            let password = read_secret_file(
                PathBuf::from(&values["DB_PASSWORD_FILE"]),
                "DB_PASSWORD_FILE",
            )?;
            Ok(sqlx::postgres::PgConnectOptions::new()
                .host(&values["DB_HOST"])
                .port(port)
                .database(&values["DB_NAME"])
                .username(&values["DB_USER"])
                .password(&password))
        }
        _ => Err(
            "DATABASE_URL, DATABASE_URL_FILE, or complete DB_* component mode is required"
                .to_owned(),
        ),
    }
}

fn read_database_options(production: bool) -> Result<sqlx::postgres::PgConnectOptions, String> {
    read_database_options_from(|key| std::env::var_os(key), production)
}

/// Read the image revision that the Dockerfile baked into the runtime
/// environment. There is no fallback: a runner without an attested image
/// revision must not claim a job whose result would be mislabeled.
fn baked_code_commit_from<F>(get: F) -> Result<String, String>
where
    F: Fn(&str) -> Option<OsString>,
{
    let value = get("LAGRANGE_CODE_COMMIT")
        .ok_or_else(|| {
            "LAGRANGE_CODE_COMMIT is required and must be baked into the image".to_owned()
        })?
        .into_string()
        .map_err(|_| "LAGRANGE_CODE_COMMIT must be valid Unicode".to_owned())?;
    if !is_exact_code_commit(&value) {
        return Err(
            "LAGRANGE_CODE_COMMIT must be an exact lowercase non-zero 40-hex Git commit".to_owned(),
        );
    }
    // Release images compile the Docker build argument into this binary in
    // addition to baking it into ENV. This closes the otherwise unavoidable
    // `docker run -e LAGRANGE_CODE_COMMIT=...` override path. Debug/test
    // binaries intentionally have no compiled value so local harnesses can
    // provide a deterministic fixture commit.
    #[cfg(not(debug_assertions))]
    match option_env!("LAGRANGE_CODE_COMMIT") {
        Some(compiled) if value == compiled => {}
        Some(_) => {
            return Err(
                "LAGRANGE_CODE_COMMIT does not match the commit compiled into this image"
                    .to_owned(),
            );
        }
        None => {
            return Err("LAGRANGE_CODE_COMMIT was not compiled into this release image".to_owned());
        }
    }
    Ok(value)
}

fn env_path(key: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).unwrap_or(fallback)
}

fn validate_existing_directory(path: &std::path::Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| format!("{label} is missing or inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} must be a non-symlink directory"));
    }
    Ok(())
}

fn validate_existing_file(path: &std::path::Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| format!("{label} is missing or inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} must be a non-symlink regular file"));
    }
    Ok(())
}

fn validate_writable_directory(path: &std::path::Path, label: &str) -> Result<(), String> {
    validate_existing_directory(path, label)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let probe = path.join(format!(
        ".lagrange-backtest-write-{}-{nonce}",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| format!("{label} is not writable"))?;
    file.write_all(b"ok")
        .map_err(|_| format!("{label} is not writable"))?;
    drop(file);
    std::fs::remove_file(&probe).map_err(|_| format!("{label} write probe could not be removed"))
}

fn validate_uv_path(path: &std::path::Path) -> Result<(), String> {
    // A bare `uv` is intentionally resolved through the explicitly retained
    // PATH entry. Any path containing a directory is deployment-controlled
    // and must be a non-symlink executable file.
    if path.has_root() || path.components().count() > 1 {
        validate_existing_file(path, "uv executable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::metadata(path)
                .map_err(|_| "uv executable is inaccessible".to_owned())?
                .permissions()
                .mode()
                & 0o111
                == 0
            {
                return Err("uv executable is not executable".to_owned());
            }
        }
    }
    Ok(())
}

fn worker_python_path(repo_root: &std::path::Path) -> String {
    format!(
        "{}{}{}",
        repo_root.join("nt/backtest-worker").display(),
        if cfg!(windows) { ";" } else { ":" },
        repo_root.join("nt/strategies").display()
    )
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

fn validate_worker_prerequisites(paths: &RunnerPaths) -> Result<(), String> {
    let direct_python = worker_python_bin(paths);
    let mut command = if let Some(python) = direct_python {
        std::process::Command::new(python)
    } else {
        validate_uv_path(&paths.uv_bin)?;
        let mut command = std::process::Command::new(&paths.uv_bin);
        command
            .arg("run")
            .arg("--project")
            .arg(paths.repo_root.join("nt"))
            .arg("--no-sync")
            .arg("python");
        command
    };
    command
        .arg("-c")
        .arg(
            "import backtest_worker, importlib, nautilus_trader; importlib.import_module('custom-data.catalog_builder')",
        )
        .current_dir(&paths.repo_root)
        .env_clear()
        .env("PYTHONPATH", worker_python_path(&paths.repo_root))
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("HOME", "/tmp")
        .env("TMPDIR", "/tmp")
        .env("UV_NO_CONFIG", "1")
        .env("UV_NO_PROGRESS", "1")
        .env("UV_CACHE_DIR", "/tmp/lagrange-uv-cache");
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    let output = command
        .output()
        .map_err(|_| "could not start the uv/Python worker prerequisite probe".to_owned())?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr)
        .lines()
        .last()
        .unwrap_or("worker prerequisite probe failed")
        .to_owned();
    Err(format!(
        "uv/Python worker prerequisites are unavailable: {detail}"
    ))
}

fn validate_runner_paths(paths: &RunnerPaths) -> Result<(), String> {
    validate_existing_directory(&paths.repo_root, "repository root")?;
    validate_existing_directory(&paths.dataset_root, "dataset root")?;
    validate_existing_directory(&paths.dataset_root.join("curated"), "dataset curated root")?;
    validate_existing_directory(
        &paths.dataset_root.join("curated/curated"),
        "dataset curated data root",
    )?;
    validate_existing_directory(
        &paths.dataset_root.join("curated/curated/bars"),
        "dataset bars zone",
    )?;
    validate_existing_directory(&paths.artifacts_root, "artifacts root")?;
    validate_writable_directory(&paths.artifacts_root, "artifacts root")?;

    let nt_dir = paths.repo_root.join("nt");
    validate_existing_directory(&nt_dir, "NT project")?;
    validate_existing_file(&nt_dir.join("pyproject.toml"), "NT pyproject")?;
    validate_existing_file(&nt_dir.join("uv.lock"), "NT uv.lock")?;
    for (path, label) in [
        (nt_dir.join("backtest-worker"), "backtest worker package"),
        (nt_dir.join("strategies"), "strategy package"),
        (nt_dir.join("custom-data"), "custom data package"),
    ] {
        validate_existing_directory(&path, label)?;
    }
    Ok(())
}

/// Identifies this process in `jobs.locked_by`.
///
/// The pid is included because the host name alone cannot distinguish two
/// replicas on one machine, and `locked_by` is what an operator reads when
/// asking which process is holding a stuck job.
fn worker_id() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string());
    format!("backtest-runner@{host}/{}", std::process::id())
}

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        result = async {
            terminate.recv().await.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "terminate signal stream closed",
                )
            })
        } => result.map(|_| ()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

/// Keep an in-flight run alive long enough to execute its cancellation and
/// atomic settlement path, but never let a non-cooperative preprocessing or
/// database future hold daemon shutdown beyond `budget`.
async fn await_run_with_shutdown_budget<F, T>(
    control: &RunnerControl,
    future: F,
    budget: Duration,
) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::pin!(future);
    if control.is_shutdown() {
        tokio::time::timeout(budget, &mut future).await.ok()
    } else {
        tokio::select! {
            biased;
            result = &mut future => Some(result),
            _ = control.wait_shutdown() => {
                tokio::time::timeout(budget, &mut future).await.ok()
            }
        }
    }
}

fn log_reconcile_report(report: &job_queue::reconciler::ReconcileReport) {
    eprintln!(
        "event_code={} scanned={} referenced_retained={} active_retained={} fresh_retained={} deleted={} malformed_skipped={} symlink_skipped={} deletion_errors={}",
        BACKTEST_RECONCILE_EVENT_CODE,
        report.scanned_generations,
        report.referenced_retained,
        report.active_retained,
        report.fresh_retained,
        report.deleted_generations,
        report.malformed_skipped,
        report.symlink_skipped,
        report.deletion_errors,
    );
}

fn reconcile_error_code(error: &ReconcileError) -> &'static str {
    if matches!(error, ReconcileError::Database(_)) {
        BACKTEST_RECONCILE_DB_UNAVAILABLE_CODE
    } else {
        BACKTEST_RECONCILE_ERROR_CODE
    }
}

fn main() -> ExitCode {
    // A blocking factor task cannot always be aborted once it has entered the
    // engine. Use an explicit runtime so process shutdown has a final bounded
    // `shutdown_timeout` as well as the async drain budget below; the default
    // `#[tokio::main]` drop waits indefinitely for such tasks.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("backtest runner runtime must build");
    let exit = runtime.block_on(async_main());
    runtime.shutdown_timeout(Duration::from_secs(1));
    exit
}

async fn async_main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("backtest-runner: {e}");
            return ExitCode::FAILURE;
        }
    };

    let repo_root = env_path(
        "LAGRANGE_REPO_ROOT",
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    );
    let paths = RunnerPaths {
        dataset_root: env_path("LAGRANGE_DATASET_ROOT", repo_root.join("data/phase0")),
        artifacts_root: env_path("LAGRANGE_ARTIFACTS_ROOT", repo_root.join("artifacts")),
        repo_root,
        uv_bin: RunnerPaths::default_uv_bin(),
        code_commit: match baked_code_commit_from(|key| std::env::var_os(key)) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("backtest-runner: invalid image provenance: {error}");
                return ExitCode::FAILURE;
            }
        },
    };

    let app_env = match resolve_app_environment(std::env::var_os("APP_ENV")) {
        Ok(app_env) => app_env,
        Err(error) => {
            eprintln!("backtest-runner: invalid application environment: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (capacity_config, reconciler_config, reconcile_interval) =
        match safety_configuration(app_env.is_production()) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("backtest-runner: safety configuration invalid: {error}");
                return ExitCode::FAILURE;
            }
        };

    // Small on purpose: this process runs one job at a time, so a large pool
    // would reserve connections it can never use while starving the API.
    let database_options = match read_database_options(app_env.is_production()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("backtest-runner: database configuration invalid: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = validate_runner_paths(&paths) {
        eprintln!("backtest-runner: runtime paths invalid: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = validate_worker_prerequisites(&paths) {
        eprintln!("backtest-runner: {error}");
        return ExitCode::FAILURE;
    }
    let pool = match PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(database_options)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("backtest-runner: cannot reach the database: {e}");
            return ExitCode::FAILURE;
        }
    };

    let queue = JobQueue::new(pool.clone(), None, QueueConfig::default());

    if args.healthcheck || args.readiness {
        let healthy: Result<bool, _> = sqlx::query_scalar(
            "SELECT to_regclass('public.jobs') IS NOT NULL AND \
                    to_regclass('public.job_attempts') IS NOT NULL AND \
                    to_regclass('public.backtest_runs') IS NOT NULL AND \
                    to_regclass('public.result_artifacts') IS NOT NULL",
        )
        .fetch_one(&pool)
        .await;
        let schema_ready = matches!(healthy, Ok(true));
        let gate = match ClaimGate::new(capacity_config) {
            Ok(gate) => gate,
            Err(error) => {
                eprintln!("backtest-runner: safety gate invalid: {error}");
                pool.close().await;
                return ExitCode::FAILURE;
            }
        };
        let snapshot = gate.refresh(&queue, &paths.artifacts_root).await;
        let ready = schema_ready && snapshot.ready;
        println!(
            "{{\"status\":\"{}\",\"database\":\"{}\",\"schema\":\"{}\",\"ready\":{},\"free_bytes\":{},\"queued_backtests\":{},\"reason\":{}}}",
            if ready { "ok" } else { "not_ready" },
            if schema_ready {
                "reachable"
            } else {
                "unavailable"
            },
            if schema_ready { "ready" } else { "not_ready" },
            ready,
            snapshot
                .free_bytes
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
            snapshot
                .queued_backtests
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
            serde_json::to_string(&snapshot.reason).unwrap_or_else(|_| "\"unknown\"".to_owned()),
        );
        let exit = if args.readiness && !ready {
            eprintln!(
                "event_code={} reason={}",
                BACKTEST_BACKPRESSURE_EVENT_CODE, snapshot.reason
            );
            ExitCode::FAILURE
        } else if args.healthcheck {
            // Liveness remains distinct from readiness: an empty queue or a
            // capacity high-water mark should not make Docker restart a
            // healthy process. The JSON still exposes the degraded state.
            if schema_ready {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        } else {
            ExitCode::SUCCESS
        };
        pool.close().await;
        return exit;
    }

    let id = worker_id();
    let resolver = DbStrategyResolver::new(pool.clone());
    eprintln!("backtest-runner: {id} draining the queue");

    // Ctrl-C/SIGTERM stops new claims and is also delivered to the child
    // supervisor.  The active claim is terminated with a bounded grace period
    // and settled as retryable before this process leaves.
    let control = RunnerControl::new();
    let signal_control = control.clone();
    tokio::spawn(async move {
        if let Err(error) = shutdown_signal().await {
            eprintln!("backtest-runner: shutdown signal handler failed: {error}");
        }
        signal_control.request_shutdown();
    });

    let gate = match ClaimGate::new(capacity_config) {
        Ok(gate) => gate,
        Err(error) => {
            eprintln!("backtest-runner: safety gate invalid: {error}");
            pool.close().await;
            return ExitCode::FAILURE;
        }
    };
    let initial_snapshot = gate.refresh(&queue, &paths.artifacts_root).await;
    if !initial_snapshot.ready {
        eprintln!(
            "event_code={} reason={} free_bytes={:?} queued_backtests={:?}",
            BACKTEST_BACKPRESSURE_EVENT_CODE,
            initial_snapshot.reason,
            initial_snapshot.free_bytes,
            initial_snapshot.queued_backtests,
        );
        if args.once {
            pool.close().await;
            return ExitCode::FAILURE;
        }
    }

    // Take one authoritative snapshot before the first claim even in daemon
    // mode. This prevents a worker from entering a ready-looking loop when
    // the queue tables are readable but the retention reference tables are
    // unavailable or ambiguous. A daemon stays alive and unready so its
    // periodic pass can recover; `--once` fails immediately for automation.
    let initial_reconciliation_ok = match tokio::time::timeout(
        DAEMON_SHUTDOWN_GRACE,
        reconcile_artifacts(&queue, &paths.artifacts_root, reconciler_config),
    )
    .await
    {
        Ok(Ok(report)) => {
            log_reconcile_report(&report);
            true
        }
        Ok(Err(error)) => {
            gate.fail_closed(error.to_string());
            eprintln!("event_code={} error={error}", reconcile_error_code(&error));
            false
        }
        Err(_) => {
            gate.fail_closed("reconciliation deadline exceeded");
            eprintln!(
                "event_code={} error=reconciliation deadline exceeded",
                BACKTEST_RECONCILE_DB_UNAVAILABLE_CODE
            );
            false
        }
    };
    if args.once && !initial_reconciliation_ok {
        pool.close().await;
        return ExitCode::FAILURE;
    }

    if !args.once {
        let reconcile_queue = queue.clone();
        let reconcile_gate = gate.clone();
        let reconcile_root = paths.artifacts_root.clone();
        let reconcile_control = control.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(reconcile_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = reconcile_control.wait_shutdown() => break,
                    _ = ticker.tick() => {
                        // Close the claim gate for the whole pass. If the DB
                        // becomes unavailable after the last healthy refresh,
                        // a stale ready snapshot must not authorize claims
                        // while this authoritative read is in flight.
                        reconcile_gate.fail_closed("authoritative reconciliation in progress");
                        match tokio::time::timeout(
                            DAEMON_SHUTDOWN_GRACE,
                            reconcile_artifacts(&reconcile_queue, &reconcile_root, reconciler_config),
                        ).await {
                            Ok(Ok(report)) => {
                                log_reconcile_report(&report);
                                // Refresh capacity only after the
                                // authoritative retention read succeeds. A
                                // queue-only success must not briefly reopen
                                // claims while result-artifact references are
                                // unavailable.
                                let snapshot = reconcile_gate.refresh(&reconcile_queue, &reconcile_root).await;
                                if !snapshot.ready {
                                    eprintln!(
                                        "event_code={} reason={} free_bytes={:?} queued_backtests={:?}",
                                        BACKTEST_BACKPRESSURE_EVENT_CODE,
                                        snapshot.reason,
                                        snapshot.free_bytes,
                                        snapshot.queued_backtests,
                                    );
                                }
                            }
                            Ok(Err(error)) => {
                                reconcile_gate.fail_closed(error.to_string());
                                eprintln!(
                                    "event_code={} error={error}",
                                    reconcile_error_code(&error),
                                );
                            }
                            Err(_) => {
                                reconcile_gate.fail_closed("reconciliation deadline exceeded");
                                eprintln!(
                                    "event_code={} error=reconciliation deadline exceeded",
                                    BACKTEST_RECONCILE_DB_UNAVAILABLE_CODE,
                                );
                            }
                        }
                    }
                }
            }
        });
    }

    loop {
        // Keep the run future alive after SIGTERM so its cancellation path can
        // settle the claim atomically. Once the signal arrives, the outer
        // budget guarantees that a non-cooperative resolver or database
        // client cannot hold process shutdown forever; an un-settled claim is
        // then left for the lease sweeper, never guessed as committed.
        let Some(outcome) = await_run_with_shutdown_budget(
            &control,
            run_once_with_control_and_gate(&queue, &id, &paths, &resolver, &control, Some(&gate)),
            DAEMON_SHUTDOWN_GRACE,
        )
        .await
        else {
            eprintln!(
                "backtest-runner: shutdown grace {:?} exceeded; leaving claim for sweeper",
                DAEMON_SHUTDOWN_GRACE
            );
            break;
        };

        match outcome {
            Ok(Outcome::Succeeded { job_id }) => eprintln!("job {job_id}: succeeded"),
            Ok(Outcome::Failed { job_id, reason }) => eprintln!("job {job_id}: failed: {reason}"),
            Ok(Outcome::Errored { job_id, reason }) => {
                eprintln!("job {job_id}: runner error, will retry: {reason}");
            }
            Ok(Outcome::Idle) => {
                if args.once || control.is_shutdown() {
                    break;
                }
                if !gate.allows_claim() {
                    let snapshot = gate.snapshot();
                    eprintln!(
                        "event_code={} claims=stopped reason={} free_bytes={:?} queued_backtests={:?}",
                        BACKTEST_BACKPRESSURE_EVENT_CODE,
                        snapshot.reason,
                        snapshot.free_bytes,
                        snapshot.queued_backtests,
                    );
                    tokio::select! {
                        _ = control.wait_shutdown() => break,
                        _ = tokio::time::sleep(ERROR_SLEEP) => {}
                    }
                    continue;
                }
                // Nothing to run is the moment to recover what a dead runner
                // left behind. See the module docs.
                let sweep = tokio::select! {
                    biased;
                    _ = control.wait_shutdown() => None,
                    result = tokio::time::timeout(
                        job_queue::runner::DAEMON_SHUTDOWN_GRACE,
                        queue.sweep(),
                    ) => Some(result),
                };
                match sweep {
                    None => break,
                    Some(Err(_)) => eprintln!("sweep exceeded its deadline"),
                    Some(Ok(Err(e))) => eprintln!("sweep failed: {e}"),
                    Some(Ok(Ok(r)))
                        if r.attempts_orphaned > 0 || r.jobs_requeued > 0 || r.jobs_failed > 0 =>
                    {
                        eprintln!(
                            "swept: {} attempts orphaned, {} jobs requeued, {} exhausted",
                            r.attempts_orphaned, r.jobs_requeued, r.jobs_failed
                        );
                    }
                    Some(Ok(Ok(_))) => {}
                }
                tokio::select! {
                    _ = control.wait_shutdown() => break,
                    _ = tokio::time::sleep(IDLE_SLEEP) => {}
                }
                continue;
            }
            Err(e) => {
                // The queue itself is unreachable. Every claimed job was
                // settled before this returned, so backing off loses nothing
                // and stops a sick database being hammered.
                eprintln!("backtest-runner: queue error: {e}");
                gate.fail_closed(format!("queue operation failed: {e}"));
                if let job_queue::QueueError::CommitUnknown {
                    run_id,
                    job_id,
                    generation_path,
                    detail,
                } = &e
                {
                    eprintln!(
                        "event_code={} run_id={} job_id={} generation_path={} detail={}",
                        job_queue::error::BACKTEST_COMMIT_UNKNOWN_CODE,
                        run_id,
                        job_id,
                        generation_path,
                        detail,
                    );
                }
                if args.once {
                    return ExitCode::FAILURE;
                }
                tokio::select! {
                    _ = control.wait_shutdown() => break,
                    _ = tokio::time::sleep(ERROR_SLEEP) => {}
                }
                continue;
            }
        }

        if args.once {
            break;
        }
        if control.is_shutdown() {
            eprintln!("backtest-runner: shutting down");
            break;
        }
    }

    // Closing returns the connections rather than leaving the server to time
    // them out, which matters when a deploy restarts every replica at once.
    let close_budget = control.remaining_shutdown_budget(DAEMON_SHUTDOWN_GRACE);
    let _ = tokio::time::timeout(close_budget, pool.close()).await;
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn production_rejects_url_and_requires_components() {
        let direct = HashMap::from([(
            "DATABASE_URL",
            OsString::from("postgres://worker:secret@db/lagrange"),
        )]);
        let error = read_database_options_from(|key| direct.get(key).cloned(), true).unwrap_err();
        assert!(error.contains("component mode"));
        assert!(!error.contains("secret"));

        let root = tempfile::tempdir().unwrap();
        let password_file = root.path().join("password");
        std::fs::write(&password_file, "secret").unwrap();
        let components = HashMap::from([
            ("DB_HOST", OsString::from("db")),
            ("DB_PORT", OsString::from("5432")),
            ("DB_NAME", OsString::from("lagrange")),
            ("DB_USER", OsString::from("worker")),
            ("DB_PASSWORD_FILE", password_file.clone().into_os_string()),
        ]);
        read_database_options_from(|key| components.get(key).cloned(), true).unwrap();

        let mut plaintext = components;
        plaintext.insert("DB_PASSWORD", OsString::from("secret"));
        let error =
            read_database_options_from(|key| plaintext.get(key).cloned(), true).unwrap_err();
        assert!(error.contains("DB_PASSWORD"));
        assert!(!error.contains("secret"));
    }

    #[test]
    fn development_and_qa_keep_url_compatibility() {
        let values = HashMap::from([(
            "DATABASE_URL",
            OsString::from("postgres://worker:secret@db/lagrange"),
        )]);
        read_database_options_from(|key| values.get(key).cloned(), false).unwrap();
    }

    #[test]
    fn path_validation_requires_dataset_shape_and_writable_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let dataset = root.path().join("dataset");
        std::fs::create_dir_all(dataset.join("curated/curated/bars")).unwrap();
        let artifacts = root.path().join("artifacts");
        std::fs::create_dir(&artifacts).unwrap();
        let nt = root.path().join("nt");
        std::fs::create_dir_all(nt.join("backtest-worker")).unwrap();
        std::fs::create_dir_all(nt.join("strategies")).unwrap();
        std::fs::create_dir_all(nt.join("custom-data")).unwrap();
        std::fs::write(nt.join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(nt.join("uv.lock"), "version = 1\n").unwrap();
        let paths = RunnerPaths {
            repo_root: root.path().to_path_buf(),
            dataset_root: dataset,
            artifacts_root: artifacts,
            uv_bin: PathBuf::from("uv"),
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        };
        validate_runner_paths(&paths).unwrap();
    }

    #[test]
    fn image_commit_must_be_exact_lowercase_40_hex() {
        let valid = OsString::from("0123456789abcdef0123456789abcdef01234567");
        assert_eq!(
            baked_code_commit_from(|key| (key == "LAGRANGE_CODE_COMMIT").then(|| valid.clone()))
                .unwrap(),
            valid.to_string_lossy()
        );
        for value in [
            "",
            "0123456789abcdef0123456789abcdef0123456",
            "0123456789abcdef0123456789abcdef012345678",
            "0123456789ABCDEF0123456789abcdef01234567",
            "0000000000000000000000000000000000000000",
        ] {
            let value = OsString::from(value);
            let error = baked_code_commit_from(|key| {
                (key == "LAGRANGE_CODE_COMMIT").then(|| value.clone())
            })
            .unwrap_err();
            assert!(error.contains("LAGRANGE_CODE_COMMIT"));
        }
        assert!(baked_code_commit_from(|_| None).is_err());
    }

    #[tokio::test]
    async fn daemon_shutdown_budget_releases_a_stalled_run() {
        let control = RunnerControl::new();
        let signal_control = control.clone();
        let task = tokio::spawn(async move {
            await_run_with_shutdown_budget(
                &signal_control,
                std::future::pending::<()>(),
                Duration::from_millis(20),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        control.request_shutdown();
        let result = task.await.expect("shutdown budget task must join");
        assert!(
            result.is_none(),
            "stalled runs must be abandoned for sweeping"
        );
    }
}
