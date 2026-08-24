//! Dedicated daemon for the sealed `owner_beta_price_recommendation` queue.
//!
//! An empty approval registry is a valid deployment state: this worker may
//! start, but individual jobs fail closed during the sealed computation.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use job_queue::owner_beta::{
    OwnerBetaOutcome, OwnerBetaRunnerConfig, OwnerBetaRunnerPaths, recover_owner_beta_claims,
    run_once,
};
use job_queue::{JobQueue, QueueConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::watch;

const DEFAULT_POLL: Duration = Duration::from_secs(2);
const DEFAULT_RECOVERY: Duration = Duration::from_secs(30);
const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(10);
const DEFAULT_LEASE: Duration = Duration::from_secs(60);
const DEFAULT_BACKOFF: Duration = Duration::from_secs(30);
const DEFAULT_COMPUTE_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_HEALTH_MAX_AGE: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Daemon,
    Once,
    Healthcheck,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Environment {
    Development,
    Qa,
    Production,
}
impl Environment {
    fn read() -> Result<Self, ()> {
        match std::env::var("APP_ENV").as_deref() {
            Ok("development") => Ok(Self::Development),
            Ok("qa") => Ok(Self::Qa),
            Ok("production") => Ok(Self::Production),
            _ => Err(()),
        }
    }
    const fn production(self) -> bool {
        matches!(self, Self::Production)
    }
}

#[derive(Clone)]
struct Config {
    mode: Mode,
    production: bool,
    artifact_root: PathBuf,
    health_path: Option<PathBuf>,
    poll: Duration,
    recovery: Duration,
    heartbeat: Duration,
    lease: Duration,
    backoff: Duration,
    compute_timeout: Duration,
    health_max_age: Duration,
    worker_id: String,
}

fn positive_duration(key: &str, default: Duration) -> Result<Duration, ()> {
    let value = match std::env::var(key) {
        Ok(value) => value.parse::<u64>().map_err(|_| ())?,
        Err(std::env::VarError::NotPresent) => default.as_secs(),
        Err(std::env::VarError::NotUnicode(_)) => return Err(()),
    };
    if value == 0 {
        Err(())
    } else {
        Ok(Duration::from_secs(value))
    }
}

fn parse_args_from(values: Vec<std::ffi::OsString>) -> Result<(Mode, Option<PathBuf>), ()> {
    let values = values.into_iter().skip(1).collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok((Mode::Daemon, None)),
        [mode] if mode == "--once" => Ok((Mode::Once, None)),
        [mode] if mode == "healthcheck" => Ok((Mode::Healthcheck, None)),
        [mode, option, path]
            if mode == "--once" && option == "--artifact-root" && !path.is_empty() =>
        {
            Ok((Mode::Once, Some(PathBuf::from(path))))
        }
        [option, path] if option == "--artifact-root" && !path.is_empty() => {
            Ok((Mode::Daemon, Some(PathBuf::from(path))))
        }
        _ => Err(()),
    }
}

fn parse_args() -> Result<(Mode, Option<PathBuf>), ()> {
    parse_args_from(std::env::args_os().collect())
}

fn valid_directory(path: &Path, absolute: bool) -> bool {
    (!absolute || path.is_absolute())
        && std::fs::symlink_metadata(path)
            .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
}

fn valid_worker_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn config() -> Result<Config, ()> {
    let (mode, specified_root) = parse_args()?;
    let environment = Environment::read()?;
    let production = environment.production();
    let specified_artifact_root = specified_root.is_some();
    let artifact_root =
        specified_root.unwrap_or_else(|| PathBuf::from("historical-price-beta-root"));
    if !matches!(mode, Mode::Healthcheck)
        && (((production || specified_artifact_root) && !artifact_root.is_absolute())
            || !valid_directory(&artifact_root, production))
    {
        return Err(());
    }
    let health_path = std::env::var_os("PRICE_BETA_HEALTH_STATE_PATH").map(PathBuf::from);
    if production
        && health_path
            .as_deref()
            .is_none_or(|path| !path.is_absolute())
    {
        return Err(());
    }
    if let Some(path) = health_path.as_deref()
        && !path
            .parent()
            .is_some_and(|parent| valid_directory(parent, false))
    {
        return Err(());
    }
    let heartbeat = positive_duration("PRICE_BETA_HEARTBEAT_SECS", DEFAULT_HEARTBEAT)?;
    let lease = positive_duration("PRICE_BETA_LEASE_SECS", DEFAULT_LEASE)?;
    if heartbeat >= lease {
        return Err(());
    }
    let worker_id = std::env::var("PRICE_BETA_WORKER_ID")
        .unwrap_or_else(|_| format!("owner-beta-{}", std::process::id()));
    if !valid_worker_id(&worker_id) {
        return Err(());
    }
    Ok(Config {
        mode,
        production,
        artifact_root,
        health_path,
        poll: positive_duration("PRICE_BETA_POLL_SECS", DEFAULT_POLL)?,
        recovery: positive_duration("PRICE_BETA_RECOVERY_SECS", DEFAULT_RECOVERY)?,
        heartbeat,
        lease,
        backoff: positive_duration("PRICE_BETA_BACKOFF_SECS", DEFAULT_BACKOFF)?,
        compute_timeout: positive_duration(
            "PRICE_BETA_COMPUTE_TIMEOUT_SECS",
            DEFAULT_COMPUTE_TIMEOUT,
        )?,
        health_max_age: positive_duration(
            "PRICE_BETA_HEALTH_MAX_AGE_SECS",
            DEFAULT_HEALTH_MAX_AGE,
        )?,
        worker_id,
    })
}

fn secret(path: PathBuf) -> Result<String, ()> {
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| ())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(());
    }
    let value = std::fs::read_to_string(path).map_err(|_| ())?;
    if value.is_empty() || value.contains(['\r', '\n']) {
        Err(())
    } else {
        Ok(value)
    }
}

fn database_options(production: bool) -> Result<PgConnectOptions, ()> {
    if std::env::var_os("DB_PASSWORD").is_some() {
        return Err(());
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
    let any_components = components.iter().any(|(_, value)| value.is_some());
    if production && (direct.is_some() || direct_file.is_some()) {
        return Err(());
    }
    if (direct.is_some() || direct_file.is_some()) && any_components {
        return Err(());
    }
    match (direct, direct_file, any_components) {
        (Some(_), Some(_), _) => Err(()),
        (Some(url), None, false) => url.parse().map_err(|_| ()),
        (None, Some(path), false) => secret(path)?.parse().map_err(|_| ()),
        (None, None, true) => {
            let value = |key| {
                components
                    .iter()
                    .find(|(name, _)| *name == key)
                    .and_then(|(_, v)| v.clone())
                    .and_then(|v| v.into_string().ok())
                    .filter(|v| !v.is_empty())
                    .ok_or(())
            };
            let port = value("DB_PORT")?
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or(())?;
            Ok(PgConnectOptions::new()
                .host(&value("DB_HOST")?)
                .port(port)
                .database(&value("DB_NAME")?)
                .username(&value("DB_USER")?)
                .password(&secret(PathBuf::from(value("DB_PASSWORD_FILE")?))?))
        }
        _ => Err(()),
    }
}

#[derive(Serialize, Deserialize)]
struct Health {
    pid: u32,
    heartbeat_unix_seconds: u64,
}
fn write_health(path: &Path) -> Result<(), ()> {
    let parent = path.parent().ok_or(())?;
    let temporary = parent.join(format!(".owner-beta-{}.tmp", std::process::id()));
    let state = Health {
        pid: std::process::id(),
        heartbeat_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ())?
            .as_secs(),
    };
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| ())?;
    let result = (|| {
        file.write_all(&serde_json::to_vec(&state).map_err(|_| ())?)
            .map_err(|_| ())?;
        file.sync_all().map_err(|_| ())?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| ())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
fn healthcheck(path: Option<&Path>, max_age: Duration) -> Result<(), ()> {
    let path = path.ok_or(())?;
    let health: Health =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| ())?).map_err(|_| ())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?
        .as_secs();
    if now.saturating_sub(health.heartbeat_unix_seconds) > max_age.as_secs() {
        return Err(());
    }
    #[cfg(unix)]
    if !Path::new("/proc").join(health.pid.to_string()).is_dir() {
        return Err(());
    }
    Ok(())
}
async fn health_writer(path: PathBuf, mut shutdown: watch::Receiver<bool>) -> Result<(), ()> {
    let mut ticker = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! { _ = ticker.tick() => { let write_path = path.clone(); tokio::task::spawn_blocking(move || write_health(&write_path)).await.map_err(|_| ())??; }
        changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return Ok(()); } }
    }
}
#[cfg(unix)]
async fn shutdown_signal() {
    let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
}
#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
fn outcome_label(outcome: &OwnerBetaOutcome) -> &'static str {
    match outcome {
        OwnerBetaOutcome::Idle => "IDLE",
        OwnerBetaOutcome::Succeeded { .. } => "SUCCEEDED",
        OwnerBetaOutcome::Retrying { .. } => "RETRYING",
        OwnerBetaOutcome::Blocked { .. } => "BLOCKED",
        OwnerBetaOutcome::Failed { .. } => "FAILED",
        OwnerBetaOutcome::Canceled { .. } => "CANCELED",
        OwnerBetaOutcome::LeaseLost { .. } => "LEASE_LOST",
        OwnerBetaOutcome::Indeterminate { .. } | OwnerBetaOutcome::RejectedIndeterminate { .. } => {
            "INDETERMINATE"
        }
        OwnerBetaOutcome::Rejected { .. } | OwnerBetaOutcome::RejectedCanceled { .. } => "REJECTED",
    }
}

fn once_failed(outcome: &OwnerBetaOutcome) -> bool {
    matches!(
        outcome,
        OwnerBetaOutcome::LeaseLost { .. }
            | OwnerBetaOutcome::Indeterminate { .. }
            | OwnerBetaOutcome::RejectedIndeterminate { .. }
    )
}

// Recovery integrity is a prerequisite for every new claim. Continuing after
// a failed recovery could execute beside an unresolved expired owner-beta
// claim, so both one-shot and daemon modes fail closed for supervisor restart.
const fn recovery_failure_exits() -> bool {
    true
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match config() {
        Ok(config) => config,
        Err(()) => {
            eprintln!(
                "{}",
                json!({"event":"owner_beta_startup","code":"CONFIG_INVALID"})
            );
            return ExitCode::FAILURE;
        }
    };
    if config.mode == Mode::Healthcheck {
        return if healthcheck(config.health_path.as_deref(), config.health_max_age).is_ok() {
            ExitCode::SUCCESS
        } else {
            eprintln!(
                "{}",
                json!({"event":"owner_beta_health","code":"UNHEALTHY"})
            );
            ExitCode::FAILURE
        };
    }
    let options = match database_options(config.production) {
        Ok(options) => options,
        Err(()) => {
            eprintln!(
                "{}",
                json!({"event":"owner_beta_startup","code":"DATABASE_CONFIG_INVALID"})
            );
            return ExitCode::FAILURE;
        }
    };
    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
    {
        Ok(pool) => pool,
        Err(_) => {
            eprintln!(
                "{}",
                json!({"event":"owner_beta_startup","code":"DATABASE_UNAVAILABLE"})
            );
            return ExitCode::FAILURE;
        }
    };
    let queue = JobQueue::new(
        pool.clone(),
        None,
        QueueConfig {
            lease: config.lease,
            backoff_base: config.backoff,
        },
    );
    let paths = OwnerBetaRunnerPaths {
        artifact_root: config.artifact_root.clone(),
    };
    let runner = OwnerBetaRunnerConfig::new(config.heartbeat, config.lease, config.compute_timeout)
        .expect("validated durations");
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let mut health_task = config
        .health_path
        .clone()
        .map(|path| tokio::spawn(health_writer(path, shutdown_rx.clone())));
    if config.mode == Mode::Daemon {
        let tx = shutdown_tx.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            let _ = tx.send(true);
        });
    }
    let mut next_recovery = tokio::time::Instant::now();
    let mut exit = ExitCode::SUCCESS;
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        if health_task
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            eprintln!(
                "{}",
                json!({"event":"owner_beta_health","code":"WRITER_UNAVAILABLE"})
            );
            exit = ExitCode::FAILURE;
            break;
        }
        let now = tokio::time::Instant::now();
        if now >= next_recovery {
            if recover_owner_beta_claims(&queue).await.is_err() {
                eprintln!(
                    "{}",
                    json!({"event":"owner_beta_recovery","code":"RECOVERY_UNAVAILABLE"})
                );
                if recovery_failure_exits() {
                    exit = ExitCode::FAILURE;
                    break;
                }
            }
            next_recovery = now + config.recovery;
        }
        match run_once(&pool, &queue, &config.worker_id, &paths, &runner).await {
            Ok(outcome) => {
                eprintln!(
                    "{}",
                    json!({"event":"owner_beta_run","outcome":outcome_label(&outcome)})
                );
                // `Succeeded`, `Retrying`, `Blocked`, `Failed`, `Canceled`, and
                // rejected claims are durable terminal handling for --once.
                if config.mode == Mode::Once && once_failed(&outcome) {
                    exit = ExitCode::FAILURE;
                }
            }
            Err(_) => {
                eprintln!(
                    "{}",
                    json!({"event":"owner_beta_run","code":"RUNNER_UNAVAILABLE"})
                );
                if config.mode == Mode::Once {
                    exit = ExitCode::FAILURE;
                }
            }
        }
        if config.mode == Mode::Once {
            break;
        }
        tokio::select! { _ = tokio::time::sleep(config.poll) => {}, changed = shutdown_rx.changed() => if changed.is_err() || *shutdown_rx.borrow() { break; } }
    }
    let _ = shutdown_tx.send(true);
    if let Some(task) = health_task.take()
        && !matches!(task.await, Ok(Ok(())))
    {
        eprintln!(
            "{}",
            json!({"event":"owner_beta_health","code":"WRITER_UNAVAILABLE"})
        );
        exit = ExitCode::FAILURE;
    }
    pool.close().await;
    exit
}

#[cfg(test)]
mod tests {
    use super::{Mode, parse_args_from};
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        std::iter::once(OsString::from("owner-beta-runner"))
            .chain(values.iter().map(OsString::from))
            .collect()
    }

    #[test]
    fn parser_accepts_only_the_declared_mode_grammar() {
        assert!(matches!(
            parse_args_from(args(&[])),
            Ok((Mode::Daemon, None))
        ));
        assert!(matches!(
            parse_args_from(args(&["--once"])),
            Ok((Mode::Once, None))
        ));
        assert!(matches!(
            parse_args_from(args(&["healthcheck"])),
            Ok((Mode::Healthcheck, None))
        ));
        assert!(matches!(
            parse_args_from(args(&["--artifact-root", "/data/a"])),
            Ok((Mode::Daemon, Some(_)))
        ));
        assert!(matches!(
            parse_args_from(args(&["--once", "--artifact-root", "/data/a"])),
            Ok((Mode::Once, Some(_)))
        ));
    }

    #[test]
    fn parser_rejects_mixed_or_duplicate_options() {
        for values in [
            &["healthcheck", "--artifact-root", "/data/a"][..],
            &["--artifact-root", "/data/a", "healthcheck"][..],
            &["--once", "--once"][..],
            &["--artifact-root", "/data/a", "--artifact-root", "/data/b"][..],
            &["--once", "healthcheck"][..],
        ] {
            assert!(parse_args_from(args(values)).is_err());
        }
    }

    #[test]
    fn recovery_failure_prevents_a_new_claim() {
        assert!(super::recovery_failure_exits());
    }
}
