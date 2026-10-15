//! Installed daily entry points share only finite diagnostics with the old runner.
//! OFF returns before reading protected configuration or opening any connection.
use super::{Environment, database_options, health_writer, shutdown_signal};
use domain::ContentHash;
use job_queue::owner_beta::runner::{
    daily_execution::{DailyExecutionContext, DailyExecutionMode},
    daily_ingest::{
        DAILY_ADMITTER_UID, DAILY_PRODUCER_UID, DAILY_READER_GROUP, DailyIngestConfig, IngestError,
        IngestOutcome, admit_daily_once, produce_daily_once,
    },
    daily_runtime::DailyProcessMode,
    daily_schedule::{DailyScheduleOutcome, schedule_daily_once},
};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Producer,
    Admitter,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Command {
    stage: Stage,
    once: bool,
}

fn parse(values: &[OsString]) -> Result<Option<Command>, ()> {
    let Some(first) = values.get(1) else {
        return Ok(None);
    };
    let stage = if first == "daily-producer" {
        Stage::Producer
    } else if first == "daily-admitter" {
        Stage::Admitter
    } else {
        return Ok(None);
    };
    let once = match &values[2..] {
        [] => false,
        [value] if value == "--once" => true,
        _ => return Err(()),
    };
    Ok(Some(Command { stage, once }))
}
pub(super) fn process_mode() -> Result<DailyProcessMode, ()> {
    match std::env::var("PRICE_BETA_DAILY_MODE") {
        Ok(value) => DailyProcessMode::parse(Some(&value)).map_err(|_| ()),
        Err(std::env::VarError::NotPresent) => Ok(DailyProcessMode::Disabled),
        Err(_) => Err(()),
    }
}
fn installed_config() -> Result<DailyIngestConfig, ()> {
    let path = std::env::var_os("PRICE_BETA_DAILY_CONFIG_FILE")
        .map(PathBuf::from)
        .ok_or(())?;
    let hash =
        ContentHash::parse(&std::env::var("PRICE_BETA_DAILY_CONFIG_SHA256").map_err(|_| ())?)
            .map_err(|_| ())?;
    DailyIngestConfig::from_pinned_file(&path, &hash).map_err(|_| ())
}
fn exact_uid(uid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: argument-free process identity query.
        unsafe { libc::geteuid() == uid }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = uid;
        false
    }
}
fn credential_environment(
    stage: Stage,
    mut has: impl FnMut(&str) -> bool,
    user: Option<&str>,
) -> bool {
    if has("DATABASE_URL") || has("DATABASE_URL_FILE") {
        return false;
    }
    match stage {
        Stage::Producer => ![
            "DB_HOST",
            "DB_PORT",
            "DB_NAME",
            "DB_USER",
            "DB_PASSWORD",
            "DB_PASSWORD_FILE",
        ]
        .into_iter()
        .any(&mut has),
        Stage::Admitter => user == Some("daily_input_admitter") && !has("DB_PASSWORD"),
    }
}
fn outcome(value: IngestOutcome) -> &'static str {
    match value {
        IngestOutcome::Disabled => "DISABLED",
        IngestOutcome::Busy => "BUSY",
        IngestOutcome::NotDue => "NOT_DUE",
        IngestOutcome::Pending => "PENDING",
        IngestOutcome::CandidateReady => "CANDIDATE_READY",
        IngestOutcome::Admitted => "ADMITTED",
        IngestOutcome::Replay => "REPLAY",
        IngestOutcome::Blocked(_) => "BLOCKED",
    }
}
fn error(value: IngestError) -> &'static str {
    match value {
        IngestError::Configuration => "CONFIGURATION",
        IngestError::State => "STATE",
        IngestError::Interrupted => "INTERRUPTED",
        IngestError::InputPending => "INPUT_PENDING",
        IngestError::InputBlocked => "INPUT_BLOCKED",
        IngestError::InvalidArtifact => "INVALID_ARTIFACT",
        IngestError::Conflict => "CONFLICT",
        IngestError::Database => "DATABASE",
        IngestError::CommitUnknown => "COMMIT_UNKNOWN",
    }
}
fn failure(code: &'static str) -> ExitCode {
    eprintln!(
        "{}",
        json!({"event":"owner_beta_daily_startup","code":code})
    );
    ExitCode::FAILURE
}
pub(super) async fn dispatch(values: Vec<OsString>) -> Option<ExitCode> {
    let command = match parse(&values) {
        Ok(None) => return None,
        Ok(Some(value)) => value,
        Err(()) => return Some(failure("ARGUMENTS_INVALID")),
    };
    Some(execute(command).await)
}
async fn execute(command: Command) -> ExitCode {
    let mode = match process_mode() {
        Ok(value) => value,
        Err(()) => return failure("MODE_INVALID"),
    };
    if mode == DailyProcessMode::Disabled {
        eprintln!(
            "{}",
            json!({"event":"owner_beta_daily_cycle","outcome":"DISABLED"})
        );
        return ExitCode::SUCCESS;
    }
    let expected_uid = match command.stage {
        Stage::Producer => DAILY_PRODUCER_UID,
        Stage::Admitter => DAILY_ADMITTER_UID,
    };
    if !exact_uid(expected_uid) {
        return failure("IDENTITY_INVALID");
    }
    let user = std::env::var("DB_USER").ok();
    if !credential_environment(
        command.stage,
        |name| std::env::var_os(name).is_some(),
        user.as_deref(),
    ) {
        return failure("CREDENTIAL_BOUNDARY");
    }
    let environment = match Environment::read() {
        Ok(value) => value,
        Err(()) => return failure("ENVIRONMENT_INVALID"),
    };
    let config = match installed_config() {
        Ok(value) => value,
        Err(()) => return failure("CONFIGURATION_INVALID"),
    };
    let health = std::env::var_os("PRICE_BETA_HEALTH_STATE_PATH").map(PathBuf::from);
    if environment.production() && health.as_ref().is_none_or(|path| !path.is_absolute()) {
        return failure("HEALTH_CONFIGURATION_INVALID");
    }
    let pool = match command.stage {
        Stage::Producer => None,
        Stage::Admitter => {
            let options = match database_options(environment.production()) {
                Ok(value) => value,
                Err(()) => return failure("DATABASE_CONFIG_INVALID"),
            };
            match PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(Duration::from_secs(10))
                .connect_with(options)
                .await
            {
                Ok(value) => Some(value),
                Err(_) => return failure("DATABASE_UNAVAILABLE"),
            }
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let signal_stop = stop.clone();
    let signal_tx = shutdown_tx.clone();
    let signal = tokio::spawn(async move {
        shutdown_signal().await;
        signal_stop.store(true, Ordering::Release);
        let _ = signal_tx.send(true);
    });
    let health_task = health.map(|path| tokio::spawn(health_writer(path, shutdown_rx.clone())));
    let mut exit = ExitCode::SUCCESS;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if health_task
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            exit = failure("HEALTH_WRITER_UNAVAILABLE");
            break;
        }
        let now = chrono::Utc::now();
        let result = match &pool {
            None => tokio::task::block_in_place(|| produce_daily_once(&config, mode, now, &stop)),
            Some(pool) => admit_daily_once(pool, &config, mode, now, &stop).await,
        };
        match result {
            Ok(value) => eprintln!(
                "{}",
                json!({"event":"owner_beta_daily_cycle","outcome":outcome(value)})
            ),
            Err(value) => {
                eprintln!(
                    "{}",
                    json!({"event":"owner_beta_daily_cycle","code":error(value)})
                );
                if command.once
                    && !(value == IngestError::Interrupted && stop.load(Ordering::Acquire))
                {
                    exit = ExitCode::FAILURE;
                }
            }
        }
        if command.once || stop.load(Ordering::Acquire) {
            break;
        }
        // No backlog of interval ticks or overlapping detached publisher.
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(60)) => {},
            changed = shutdown_rx.changed() => if changed.is_err() || *shutdown_rx.borrow() { break; }
        }
    }
    stop.store(true, Ordering::Release);
    let _ = shutdown_tx.send(true);
    signal.abort();
    let _ = signal.await;
    if let Some(task) = health_task
        && !matches!(task.await, Ok(Ok(())))
    {
        exit = ExitCode::FAILURE;
    }
    if let Some(pool) = pool {
        pool.close().await;
    }
    exit
}

/// Mixed worker reads installed pins, never producer/admitter mutable state or Raw.
pub(super) fn worker_config(historical: &Path) -> Result<Option<DailyIngestConfig>, ()> {
    if process_mode()? == DailyProcessMode::Disabled {
        return Ok(None);
    }
    if !exact_uid(DAILY_READER_GROUP) {
        return Err(());
    }
    let config = installed_config()?;
    if config.historical_path() != historical {
        return Err(());
    }
    Ok(Some(config))
}
pub(super) fn execution(config: Option<&DailyIngestConfig>) -> DailyExecutionMode<'_> {
    match config {
        None => DailyExecutionMode::Disabled,
        Some(config) => DailyExecutionMode::OwnerOnly(DailyExecutionContext {
            admission_root: config.admission_path(),
            trusted_owner: DAILY_ADMITTER_UID,
            reader_group: DAILY_READER_GROUP,
            policy: config.policy(),
            replay_limits: config.replay_limits(),
        }),
    }
}
pub(super) async fn schedule(pool: &PgPool, config: Option<&DailyIngestConfig>) {
    if config.is_none() {
        return;
    }
    match schedule_daily_once(pool, DailyProcessMode::OwnerOnly).await {
        Ok(value) => {
            let label = match value {
                DailyScheduleOutcome::Disabled => "DISABLED",
                DailyScheduleOutcome::Busy => "BUSY",
                DailyScheduleOutcome::NotDue => "NOT_DUE",
                DailyScheduleOutcome::Completed(_) => "COMPLETED",
            };
            eprintln!(
                "{}",
                json!({"event":"owner_beta_daily_schedule","outcome":label})
            );
        }
        Err(_) => eprintln!(
            "{}",
            json!({"event":"owner_beta_daily_schedule","code":"SCHEDULE_UNAVAILABLE"})
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        std::iter::once(OsString::from("owner-beta-runner"))
            .chain(values.iter().map(OsString::from))
            .collect()
    }
    #[test]
    fn daily_arguments_preserve_historical_dispatch_and_reject_ambiguous_modes() {
        for values in [
            &[][..],
            &["--once"][..],
            &["healthcheck"][..],
            &["--artifact-root", "/x"][..],
        ] {
            assert!(matches!(parse(&args(values)), Ok(None)));
        }
        for (name, stage) in [
            ("daily-producer", Stage::Producer),
            ("daily-admitter", Stage::Admitter),
        ] {
            assert!(matches!(parse(&args(&[name])), Ok(Some(c)) if c.stage == stage && !c.once));
            assert!(
                matches!(parse(&args(&[name,"--once"])), Ok(Some(c)) if c.stage == stage && c.once)
            );
            for tail in [
                &["--once", "--once"][..],
                &["--artifact-root", "/x"][..],
                &["healthcheck"][..],
                &[""][..],
            ] {
                let mut values = vec![name];
                values.extend_from_slice(tail);
                assert!(parse(&args(&values)).is_err());
            }
        }
    }
    #[test]
    fn daily_credential_boundary_keeps_producer_off_database_and_admitter_direct() {
        assert!(credential_environment(Stage::Producer, |_| false, None));
        for forbidden in [
            "DATABASE_URL",
            "DATABASE_URL_FILE",
            "DB_HOST",
            "DB_PORT",
            "DB_NAME",
            "DB_USER",
            "DB_PASSWORD",
            "DB_PASSWORD_FILE",
        ] {
            assert!(!credential_environment(
                Stage::Producer,
                |name| name == forbidden,
                None
            ));
        }
        assert!(credential_environment(
            Stage::Admitter,
            |name| name == "DB_USER",
            Some("daily_input_admitter")
        ));
        for user in [
            None,
            Some("worker"),
            Some("app"),
            Some("daily_input_admitter "),
        ] {
            assert!(!credential_environment(Stage::Admitter, |_| false, user));
        }
        assert!(!credential_environment(
            Stage::Admitter,
            |name| name == "DATABASE_URL",
            Some("daily_input_admitter")
        ));
        assert!(!credential_environment(
            Stage::Admitter,
            |name| name == "DB_PASSWORD",
            Some("daily_input_admitter")
        ));
    }
}
