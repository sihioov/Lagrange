//! Sequential daemon for the fixed-universe recommendation queue.

use std::ffi::OsString;
#[cfg(any(unix, test))]
use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use job_queue::recommendation::child::TargetChildPaths;
use job_queue::recommendation::compute::AttestedUniverse;
use job_queue::recommendation::input::DatasetPin;
use job_queue::recommendation::schedule::{
    ScheduleError, eligible_schedule_date, run_schedule_cycle,
};
use job_queue::recommendation::{
    RecommendationOutcome, RecommendationRunnerConfig, RecommendationRunnerPaths, run_once,
};
use job_queue::{JobQueue, QueueConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::watch;
use uuid::Uuid;

const DEFAULT_HEALTH_MAX_AGE: Duration = Duration::from_secs(180);

const HELP: &str = "recommendation-runner [OPTIONS]\nrecommendation-runner healthcheck\n\n\
Drains only `recommendation` jobs and publishes validated fixed-ETF targets.\n\n\
Options:\n  \
  --once                       claim at most one job\n  \
  --worker-id ID               explicit queue lease owner\n  \
  --repo-root PATH             repository root containing nt/\n  \
  --data-root PATH             allowed root for pinned curated data\n  \
  --universe-manifest PATH     pinned kr-etf-core-v1 manifest\n  \
  --uv-bin PATH                absolute uv executable\n  \
  --temp-root PATH             pre-created child scratch root\n  \
  --poll-ms N                  idle poll interval (default 2000)\n  \
  --sweep-ms N                 orphan sweep interval (default 30000)\n  \
  --heartbeat-ms N             lease heartbeat interval (default 10000)\n  \
  --lease-ms N                 queue lease (default 60000)\n  \
  --backoff-ms N               first retry backoff (default 30000)\n  \
  --child-timeout-ms N         child deadline (default 120000)\n\n\
Environment:\n  \
  DATABASE_URL, DATABASE_URL_FILE, or DB_HOST/DB_PORT/DB_NAME/DB_USER/DB_PASSWORD_FILE\n  \
  RECOMMENDATION_HEALTH_STATE_PATH (required in production)\n  \
  APP_ENV (production requires every path option explicitly)";

#[derive(Debug, Clone)]
struct Args {
    once: bool,
    healthcheck: bool,
    worker_id: Option<String>,
    repo_root: Option<PathBuf>,
    data_root: Option<PathBuf>,
    universe_manifest: Option<PathBuf>,
    uv_bin: Option<PathBuf>,
    temp_root: Option<PathBuf>,
    poll: Duration,
    sweep: Duration,
    heartbeat: Duration,
    lease: Duration,
    backoff: Duration,
    child_timeout: Duration,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            once: false,
            healthcheck: false,
            worker_id: None,
            repo_root: None,
            data_root: None,
            universe_manifest: None,
            uv_bin: None,
            temp_root: None,
            poll: Duration::from_secs(2),
            sweep: Duration::from_secs(30),
            heartbeat: Duration::from_secs(10),
            lease: Duration::from_secs(60),
            backoff: Duration::from_secs(30),
            child_timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Debug)]
struct ResolvedPaths {
    runner: RecommendationRunnerPaths,
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

fn parse_args_from<I, S>(values: I) -> Result<Args, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut values = values.into_iter().map(Into::into);
    let _program = values.next();
    let mut args = Args::default();
    let remaining = values.collect::<Vec<_>>();
    if matches!(remaining.as_slice(), [value] if value == "healthcheck") {
        args.healthcheck = true;
        return Ok(args);
    }
    let mut pending = remaining.into_iter().peekable();
    while let Some(raw) = pending.next() {
        let arg = raw
            .into_string()
            .map_err(|_| "arguments must be valid Unicode".to_owned())?;
        match arg.as_str() {
            "--once" => args.once = true,
            "--help" | "-h" => return Err(HELP.to_owned()),
            "--worker-id" => args.worker_id = Some(next_text(&mut pending, &arg)?),
            "--repo-root" => args.repo_root = Some(PathBuf::from(next_text(&mut pending, &arg)?)),
            "--data-root" => args.data_root = Some(PathBuf::from(next_text(&mut pending, &arg)?)),
            "--universe-manifest" => {
                args.universe_manifest = Some(PathBuf::from(next_text(&mut pending, &arg)?));
            }
            "--uv-bin" => args.uv_bin = Some(PathBuf::from(next_text(&mut pending, &arg)?)),
            "--temp-root" => args.temp_root = Some(PathBuf::from(next_text(&mut pending, &arg)?)),
            "--poll-ms" => args.poll = next_duration(&mut pending, &arg)?,
            "--sweep-ms" => args.sweep = next_duration(&mut pending, &arg)?,
            "--heartbeat-ms" => args.heartbeat = next_duration(&mut pending, &arg)?,
            "--lease-ms" => args.lease = next_duration(&mut pending, &arg)?,
            "--backoff-ms" => args.backoff = next_duration(&mut pending, &arg)?,
            "--child-timeout-ms" => args.child_timeout = next_duration(&mut pending, &arg)?,
            _ => return Err(format!("unrecognized argument {arg:?} (try --help)")),
        }
    }
    RecommendationRunnerConfig::new(args.heartbeat, args.lease, args.child_timeout)
        .map_err(|error| error.to_string())?;
    if args.poll.is_zero() || args.sweep.is_zero() || args.backoff.is_zero() {
        return Err("runner durations must be positive".to_owned());
    }
    if args
        .worker_id
        .as_ref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err("worker id must not be empty".to_owned());
    }
    Ok(args)
}

fn next_text<I>(values: &mut std::iter::Peekable<I>, option: &str) -> Result<String, String>
where
    I: Iterator<Item = OsString>,
{
    values
        .next()
        .ok_or_else(|| format!("{option} requires a value"))?
        .into_string()
        .map_err(|_| format!("{option} value must be valid Unicode"))
}

fn next_duration<I>(values: &mut std::iter::Peekable<I>, option: &str) -> Result<Duration, String>
where
    I: Iterator<Item = OsString>,
{
    let value = next_text(values, option)?;
    let millis = value
        .parse::<u64>()
        .map_err(|_| format!("{option} must be a positive integer number of milliseconds"))?;
    if millis == 0 {
        return Err(format!("{option} must be positive"));
    }
    Ok(Duration::from_millis(millis))
}

fn resolve_paths(
    args: &Args,
    app_env: AppEnvironment,
    cwd: PathBuf,
) -> Result<ResolvedPaths, String> {
    let production = app_env.is_production();
    if production
        && [
            args.repo_root.as_ref(),
            args.data_root.as_ref(),
            args.universe_manifest.as_ref(),
            args.uv_bin.as_ref(),
            args.temp_root.as_ref(),
        ]
        .iter()
        .any(|value| value.is_none())
    {
        return Err(
            "production requires every repository/data/universe/uv/temp path explicitly".to_owned(),
        );
    }
    let repo_root = args.repo_root.clone().unwrap_or(cwd);
    let data_root = args
        .data_root
        .clone()
        .unwrap_or_else(|| repo_root.join("data/phase0"));
    let universe_manifest = args
        .universe_manifest
        .clone()
        .unwrap_or_else(|| repo_root.join("configs/universes/kr-etf-core-v1.yaml"));
    let uv_bin = args.uv_bin.clone().unwrap_or_else(default_uv_bin);
    let temp_root = args
        .temp_root
        .clone()
        .unwrap_or_else(|| repo_root.join(".tmp/recommendations"));
    let all = [
        &repo_root,
        &data_root,
        &universe_manifest,
        &uv_bin,
        &temp_root,
    ];
    if production && all.iter().any(|path| !path.is_absolute()) {
        return Err("production paths must be absolute".to_owned());
    }
    if universe_manifest.file_name().and_then(|name| name.to_str()) != Some("kr-etf-core-v1.yaml") {
        return Err("universe manifest must use the immutable kr-etf-core-v1.yaml pin".to_owned());
    }
    validate_existing_directory(&repo_root, "repository root")?;
    validate_existing_directory(&data_root, "data root")?;
    validate_existing_file(&universe_manifest, "universe manifest")?;
    validate_existing_file(&uv_bin, "uv executable")?;
    validate_existing_directory(&temp_root, "temp root")?;
    let universe_yaml = std::fs::read_to_string(&universe_manifest)
        .map_err(|_| "universe manifest is unreadable".to_owned())?;
    AttestedUniverse::from_manifest_yaml(&universe_yaml).map_err(|error| {
        format!("universe manifest is not the immutable 11-member pin: {error}")
    })?;
    Ok(ResolvedPaths {
        runner: RecommendationRunnerPaths {
            data_root,
            universe_manifest,
            child: TargetChildPaths {
                uv_bin,
                repo_root,
                temp_root,
            },
        },
    })
}

fn validate_existing_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| format!("{label} is missing or inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} must be a non-symlink directory"));
    }
    Ok(())
}

fn validate_existing_file(path: &Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| format!("{label} is missing or inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} must be a non-symlink regular file"));
    }
    Ok(())
}

fn default_uv_bin() -> PathBuf {
    let name = if cfg!(windows) { "uv.exe" } else { "uv" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| candidate.canonicalize().ok())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn read_secret_file(path: PathBuf, key: &str) -> Result<String, String> {
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|_| format!("{key} is inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{key} must be a non-symlink regular file"));
    }
    let value = std::fs::read_to_string(path).map_err(|_| format!("{key} is unreadable"))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(format!("{key} is empty"))
    } else {
        Ok(value)
    }
}

fn read_database_options_from<F>(get: F) -> Result<PgConnectOptions, String>
where
    F: Fn(&str) -> Option<OsString>,
{
    let direct = get("DATABASE_URL").and_then(|value| value.into_string().ok());
    let file = get("DATABASE_URL_FILE").map(PathBuf::from);
    let keys = [
        "DB_HOST",
        "DB_PORT",
        "DB_NAME",
        "DB_USER",
        "DB_PASSWORD_FILE",
    ];
    let components = keys.map(|key| (key, get(key)));
    let any_component = components.iter().any(|(_, value)| value.is_some());
    if (direct.is_some() || file.is_some()) && any_component {
        return Err(
            "DATABASE_URL(_FILE) and DB_* component modes are mutually exclusive".to_owned(),
        );
    }
    match (direct, file, any_component) {
        (Some(_), Some(_), _) => {
            Err("set exactly one of DATABASE_URL or DATABASE_URL_FILE".to_owned())
        }
        (Some(url), None, false) if !url.trim().is_empty() => url
            .parse()
            .map_err(|_| "DATABASE_URL is invalid".to_owned()),
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
            Ok(PgConnectOptions::new()
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

fn read_database_options() -> Result<PgConnectOptions, String> {
    read_database_options_from(|key| std::env::var_os(key))
}

fn schedule_pin_from_parts(
    parts: [Option<String>; 5],
    required: bool,
) -> Result<Option<DatasetPin>, String> {
    if parts.iter().all(Option::is_none) {
        return if required {
            Err("production requires the immutable recommendation schedule dataset pin".to_owned())
        } else {
            Ok(None)
        };
    }
    if parts.iter().any(Option::is_none) {
        return Err("recommendation schedule dataset pin must be configured completely".to_owned());
    }
    let [id, dataset_id, version, curated_version, manifest_sha256] = parts.map(Option::unwrap);
    let id = Uuid::parse_str(&id)
        .map_err(|_| "RECOMMENDATION_DATASET_VERSION_ID must be a UUID".to_owned())?;
    let curated_version = curated_version
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "RECOMMENDATION_CURATED_VERSION must be positive".to_owned())?;
    if dataset_id.is_empty()
        || version.is_empty()
        || manifest_sha256.len() != 64
        || !manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err("recommendation schedule dataset pin is invalid".to_owned());
    }
    Ok(Some(DatasetPin {
        id,
        dataset_id,
        version,
        curated_version,
        manifest_sha256,
    }))
}

fn read_schedule_pin(required: bool) -> Result<Option<DatasetPin>, String> {
    schedule_pin_from_parts(
        [
            "RECOMMENDATION_DATASET_VERSION_ID",
            "RECOMMENDATION_DATASET_ID",
            "RECOMMENDATION_DATASET_VERSION",
            "RECOMMENDATION_CURATED_VERSION",
            "RECOMMENDATION_DATASET_MANIFEST_SHA256",
        ]
        .map(|key| std::env::var(key).ok()),
        required,
    )
}

fn worker_id(explicit: Option<String>) -> String {
    explicit.unwrap_or_else(|| {
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown-host".to_owned());
        format!("recommendation-runner@{host}/{}", std::process::id())
    })
}

fn event(kind: &str, fields: serde_json::Value) {
    eprintln!("{}", json!({ "event": kind, "fields": fields }));
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduleHealth {
    attempted_at: DateTime<Utc>,
    outcome: String,
    scheduled: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthState {
    pid: u32,
    heartbeat_at: DateTime<Utc>,
    last_schedule: Option<ScheduleHealth>,
}

impl HealthState {
    fn new(now: DateTime<Utc>, pid: u32) -> Self {
        Self {
            pid,
            heartbeat_at: now,
            last_schedule: None,
        }
    }
}

fn health_state_path(required: bool) -> Result<Option<PathBuf>, String> {
    match std::env::var_os("RECOMMENDATION_HEALTH_STATE_PATH") {
        Some(value) if !value.is_empty() => Ok(Some(PathBuf::from(value))),
        _ if required => {
            Err("RECOMMENDATION_HEALTH_STATE_PATH is required in production".to_owned())
        }
        _ => Ok(None),
    }
}

fn health_max_age() -> Result<Duration, String> {
    let seconds = std::env::var("RECOMMENDATION_HEALTH_MAX_AGE_SECS")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| "RECOMMENDATION_HEALTH_MAX_AGE_SECS must be a positive integer".to_owned())?
        .unwrap_or(DEFAULT_HEALTH_MAX_AGE.as_secs());
    if seconds == 0 {
        return Err("RECOMMENDATION_HEALTH_MAX_AGE_SECS must be positive".to_owned());
    }
    Ok(Duration::from_secs(seconds))
}

fn write_health_state(path: &Path, state: &HealthState) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "health state path must have a parent directory".to_owned())?;
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|_| "health state directory is inaccessible".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("health state directory must be a non-symlink directory".to_owned());
    }
    let temporary = parent.join(format!(
        ".recommendation-health-{}-{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    let bytes =
        serde_json::to_vec(state).map_err(|_| "health state serialization failed".to_owned())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "health state temporary file is inaccessible".to_owned())?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|_| "health state write failed".to_owned())?;
        file.write_all(b"\n")
            .map_err(|_| "health state write failed".to_owned())?;
        file.sync_all()
            .map_err(|_| "health state sync failed".to_owned())?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| "health state replacement failed".to_owned())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_health_state(
    path: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
) -> Result<HealthState, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "health state is absent".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("health state must be a non-symlink regular file".to_owned());
    }
    let state: HealthState = serde_json::from_slice(
        &std::fs::read(path).map_err(|_| "health state is unreadable".to_owned())?,
    )
    .map_err(|_| "health state is malformed".to_owned())?;
    let age = now.signed_duration_since(state.heartbeat_at);
    if age < chrono::Duration::zero() || age.to_std().unwrap_or(Duration::MAX) > max_age {
        return Err("health state heartbeat is stale".to_owned());
    }
    if !process_alive(state.pid) {
        return Err("health state process is not alive".to_owned());
    }
    Ok(state)
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).is_dir()
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    // The deployment contract is Linux/systemd. A fresh heartbeat remains the
    // portable development-host liveness signal.
    true
}

async fn run_healthcheck(
    pool: &sqlx::PgPool,
    state_path: &Path,
    max_age: Duration,
) -> Result<(), String> {
    let state = read_health_state(state_path, Utc::now(), max_age)?;
    let (queue_age_seconds, blocked_runs): (Option<i64>, i64) = sqlx::query_as(
        "SELECT EXTRACT(EPOCH FROM now() - min(created_at))::bigint, \
                (SELECT count(*) FROM recommendation_runs WHERE status = 'BLOCKED') \
         FROM jobs WHERE job_type = 'recommendation' AND status IN ('QUEUED', 'RUNNING')",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "recommendation database is unavailable".to_owned())?;
    println!(
        "{}",
        json!({
            "status": "ok",
            "process": { "pid": state.pid, "heartbeat_at": state.heartbeat_at },
            "database": "reachable",
            "last_schedule": state.last_schedule,
            "queue_age_seconds": queue_age_seconds,
            "blocked_recommendation_runs": blocked_runs,
        })
    );
    Ok(())
}

#[derive(Debug, Default)]
struct ScheduleCadence {
    completed_key: Option<NaiveDate>,
}

impl ScheduleCadence {
    fn pending_key(&self, now_kst: DateTime<FixedOffset>) -> Option<NaiveDate> {
        eligible_schedule_date(now_kst).filter(|key| Some(*key) != self.completed_key)
    }

    fn complete(&mut self, key: NaiveDate) {
        self.completed_key = Some(key);
    }
}

#[cfg(any(unix, test))]
async fn wait_for_shutdown<C, T>(ctrl_c: C, terminate: T) -> std::io::Result<()>
where
    C: Future<Output = std::io::Result<()>>,
    T: Future<Output = std::io::Result<()>>,
{
    tokio::select! {
        result = ctrl_c => result,
        result = terminate => result,
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    wait_for_shutdown(tokio::signal::ctrl_c(), async move {
        terminate.recv().await.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "terminate signal stream closed",
            )
        })?;
        Ok(())
    })
    .await
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args_from(std::env::args_os()) {
        Ok(args) => args,
        Err(message) if message == HELP => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "INVALID_CONFIG", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let app_env = match resolve_app_environment(std::env::var_os("APP_ENV")) {
        Ok(app_env) => app_env,
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "INVALID_APP_ENV", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let health_state_path = match health_state_path(app_env.is_production()) {
        Ok(path) => path,
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "INVALID_HEALTH_CONFIG", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let database_options = match read_database_options() {
        Ok(options) => options,
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "DATABASE_CONFIG_INVALID", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let schedule_pin = match read_schedule_pin(app_env.is_production()) {
        Ok(pin) => pin,
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "INVALID_SCHEDULE_PIN", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let pool = match PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(database_options)
        .await
    {
        Ok(pool) => pool,
        Err(_) => {
            event("startup_failed", json!({ "code": "DATABASE_UNAVAILABLE" }));
            return ExitCode::FAILURE;
        }
    };
    if args.healthcheck {
        let Some(state_path) = health_state_path.as_deref() else {
            event(
                "health_failed",
                json!({ "code": "HEALTH_STATE_UNCONFIGURED" }),
            );
            pool.close().await;
            return ExitCode::FAILURE;
        };
        let health_result = match health_max_age() {
            Ok(max_age) => run_healthcheck(&pool, state_path, max_age).await,
            Err(message) => Err(message),
        };
        let exit = match health_result {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                event(
                    "health_failed",
                    json!({ "code": "RECOMMENDATION_UNHEALTHY", "message": message }),
                );
                ExitCode::FAILURE
            }
        };
        pool.close().await;
        return exit;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(_) => {
            event(
                "startup_failed",
                json!({ "code": "CURRENT_DIRECTORY_UNAVAILABLE" }),
            );
            return ExitCode::FAILURE;
        }
    };
    let paths = match resolve_paths(&args, app_env, cwd) {
        Ok(paths) => paths,
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "INVALID_PATH_CONFIG", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let id = worker_id(args.worker_id.clone());
    let queue = JobQueue::new(
        pool.clone(),
        None,
        QueueConfig {
            lease: args.lease,
            backoff_base: args.backoff,
        },
    );
    let runner_config =
        RecommendationRunnerConfig::new(args.heartbeat, args.lease, args.child_timeout)
            .expect("arguments were validated")
            .with_production(app_env.is_production());
    event(
        "runner_started",
        json!({ "worker_id": id, "once": args.once }),
    );
    let mut health_state = HealthState::new(Utc::now(), std::process::id());
    if let Some(path) = health_state_path.as_deref()
        && let Err(message) = write_health_state(path, &health_state)
    {
        event(
            "startup_failed",
            json!({ "code": "HEALTH_STATE_UNAVAILABLE", "message": message }),
        );
        pool.close().await;
        return ExitCode::FAILURE;
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    if !args.once {
        let signal_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            if shutdown_signal().await.is_ok() {
                let _ = signal_tx.send(true);
            }
        });
    }
    let sweep_task = if args.once {
        None
    } else {
        let sweep_queue = queue.clone();
        let mut stop = shutdown_rx.clone();
        let sweep_interval = args.sweep;
        Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(sweep_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() { break; }
                    }
                    _ = ticker.tick() => match sweep_queue.sweep().await {
                        Ok(report) if report.attempts_orphaned > 0 || report.jobs_requeued > 0 || report.jobs_failed > 0 => {
                            event("queue_swept", json!({
                                "attempts_orphaned": report.attempts_orphaned,
                                "jobs_requeued": report.jobs_requeued,
                                "jobs_failed": report.jobs_failed,
                            }));
                        }
                        Ok(_) => {}
                        Err(_) => event("sweep_failed", json!({ "code": "QUEUE_UNAVAILABLE" })),
                    }
                }
            }
        }))
    };

    let mut exit = ExitCode::SUCCESS;
    let seoul = FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset");
    let mut schedule_cadence = ScheduleCadence::default();
    let mut next_schedule_attempt = tokio::time::Instant::now();
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        if let Some(pin) = schedule_pin.as_ref() {
            let now_kst = Utc::now().with_timezone(&seoul);
            if let Some(key) = schedule_cadence.pending_key(now_kst)
                && tokio::time::Instant::now() >= next_schedule_attempt
            {
                match run_schedule_cycle(pool.clone(), pin.clone(), now_kst).await {
                    Ok(report) => {
                        schedule_cadence.complete(key);
                        health_state.last_schedule = Some(ScheduleHealth {
                            attempted_at: Utc::now(),
                            outcome: "succeeded".to_owned(),
                            scheduled: Some(report.scheduled as u64),
                        });
                        event(
                            "schedule_cycle",
                            json!({
                                "as_of": report.as_of,
                                "scheduled": report.scheduled,
                            }),
                        );
                    }
                    Err(ScheduleError::NoConfirmedClose | ScheduleError::DatasetUnavailable) => {
                        health_state.last_schedule = Some(ScheduleHealth {
                            attempted_at: Utc::now(),
                            outcome: "blocked".to_owned(),
                            scheduled: None,
                        });
                        event(
                            "schedule_blocked",
                            json!({ "code": "SCHEDULE_INPUT_UNAVAILABLE" }),
                        );
                        next_schedule_attempt =
                            tokio::time::Instant::now() + Duration::from_secs(60);
                    }
                    Err(ScheduleError::Database(_)) => {
                        health_state.last_schedule = Some(ScheduleHealth {
                            attempted_at: Utc::now(),
                            outcome: "failed".to_owned(),
                            scheduled: None,
                        });
                        event("schedule_failed", json!({ "code": "QUEUE_UNAVAILABLE" }));
                        next_schedule_attempt =
                            tokio::time::Instant::now() + Duration::from_secs(60);
                    }
                }
            }
        }
        health_state.heartbeat_at = Utc::now();
        if let Some(path) = health_state_path.as_deref()
            && let Err(message) = write_health_state(path, &health_state)
        {
            event(
                "runner_error",
                json!({ "code": "HEALTH_STATE_UNAVAILABLE", "message": message }),
            );
            exit = ExitCode::FAILURE;
            break;
        }
        match run_once(&pool, &queue, &id, &paths.runner, &runner_config).await {
            Ok(RecommendationOutcome::Idle) => {
                if args.once {
                    break;
                }
            }
            Ok(RecommendationOutcome::Succeeded { job_id, run_id }) => {
                event(
                    "job_succeeded",
                    json!({ "job_id": job_id, "run_id": run_id }),
                );
            }
            Ok(RecommendationOutcome::Blocked { job_id, code }) => {
                event("job_blocked", json!({ "job_id": job_id, "code": code }));
            }
            Ok(RecommendationOutcome::Failed { job_id, code }) => {
                event("job_failed", json!({ "job_id": job_id, "code": code }));
            }
            Ok(RecommendationOutcome::Retrying { job_id, code }) => {
                event("job_retrying", json!({ "job_id": job_id, "code": code }));
            }
            Err(_) => {
                event("runner_error", json!({ "code": "QUEUE_UNAVAILABLE" }));
                if args.once {
                    exit = ExitCode::FAILURE;
                    break;
                }
            }
        }
        if args.once || *shutdown_rx.borrow() {
            break;
        }
        let mut stop = shutdown_rx.clone();
        tokio::select! {
            _ = tokio::time::sleep(args.poll) => {}
            _ = stop.changed() => {}
        }
    }
    let _ = shutdown_tx.send(true);
    if let Some(task) = sweep_task {
        let _ = task.await;
    }
    event("runner_stopped", json!({ "worker_id": id }));
    pool.close().await;
    exit
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn scheduler_catches_up_at_startup_and_runs_the_new_close_key_once() {
        let seoul = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        let startup = seoul.with_ymd_and_hms(2026, 5, 11, 9, 0, 0).unwrap();
        let before_close = seoul.with_ymd_and_hms(2026, 5, 11, 16, 29, 59).unwrap();
        let at_close = seoul.with_ymd_and_hms(2026, 5, 11, 16, 30, 0).unwrap();
        let mut cadence = ScheduleCadence::default();

        let startup_key = cadence
            .pending_key(startup)
            .expect("startup catches up the latest eligible close bound");
        assert_eq!(startup_key.to_string(), "2026-05-10");
        cadence.complete(startup_key);
        assert_eq!(cadence.pending_key(before_close), None);

        let close_key = cadence
            .pending_key(at_close)
            .expect("16:30 KST opens exactly one new daily key");
        assert_eq!(close_key.to_string(), "2026-05-11");
        cadence.complete(close_key);
        assert_eq!(cadence.pending_key(at_close), None);
    }

    #[test]
    fn scheduler_failure_does_not_complete_the_key_and_can_retry() {
        let seoul = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        let now = seoul.with_ymd_and_hms(2026, 5, 11, 16, 30, 0).unwrap();
        let cadence = ScheduleCadence::default();

        let failed_key = cadence.pending_key(now).expect("first attempt is due");
        assert_eq!(cadence.pending_key(now), Some(failed_key));
    }

    #[test]
    fn parser_rejects_zero_and_heartbeat_not_before_lease() {
        let zero = parse_args_from(["recommendation-runner", "--heartbeat-ms", "0"])
            .expect_err("zero duration must fail");
        assert!(zero.contains("positive"));

        let late = parse_args_from([
            "recommendation-runner",
            "--heartbeat-ms",
            "1000",
            "--lease-ms",
            "1000",
        ])
        .expect_err("heartbeat equal to lease must fail");
        assert!(late.contains("shorter"));
    }

    #[test]
    fn production_requires_explicit_absolute_immutable_paths() {
        let args = parse_args_from(["recommendation-runner", "--once"]).unwrap();
        let error = resolve_paths(&args, AppEnvironment::Production, PathBuf::from("C:\\repo"))
            .expect_err("production defaults must fail closed");
        assert!(error.contains("production"));

        let relative = parse_args_from([
            "recommendation-runner",
            "--repo-root",
            ".",
            "--data-root",
            "data",
            "--universe-manifest",
            "universe.yaml",
            "--uv-bin",
            "uv",
            "--temp-root",
            "tmp",
        ])
        .unwrap();
        let error = resolve_paths(
            &relative,
            AppEnvironment::Production,
            PathBuf::from("C:\\repo"),
        )
        .expect_err("relative production paths must fail closed");
        assert!(error.contains("absolute"));
    }

    #[test]
    fn parser_accepts_explicit_polling_and_one_shot() {
        let args = parse_args_from([
            "recommendation-runner",
            "--once",
            "--poll-ms",
            "250",
            "--sweep-ms",
            "5000",
            "--heartbeat-ms",
            "1000",
            "--lease-ms",
            "10000",
            "--child-timeout-ms",
            "60000",
        ])
        .unwrap();
        assert!(args.once);
        assert_eq!(args.poll.as_millis(), 250);
        assert_eq!(args.sweep.as_millis(), 5000);
    }

    #[test]
    fn app_environment_is_closed_and_canonical() {
        assert_eq!(
            resolve_app_environment(Some("development".into())).unwrap(),
            AppEnvironment::Development
        );
        assert_eq!(
            resolve_app_environment(Some("qa".into())).unwrap(),
            AppEnvironment::Qa
        );
        assert_eq!(
            resolve_app_environment(Some("production".into())).unwrap(),
            AppEnvironment::Production
        );
        assert!(
            resolve_app_environment(None)
                .unwrap_err()
                .contains("required")
        );
        for invalid in ["prod", "Production", "production ", "staging", ""] {
            assert!(
                resolve_app_environment(Some(invalid.into())).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn schedule_pin_is_all_or_nothing_and_required_in_production() {
        assert!(
            schedule_pin_from_parts([None, None, None, None, None], false)
                .unwrap()
                .is_none()
        );
        assert!(schedule_pin_from_parts([None, None, None, None, None], true).is_err());
        assert!(
            schedule_pin_from_parts(
                [
                    Some(Uuid::nil().to_string()),
                    Some("krx_eod_bars".into()),
                    None,
                    Some("2".into()),
                    Some("a".repeat(64)),
                ],
                false
            )
            .is_err()
        );
        let pin = schedule_pin_from_parts(
            [
                Some(Uuid::nil().to_string()),
                Some("krx_eod_bars".into()),
                Some("v2".into()),
                Some("2".into()),
                Some("a".repeat(64)),
            ],
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pin.curated_version, 2);
    }

    #[test]
    fn health_state_rejects_missing_stale_and_malformed_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recommendation-runner.json");
        let now = Utc::now();

        assert!(read_health_state(&path, now, Duration::from_secs(60)).is_err());

        std::fs::write(&path, "not json").unwrap();
        assert!(read_health_state(&path, now, Duration::from_secs(60)).is_err());

        std::fs::write(
            &path,
            format!(
                r#"{{"pid":{},"heartbeat_at":"{}","last_schedule":null}}"#,
                std::process::id(),
                (now - chrono::Duration::seconds(61)).to_rfc3339(),
            ),
        )
        .unwrap();
        assert!(read_health_state(&path, now, Duration::from_secs(60)).is_err());
    }

    #[test]
    fn health_state_accepts_a_fresh_runner_heartbeat() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recommendation-runner.json");
        let now = Utc::now();
        write_health_state(&path, &HealthState::new(now, std::process::id())).unwrap();

        let state = read_health_state(&path, now, Duration::from_secs(60)).unwrap();
        assert_eq!(state.pid, std::process::id());
    }

    #[test]
    fn default_health_window_outlasts_the_longest_child_run() {
        assert!(
            DEFAULT_HEALTH_MAX_AGE > Args::default().child_timeout,
            "a healthy runner must not go stale while one child is within its deadline"
        );
    }

    #[test]
    fn component_database_mode_requires_every_field_without_constructing_a_url() {
        let root = tempfile::tempdir().unwrap();
        let password_file = root.path().join("password");
        std::fs::write(&password_file, "pa:ss@word\n").unwrap();
        let values = std::collections::HashMap::from([
            ("DB_HOST", "db.internal".into()),
            ("DB_PORT", "5432".into()),
            ("DB_NAME", "lagrange".into()),
            ("DB_USER", "worker".into()),
            ("DB_PASSWORD_FILE", password_file.into_os_string()),
        ]);
        read_database_options_from(|key| values.get(key).cloned()).unwrap();

        let incomplete = std::collections::HashMap::from([("DB_HOST", "db".into())]);
        assert!(read_database_options_from(|key| incomplete.get(key).cloned()).is_err());
    }

    #[test]
    fn database_modes_reject_mixed_url_and_component_configuration() {
        let values = std::collections::HashMap::from([
            (
                "DATABASE_URL",
                "postgres://worker:secret@db/lagrange".into(),
            ),
            ("DB_HOST", "db".into()),
        ]);
        let error = read_database_options_from(|key| values.get(key).cloned()).unwrap_err();
        assert!(error.contains("mutually exclusive"));
        assert!(!error.contains("secret"));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn app_environment_rejects_non_unicode_without_echoing_it() {
        #[cfg(unix)]
        let invalid = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0xff])
        };
        #[cfg(windows)]
        let invalid = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0xd800])
        };

        let error = resolve_app_environment(Some(invalid)).expect_err("APP_ENV must be Unicode");
        assert_eq!(error, "APP_ENV must be valid Unicode");
    }

    #[tokio::test]
    async fn shutdown_selection_observes_terminate() {
        let result = wait_for_shutdown(
            std::future::pending::<std::io::Result<()>>(),
            std::future::ready(Ok(())),
        )
        .await;
        assert!(result.is_ok());
    }
}
