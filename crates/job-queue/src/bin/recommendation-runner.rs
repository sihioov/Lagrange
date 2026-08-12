//! Sequential daemon for the fixed-universe recommendation queue.

use std::ffi::OsString;
#[cfg(any(unix, test))]
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use job_queue::recommendation::child::TargetChildPaths;
use job_queue::recommendation::{
    RecommendationOutcome, RecommendationRunnerConfig, RecommendationRunnerPaths, run_once,
};
use job_queue::{JobQueue, QueueConfig};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::watch;

const HELP: &str = "recommendation-runner [OPTIONS]\n\n\
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
  DATABASE_URL or DATABASE_URL_FILE (exactly one)\n  \
  APP_ENV (production requires every path option explicitly)";

#[derive(Debug, Clone)]
struct Args {
    once: bool,
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
    let mut pending = values.peekable();
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

fn read_database_url() -> Result<String, String> {
    let direct = std::env::var("DATABASE_URL").ok();
    let file = std::env::var_os("DATABASE_URL_FILE").map(PathBuf::from);
    match (direct, file) {
        (Some(_), Some(_)) => {
            Err("set exactly one of DATABASE_URL or DATABASE_URL_FILE".to_owned())
        }
        (Some(url), None) if !url.trim().is_empty() => Ok(url),
        (None, Some(path)) => {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| "DATABASE_URL_FILE is inaccessible".to_owned())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("DATABASE_URL_FILE must be a non-symlink regular file".to_owned());
            }
            let value = std::fs::read_to_string(path)
                .map_err(|_| "DATABASE_URL_FILE is unreadable".to_owned())?;
            let value = value.trim().to_owned();
            if value.is_empty() {
                Err("DATABASE_URL_FILE is empty".to_owned())
            } else {
                Ok(value)
            }
        }
        _ => Err("DATABASE_URL or DATABASE_URL_FILE is required".to_owned()),
    }
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
    let database_url = match read_database_url() {
        Ok(url) => url,
        Err(message) => {
            event(
                "startup_failed",
                json!({ "code": "DATABASE_CONFIG_INVALID", "message": message }),
            );
            return ExitCode::FAILURE;
        }
    };
    let pool = match PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
    {
        Ok(pool) => pool,
        Err(_) => {
            event("startup_failed", json!({ "code": "DATABASE_UNAVAILABLE" }));
            return ExitCode::FAILURE;
        }
    };
    drop(database_url);
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
    loop {
        if *shutdown_rx.borrow() {
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
