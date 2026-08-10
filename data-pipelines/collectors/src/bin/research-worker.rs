use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use collectors::{
    HealthcheckConfig, WaitOutcome, WorkerControl, WorkerError, WorkerRunOutcome, bootstrap_worker,
    build_postgres_pool, healthcheck,
};
use domain::TradingDate;
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

const ENV_KEYS: &[&str] = &[
    "APP_ENV",
    "RESEARCH_FETCH_MODE",
    "RESEARCH_RUN_AT_KST",
    "RESEARCH_MAX_PUBLICATION_AGE_SECS",
    "RESEARCH_RAW_ROOT",
    "RESEARCH_SYNTHETIC_BUNDLE",
    "DB_HOST",
    "DB_PORT",
    "DB_NAME",
    "DB_USER",
    "DB_PASSWORD_FILE",
    "KRX_CREDENTIAL_FILE",
];

enum Command {
    Daemon,
    Once(TradingDate),
    Healthcheck,
    Help,
}

#[derive(Serialize)]
struct ErrorRecord {
    status: &'static str,
    error_code: &'static str,
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
        Err(error) => return report_error(&error),
    };
    if matches!(command, Command::Help) {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let values = environment_map();
    let result = match command {
        Command::Healthcheck => run_healthcheck(&values).await,
        Command::Once(date) => run_once(&values, date).await,
        Command::Daemon => run_daemon(&values).await,
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
        Err(error) => report_error(&error),
    }
}

fn parse_args(args: &[String]) -> Result<Command, WorkerError> {
    match args {
        [] => Ok(Command::Daemon),
        [flag] if flag == "--help" || flag == "-h" => Ok(Command::Help),
        [command] if command == "healthcheck" => Ok(Command::Healthcheck),
        [once, date_flag, date] if once == "--once" && date_flag == "--date" => {
            TradingDate::parse(date)
                .map(Command::Once)
                .map_err(|_| WorkerError::InvalidConfig { key: "--date" })
        }
        _ => Err(WorkerError::InvalidConfig { key: "arguments" }),
    }
}

fn environment_map() -> HashMap<String, String> {
    ENV_KEYS
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
        let _ = tokio::signal::ctrl_c().await;
        let _ = sender.send(true);
    });
    SystemControl {
        shutdown: Mutex::new(receiver),
    }
}

async fn run_once(
    values: &HashMap<String, String>,
    date: TradingDate,
) -> Result<SuccessRecord, WorkerError> {
    let worker = bootstrap_worker(values)?;
    let outcome = worker.run_once(date, &system_control()).await?;
    Ok(run_record(outcome, Some(date)))
}

async fn run_daemon(values: &HashMap<String, String>) -> Result<SuccessRecord, WorkerError> {
    let worker = bootstrap_worker(values)?;
    let outcome = worker.run_daemon(&system_control()).await?;
    Ok(run_record(outcome, None))
}

async fn run_healthcheck(values: &HashMap<String, String>) -> Result<SuccessRecord, WorkerError> {
    let config = HealthcheckConfig::from_map(values)?;
    let pool = build_postgres_pool(&config.database);
    let status = healthcheck(&pool, Utc::now(), config.max_publication_age).await?;
    pool.close().await;
    Ok(SuccessRecord {
        status: "ok",
        phase: "health",
        outcome: "healthy",
        batch_id: None,
        date: None,
        newest_eod_at: Some(status.newest_eod_at.to_rfc3339()),
        age_seconds: Some(status.age.as_secs()),
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
    }
}

fn report_error(error: &WorkerError) -> ExitCode {
    let record = ErrorRecord {
        status: "error",
        error_code: error_code(error),
        phase: error.phase().as_str(),
        class: error.failure_class().as_str(),
        batch_id: error.batch_id().map(|batch_id| batch_id.to_string()),
        message: error.to_string(),
    };
    println!(
        "{}",
        serde_json::to_string(&record).unwrap_or_else(|_| {
            "{\"status\":\"error\",\"error_code\":\"SERIALIZATION_FAILED\",\"phase\":\"config\",\"class\":\"permanent\",\"batch_id\":null}".to_owned()
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
    }
}
