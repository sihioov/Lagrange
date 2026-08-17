use std::collections::HashMap;
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use collectors::{
    HealthcheckConfig, RecoveryPosition, WORKER_ENV_KEYS, WaitOutcome, WorkerControl, WorkerError,
    WorkerEvent, WorkerObserver, WorkerRunOutcome, bootstrap_worker, build_postgres_pool,
    candidate_healthcheck, healthcheck, run_internal_ingest, run_internal_recovery_page_stream,
};
use domain::{TradingDate, UtcTimestamp};
use serde::Serialize;
use tokio::sync::{Mutex, watch};

const USAGE: &str = "\
research-worker [--once --date YYYY-MM-DD]
research-worker healthcheck
research-worker --help

Default mode is a daemon scheduled by RESEARCH_RUN_AT_KST (16:30 by default).
Database credentials use DB_HOST, DB_PORT, DB_NAME, DB_USER, and DB_PASSWORD_FILE.
DATABASE_URL is not used by this worker.
";

enum Command {
    Daemon,
    Once(TradingDate),
    Healthcheck,
    Help,
    InternalRecover(RecoveryPosition),
    InternalIngest(TradingDate, UtcTimestamp),
}

#[derive(Serialize)]
struct ErrorRecord {
    status: &'static str,
    error_code: &'static str,
    provider: &'static str,
    market: &'static str,
    target_date: Option<String>,
    phase: &'static str,
    class: &'static str,
    batch_id: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct SuccessRecord {
    status: &'static str,
    phase: &'static str,
    outcome: &'static str,
    batch_id: Option<String>,
    date: Option<String>,
    newest_eod_at: Option<String>,
    age_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_high_water: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_more: Option<bool>,
}

#[derive(Serialize)]
struct EventRecord {
    event: &'static str,
    provider: &'static str,
    market: &'static str,
    target_date: Option<String>,
    phase: &'static str,
    class: &'static str,
    batch_id: Option<String>,
}

struct JsonObserver;

impl WorkerObserver for JsonObserver {
    fn emit(&self, event: WorkerEvent) {
        let record = EventRecord {
            event: event.kind.as_str(),
            provider: event.provider,
            market: event.market,
            target_date: event.target_date.map(|date| date.to_iso()),
            phase: event.phase.as_str(),
            class: event.class.as_str(),
            batch_id: event.batch_id.map(|batch_id| batch_id.to_string()),
        };
        if let Ok(line) = serde_json::to_string(&record) {
            println!("{line}");
            let _ = std::io::stdout().flush();
        }
    }
}

struct SystemControl {
    shutdown: Mutex<watch::Receiver<bool>>,
}

#[async_trait]
impl WorkerControl for SystemControl {
    fn now_utc(&self) -> chrono::DateTime<Utc> {
        Utc::now()
    }

    async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
        let mut receiver = self.shutdown.lock().await;
        if *receiver.borrow() {
            return WaitOutcome::Shutdown;
        }
        match duration {
            Some(duration) => tokio::select! {
                _ = tokio::time::sleep(duration) => WaitOutcome::Elapsed,
                changed = receiver.changed() => {
                    if changed.is_err() || *receiver.borrow() {
                        WaitOutcome::Shutdown
                    } else {
                        WaitOutcome::Elapsed
                    }
                }
            },
            None => {
                let _ = receiver.changed().await;
                WaitOutcome::Shutdown
            }
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse_args(&args) {
        Ok(command) => command,
        Err(error) => return report_error(&error, None),
    };
    if matches!(command, Command::Help) {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let target_date = command_target_date(&command);
    let values = environment_map();
    let result = match command {
        Command::Healthcheck => run_healthcheck(&values).await,
        Command::Once(date) => run_once(&values, date).await,
        Command::Daemon => run_daemon(&values).await,
        Command::InternalRecover(after) => run_internal_recover(&values, after).await,
        Command::InternalIngest(date, now) => run_internal_collect(&values, date, now).await,
        Command::Help => unreachable!("help returned before worker setup"),
    };
    match result {
        Ok(record) => {
            println!(
                "{}",
                serde_json::to_string(&record).expect("success record serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => report_error(&error, target_date.or_else(|| error.target_date())),
    }
}

fn command_target_date(command: &Command) -> Option<TradingDate> {
    match command {
        Command::Once(date) | Command::InternalIngest(date, _) => Some(*date),
        Command::Daemon | Command::Healthcheck | Command::Help | Command::InternalRecover(_) => {
            None
        }
    }
}

fn parse_args(args: &[String]) -> Result<Command, WorkerError> {
    match args {
        [] => Ok(Command::Daemon),
        [flag] if flag == "--help" || flag == "-h" => Ok(Command::Help),
        [command] if command == "healthcheck" => Ok(Command::Healthcheck),
        [command, recovery_args @ ..] if command == "__research-internal-recover" => {
            parse_internal_recovery_args(recovery_args).map(Command::InternalRecover)
        }
        [command, date, now] if command == "__research-internal-ingest" => {
            let date = TradingDate::parse(date).map_err(|_| WorkerError::InvalidConfig {
                key: "internal-date",
            })?;
            let now = UtcTimestamp::parse_rfc3339(now).map_err(|_| WorkerError::InvalidConfig {
                key: "internal-now",
            })?;
            Ok(Command::InternalIngest(date, now))
        }
        [once, date_flag, date] if once == "--once" && date_flag == "--date" => {
            TradingDate::parse(date)
                .map(Command::Once)
                .map_err(|_| WorkerError::InvalidConfig { key: "--date" })
        }
        _ => Err(WorkerError::InvalidConfig { key: "arguments" }),
    }
}

fn parse_internal_recovery_args(args: &[String]) -> Result<RecoveryPosition, WorkerError> {
    if !args.len().is_multiple_of(2) {
        return Err(WorkerError::InvalidConfig {
            key: "internal-recovery-position",
        });
    }
    let mut position = RecoveryPosition::default();
    for pair in args.chunks_exact(2) {
        let value = pair[1].parse().map_err(|_| WorkerError::InvalidConfig {
            key: "internal-recovery-position",
        })?;
        let target = match pair[0].as_str() {
            "--snapshot-after" => &mut position.snapshot_after,
            "--snapshot-high-water" => &mut position.snapshot_high_water,
            "--after" => &mut position.cursor,
            _ => {
                return Err(WorkerError::InvalidConfig {
                    key: "internal-recovery-position",
                });
            }
        };
        if target.replace(value).is_some() {
            return Err(WorkerError::InvalidConfig {
                key: "internal-recovery-position",
            });
        }
    }
    if position.cursor.is_some() && position.snapshot_high_water.is_none() {
        return Err(WorkerError::InvalidConfig {
            key: "internal-recovery-position",
        });
    }
    Ok(position)
}

fn environment_map() -> HashMap<String, String> {
    WORKER_ENV_KEYS
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_owned(), value))
        })
        .collect()
}

fn system_control() -> SystemControl {
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        let _ = sender.send(true);
    });
    SystemControl {
        shutdown: Mutex::new(receiver),
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let Ok(mut terminate) = signal(SignalKind::terminate()) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn run_once(
    values: &HashMap<String, String>,
    date: TradingDate,
) -> Result<SuccessRecord, WorkerError> {
    let worker = bootstrap_worker(values)?.with_observer(Arc::new(JsonObserver));
    let outcome = worker.run_once(date, &system_control()).await?;
    Ok(run_record(outcome, Some(date)))
}

async fn run_daemon(values: &HashMap<String, String>) -> Result<SuccessRecord, WorkerError> {
    let worker = bootstrap_worker(values)?.with_observer(Arc::new(JsonObserver));
    let outcome = worker.run_daemon(&system_control()).await?;
    Ok(run_record(outcome, None))
}

async fn run_healthcheck(values: &HashMap<String, String>) -> Result<SuccessRecord, WorkerError> {
    let config = HealthcheckConfig::from_map(values)?;
    let pool = build_postgres_pool(&config.database);
    let status = healthcheck(&pool, Utc::now(), config.max_publication_age).await?;
    if config.candidate_sources_enabled {
        candidate_healthcheck(
            &pool,
            &config.curated_root,
            Utc::now(),
            config.max_publication_age,
            config.expected_fetch_mode,
            config.run_at_kst,
        )
        .await?;
    }
    pool.close().await;
    Ok(SuccessRecord {
        status: "ok",
        phase: "health",
        outcome: "healthy",
        batch_id: None,
        date: None,
        newest_eod_at: Some(status.newest_eod_at.to_rfc3339()),
        age_seconds: Some(status.age.as_secs()),
        cursor: None,
        snapshot_high_water: None,
        has_more: None,
    })
}

async fn run_internal_recover(
    values: &HashMap<String, String>,
    position: RecoveryPosition,
) -> Result<SuccessRecord, WorkerError> {
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let page = run_internal_recovery_page_stream(values, position, &mut writer).await?;
    Ok(SuccessRecord {
        status: "ok",
        phase: "recovery",
        outcome: "recovered",
        batch_id: None,
        date: None,
        newest_eod_at: None,
        age_seconds: None,
        cursor: page.cursor.map(|cursor| cursor.to_string()),
        snapshot_high_water: Some(
            page.snapshot_high_water
                .map(|high_water| high_water.to_string()),
        ),
        has_more: Some(page.has_more),
    })
}

async fn run_internal_collect(
    values: &HashMap<String, String>,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<SuccessRecord, WorkerError> {
    let batch_id = run_internal_ingest(values, date, now).await?;
    Ok(SuccessRecord {
        status: "ok",
        phase: "publication",
        outcome: "published",
        batch_id: Some(batch_id.to_string()),
        date: Some(date.to_iso()),
        newest_eod_at: None,
        age_seconds: None,
        cursor: None,
        snapshot_high_water: None,
        has_more: None,
    })
}

fn run_record(outcome: WorkerRunOutcome, date: Option<TradingDate>) -> SuccessRecord {
    let (name, batch_id) = match outcome {
        WorkerRunOutcome::AlreadyPublished => ("already_published", None),
        WorkerRunOutcome::Published(batch_id) => ("published", Some(batch_id.to_string())),
        WorkerRunOutcome::Shutdown => ("shutdown", None),
    };
    SuccessRecord {
        status: "ok",
        phase: "complete",
        outcome: name,
        batch_id,
        date: date.map(|date| date.to_iso()),
        newest_eod_at: None,
        age_seconds: None,
        cursor: None,
        snapshot_high_water: None,
        has_more: None,
    }
}

fn report_error(error: &WorkerError, target_date: Option<TradingDate>) -> ExitCode {
    let record = ErrorRecord {
        status: "error",
        error_code: error_code(error),
        provider: "KRX",
        market: "KR",
        target_date: target_date.map(|date| date.to_iso()),
        phase: error.phase().as_str(),
        class: error.failure_class().as_str(),
        batch_id: error.batch_id().map(|batch_id| batch_id.to_string()),
        message: error.to_string(),
    };
    println!(
        "{}",
        serde_json::to_string(&record).unwrap_or_else(|_| {
            "{\"status\":\"error\",\"error_code\":\"SERIALIZATION_FAILED\",\"provider\":\"KRX\",\"market\":\"KR\",\"target_date\":null,\"phase\":\"config\",\"class\":\"permanent\",\"batch_id\":null}".to_owned()
        })
    );
    ExitCode::from(2)
}

fn error_code(error: &WorkerError) -> &'static str {
    match error {
        WorkerError::MissingConfig { .. } => "MISSING_CONFIG",
        WorkerError::InvalidConfig { .. } => "INVALID_CONFIG",
        WorkerError::SyntheticForbidden { .. } => "SYNTHETIC_FORBIDDEN",
        WorkerError::SecretFile { .. } => "SECRET_FILE_UNAVAILABLE",
        WorkerError::Io { .. } => "WORKER_IO_FAILED",
        WorkerError::Timeout { .. } => "WORKER_TIMEOUT",
        WorkerError::ProviderNotConfigured => "PROVIDER_NOT_CONFIGURED",
        WorkerError::Provider(_) => "PROVIDER_UNAVAILABLE",
        WorkerError::Database { .. } => "DATABASE_UNAVAILABLE",
        WorkerError::Unhealthy { .. } => "UNHEALTHY",
        WorkerError::Pipeline(_) => "PIPELINE_FAILED",
        WorkerError::CandidatePipeline(_) => "CANDIDATE_PIPELINE_FAILED",
        WorkerError::Curation(_) => "PRICE_CURATION_FAILED",
        WorkerError::ChildIo { .. } => "HELPER_IO_FAILED",
        WorkerError::ChildContainment { .. } => "HELPER_CONTAINMENT_FAILED",
        WorkerError::ChildOutput { .. } => "HELPER_OUTPUT_INVALID",
        WorkerError::ChildFailure { .. } => "HELPER_FAILED",
        WorkerError::Cycle { source, .. } => error_code(source),
        WorkerError::Shutdown => "SHUTDOWN",
    }
}
