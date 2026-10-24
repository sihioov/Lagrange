//! Dedicated parent/child executable for the owner-beta simulation queue.

use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, process::ExitCode};
#[cfg(target_os = "linux")]
use std::{fs, path::Path, time::Duration};

use job_queue::owner_beta::simulation_execution::run_simulation_child_mode;
#[cfg(target_os = "linux")]
use job_queue::{
    JobQueue, QueueConfig,
    owner_beta::simulation_execution::{
        SimulationWorkerOutcome, TrustedSimulationRuntime, run_simulation_once,
    },
};
#[cfg(target_os = "linux")]
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
#[cfg(target_os = "linux")]
use tokio::sync::watch;

#[cfg(target_os = "linux")]
const DAEMON_POLL: Duration = Duration::from_secs(2);

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownTrigger {
    CtrlC,
    Sigterm,
}

#[cfg(target_os = "linux")]
fn request_shutdown(shutdown: &watch::Sender<bool>, _trigger: ShutdownTrigger) {
    shutdown.send_replace(true);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Once,
    Daemon,
}

struct ParentArgs {
    mode: Mode,
    artifact_root: PathBuf,
    build_receipt: PathBuf,
    expected_receipt_sha256: String,
    worker_id: String,
}

struct ChildArgs {
    artifact_root: PathBuf,
    build_receipt: PathBuf,
    expected_receipt_sha256: String,
}

enum ParsedArgs {
    Parent(ParentArgs),
    Child(ChildArgs),
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(args) => args,
        Err(()) => {
            eprintln!("simulation runner arguments are invalid");
            return ExitCode::from(2);
        }
    };
    match args {
        ParsedArgs::Child(args) => ExitCode::from(run_simulation_child_mode(
            &args.artifact_root,
            &args.build_receipt,
            &args.expected_receipt_sha256,
        )),
        ParsedArgs::Parent(args) => run_parent(args),
    }
}

fn run_parent(args: ParentArgs) -> ExitCode {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        eprintln!("simulation runner supports Linux only");
        ExitCode::from(7)
    }
    #[cfg(target_os = "linux")]
    run_parent_linux(args)
}

#[cfg(target_os = "linux")]
fn run_parent_linux(args: ParentArgs) -> ExitCode {
    let trusted_runtime = match TrustedSimulationRuntime::load(
        &args.artifact_root,
        &args.build_receipt,
        &args.expected_receipt_sha256,
    ) {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("simulation runner provenance rejected");
            return ExitCode::from(3);
        }
    };
    let options = match database_options() {
        Ok(options) => options,
        Err(()) => {
            eprintln!("simulation runner database configuration rejected");
            return ExitCode::from(4);
        }
    };
    let tokio_runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("simulation runner runtime unavailable");
            return ExitCode::from(5);
        }
    };
    match tokio_runtime.block_on(run_parent_async(args, options, trusted_runtime)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => {
            eprintln!("simulation runner stopped with a static failure");
            ExitCode::from(6)
        }
    }
}

#[cfg(target_os = "linux")]
async fn run_parent_async(
    args: ParentArgs,
    options: PgConnectOptions,
    trusted_runtime: TrustedSimulationRuntime,
) -> Result<(), ()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| ())?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|_| ())?;
    let queue = JobQueue::new(pool.clone(), None, QueueConfig::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal = tokio::spawn(async move {
        let trigger = tokio::select! {
            _ = tokio::signal::ctrl_c() => ShutdownTrigger::CtrlC,
            _ = terminate.recv() => ShutdownTrigger::Sigterm,
        };
        request_shutdown(&shutdown_tx, trigger);
    });

    let result = match args.mode {
        Mode::Once => {
            match run_simulation_once(&queue, &args.worker_id, &trusted_runtime, shutdown_rx).await
            {
                Ok(outcome) => {
                    report_outcome(outcome);
                    Ok(())
                }
                Err(_) => Err(()),
            }
        }
        Mode::Daemon => run_daemon(&queue, &args.worker_id, &trusted_runtime, shutdown_rx).await,
    };
    signal.abort();
    let _ = signal.await;
    pool.close().await;
    result
}

#[cfg(target_os = "linux")]
async fn run_daemon(
    queue: &JobQueue,
    worker_id: &str,
    runtime: &TrustedSimulationRuntime,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ()> {
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let outcome = run_simulation_once(queue, worker_id, runtime, shutdown.clone())
            .await
            .map_err(|_| ())?;
        report_outcome(outcome);
        if outcome == SimulationWorkerOutcome::Idle {
            tokio::select! {
                _ = tokio::time::sleep(DAEMON_POLL) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn report_outcome(outcome: SimulationWorkerOutcome) {
    let label = match outcome {
        SimulationWorkerOutcome::Idle => "idle",
        SimulationWorkerOutcome::Published => "published",
        SimulationWorkerOutcome::Retrying => "retrying",
        SimulationWorkerOutcome::Failed => "failed",
        SimulationWorkerOutcome::Canceled => "canceled",
        SimulationWorkerOutcome::ClaimLost => "claim-lost",
        SimulationWorkerOutcome::CommitUnresolved => "commit-unresolved",
    };
    eprintln!("simulation runner outcome: {label}");
}

fn parse_args(values: Vec<OsString>) -> Result<ParsedArgs, ()> {
    if values.first().is_some_and(|value| value == "--child") {
        let options = parse_options(&values[1..])?;
        require_exact_keys(
            &options,
            &[
                "--artifact-root",
                "--build-receipt",
                "--expected-receipt-sha256",
            ],
        )?;
        let args = ChildArgs {
            artifact_root: required_path(&options, "--artifact-root")?,
            build_receipt: required_path(&options, "--build-receipt")?,
            expected_receipt_sha256: required_string(&options, "--expected-receipt-sha256")?,
        };
        if !args.artifact_root.is_absolute()
            || !args.build_receipt.is_absolute()
            || !valid_sha256(&args.expected_receipt_sha256)
        {
            return Err(());
        }
        return Ok(ParsedArgs::Child(args));
    }

    let (mode, rest) = match values.first().and_then(|value| value.to_str()) {
        Some("--once") => (Mode::Once, &values[1..]),
        Some("--daemon") => (Mode::Daemon, &values[1..]),
        _ => return Err(()),
    };
    let options = parse_options(rest)?;
    require_exact_keys(
        &options,
        &[
            "--artifact-root",
            "--build-receipt",
            "--expected-receipt-sha256",
            "--worker-id",
        ],
    )?;
    let args = ParentArgs {
        mode,
        artifact_root: required_path(&options, "--artifact-root")?,
        build_receipt: required_path(&options, "--build-receipt")?,
        expected_receipt_sha256: required_string(&options, "--expected-receipt-sha256")?,
        worker_id: required_string(&options, "--worker-id")?,
    };
    if !args.artifact_root.is_absolute()
        || !args.build_receipt.is_absolute()
        || !valid_sha256(&args.expected_receipt_sha256)
        || !valid_worker_id(&args.worker_id)
    {
        return Err(());
    }
    Ok(ParsedArgs::Parent(args))
}

fn parse_options(values: &[OsString]) -> Result<BTreeMap<String, OsString>, ()> {
    if values.len() % 2 != 0 {
        return Err(());
    }
    let mut options = BTreeMap::new();
    for pair in values.chunks_exact(2) {
        let key = pair[0].to_str().ok_or(())?.to_owned();
        if !key.starts_with("--") || options.insert(key, pair[1].clone()).is_some() {
            return Err(());
        }
    }
    Ok(options)
}

fn require_exact_keys(options: &BTreeMap<String, OsString>, expected: &[&str]) -> Result<(), ()> {
    if options.len() != expected.len() || expected.iter().any(|key| !options.contains_key(*key)) {
        Err(())
    } else {
        Ok(())
    }
}

fn required_path(options: &BTreeMap<String, OsString>, name: &str) -> Result<PathBuf, ()> {
    options
        .get(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(())
}

fn required_string(options: &BTreeMap<String, OsString>, name: &str) -> Result<String, ()> {
    options
        .get(name)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_worker_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

#[cfg(target_os = "linux")]
fn database_options() -> Result<PgConnectOptions, ()> {
    let app_env = std::env::var("APP_ENV").map_err(|_| ())?;
    let production = app_env == "production";
    if !matches!(app_env.as_str(), "development" | "qa" | "production")
        || std::env::var_os("DB_PASSWORD").is_some()
    {
        return Err(());
    }
    let direct = std::env::var("DATABASE_URL").ok();
    let direct_file = std::env::var_os("DATABASE_URL_FILE").map(PathBuf::from);
    let component_keys = [
        "DB_HOST",
        "DB_PORT",
        "DB_NAME",
        "DB_USER",
        "DB_PASSWORD_FILE",
    ];
    let components = component_keys.map(|key| (key, std::env::var_os(key)));
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
        (None, Some(path), false) => read_secret_file(&path)?.parse().map_err(|_| ()),
        (None, None, true) => {
            let value = |key: &str| {
                components
                    .iter()
                    .find(|(name, _)| *name == key)
                    .and_then(|(_, value)| value.clone())
                    .and_then(|value| value.into_string().ok())
                    .filter(|value| !value.is_empty())
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
                .password(&read_secret_file(Path::new(&value("DB_PASSWORD_FILE")?))?))
        }
        _ => Err(()),
    }
}

#[cfg(target_os = "linux")]
fn read_secret_file(path: &Path) -> Result<String, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 16 * 1024 {
        return Err(());
    }
    let value = fs::read_to_string(path).map_err(|_| ())?;
    if value.is_empty() || value.contains(['\r', '\n']) {
        Err(())
    } else {
        Ok(value)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_and_sigterm_request_the_same_cooperative_shutdown() {
        for trigger in [ShutdownTrigger::CtrlC, ShutdownTrigger::Sigterm] {
            let (shutdown, mut receiver) = watch::channel(false);
            request_shutdown(&shutdown, trigger);
            assert!(*receiver.borrow_and_update());
        }
    }
}
