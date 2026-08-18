use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use collectors::{
    HealthcheckConfig, RecoveryPosition, WORKER_ENV_KEYS, WaitOutcome, WorkerControl, WorkerError,
    WorkerEvent, WorkerObserver, WorkerRunOutcome, bootstrap_worker, build_postgres_pool,
    candidate_healthcheck, healthcheck, run_credentialed_backfill_range_stream,
    run_internal_ingest, run_internal_recovery_page_stream,
};
use domain::{TradingDate, UtcTimestamp};
use serde::Serialize;
use tokio::sync::{Mutex, watch};

const USAGE: &str = "\
research-worker [--once --date YYYY-MM-DD]
research-worker --backfill-range --start YYYY-MM-DD --end YYYY-MM-DD
research-worker healthcheck
research-worker --help

Default mode is a daemon scheduled by RESEARCH_RUN_AT_KST (16:30 by default).
Database credentials use DB_HOST, DB_PORT, DB_NAME, DB_USER, and DB_PASSWORD_FILE.
DATABASE_URL is not used by this worker.
";

enum Command {
    Daemon,
    Once(TradingDate),
    BackfillRange {
        start: TradingDate,
        end: TradingDate,
    },
    Healthcheck,
    Help,
    InternalRecover(RecoveryPosition),
    InternalIngest(TradingDate, UtcTimestamp),
}

#[derive(Serialize)]
struct ErrorRecord {
    status: &'static str,
    error_code: String,
    provider: &'static str,
    market: &'static str,
    target_date: Option<String>,
    phase: &'static str,
    class: &'static str,
    batch_id: Option<String>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    per_universe: Option<BTreeMap<String, Vec<String>>>,
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
        Command::BackfillRange { start, end } => run_backfill_range(&values, start, end).await,
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
        Command::Daemon
        | Command::BackfillRange { .. }
        | Command::Healthcheck
        | Command::Help
        | Command::InternalRecover(_) => None,
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
        [range, start_flag, start, end_flag, end]
            if range == "--backfill-range" && start_flag == "--start" && end_flag == "--end" =>
        {
            let start = TradingDate::parse(start).map_err(|_| WorkerError::InvalidConfig {
                key: "--backfill-range",
            })?;
            let end = TradingDate::parse(end).map_err(|_| WorkerError::InvalidConfig {
                key: "--backfill-range",
            })?;
            if end < start {
                return Err(WorkerError::InvalidConfig {
                    key: "--backfill-range",
                });
            }
            Ok(Command::BackfillRange { start, end })
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

async fn run_backfill_range(
    values: &HashMap<String, String>,
    start: TradingDate,
    end: TradingDate,
) -> Result<SuccessRecord, WorkerError> {
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    run_credentialed_backfill_range_stream(values, start, end, &mut writer).await?;
    Ok(SuccessRecord {
        status: "ok",
        phase: "complete",
        outcome: "backfilled",
        batch_id: None,
        date: None,
        newest_eod_at: None,
        age_seconds: None,
        cursor: None,
        snapshot_high_water: None,
        has_more: None,
        per_universe: None,
    })
}

async fn run_daemon(values: &HashMap<String, String>) -> Result<SuccessRecord, WorkerError> {
    let worker = bootstrap_worker(values)?.with_observer(Arc::new(JsonObserver));
    let outcome = worker.run_daemon(&system_control()).await?;
    Ok(run_record(outcome, None))
}

async fn run_healthcheck(values: &HashMap<String, String>) -> Result<SuccessRecord, WorkerError> {
    let config = HealthcheckConfig::from_map(values)?;
    let pool = build_postgres_pool(&config.database);
    let status = healthcheck(
        &pool,
        Utc::now(),
        config.max_publication_age,
        config.expected_fetch_mode,
    )
    .await?;
    let per_universe = if config.candidate_sources_enabled {
        Some(
            candidate_healthcheck(
                &pool,
                &config.curated_root,
                Utc::now(),
                config.max_publication_age,
                config.expected_fetch_mode,
                config.run_at_kst,
            )
            .await?
            .per_universe,
        )
    } else {
        None
    };
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
        per_universe,
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
        per_universe: None,
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
        per_universe: None,
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
        per_universe: None,
    }
}

fn report_error(error: &WorkerError, target_date: Option<TradingDate>) -> ExitCode {
    let provider = match std::env::var("RESEARCH_FETCH_MODE").as_deref() {
        Ok("credentialed") => "KIS-NORMALIZED",
        _ => "KRX",
    };
    let record = error_record(error, target_date, provider);
    println!(
        "{}",
        serde_json::to_string(&record).unwrap_or_else(|_| {
            "{\"status\":\"error\",\"error_code\":\"SERIALIZATION_FAILED\",\"provider\":\"KRX\",\"market\":\"KR\",\"target_date\":null,\"phase\":\"config\",\"class\":\"permanent\",\"batch_id\":null}".to_owned()
        })
    );
    ExitCode::from(2)
}

fn error_record(
    error: &WorkerError,
    target_date: Option<TradingDate>,
    provider: &'static str,
) -> ErrorRecord {
    let diagnostic = error.safe_diagnostic();
    let code = error_code(error);
    let message = match diagnostic {
        Some(value) => format!("operation failed with {}", value.error_code),
        None => error.to_string(),
    };
    ErrorRecord {
        status: "error",
        error_code: code,
        provider,
        market: "KR",
        target_date: target_date.map(|date| date.to_iso()),
        phase: error.phase().as_str(),
        class: error.failure_class().as_str(),
        batch_id: error.batch_id().map(|batch_id| batch_id.to_string()),
        message,
        endpoint: diagnostic
            .and_then(|value| value.endpoint)
            .map(str::to_owned),
        http_status: diagnostic.and_then(|value| value.http_status),
        response_kind: diagnostic
            .and_then(|value| value.response_kind)
            .map(str::to_owned),
        file_name: diagnostic
            .and_then(|value| value.file_name)
            .map(str::to_owned),
    }
}

fn error_code(error: &WorkerError) -> String {
    if let Some(diagnostic) = error.safe_diagnostic() {
        return diagnostic.error_code.to_owned();
    }
    match error {
        WorkerError::MissingConfig { .. } => "MISSING_CONFIG",
        WorkerError::InvalidConfig { .. } => "INVALID_CONFIG",
        WorkerError::SyntheticForbidden { .. } => "SYNTHETIC_FORBIDDEN",
        WorkerError::SecretFile { .. } => "SECRET_FILE_UNAVAILABLE",
        WorkerError::Io { .. } => "WORKER_IO_FAILED",
        WorkerError::Timeout { .. } => "WORKER_TIMEOUT",
        WorkerError::ProviderNotConfigured => "PROVIDER_NOT_CONFIGURED",
        WorkerError::Provider(_) => "PROVIDER_UNAVAILABLE",
        WorkerError::KisClient(_) => "KIS_CLIENT_UNAVAILABLE",
        WorkerError::Database { .. } => "DATABASE_UNAVAILABLE",
        WorkerError::Unhealthy { .. } => "UNHEALTHY",
        WorkerError::Pipeline(_) => "PIPELINE_FAILED",
        WorkerError::CandidatePipeline(_) => "CANDIDATE_PIPELINE_FAILED",
        WorkerError::Curation(_) => "PRICE_CURATION_FAILED",
        WorkerError::ChildIo { .. } => "HELPER_IO_FAILED",
        WorkerError::ChildContainment { .. } => "HELPER_CONTAINMENT_FAILED",
        WorkerError::ChildOutput { .. } => "HELPER_OUTPUT_INVALID",
        WorkerError::ChildFailure { .. } => "HELPER_FAILED",
        WorkerError::Cycle { source, .. } => return error_code(source),
        WorkerError::Shutdown => "SHUTDOWN",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use collectors::{FailureClass, WorkerPhase};

    use super::*;

    #[test]
    fn helper_failure_event_exposes_only_safe_provider_diagnostic() {
        let error = WorkerError::ChildFailure {
            phase: WorkerPhase::Ingest,
            class: FailureClass::Permanent,
            batch_id: None,
            error_code: "BROKER_REJECTED".to_owned(),
            endpoint: Some(
                "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".to_owned(),
            ),
            http_status: Some(403),
            response_context: None,
        };
        let value = serde_json::to_value(error_record(&error, None, "KIS-NORMALIZED")).unwrap();
        assert_eq!(value["error_code"], "BROKER_REJECTED");
        assert_eq!(
            value["endpoint"],
            "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice"
        );
        assert_eq!(value["http_status"], 403);
        assert_eq!(value["message"], "operation failed with BROKER_REJECTED");
        assert!(!value.to_string().contains("body"));

        let validation_error = WorkerError::ChildFailure {
            phase: WorkerPhase::Ingest,
            class: FailureClass::Permanent,
            batch_id: None,
            error_code: "KIS_RESPONSE_SCHEMA_INVALID".to_owned(),
            endpoint: Some("/uapi/domestic-stock/v1/quotations/chk-holiday".to_owned()),
            http_status: None,
            response_context: Some(Box::new(collectors::ChildResponseContext {
                response_kind: "calendar".to_owned(),
                file_name: "calendar-page-01.json".to_owned(),
            })),
        };
        let value =
            serde_json::to_value(error_record(&validation_error, None, "KIS-NORMALIZED")).unwrap();
        assert_eq!(value["response_kind"], "calendar");
        assert_eq!(value["file_name"], "calendar-page-01.json");
        assert!(value.get("http_status").is_none());
    }
}
