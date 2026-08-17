//! Production daemon for the common stock-research candidate pipeline.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use job_queue::candidate::runner::{
    CandidateOutcome, CandidateRunnerConfig, CandidateRunnerPaths, run_once,
};
use job_queue::candidate::schedule::{CandidateScheduleError, schedule_latest_candidate_run};
use job_queue::{JobQueue, QueueConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::watch;
use uuid::Uuid;

const DEFAULT_POLL: Duration = Duration::from_secs(2);
const DEFAULT_SCHEDULE_POLL: Duration = Duration::from_secs(60);
const DEFAULT_SWEEP: Duration = Duration::from_secs(30);
const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(10);
const DEFAULT_LEASE: Duration = Duration::from_secs(60);
const DEFAULT_BACKOFF: Duration = Duration::from_secs(30);
const DEFAULT_HEALTH_MAX_AGE: Duration = Duration::from_secs(180);

const HELP: &str = "candidate-runner [--once]\n\
candidate-runner healthcheck\n\
candidate-runner readiness\n\n\
Schedules fully pinned KOSPI-200 candidate runs and drains only candidate_compute jobs.\n\n\
Production environment:\n  \
APP_ENV=production\n  \
DB_HOST/DB_PORT/DB_NAME/DB_USER/DB_PASSWORD_FILE\n  \
CANDIDATE_DATA_ROOT=/data/curated\n  \
CANDIDATE_HEALTH_STATE_PATH=/run/candidate-health/health.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Daemon,
    Once,
    Healthcheck,
    Readiness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Environment {
    Development,
    Qa,
    Production,
}

impl Environment {
    fn read() -> Result<Self, String> {
        match std::env::var("APP_ENV").as_deref() {
            Ok("development") => Ok(Self::Development),
            Ok("qa") => Ok(Self::Qa),
            Ok("production") => Ok(Self::Production),
            Ok(_) => Err("APP_ENV must be development, qa, or production".into()),
            Err(_) => Err("APP_ENV is required".into()),
        }
    }

    const fn production(self) -> bool {
        matches!(self, Self::Production)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Progress {
    pid: u32,
    heartbeat_at: DateTime<Utc>,
    last_progress_at: DateTime<Utc>,
    last_schedule_outcome: String,
    last_run_outcome: String,
}

impl Progress {
    fn starting() -> Self {
        let now = Utc::now();
        Self {
            pid: std::process::id(),
            heartbeat_at: now,
            last_progress_at: now,
            last_schedule_outcome: "STARTING".into(),
            last_run_outcome: "STARTING".into(),
        }
    }
}

#[derive(Debug)]
struct Config {
    mode: Mode,
    environment: Environment,
    data_root: PathBuf,
    health_path: Option<PathBuf>,
    poll: Duration,
    schedule_poll: Duration,
    sweep: Duration,
    heartbeat: Duration,
    lease: Duration,
    backoff: Duration,
    health_max_age: Duration,
    worker_id: String,
}

impl Config {
    fn read() -> Result<Self, String> {
        let mode = parse_mode(std::env::args_os())?;
        let environment = Environment::read()?;
        let data_root = std::env::var_os("CANDIDATE_DATA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/curated"));
        if environment.production() && !data_root.is_absolute() {
            return Err("production CANDIDATE_DATA_ROOT must be absolute".into());
        }
        validate_directory(&data_root, "CANDIDATE_DATA_ROOT")?;
        let health_path = std::env::var_os("CANDIDATE_HEALTH_STATE_PATH")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        if environment.production() && health_path.is_none() {
            return Err("CANDIDATE_HEALTH_STATE_PATH is required in production".into());
        }
        if let Some(path) = health_path.as_deref() {
            validate_health_path(path, environment.production())?;
        }
        let poll = env_duration("CANDIDATE_POLL_MS", DEFAULT_POLL, 1)?;
        let schedule_poll =
            env_duration("CANDIDATE_SCHEDULE_POLL_SECS", DEFAULT_SCHEDULE_POLL, 1000)?;
        let sweep = env_duration("CANDIDATE_SWEEP_MS", DEFAULT_SWEEP, 1)?;
        let heartbeat = env_duration("CANDIDATE_HEARTBEAT_MS", DEFAULT_HEARTBEAT, 1)?;
        let lease = env_duration("CANDIDATE_LEASE_MS", DEFAULT_LEASE, 1)?;
        let backoff = env_duration("CANDIDATE_BACKOFF_MS", DEFAULT_BACKOFF, 1)?;
        let health_max_age = Duration::from_secs(
            env_u64(
                "CANDIDATE_HEALTH_MAX_AGE_SECS",
                Some(DEFAULT_HEALTH_MAX_AGE.as_secs()),
            )?
            .expect("default supplied"),
        );
        CandidateRunnerConfig::new(heartbeat, lease).map_err(|error| error.to_string())?;
        if health_max_age.is_zero() {
            return Err("CANDIDATE_HEALTH_MAX_AGE_SECS must be positive".into());
        }
        let worker_id = std::env::var("CANDIDATE_WORKER_ID").unwrap_or_else(|_| {
            let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".into());
            format!("candidate-runner@{host}/{}", std::process::id())
        });
        if worker_id.trim().is_empty() || worker_id.len() > 128 {
            return Err("CANDIDATE_WORKER_ID must contain 1 to 128 characters".into());
        }
        Ok(Self {
            mode,
            environment,
            data_root,
            health_path,
            poll,
            schedule_poll,
            sweep,
            heartbeat,
            lease,
            backoff,
            health_max_age,
            worker_id,
        })
    }
}

fn parse_mode(values: impl IntoIterator<Item = OsString>) -> Result<Mode, String> {
    let args = values.into_iter().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(Mode::Daemon),
        [value] if value == "--once" => Ok(Mode::Once),
        [value] if value == "healthcheck" => Ok(Mode::Healthcheck),
        [value] if value == "readiness" => Ok(Mode::Readiness),
        [value] if value == "--help" || value == "-h" => Err(HELP.into()),
        _ => Err("invalid arguments (try --help)".into()),
    }
}

fn env_u64(key: &str, default: Option<u64>) -> Result<Option<u64>, String> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| format!("{key} must be a positive integer")),
        Err(_) => Ok(default),
    }
}

fn env_duration(key: &str, default: Duration, unit_ms: u64) -> Result<Duration, String> {
    let value = env_u64(key, None)?.unwrap_or_else(|| {
        if unit_ms == 1 {
            u64::try_from(default.as_millis()).unwrap_or(u64::MAX)
        } else {
            default.as_secs()
        }
    });
    Ok(Duration::from_millis(value.saturating_mul(unit_ms)))
}

fn validate_directory(path: &Path, key: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| format!("{key} is inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{key} must be a non-symlink directory"));
    }
    Ok(())
}

fn validate_health_path(path: &Path, require_absolute: bool) -> Result<(), String> {
    if require_absolute && !path.is_absolute() {
        return Err("production CANDIDATE_HEALTH_STATE_PATH must be absolute".into());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "CANDIDATE_HEALTH_STATE_PATH must have a parent directory".to_owned())?;
    validate_directory(parent, "candidate health directory")?;
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err("candidate health state must be a non-symlink regular file".into());
    }
    Ok(())
}

fn read_secret_file(path: PathBuf, key: &str) -> Result<String, String> {
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|_| format!("{key} is inaccessible"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{key} must be a non-symlink regular file"));
    }
    let value = std::fs::read_to_string(path).map_err(|_| format!("{key} is unreadable"))?;
    if value.contains(['\r', '\n']) || value.is_empty() {
        return Err(format!(
            "{key} must contain exactly one nonempty line without CR/LF"
        ));
    }
    Ok(value)
}

fn database_options(production: bool) -> Result<PgConnectOptions, String> {
    if std::env::var_os("DB_PASSWORD").is_some() {
        return Err("DB_PASSWORD is forbidden; use DB_PASSWORD_FILE".into());
    }
    let direct = std::env::var("DATABASE_URL").ok();
    let direct_file = std::env::var_os("DATABASE_URL_FILE").map(PathBuf::from);
    let keys = [
        "DB_HOST",
        "DB_PORT",
        "DB_NAME",
        "DB_USER",
        "DB_PASSWORD_FILE",
    ];
    let components = keys.map(|key| (key, std::env::var_os(key)));
    let any_component = components.iter().any(|(_, value)| value.is_some());
    if production && (direct.is_some() || direct_file.is_some()) {
        return Err("production requires DB_* component mode".into());
    }
    if (direct.is_some() || direct_file.is_some()) && any_component {
        return Err("DATABASE_URL(_FILE) and DB_* modes are mutually exclusive".into());
    }
    match (direct, direct_file, any_component) {
        (Some(_), Some(_), _) => Err("set only one DATABASE_URL source".into()),
        (Some(url), None, false) if !url.trim().is_empty() => {
            url.parse().map_err(|_| "DATABASE_URL is invalid".into())
        }
        (None, Some(path), false) => read_secret_file(path, "DATABASE_URL_FILE")?
            .parse()
            .map_err(|_| "DATABASE_URL_FILE is invalid".into()),
        (None, None, true) => {
            let mut values = std::collections::HashMap::new();
            for (key, value) in components {
                let value = value
                    .ok_or_else(|| format!("{key} is required with component mode"))?
                    .into_string()
                    .map_err(|_| format!("{key} must be Unicode"))?;
                if value.is_empty() {
                    return Err(format!("{key} must not be empty"));
                }
                values.insert(key, value);
            }
            let port = values["DB_PORT"]
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| "DB_PORT must be a TCP port".to_owned())?;
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
        _ => Err("database configuration is required".into()),
    }
}

fn write_health(path: &Path, progress: &Progress) -> Result<(), String> {
    validate_health_path(path, false)?;
    let parent = path.parent().expect("validated parent");
    let temporary = parent.join(format!(
        ".candidate-health-{}-{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec(progress).map_err(|_| "health serialization failed")?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "health temporary file is inaccessible".to_owned())?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|_| "health write failed".to_owned())?;
        file.write_all(b"\n")
            .map_err(|_| "health write failed".to_owned())?;
        file.sync_all()
            .map_err(|_| "health sync failed".to_owned())?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| "health replace failed".to_owned())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn read_health(path: &Path, max_age: Duration) -> Result<Progress, String> {
    validate_health_path(path, false)?;
    let state: Progress = serde_json::from_slice(
        &std::fs::read(path).map_err(|_| "candidate health state is unreadable".to_owned())?,
    )
    .map_err(|_| "candidate health state is malformed".to_owned())?;
    let age = Utc::now().signed_duration_since(state.heartbeat_at);
    if age < chrono::Duration::zero() || age.to_std().unwrap_or(Duration::MAX) > max_age {
        return Err("candidate heartbeat is stale".into());
    }
    #[cfg(unix)]
    if !Path::new("/proc").join(state.pid.to_string()).is_dir() {
        return Err("candidate process is not alive".into());
    }
    Ok(state)
}

async fn health_writer(
    path: PathBuf,
    mut progress_rx: watch::Receiver<Progress>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut ticker = tokio::time::interval(Duration::from_secs(10));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            changed = progress_rx.changed() => {
                if changed.is_err() { return Ok(()); }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return Ok(()); }
            }
        }
        let mut progress = progress_rx.borrow().clone();
        progress.heartbeat_at = Utc::now();
        let write_path = path.clone();
        tokio::task::spawn_blocking(move || write_health(&write_path, &progress))
            .await
            .map_err(|_| "health writer task failed".to_owned())??;
    }
}

async fn probe(pool: &sqlx::PgPool, config: &Config, readiness: bool) -> Result<(), String> {
    let path = config
        .health_path
        .as_deref()
        .ok_or_else(|| "candidate health state is unconfigured".to_owned())?;
    let progress = read_health(path, config.health_max_age)?;
    let (active, latest_feed, expected_feed, queued, oldest_age): (
        bool,
        Option<chrono::NaiveDate>,
        Option<chrono::NaiveDate>,
        i64,
        Option<i64>,
    ) = sqlx::query_as(
            "SELECT
                COALESCE((SELECT active FROM candidate_scheduler_control WHERE control_key='scheduler'), false),
                (SELECT max(as_of_date) FROM candidate_feed_snapshots WHERE status='PUBLISHED'),
                (SELECT max(calendar.session_date)
                   FROM trading_calendars AS calendar
                   JOIN candidate_scheduler_control AS control
                     ON control.control_key = 'scheduler'
                  WHERE calendar.exchange = 'KRX'
                    AND calendar.session_type = 'TRADING'
                    AND calendar.timezone = 'Asia/Seoul'
                    AND calendar.source_batch_id IS NOT NULL
                    AND calendar.content_sha256 IS NOT NULL
                    AND calendar.retrieved_at IS NOT NULL
                    AND (
                      calendar.session_date <
                        (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
                      OR (
                        calendar.session_date =
                          (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
                        AND (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::time
                          >= control.wake_at_kst
                      )
                    )),
                (SELECT count(*) FROM jobs WHERE job_type='candidate_compute' AND status IN ('QUEUED','RUNNING')),
                (SELECT EXTRACT(EPOCH FROM clock_timestamp() - min(created_at))::bigint
                   FROM jobs WHERE job_type='candidate_compute' AND status IN ('QUEUED','RUNNING'))",
        )
        .fetch_one(pool)
        .await
        .map_err(|_| "candidate database is unavailable".to_owned())?;
    let progress_age = Utc::now().signed_duration_since(progress.last_progress_at);
    if progress_age < chrono::Duration::zero()
        || progress_age.to_std().unwrap_or(Duration::MAX) > config.health_max_age * 3
    {
        return Err("candidate main loop made no recent progress".into());
    }
    if readiness && !active {
        return Err("candidate scheduler is disabled".into());
    }
    if readiness && (expected_feed.is_none() || latest_feed != expected_feed) {
        return Err("candidate feed is not current for the latest closed KRX session".into());
    }
    println!(
        "{}",
        json!({
            "status": if readiness { "ready" } else { "healthy" },
            "pid": progress.pid,
            "heartbeat_at": progress.heartbeat_at,
            "last_progress_at": progress.last_progress_at,
            "last_schedule_outcome": progress.last_schedule_outcome,
            "last_run_outcome": progress.last_run_outcome,
            "scheduler_active": active,
            "latest_feed_as_of": latest_feed,
            "expected_feed_as_of": expected_feed,
            "queued_or_running": queued,
            "oldest_queue_age_seconds": oldest_age,
        })
    );
    Ok(())
}

fn schedule_label(
    result: &Result<
        job_queue::candidate::schedule::CandidateScheduleReport,
        CandidateScheduleError,
    >,
) -> String {
    match result {
        Ok(report) => format!("SCHEDULED:{}", report.run_id),
        Err(CandidateScheduleError::SourceUnavailable) => "WAITING_FOR_SOURCES".into(),
        Err(CandidateScheduleError::Invalid(_)) => "INVALID_SOURCE_CONFIG".into(),
        Err(CandidateScheduleError::Database(_)) => "DATABASE_UNAVAILABLE".into(),
    }
}

fn outcome_label(outcome: &CandidateOutcome) -> String {
    match outcome {
        CandidateOutcome::Idle => "IDLE".into(),
        CandidateOutcome::Succeeded { run_id, .. } => format!("SUCCEEDED:{run_id}"),
        CandidateOutcome::Retrying { code, .. } => format!("RETRYING:{code}"),
        CandidateOutcome::Blocked { code, .. } => format!("BLOCKED:{code}"),
        CandidateOutcome::Failed { code, .. } => format!("FAILED:{code}"),
        CandidateOutcome::LeaseLost { .. } => "LEASE_LOST".into(),
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::read() {
        Ok(config) => config,
        Err(message) if message == HELP => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!(
                "{}",
                json!({ "event": "candidate_startup_failed", "message": message })
            );
            return ExitCode::FAILURE;
        }
    };
    let options = match database_options(config.environment.production()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!(
                "{}",
                json!({ "event": "candidate_startup_failed", "message": message })
            );
            return ExitCode::FAILURE;
        }
    };
    let pool = match PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
    {
        Ok(pool) => pool,
        Err(_) => {
            eprintln!(
                "{}",
                json!({ "event": "candidate_startup_failed", "code": "DATABASE_UNAVAILABLE" })
            );
            return ExitCode::FAILURE;
        }
    };
    if matches!(config.mode, Mode::Healthcheck | Mode::Readiness) {
        let result = probe(&pool, &config, config.mode == Mode::Readiness).await;
        pool.close().await;
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!(
                    "{}",
                    json!({ "event": "candidate_probe_failed", "message": message })
                );
                ExitCode::FAILURE
            }
        };
    }

    let queue = JobQueue::new(
        pool.clone(),
        None,
        QueueConfig {
            lease: config.lease,
            backoff_base: config.backoff,
        },
    );
    let runner_paths = CandidateRunnerPaths {
        data_root: config.data_root.clone(),
    };
    let runner_config = CandidateRunnerConfig::new(config.heartbeat, config.lease)
        .expect("configuration validated");
    let (progress_tx, progress_rx) = watch::channel(Progress::starting());
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let health_task = config
        .health_path
        .clone()
        .map(|path| tokio::spawn(health_writer(path, progress_rx, shutdown_rx.clone())));
    if config.mode == Mode::Daemon {
        let signal_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            let _ = signal_tx.send(true);
        });
    }

    let seoul = FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset");
    let mut next_schedule = tokio::time::Instant::now();
    let mut next_sweep = tokio::time::Instant::now();
    let mut exit = ExitCode::SUCCESS;
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        let now = tokio::time::Instant::now();
        let mut progress = progress_tx.borrow().clone();
        if now >= next_schedule {
            let schedule =
                schedule_latest_candidate_run(&pool, Utc::now().with_timezone(&seoul)).await;
            progress.last_schedule_outcome = schedule_label(&schedule);
            if let Err(error) = &schedule
                && !matches!(error, CandidateScheduleError::SourceUnavailable)
            {
                eprintln!(
                    "{}",
                    json!({ "event": "candidate_schedule_failed", "message": error.to_string() })
                );
            }
            next_schedule = now + config.schedule_poll;
        }
        if now >= next_sweep {
            if let Err(error) = queue.sweep().await {
                eprintln!(
                    "{}",
                    json!({ "event": "candidate_sweep_failed", "message": error.to_string() })
                );
            }
            next_sweep = now + config.sweep;
        }
        match run_once(
            &pool,
            &queue,
            &config.worker_id,
            &runner_paths,
            &runner_config,
        )
        .await
        {
            Ok(outcome) => {
                progress.last_run_outcome = outcome_label(&outcome);
            }
            Err(error) => {
                progress.last_run_outcome = "RUNNER_UNAVAILABLE".into();
                eprintln!(
                    "{}",
                    json!({ "event": "candidate_runner_failed", "message": error.to_string() })
                );
                if config.mode == Mode::Once {
                    exit = ExitCode::FAILURE;
                }
            }
        }
        progress.last_progress_at = Utc::now();
        let _ = progress_tx.send(progress);
        if config.mode == Mode::Once {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(config.poll) => {}
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() { break; }
            }
        }
    }
    let _ = shutdown_tx.send(true);
    if let Some(task) = health_task {
        let _ = task.await;
    }
    pool.close().await;
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_is_strict() {
        assert_eq!(
            parse_mode([OsString::from("candidate-runner"), OsString::from("--once")]).unwrap(),
            Mode::Once
        );
        assert!(parse_mode([OsString::from("candidate-runner"), OsString::from("nope")]).is_err());
    }

    #[test]
    fn schedule_labels_do_not_treat_missing_sources_as_success() {
        let value = schedule_label(&Err(CandidateScheduleError::SourceUnavailable));
        assert_eq!(value, "WAITING_FOR_SOURCES");
    }
}
