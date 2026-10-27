//! Dedicated parent/child executable for the owner-beta simulation queue.

use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, process::ExitCode};
#[cfg(target_os = "linux")]
use std::{fs, path::Path, time::Duration};

use job_queue::owner_beta::simulation_execution::{
    SimulationProcessConfig, SimulationProcessReadError, run_simulation_child_mode,
};
#[cfg(target_os = "linux")]
use job_queue::owner_beta::simulation_execution::{
    check_simulation_process_health, read_simulation_process_config, run_official_simulation_daemon,
};
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

struct HealthArgs {
    expected_commit: String,
    expected_config_sha256: String,
    expected_receipt_sha256: String,
}

enum ParsedArgs {
    Parent(ParentArgs),
    Child(ChildArgs),
    Health(HealthArgs),
    Official(HealthArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthOutcome {
    Healthy,
    Rejected,
    UnsupportedPlatform,
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
        ParsedArgs::Health(args) => run_health(args),
        ParsedArgs::Official(args) => run_official(args),
    }
}

fn run_official(args: HealthArgs) -> ExitCode {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        eprintln!("simulation runner supports Linux only");
        ExitCode::from(7)
    }
    #[cfg(target_os = "linux")]
    {
        let result = (|| {
            if std::env::var("APP_ENV").ok().as_deref() != Some("production")
                || std::env::var("DB_USER").ok().as_deref() != Some("worker")
                || std::env::var_os("DATABASE_URL").is_some()
                || std::env::var_os("DATABASE_URL_FILE").is_some()
            {
                return Err(());
            }
            let options = database_options()?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| ())?;
            runtime.block_on(async move {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .map_err(|_| ())?;
                let (sender, receiver) = watch::channel(false);
                let signal = tokio::spawn(async move {
                    let trigger = tokio::select! {
                        _ = tokio::signal::ctrl_c() => ShutdownTrigger::CtrlC,
                        _ = terminate.recv() => ShutdownTrigger::Sigterm,
                    };
                    request_shutdown(&sender, trigger);
                });
                let result = run_official_simulation_daemon(
                    args.expected_commit,
                    args.expected_config_sha256,
                    args.expected_receipt_sha256,
                    options,
                    receiver,
                )
                .await;
                signal.abort();
                let _ = signal.await;
                result
            })
        })();
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(()) => {
                eprintln!("simulation runner official execution rejected");
                ExitCode::from(9)
            }
        }
    }
}

fn run_health(args: HealthArgs) -> ExitCode {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        return emit_health_outcome(HealthOutcome::UnsupportedPlatform);
    }

    #[cfg(target_os = "linux")]
    {
        let outcome = dispatch_health(
            &args,
            true,
            read_simulation_process_config,
            check_simulation_process_health,
        );
        emit_health_outcome(outcome)
    }
}

fn dispatch_health<ReadConfig, CheckHealth>(
    args: &HealthArgs,
    linux_supported: bool,
    read_config: ReadConfig,
    check_health: CheckHealth,
) -> HealthOutcome
where
    ReadConfig: FnOnce(&str, &str) -> Result<SimulationProcessConfig, SimulationProcessReadError>,
    CheckHealth: FnOnce(&SimulationProcessConfig) -> Result<(), SimulationProcessReadError>,
{
    if !linux_supported {
        return HealthOutcome::UnsupportedPlatform;
    }
    let config = match read_config(&args.expected_commit, &args.expected_config_sha256) {
        Ok(config) => config,
        Err(_) => return HealthOutcome::Rejected,
    };
    if config.receipt_sha256() != format!("sha256:{}", args.expected_receipt_sha256) {
        return HealthOutcome::Rejected;
    }
    match check_health(&config) {
        Ok(()) => HealthOutcome::Healthy,
        Err(_) => HealthOutcome::Rejected,
    }
}

fn emit_health_outcome(outcome: HealthOutcome) -> ExitCode {
    let (status, diagnostic) = health_response(outcome);
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
    }
    ExitCode::from(status)
}

fn health_response(outcome: HealthOutcome) -> (u8, Option<&'static str>) {
    match outcome {
        HealthOutcome::Healthy => (0, None),
        HealthOutcome::Rejected => (8, Some("simulation runner health check rejected")),
        HealthOutcome::UnsupportedPlatform => (7, Some("simulation runner supports Linux only")),
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
    if values
        .first()
        .is_some_and(|value| value == "--health" || value == "--official-daemon")
    {
        let official = values[0] == "--official-daemon";
        let options = parse_options(&values[1..])?;
        require_exact_keys(
            &options,
            &[
                "--expected-commit",
                "--expected-config-sha256",
                "--expected-receipt-sha256",
            ],
        )?;
        let args = HealthArgs {
            expected_commit: required_string(&options, "--expected-commit")?,
            expected_config_sha256: required_string(&options, "--expected-config-sha256")?,
            expected_receipt_sha256: required_string(&options, "--expected-receipt-sha256")?,
        };
        if !valid_health_commit(&args.expected_commit)
            || !valid_prefixed_sha256(&args.expected_config_sha256)
            || !valid_sha256(&args.expected_receipt_sha256)
        {
            return Err(());
        }
        return Ok(if official {
            ParsedArgs::Official(args)
        } else {
            ParsedArgs::Health(args)
        });
    }

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
    if !values.len().is_multiple_of(2) {
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

fn valid_prefixed_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(valid_sha256)
}

fn valid_health_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;

    const HEALTH_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const CONFIG_SHA256: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const RECEIPT_SHA256: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn valid_health_values() -> Vec<OsString> {
        [
            "--health",
            "--expected-config-sha256",
            CONFIG_SHA256,
            "--expected-receipt-sha256",
            RECEIPT_SHA256,
            "--expected-commit",
            HEALTH_COMMIT,
        ]
        .into_iter()
        .map(OsString::from)
        .collect()
    }

    fn parse_health(values: &[&str]) -> Result<HealthArgs, ()> {
        let parsed = parse_args(values.iter().map(OsString::from).collect())?;
        match parsed {
            ParsedArgs::Health(args) => Ok(args),
            ParsedArgs::Parent(_) | ParsedArgs::Child(_) | ParsedArgs::Official(_) => Err(()),
        }
    }

    #[test]
    fn official_cli_requires_exact_independent_pins() {
        let mut values = valid_health_values();
        values[0] = "--official-daemon".into();
        assert!(matches!(
            parse_args(values.clone()),
            Ok(ParsedArgs::Official(_))
        ));
        let mut duplicate = values.clone();
        duplicate.extend([
            OsString::from("--expected-commit"),
            OsString::from(HEALTH_COMMIT),
        ]);
        assert!(parse_args(duplicate).is_err());
        let mut override_path = values.clone();
        override_path.extend([OsString::from("--artifact-root"), OsString::from("/tmp")]);
        assert!(parse_args(override_path).is_err());
        values[2] = RECEIPT_SHA256.into();
        assert!(
            parse_args(values).is_err(),
            "a bare receipt pin cannot stand in for a config pin"
        );
    }

    fn checked_config(receipt_sha256: &str) -> SimulationProcessConfig {
        let value = serde_json::json!({
            "schema": "owner-beta-simulation-process-v1",
            "code_commit": HEALTH_COMMIT,
            "manifest_format": "LAGRANGE_RELEASE_MANIFEST_V3",
            "manifest_sha256": format!("sha256:{}", "1".repeat(64)),
            "image_id": format!("sha256:{}", "2".repeat(64)),
            "receipt_sha256": format!("sha256:{receipt_sha256}"),
            "binary_sha256": format!("sha256:{}", "4".repeat(64)),
            "source_input_sha256": format!("sha256:{}", "5".repeat(64)),
            "recipe_sha256": format!("sha256:{}", "6".repeat(64)),
            "build_origin": "official-release-v1",
            "package": "job-queue",
            "bin": "owner-beta-simulation-runner",
            "target": "x86_64-unknown-linux-musl",
            "profile": "release",
            "toolchain": "1.97.1",
            "owner": "11111111-2222-3333-4444-555555555555",
            "entitlement_reference": "fixture:synthetic-owner-rights-v1",
            "entitlement_sha256": format!("sha256:{}", "7".repeat(64)),
            "historical_root": "/synthetic/history/δ",
            "reader_uid": 10001,
            "reader_gid": 10001,
            "paths": {
                "config": "/run/owner-beta-simulation/config.json",
                "artifact_root": "/data/owner-beta-artifacts",
                "receipt": "/usr/share/lagrange/owner-beta-simulation/artifact.json",
                "complete": "/usr/share/lagrange/owner-beta-simulation/COMPLETE",
                "executable": "/usr/local/bin/owner-beta-simulation-runner",
                "worker_id": "owner-beta-simulation-runner"
            },
            "limits": {
                "memory_bytes": 1234567,
                "nano_cpus": 2345678,
                "health_tmpfs_bytes": 3456789,
                "pids_limit": 42,
                "startup_timeout_seconds": 123,
                "health_max_age_seconds": 234,
                "stop_timeout_seconds": 345
            }
        });
        let bytes = serde_json::to_vec(&value).expect("synthetic config serializes");
        let expected_config_sha256 = format!("sha256:{:x}", Sha256::digest(&bytes));
        SimulationProcessConfig::from_json(&bytes, HEALTH_COMMIT, &expected_config_sha256)
            .expect("synthetic config is checked against its raw bytes")
    }

    fn health_args_for(
        config: &SimulationProcessConfig,
        expected_receipt_sha256: &str,
    ) -> HealthArgs {
        let parsed = parse_args(
            [
                OsString::from("--health"),
                OsString::from("--expected-commit"),
                OsString::from(HEALTH_COMMIT),
                OsString::from("--expected-config-sha256"),
                OsString::from(config.config_sha256()),
                OsString::from("--expected-receipt-sha256"),
                OsString::from(expected_receipt_sha256),
            ]
            .into_iter()
            .collect(),
        )
        .expect("synthetic health pins parse");
        let ParsedArgs::Health(args) = parsed else {
            panic!("health mode remains its own dispatch variant");
        };
        args
    }

    #[test]
    fn health_parser_accepts_only_the_three_explicit_pins_in_any_order() {
        let args = parse_args(valid_health_values()).expect("closed health grammar accepts pins");
        assert!(matches!(args, ParsedArgs::Health(_)));

        let reordered = parse_health(&[
            "--health",
            "--expected-receipt-sha256",
            RECEIPT_SHA256,
            "--expected-commit",
            HEALTH_COMMIT,
            "--expected-config-sha256",
            CONFIG_SHA256,
        ])
        .expect("health pins may be reordered");
        assert_eq!(reordered.expected_commit, HEALTH_COMMIT);
        assert_eq!(reordered.expected_config_sha256, CONFIG_SHA256);
        assert_eq!(reordered.expected_receipt_sha256, RECEIPT_SHA256);

        let zero_sha256 = "0".repeat(64);
        let zero_config_sha256 = format!("sha256:{zero_sha256}");
        let zero_pins = parse_health(&[
            "--health",
            "--expected-commit",
            HEALTH_COMMIT,
            "--expected-config-sha256",
            &zero_config_sha256,
            "--expected-receipt-sha256",
            &zero_sha256,
        ]);
        assert!(
            zero_pins.is_ok(),
            "SHA syntax does not require nonzero digests"
        );
    }

    #[test]
    fn health_parser_rejects_missing_duplicate_unknown_mixed_and_malformed_pins() {
        let bare_config = "a".repeat(64);
        let short_prefixed_config = format!("sha256:{}", "a".repeat(63));
        let double_config = format!("sha256:sha256:{}", "a".repeat(64));
        let uppercase_config = format!("sha256:{}", "A".repeat(64));
        let whitespace_config = format!("{CONFIG_SHA256} ");
        let prefixed_receipt = format!("sha256:{RECEIPT_SHA256}");
        let uppercase_receipt = "B".repeat(64);
        let short_receipt = "b".repeat(63);
        let whitespace_receipt = format!("{RECEIPT_SHA256} ");
        let whitespace_commit = format!("{HEALTH_COMMIT} ");
        for values in [
            vec!["--health"],
            vec!["--health", "--expected-commit", HEALTH_COMMIT],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
                "--path",
                "/tmp/override",
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
                "--once",
                "",
            ],
            vec![
                "--health",
                "--expected-commit",
                "",
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                "",
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                "",
            ],
            vec![
                "--health",
                "--expected-commit",
                &whitespace_commit,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                "0000000000000000000000000000000000000000",
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                "0123456789ABCDEF0123456789abcdef01234567",
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                "0123456789abcdef0123456789abcdef0123456",
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                &bare_config,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                &short_prefixed_config,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                &double_config,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                &uppercase_config,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                &whitespace_config,
                "--expected-receipt-sha256",
                RECEIPT_SHA256,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                &prefixed_receipt,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                &uppercase_receipt,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                &short_receipt,
            ],
            vec![
                "--health",
                "--expected-commit",
                HEALTH_COMMIT,
                "--expected-config-sha256",
                CONFIG_SHA256,
                "--expected-receipt-sha256",
                &whitespace_receipt,
            ],
        ] {
            assert!(parse_args(values.into_iter().map(OsString::from).collect()).is_err());
        }

        for omitted_key in [
            "--expected-commit",
            "--expected-config-sha256",
            "--expected-receipt-sha256",
        ] {
            let mut values = valid_health_values();
            let index = values
                .iter()
                .position(|value| value.to_str() == Some(omitted_key))
                .expect("each required pin is present in the valid argument list");
            values.drain(index..index + 2);
            assert!(
                parse_args(values).is_err(),
                "missing {omitted_key} pair is rejected"
            );
        }

        let mut odd_trailing_option = valid_health_values();
        odd_trailing_option.push(OsString::from("--dangling"));
        assert!(parse_args(odd_trailing_option).is_err());

        for (key, value) in [
            ("--expected-commit", HEALTH_COMMIT),
            ("--expected-config-sha256", CONFIG_SHA256),
            ("--expected-receipt-sha256", RECEIPT_SHA256),
        ] {
            let mut duplicate = valid_health_values();
            duplicate.extend([OsString::from(key), OsString::from(value)]);
            assert!(
                parse_args(duplicate).is_err(),
                "duplicate {key} is rejected"
            );
        }
        for (key, value) in [
            ("--path", "/tmp/config.json"),
            ("--pid", "1234"),
            ("--worker-id", "fixture-worker"),
        ] {
            let mut unknown = valid_health_values();
            unknown.extend([OsString::from(key), OsString::from(value)]);
            assert!(parse_args(unknown).is_err(), "unknown {key} is rejected");
        }
    }

    #[cfg(unix)]
    #[test]
    fn health_parser_rejects_non_utf8_pin_values() {
        use std::os::unix::ffi::OsStringExt;

        let mut values = valid_health_values();
        let last = values.len() - 1;
        values[last] = OsString::from_vec(vec![0xff]);
        assert!(parse_args(values).is_err());
    }

    #[test]
    fn health_dispatch_reads_config_then_checks_receipt_then_probes_without_other_effects() {
        let config = checked_config(RECEIPT_SHA256);
        let args = health_args_for(&config, RECEIPT_SHA256);
        let events = RefCell::new(Vec::new());
        let outcome = dispatch_health(
            &args,
            true,
            |commit, config_sha256| {
                events.borrow_mut().push("config-read");
                assert_eq!(commit, HEALTH_COMMIT);
                assert_eq!(config_sha256, config.config_sha256());
                Ok(config.clone())
            },
            |checked| {
                events.borrow_mut().push("health-check");
                assert_eq!(checked, &config);
                Ok(())
            },
        );
        assert_eq!(outcome, HealthOutcome::Healthy);
        assert_eq!(events.borrow().clone(), vec!["config-read", "health-check"]);
        assert_eq!(health_response(outcome), (0, None));
    }

    #[test]
    fn health_dispatch_stops_before_probe_on_config_error_or_receipt_mismatch() {
        let matching_config = checked_config(RECEIPT_SHA256);
        let args = health_args_for(&matching_config, RECEIPT_SHA256);
        let events = RefCell::new(Vec::new());
        let failed_config = dispatch_health(
            &args,
            true,
            |_, _| {
                events.borrow_mut().push("config-read");
                Err(SimulationProcessReadError::Unavailable)
            },
            |_| {
                events.borrow_mut().push("health-check");
                Ok(())
            },
        );
        assert_eq!(failed_config, HealthOutcome::Rejected);
        assert_eq!(events.borrow().clone(), vec!["config-read"]);

        events.borrow_mut().clear();
        let mismatched_receipt = checked_config(&"c".repeat(64));
        let mismatch_args = health_args_for(&mismatched_receipt, RECEIPT_SHA256);
        let mismatch = dispatch_health(
            &mismatch_args,
            true,
            |_, _| {
                events.borrow_mut().push("config-read");
                Ok(mismatched_receipt.clone())
            },
            |_| {
                events.borrow_mut().push("health-check");
                Ok(())
            },
        );
        assert_eq!(mismatch, HealthOutcome::Rejected);
        assert_eq!(events.borrow().clone(), vec!["config-read"]);
        assert_eq!(
            health_response(mismatch),
            (8, Some("simulation runner health check rejected"))
        );
    }

    #[test]
    fn health_dispatch_maps_probe_failure_and_non_linux_without_adapter_io() {
        let config = checked_config(RECEIPT_SHA256);
        let args = health_args_for(&config, RECEIPT_SHA256);
        let probe_failure = dispatch_health(
            &args,
            true,
            |_, _| Ok(config.clone()),
            |_| Err(SimulationProcessReadError::HealthRejected),
        );
        assert_eq!(probe_failure, HealthOutcome::Rejected);
        assert_eq!(
            health_response(probe_failure),
            (8, Some("simulation runner health check rejected"))
        );

        let unsupported = dispatch_health(
            &args,
            false,
            |_, _| panic!("non-Linux health dispatch must not read config"),
            |_| panic!("non-Linux health dispatch must not probe health"),
        );
        assert_eq!(unsupported, HealthOutcome::UnsupportedPlatform);
        assert_eq!(
            health_response(unsupported),
            (7, Some("simulation runner supports Linux only"))
        );
    }

    #[test]
    fn existing_parent_and_child_argument_shapes_remain_accepted() {
        let receipt = "a".repeat(64);
        let parent = parse_args(
            [
                "--once",
                "--artifact-root",
                "/tmp/artifacts",
                "--build-receipt",
                "/tmp/receipt.json",
                "--expected-receipt-sha256",
                &receipt,
                "--worker-id",
                "fixture-worker",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
        )
        .expect("existing parent shape remains accepted");
        assert!(matches!(
            parent,
            ParsedArgs::Parent(ParentArgs {
                mode: Mode::Once,
                ..
            })
        ));

        let daemon = parse_args(
            [
                "--daemon",
                "--artifact-root",
                "/tmp/artifacts",
                "--build-receipt",
                "/tmp/receipt.json",
                "--expected-receipt-sha256",
                &receipt,
                "--worker-id",
                "fixture-worker",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
        )
        .expect("existing daemon shape remains accepted");
        assert!(matches!(
            daemon,
            ParsedArgs::Parent(ParentArgs {
                mode: Mode::Daemon,
                ..
            })
        ));

        let child = parse_args(
            [
                "--child",
                "--artifact-root",
                "/tmp/artifacts",
                "--build-receipt",
                "/tmp/receipt.json",
                "--expected-receipt-sha256",
                &receipt,
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
        )
        .expect("existing child shape remains accepted");
        assert!(matches!(child, ParsedArgs::Child(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ctrl_c_and_sigterm_request_the_same_cooperative_shutdown() {
        for trigger in [ShutdownTrigger::CtrlC, ShutdownTrigger::Sigterm] {
            let (shutdown, mut receiver) = watch::channel(false);
            request_shutdown(&shutdown, trigger);
            assert!(*receiver.borrow_and_update());
        }
    }
}
