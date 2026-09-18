//! Production queue daemon for owner-managed equity universe V2.

use std::future::Future;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use collectors::intraday_quotes::{
    INTRADAY_SESSION_WINDOWS_SOURCE_ENV, IntradaySessionWindowContract, IntradaySessionWindowSource,
};
use job_queue::owner_equity_v2::{
    IntradayProducer, IntradayProducerConfig, OwnerEquityRunOutcome, OwnerEquityRunnerConfig,
    OwnerEquityRuntimeLimits, OwnerEquityScheduleError, OwnerEquitySchedulePins,
    OwnerIntradayQuoteRepository, ProductionOwnerEquityAdapter, eligible_schedule_date,
    recover_owner_equity_claims, run_owner_equity_runner_once, run_owner_equity_schedule_cycle,
};
use job_queue::{JobQueue, QueueConfig};
use kis_client::clock::Clock;
use kis_client::live_transport::LiveTransport;
use kis_client::secret::SystemCredentialSource;
use kis_client::token_issuer::KisTokenIssuer;
use kis_client::{
    BucketKey, CoordinatedReadAuth, CredentialRef, IntradayQuotesMode, KisMarketDataClient,
    ProductionReadCoordination, Quota, RateLimiter, ReadCoordinationMode, ReadCredentialSnapshot,
    SystemClock, TokenIssuer, TokenManager, TokioSleeper,
};
use market_data::owner_equity_v2::{
    DAILY_BARS_PATH, DAILY_BARS_TR_ID, REFERENCE_PATH, REFERENCE_TR_ID,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::watch;

const DEFAULT_POLL: Duration = Duration::from_secs(2);
const DEFAULT_RECOVERY: Duration = Duration::from_secs(30);
const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(10);
const DEFAULT_LEASE: Duration = Duration::from_secs(60);
const DEFAULT_BACKOFF: Duration = Duration::from_secs(30);
const DEFAULT_WORK_TIMEOUT: Duration = Duration::from_secs(900);
const DEFAULT_HEALTH_MAX_AGE: Duration = Duration::from_secs(180);
const KIS_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

type LiveReader = KisMarketDataClient<LiveTransport, TokioSleeper, SystemCredentialSource>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Daemon,
    Once,
    Healthcheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Environment {
    Development,
    Qa,
    Production,
}

impl Environment {
    fn read() -> Result<Self, ConfigError> {
        match std::env::var("APP_ENV").as_deref() {
            Ok("development") => Ok(Self::Development),
            Ok("qa") => Ok(Self::Qa),
            Ok("production") => Ok(Self::Production),
            _ => Err(ConfigError::Invalid),
        }
    }

    const fn production(self) -> bool {
        matches!(self, Self::Production)
    }
}

#[derive(Debug, Clone)]
struct Config {
    mode: Mode,
    intraday_mode: IntradayQuotesMode,
    session_window_source: IntradaySessionWindowSource,
    production: bool,
    raw_root: PathBuf,
    artifact_root: PathBuf,
    app_key_file: PathBuf,
    app_secret_file: PathBuf,
    health_path: Option<PathBuf>,
    worker_id: String,
    poll: Duration,
    recovery: Duration,
    heartbeat: Duration,
    lease: Duration,
    backoff: Duration,
    work_timeout: Duration,
    health_max_age: Duration,
    limits: OwnerEquityRuntimeLimits,
    schedule_pins: Option<OwnerEquitySchedulePins>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigError {
    Invalid,
}

fn parse_args_from(values: Vec<std::ffi::OsString>) -> Result<Mode, ConfigError> {
    let values = values.into_iter().skip(1).collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(Mode::Daemon),
        [mode] if mode == "--once" => Ok(Mode::Once),
        [mode] if mode == "healthcheck" => Ok(Mode::Healthcheck),
        _ => Err(ConfigError::Invalid),
    }
}

fn should_start_intraday(mode: Mode, intraday_mode: IntradayQuotesMode) -> bool {
    mode == Mode::Daemon && intraday_mode == IntradayQuotesMode::OwnerOnly
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IntradayQuoteStartup {
    Disabled,
    Unavailable(collectors::intraday_quotes::IntradaySessionWindowError),
    Ready(Arc<IntradaySessionWindowContract>),
}

fn prepare_intraday_quote_startup<F>(
    mode: Mode,
    intraday_mode: IntradayQuotesMode,
    load_windows: F,
) -> IntradayQuoteStartup
where
    F: FnOnce() -> Result<
        IntradaySessionWindowContract,
        collectors::intraday_quotes::IntradaySessionWindowError,
    >,
{
    if !should_start_intraday(mode, intraday_mode) {
        return IntradayQuoteStartup::Disabled;
    }
    match load_windows() {
        Ok(windows) => IntradayQuoteStartup::Ready(Arc::new(windows)),
        Err(error) => IntradayQuoteStartup::Unavailable(error),
    }
}

async fn run_intraday_quote_startup<
    LoadWindows,
    StartQuote,
    ContinueEod,
    QuoteTask,
    EodFuture,
    EodResult,
>(
    mode: Mode,
    intraday_mode: IntradayQuotesMode,
    load_windows: LoadWindows,
    start_quote: StartQuote,
    continue_eod: ContinueEod,
) -> EodResult
where
    LoadWindows: FnOnce() -> Result<
        IntradaySessionWindowContract,
        collectors::intraday_quotes::IntradaySessionWindowError,
    >,
    StartQuote: FnOnce(Arc<IntradaySessionWindowContract>) -> Option<QuoteTask>,
    ContinueEod: FnOnce(Option<QuoteTask>) -> EodFuture,
    EodFuture: Future<Output = EodResult>,
{
    let intraday_task = match prepare_intraday_quote_startup(mode, intraday_mode, load_windows) {
        IntradayQuoteStartup::Ready(windows) => start_quote(windows),
        IntradayQuoteStartup::Unavailable(_) => {
            eprintln!(
                "{}",
                json!({
                    "event":"owner_equity_v2_intraday",
                    "code":"SESSION_WINDOWS_UNAVAILABLE"
                })
            );
            None
        }
        IntradayQuoteStartup::Disabled => None,
    };
    continue_eod(intraday_task).await
}

fn positive_duration(name: &str, default: Duration) -> Result<Duration, ConfigError> {
    let seconds = optional_u64(name)?.unwrap_or(default.as_secs());
    if seconds == 0 {
        Err(ConfigError::Invalid)
    } else {
        Ok(Duration::from_secs(seconds))
    }
}

fn optional_u64(name: &str) -> Result<Option<u64>, ConfigError> {
    match std::env::var(name) {
        Ok(value) => value.parse().map(Some).map_err(|_| ConfigError::Invalid),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::Invalid),
    }
}

fn required_path(name: &str) -> Result<PathBuf, ConfigError> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(ConfigError::Invalid)
}

fn required_string(name: &str) -> Result<String, ConfigError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Invalid)
}

fn intraday_session_window_source() -> Result<IntradaySessionWindowSource, ConfigError> {
    let value = std::env::var_os(INTRADAY_SESSION_WINDOWS_SOURCE_ENV)
        .map(|value| value.into_string().map_err(|_| ConfigError::Invalid))
        .transpose()?;
    IntradaySessionWindowSource::from_optional_str(value.as_deref())
        .map_err(|_| ConfigError::Invalid)
}

fn valid_root(path: &Path, production: bool) -> bool {
    (!production || path.is_absolute())
        && path != Path::new("/")
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        && std::fs::symlink_metadata(path)
            .map(|metadata| {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return false;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0
                    {
                        return false;
                    }
                }
                path.canonicalize().is_ok_and(|canonical| canonical == path)
            })
            .unwrap_or(false)
}

fn valid_secret_reference(path: &Path, production: bool) -> bool {
    (!production || path.is_absolute())
        && path != Path::new("/")
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        && std::fs::symlink_metadata(path)
            .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
}

fn valid_worker_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn reject_forbidden_environment() -> Result<(), ConfigError> {
    const FORBIDDEN: &[&str] = &[
        "KIS_APP_KEY",
        "KIS_APP_SECRET",
        "DB_PASSWORD",
        concat!("CA", "NO"),
        concat!("ACNT_", "PRDT_CD"),
        concat!("KIS_", "ACCOUNT_REF"),
    ];
    if FORBIDDEN
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        Err(ConfigError::Invalid)
    } else {
        Ok(())
    }
}

fn load_config() -> Result<Config, ConfigError> {
    reject_forbidden_environment()?;
    let mode = parse_args_from(std::env::args_os().collect())?;
    let environment = Environment::read()?;
    let production = environment.production();
    let session_window_source = intraday_session_window_source()?;
    let heartbeat = positive_duration("OWNER_EQUITY_V2_HEARTBEAT_SECS", DEFAULT_HEARTBEAT)?;
    let lease = positive_duration("OWNER_EQUITY_V2_LEASE_SECS", DEFAULT_LEASE)?;
    if heartbeat >= lease {
        return Err(ConfigError::Invalid);
    }
    let health_path = std::env::var_os("OWNER_EQUITY_V2_HEALTH_STATE_PATH").map(PathBuf::from);
    if production
        && health_path
            .as_deref()
            .is_none_or(|path| !path.is_absolute())
    {
        return Err(ConfigError::Invalid);
    }
    if let Some(parent) = health_path.as_deref().and_then(Path::parent)
        && !valid_root(parent, production)
    {
        return Err(ConfigError::Invalid);
    }
    if mode == Mode::Healthcheck {
        return Ok(Config {
            mode,
            intraday_mode: IntradayQuotesMode::Disabled,
            session_window_source,
            production,
            raw_root: PathBuf::new(),
            artifact_root: PathBuf::new(),
            app_key_file: PathBuf::new(),
            app_secret_file: PathBuf::new(),
            health_path,
            worker_id: "owner-equity-v2-healthcheck".to_owned(),
            poll: DEFAULT_POLL,
            recovery: DEFAULT_RECOVERY,
            heartbeat,
            lease,
            backoff: DEFAULT_BACKOFF,
            work_timeout: DEFAULT_WORK_TIMEOUT,
            health_max_age: positive_duration(
                "OWNER_EQUITY_V2_HEALTH_MAX_AGE_SECS",
                DEFAULT_HEALTH_MAX_AGE,
            )?,
            limits: OwnerEquityRuntimeLimits::default(),
            schedule_pins: None,
        });
    }
    let read_coordination =
        ProductionReadCoordination::from_env().map_err(|_| ConfigError::Invalid)?;
    let max_active = optional_u64("OWNER_EQUITY_V2_MAX_ACTIVE")?.unwrap_or(100);
    let initial_gets = optional_u64("OWNER_EQUITY_V2_INITIAL_GET_CEILING")?.unwrap_or(7);
    let incremental_gets = optional_u64("OWNER_EQUITY_V2_INCREMENTAL_GET_CEILING")?.unwrap_or(2);
    let total_gets = optional_u64("OWNER_EQUITY_V2_TOTAL_BACKFILL_GET_CEILING")?.unwrap_or(700);
    let concurrency = optional_u64("OWNER_EQUITY_V2_CONCURRENCY")?.unwrap_or(1);
    let estimated_bytes =
        optional_u64("OWNER_EQUITY_V2_ESTIMATED_BYTES_PER_GET")?.unwrap_or(1_048_576);
    let limits = OwnerEquityRuntimeLimits {
        maximum_active_instruments: u32::try_from(max_active).map_err(|_| ConfigError::Invalid)?,
        initial_get_ceiling_per_job: usize::try_from(initial_gets)
            .map_err(|_| ConfigError::Invalid)?,
        incremental_get_ceiling_per_job: usize::try_from(incremental_gets)
            .map_err(|_| ConfigError::Invalid)?,
        total_initial_backfill_get_ceiling: usize::try_from(total_gets)
            .map_err(|_| ConfigError::Invalid)?,
        concurrency: usize::try_from(concurrency).map_err(|_| ConfigError::Invalid)?,
        estimated_bytes_per_get: estimated_bytes,
    }
    .validate()
    .map_err(|_| ConfigError::Invalid)?;
    let worker_id = match std::env::var("OWNER_EQUITY_V2_WORKER_ID") {
        Ok(value) => value,
        Err(_) if !production => "owner-equity-v2-local".to_owned(),
        Err(_) => return Err(ConfigError::Invalid),
    };
    if !valid_worker_id(&worker_id) {
        return Err(ConfigError::Invalid);
    }
    let raw_root = required_path("OWNER_EQUITY_V2_RAW_ROOT")?;
    let artifact_root = required_path("OWNER_EQUITY_V2_ARTIFACT_ROOT")?;
    if !valid_root(&raw_root, production) || !valid_root(&artifact_root, production) {
        return Err(ConfigError::Invalid);
    }
    let app_key_file = required_path("KIS_APP_KEY_FILE")?;
    let app_secret_file = required_path("KIS_APP_SECRET_FILE")?;
    if !valid_secret_reference(&app_key_file, production)
        || !valid_secret_reference(&app_secret_file, production)
    {
        return Err(ConfigError::Invalid);
    }
    let schedule_pins = OwnerEquitySchedulePins::new(
        required_string("LAGRANGE_CODE_COMMIT")?,
        required_string("OWNER_EQUITY_V2_ENTITLEMENT_REFERENCE")?,
        required_string("OWNER_EQUITY_V2_ENTITLEMENT_SHA256")?,
    )
    .map_err(|_| ConfigError::Invalid)?;
    Ok(Config {
        mode,
        intraday_mode: read_coordination.intraday_mode(),
        session_window_source,
        production,
        raw_root,
        artifact_root,
        app_key_file,
        app_secret_file,
        health_path,
        worker_id,
        poll: positive_duration("OWNER_EQUITY_V2_POLL_SECS", DEFAULT_POLL)?,
        recovery: positive_duration("OWNER_EQUITY_V2_RECOVERY_SECS", DEFAULT_RECOVERY)?,
        heartbeat,
        lease,
        backoff: positive_duration("OWNER_EQUITY_V2_BACKOFF_SECS", DEFAULT_BACKOFF)?,
        work_timeout: positive_duration("OWNER_EQUITY_V2_WORK_TIMEOUT_SECS", DEFAULT_WORK_TIMEOUT)?,
        health_max_age: positive_duration(
            "OWNER_EQUITY_V2_HEALTH_MAX_AGE_SECS",
            DEFAULT_HEALTH_MAX_AGE,
        )?,
        limits,
        schedule_pins: Some(schedule_pins),
    })
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

fn read_secret_file(path: PathBuf) -> Result<String, ConfigError> {
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| ConfigError::Invalid)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ConfigError::Invalid);
    }
    let value = std::fs::read_to_string(path).map_err(|_| ConfigError::Invalid)?;
    if value.is_empty() || value.contains(['\r', '\n']) {
        Err(ConfigError::Invalid)
    } else {
        Ok(value)
    }
}

fn database_options(production: bool) -> Result<PgConnectOptions, ConfigError> {
    let direct = std::env::var("DATABASE_URL").ok();
    let direct_file = std::env::var_os("DATABASE_URL_FILE").map(PathBuf::from);
    let component_names = [
        "DB_HOST",
        "DB_PORT",
        "DB_NAME",
        "DB_USER",
        "DB_PASSWORD_FILE",
    ];
    let components = component_names.map(|name| (name, std::env::var_os(name)));
    let any_components = components.iter().any(|(_, value)| value.is_some());
    if production && (direct.is_some() || direct_file.is_some()) {
        return Err(ConfigError::Invalid);
    }
    if (direct.is_some() || direct_file.is_some()) && any_components {
        return Err(ConfigError::Invalid);
    }
    match (direct, direct_file, any_components) {
        (Some(_), Some(_), _) => Err(ConfigError::Invalid),
        (Some(url), None, false) => url.parse().map_err(|_| ConfigError::Invalid),
        (None, Some(path), false) => read_secret_file(path)?
            .parse()
            .map_err(|_| ConfigError::Invalid),
        (None, None, true) => {
            let value = |name| {
                components
                    .iter()
                    .find(|(key, _)| *key == name)
                    .and_then(|(_, value)| value.clone())
                    .and_then(|value| value.into_string().ok())
                    .filter(|value| !value.is_empty())
                    .ok_or(ConfigError::Invalid)
            };
            let port = value("DB_PORT")?
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .ok_or(ConfigError::Invalid)?;
            Ok(PgConnectOptions::new()
                .host(&value("DB_HOST")?)
                .port(port)
                .database(&value("DB_NAME")?)
                .username(&value("DB_USER")?)
                .password(&read_secret_file(PathBuf::from(value(
                    "DB_PASSWORD_FILE",
                )?))?))
        }
        _ => Err(ConfigError::Invalid),
    }
}

fn build_live_reader(config: &Config) -> Result<LiveReader, ConfigError> {
    let coordination = ProductionReadCoordination::from_env().map_err(|_| ConfigError::Invalid)?;
    let app_key = CredentialRef::file(config.app_key_file.to_string_lossy().into_owned());
    let app_secret = CredentialRef::file(config.app_secret_file.to_string_lossy().into_owned());
    let token_transport =
        LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(|_| ConfigError::Invalid)?;
    let read_transport = LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(|_| ConfigError::Invalid)?;
    let clock = Arc::new(SystemClock);
    let snapshot = match coordination.mode() {
        ReadCoordinationMode::Legacy => None,
        ReadCoordinationMode::SharedRequired => Some(
            ReadCredentialSnapshot::resolve(
                &SystemCredentialSource,
                app_key.clone(),
                app_secret.clone(),
                coordination.generation().expect("shared generation parsed"),
            )
            .map_err(|_| ConfigError::Invalid)?,
        ),
    };
    let issuer: Arc<dyn TokenIssuer> = match snapshot.as_ref() {
        Some(snapshot) => Arc::new(KisTokenIssuer::new(
            token_transport,
            snapshot.clone(),
            app_key.clone(),
            app_secret.clone(),
            kis_system_now_ms,
        )),
        None => Arc::new(KisTokenIssuer::new(
            token_transport,
            SystemCredentialSource,
            app_key.clone(),
            app_secret.clone(),
            kis_system_now_ms,
        )),
    };
    let tokens = Arc::new(TokenManager::new(clock.clone(), Arc::clone(&issuer)));
    let quota = Quota::new(1, 1);
    let limiter = RateLimiter::new(clock, quota)
        .with_quota(BucketKey::new(REFERENCE_PATH, REFERENCE_TR_ID), quota)
        .with_quota(BucketKey::new(DAILY_BARS_PATH, DAILY_BARS_TR_ID), quota);
    let mut client = KisMarketDataClient::new(
        read_transport,
        TokioSleeper,
        tokens,
        Arc::new(limiter),
        SystemCredentialSource,
        app_key,
        app_secret,
    );
    if let Some(snapshot) = snapshot {
        let auth = CoordinatedReadAuth::new(
            coordination
                .production_config()
                .expect("shared production config"),
            snapshot,
            Arc::new(SystemClock),
            issuer,
        )
        .map_err(|_| ConfigError::Invalid)?;
        let intraday_transport =
            LiveTransport::live(Duration::from_secs(3)).map_err(|_| ConfigError::Invalid)?;
        client = client.with_shared_read_coordination(auth, intraday_transport);
    }
    Ok(client)
}

fn kis_system_now_ms() -> i64 {
    SystemClock.now_ms()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Health {
    pid: u32,
    heartbeat_unix_seconds: u64,
}

fn write_health(path: &Path) -> Result<(), ConfigError> {
    let parent = path.parent().ok_or(ConfigError::Invalid)?;
    if !valid_root(parent, false) {
        return Err(ConfigError::Invalid);
    }
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| ConfigError::Invalid)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ConfigError::Invalid);
        }
    }
    let state = Health {
        pid: std::process::id(),
        heartbeat_unix_seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ConfigError::Invalid)?
            .as_secs(),
    };
    let temporary = parent.join(format!(".owner-equity-v2-health-{}.tmp", state.pid));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| ConfigError::Invalid)?;
    let result = (|| {
        file.write_all(&serde_json::to_vec(&state).map_err(|_| ConfigError::Invalid)?)
            .map_err(|_| ConfigError::Invalid)?;
        file.sync_all().map_err(|_| ConfigError::Invalid)?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| ConfigError::Invalid)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn healthcheck(path: Option<&Path>, max_age: Duration) -> Result<(), ConfigError> {
    let path = path.ok_or(ConfigError::Invalid)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ConfigError::Invalid)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ConfigError::Invalid);
    }
    let state: Health =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| ConfigError::Invalid)?)
            .map_err(|_| ConfigError::Invalid)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ConfigError::Invalid)?
        .as_secs();
    if now.saturating_sub(state.heartbeat_unix_seconds) > max_age.as_secs() {
        return Err(ConfigError::Invalid);
    }
    #[cfg(unix)]
    if !Path::new("/proc").join(state.pid.to_string()).is_dir() {
        return Err(ConfigError::Invalid);
    }
    Ok(())
}

async fn health_writer(path: PathBuf, mut shutdown: watch::Receiver<bool>) -> Result<(), ()> {
    let mut ticker = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let write_path = path.clone();
                tokio::task::spawn_blocking(move || write_health(&write_path))
                    .await.map_err(|_| ())?.map_err(|_| ())?;
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return Ok(()); }
            }
        }
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

fn outcome_label(outcome: OwnerEquityRunOutcome) -> &'static str {
    match outcome {
        OwnerEquityRunOutcome::Idle => "IDLE",
        OwnerEquityRunOutcome::Published => "PUBLISHED",
        OwnerEquityRunOutcome::InsufficientHistory => "INSUFFICIENT_HISTORY",
        OwnerEquityRunOutcome::Retrying => "RETRYING",
        OwnerEquityRunOutcome::Failed => "FAILED",
        OwnerEquityRunOutcome::Disabled => "DISABLED",
        OwnerEquityRunOutcome::Canceled => "CANCELED",
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(_) => {
            eprintln!(
                "{}",
                json!({"event":"owner_equity_v2_startup","code":"CONFIG_INVALID"})
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
                json!({"event":"owner_equity_v2_health","code":"UNHEALTHY"})
            );
            ExitCode::FAILURE
        };
    }
    let options = match database_options(config.production) {
        Ok(options) => options,
        Err(_) => {
            eprintln!(
                "{}",
                json!({"event":"owner_equity_v2_startup","code":"DATABASE_CONFIG_INVALID"})
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
                json!({"event":"owner_equity_v2_startup","code":"DATABASE_UNAVAILABLE"})
            );
            return ExitCode::FAILURE;
        }
    };
    let reader = match build_live_reader(&config) {
        Ok(reader) => reader,
        Err(_) => {
            eprintln!(
                "{}",
                json!({"event":"owner_equity_v2_startup","code":"PROVIDER_CONFIG_INVALID"})
            );
            return ExitCode::FAILURE;
        }
    };
    let adapter = match ProductionOwnerEquityAdapter::new(
        &config.raw_root,
        &config.artifact_root,
        reader,
        config.limits,
    ) {
        Ok(adapter) => adapter,
        Err(_) => {
            eprintln!(
                "{}",
                json!({"event":"owner_equity_v2_startup","code":"STORAGE_CONFIG_INVALID"})
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
    let runner = OwnerEquityRunnerConfig::new(config.heartbeat, config.lease, config.work_timeout)
        .expect("validated runner configuration");
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let health_task = config
        .health_path
        .clone()
        .map(|path| tokio::spawn(health_writer(path, shutdown_rx.clone())));
    if config.mode == Mode::Daemon {
        let signal_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            let _ = signal_tx.send(true);
        });
    }
    let quote_shutdown_rx = shutdown_rx.clone();
    let session_window_source = config.session_window_source;
    run_intraday_quote_startup(
        config.mode,
        config.intraday_mode,
        move || IntradaySessionWindowContract::from_source(session_window_source),
        |windows| {
            let producer_config = match IntradayProducerConfig::for_worker(&config.worker_id) {
                Ok(config) => Some(config),
                Err(_) => {
                    eprintln!(
                        "{}",
                        json!({
                            "event":"owner_equity_v2_intraday",
                            "code":"PRODUCER_CONFIG_INVALID"
                        })
                    );
                    None
                }
            };
            producer_config.map(|producer_config| {
                let producer = IntradayProducer::new(
                    OwnerIntradayQuoteRepository::new(pool.clone()),
                    adapter.intraday_reader(),
                    windows,
                    producer_config,
                );
                let intraday_shutdown = quote_shutdown_rx.clone();
                tokio::spawn(async move { producer.run_daemon(intraday_shutdown).await })
            })
        },
        |mut intraday_task| async {
            let mut next_recovery = tokio::time::Instant::now();
            let seoul = FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset");
            let mut schedule_cadence = ScheduleCadence::default();
            let mut next_schedule_attempt = tokio::time::Instant::now();
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
                        json!({"event":"owner_equity_v2_health","code":"WRITER_UNAVAILABLE"})
                    );
                    exit = ExitCode::FAILURE;
                    break;
                }
                let intraday_finished = intraday_task
                    .as_ref()
                    .is_some_and(tokio::task::JoinHandle::is_finished);
                if intraday_finished {
                    let task = intraday_task
                        .take()
                        .expect("finished intraday task remains owned by runner");
                    match task.await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) | Err(_) => {
                            eprintln!(
                                "{}",
                                json!({"event":"owner_equity_v2_intraday","code":"PRODUCER_UNAVAILABLE"})
                            );
                            // The quote task is intentionally independent: its
                            // failure must not cancel or block the EOD worker.
                            exit = ExitCode::FAILURE;
                        }
                    }
                }
                let now = tokio::time::Instant::now();
                let now_kst = Utc::now().with_timezone(&seoul);
                if let Some(key) = schedule_cadence.pending_key(now_kst)
                    && now >= next_schedule_attempt
                {
                    let pins = config
                        .schedule_pins
                        .as_ref()
                        .expect("non-healthcheck configuration has schedule pins");
                    match run_owner_equity_schedule_cycle(&pool, pins, now_kst).await {
                        Ok(report) => {
                            schedule_cadence.complete(key);
                            println!(
                                "{}",
                                json!({
                                    "event":"owner_equity_v2_schedule",
                                    "as_of":report.as_of,
                                    "scheduled":report.scheduled
                                })
                            );
                        }
                        Err(OwnerEquityScheduleError::NoConfirmedClose) => {
                            eprintln!(
                                "{}",
                                json!({
                                    "event":"owner_equity_v2_schedule",
                                    "code":"CONFIRMED_CLOSE_UNAVAILABLE"
                                })
                            );
                            next_schedule_attempt =
                                tokio::time::Instant::now() + Duration::from_secs(60);
                        }
                        Err(
                            OwnerEquityScheduleError::Database
                            | OwnerEquityScheduleError::InvalidPins,
                        ) => {
                            eprintln!(
                                "{}",
                                json!({
                                    "event":"owner_equity_v2_schedule",
                                    "code":"SCHEDULER_UNAVAILABLE"
                                })
                            );
                            next_schedule_attempt =
                                tokio::time::Instant::now() + Duration::from_secs(60);
                        }
                    }
                }
                if now >= next_recovery {
                    if recover_owner_equity_claims(&queue).await.is_err() {
                        eprintln!(
                            "{}",
                            json!({"event":"owner_equity_v2_recovery","code":"RECOVERY_UNAVAILABLE"})
                        );
                        exit = ExitCode::FAILURE;
                        break;
                    }
                    next_recovery = now + config.recovery;
                }
                let work =
                    run_owner_equity_runner_once(&pool, &queue, &config.worker_id, &adapter, runner);
                tokio::pin!(work);
                let outcome = tokio::select! {
                    result = &mut work => result,
                    changed = shutdown_rx.changed(), if config.mode == Mode::Daemon => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            work.await
                        } else {
                            continue;
                        }
                    }
                };
                match outcome {
                    Ok(outcome) => {
                        println!(
                            "{}",
                            json!({"event":"owner_equity_v2_run","outcome":outcome_label(outcome)})
                        );
                        if config.mode == Mode::Once {
                            break;
                        }
                    }
                    Err(_) => {
                        eprintln!(
                            "{}",
                            json!({"event":"owner_equity_v2_run","code":"WORKER_UNAVAILABLE"})
                        );
                        exit = ExitCode::FAILURE;
                        break;
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(config.poll) => {}
                    changed = shutdown_rx.changed(), if config.mode == Mode::Daemon => {
                        if changed.is_err() || *shutdown_rx.borrow() { break; }
                    }
                }
            }
            let _ = shutdown_tx.send(true);
            if let Some(task) = health_task
                && task.await.is_err()
            {
                exit = ExitCode::FAILURE;
            }
            if let Some(task) = intraday_task {
                match task.await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) | Err(_) => {
                        eprintln!(
                            "{}",
                            json!({
                                "event":"owner_equity_v2_intraday",
                                "code":"PRODUCER_UNAVAILABLE"
                            })
                        );
                        exit = ExitCode::FAILURE;
                    }
                }
            }
            exit
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sha2::{Digest, Sha256};
    use std::{cell::RefCell, rc::Rc};

    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        std::iter::once("owner-equity-v2-runner")
            .chain(values.iter().copied())
            .map(Into::into)
            .collect()
    }

    #[test]
    fn modes_are_exact_and_once_is_distinct() {
        assert_eq!(parse_args_from(args(&[])), Ok(Mode::Daemon));
        assert_eq!(parse_args_from(args(&["--once"])), Ok(Mode::Once));
        assert_eq!(
            parse_args_from(args(&["healthcheck"])),
            Ok(Mode::Healthcheck)
        );
        assert_eq!(
            parse_args_from(args(&["--once", "healthcheck"])),
            Err(ConfigError::Invalid)
        );
        assert!(!should_start_intraday(
            Mode::Once,
            IntradayQuotesMode::OwnerOnly
        ));
        assert!(!should_start_intraday(
            Mode::Daemon,
            IntradayQuotesMode::Disabled
        ));
        assert!(should_start_intraday(
            Mode::Daemon,
            IntradayQuotesMode::OwnerOnly
        ));
    }

    #[test]
    fn session_window_source_is_explicit_and_legacy_compatible() {
        assert_eq!(
            IntradaySessionWindowSource::from_optional_str(None).unwrap(),
            IntradaySessionWindowSource::ReleaseV1
        );
        assert_eq!(
            IntradaySessionWindowSource::from_optional_str(Some("release_v1")).unwrap(),
            IntradaySessionWindowSource::ReleaseV1
        );
        assert_eq!(
            IntradaySessionWindowSource::from_optional_str(Some("operational_v1")).unwrap(),
            IntradaySessionWindowSource::OperationalV1
        );
        assert!(IntradaySessionWindowSource::from_optional_str(Some("unknown")).is_err());
    }

    fn valid_window_contract_fixture() -> IntradaySessionWindowContract {
        let bytes = br#"{
            "schema_version":1,
            "exchange":"KRX",
            "timezone":"Asia/Seoul",
            "entries":[]
        }"#;
        let hash = format!("sha256:{:x}", Sha256::digest(bytes));
        IntradaySessionWindowContract::from_bytes(bytes, &hash)
            .expect("valid injected window fixture")
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StartupEvent {
        Load,
        Start,
        Eod,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeQuoteTask {
        Started,
    }

    fn record_event(events: &Rc<RefCell<Vec<StartupEvent>>>, event: StartupEvent) {
        events.borrow_mut().push(event);
    }

    #[tokio::test]
    async fn invalid_intraday_windows_continue_to_eod_without_starting_quote() {
        for error in [
            collectors::intraday_quotes::IntradaySessionWindowError::Missing,
            collectors::intraday_quotes::IntradaySessionWindowError::Invalid,
            collectors::intraday_quotes::IntradaySessionWindowError::HashMismatch,
        ] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let load_events = Rc::clone(&events);
            let start_events = Rc::clone(&events);
            let eod_events = Rc::clone(&events);
            let result = run_intraday_quote_startup(
                Mode::Daemon,
                IntradayQuotesMode::OwnerOnly,
                move || {
                    record_event(&load_events, StartupEvent::Load);
                    Err(error)
                },
                move |_| -> Option<FakeQuoteTask> {
                    record_event(&start_events, StartupEvent::Start);
                    panic!("invalid session windows must not start the quote task");
                },
                move |task| async move {
                    record_event(&eod_events, StartupEvent::Eod);
                    assert_eq!(task, None);
                    0xC3_u16
                },
            )
            .await;

            assert_eq!(result, 0xC3_u16);
            assert_eq!(&*events.borrow(), &[StartupEvent::Load, StartupEvent::Eod]);
        }
    }

    #[tokio::test]
    async fn valid_owner_only_startup_delivers_windows_and_continues_to_eod() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let load_events = Rc::clone(&events);
        let start_events = Rc::clone(&events);
        let eod_events = Rc::clone(&events);
        let expected_windows = Arc::new(valid_window_contract_fixture());
        let delivered_windows = Rc::new(RefCell::new(None));
        let loader_windows = Arc::clone(&expected_windows);
        let delivered_for_factory = Rc::clone(&delivered_windows);

        let result = run_intraday_quote_startup(
            Mode::Daemon,
            IntradayQuotesMode::OwnerOnly,
            move || {
                record_event(&load_events, StartupEvent::Load);
                Ok(loader_windows.as_ref().clone())
            },
            move |windows| {
                record_event(&start_events, StartupEvent::Start);
                delivered_for_factory
                    .borrow_mut()
                    .replace(Arc::clone(&windows));
                Some(FakeQuoteTask::Started)
            },
            move |task| async move {
                record_event(&eod_events, StartupEvent::Eod);
                assert_eq!(task, Some(FakeQuoteTask::Started));
                0xC3_u16
            },
        )
        .await;

        assert_eq!(result, 0xC3_u16);
        assert_eq!(delivered_windows.borrow().as_ref(), Some(&expected_windows));
        assert_eq!(
            &*events.borrow(),
            &[StartupEvent::Load, StartupEvent::Start, StartupEvent::Eod,]
        );
    }

    #[tokio::test]
    async fn valid_windows_with_no_quote_task_still_continue_to_eod() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let load_events = Rc::clone(&events);
        let start_events = Rc::clone(&events);
        let eod_events = Rc::clone(&events);
        let result = run_intraday_quote_startup(
            Mode::Daemon,
            IntradayQuotesMode::OwnerOnly,
            move || {
                record_event(&load_events, StartupEvent::Load);
                Ok(valid_window_contract_fixture())
            },
            move |_| {
                record_event(&start_events, StartupEvent::Start);
                None::<FakeQuoteTask>
            },
            move |task| async move {
                record_event(&eod_events, StartupEvent::Eod);
                assert_eq!(task, None);
                0xC4_u16
            },
        )
        .await;

        assert_eq!(result, 0xC4_u16);
        assert_eq!(
            &*events.borrow(),
            &[StartupEvent::Load, StartupEvent::Start, StartupEvent::Eod,]
        );
    }

    #[tokio::test]
    async fn disabled_and_once_skip_loader_and_factory_but_continue_to_eod() {
        for (mode, intraday_mode) in [
            (Mode::Once, IntradayQuotesMode::OwnerOnly),
            (Mode::Daemon, IntradayQuotesMode::Disabled),
        ] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let load_events = Rc::clone(&events);
            let start_events = Rc::clone(&events);
            let eod_events = Rc::clone(&events);
            let result = run_intraday_quote_startup(
                mode,
                intraday_mode,
                move || -> Result<
                    IntradaySessionWindowContract,
                    collectors::intraday_quotes::IntradaySessionWindowError,
                > {
                    record_event(&load_events, StartupEvent::Load);
                    panic!("disabled quote startup must not load session windows");
                },
                move |_| -> Option<FakeQuoteTask> {
                    record_event(&start_events, StartupEvent::Start);
                    panic!("disabled quote startup must not start the quote task");
                },
                move |task| async move {
                    record_event(&eod_events, StartupEvent::Eod);
                    assert_eq!(task, None);
                    0xC5_u16
                },
            )
            .await;

            assert_eq!(result, 0xC5_u16);
            assert_eq!(&*events.borrow(), &[StartupEvent::Eod]);
        }
    }

    #[test]
    fn scheduler_catches_up_once_per_confirmed_close_key() {
        let seoul = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        let before = seoul.with_ymd_and_hms(2026, 8, 31, 16, 29, 59).unwrap();
        let at = seoul.with_ymd_and_hms(2026, 8, 31, 16, 30, 0).unwrap();
        let mut cadence = ScheduleCadence::default();
        let catchup = cadence.pending_key(before).unwrap();
        assert_eq!(catchup.to_string(), "2026-08-30");
        cadence.complete(catchup);
        assert_eq!(cadence.pending_key(before), None);
        let current = cadence.pending_key(at).unwrap();
        assert_eq!(current.to_string(), "2026-08-31");
    }

    #[test]
    fn worker_id_is_stable_safe_ascii() {
        assert!(valid_worker_id("owner-equity-v2-prod-a"));
        assert!(!valid_worker_id(""));
        assert!(!valid_worker_id("owner/equity"));
        assert!(!valid_worker_id(&"x".repeat(129)));
    }

    #[test]
    fn roots_reject_symlink_parent_segments_and_files() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        }
        assert!(valid_root(root.path(), true));
        assert!(!valid_root(Path::new("/"), true));
        assert!(!valid_root(&root.path().join(".."), true));
        assert!(!valid_root(&root.path().join("missing"), true));
    }

    #[test]
    fn health_round_trip_is_value_free() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        }
        let path = root.path().join("health.json");
        write_health(&path).unwrap();
        healthcheck(Some(&path), Duration::from_secs(10)).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("secret"));
        assert!(!text.contains("credential"));
    }

    #[test]
    fn outcome_and_error_logs_have_no_values() {
        for outcome in [
            OwnerEquityRunOutcome::Idle,
            OwnerEquityRunOutcome::Published,
            OwnerEquityRunOutcome::InsufficientHistory,
            OwnerEquityRunOutcome::Retrying,
            OwnerEquityRunOutcome::Failed,
            OwnerEquityRunOutcome::Disabled,
            OwnerEquityRunOutcome::Canceled,
        ] {
            assert!(
                outcome_label(outcome)
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            );
        }
    }

    #[test]
    fn daemon_source_keeps_recovery_shutdown_token_and_rate_seams() {
        let source = include_str!("owner-equity-v2-runner.rs");
        let production = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert!(production.contains("recover_owner_equity_claims(&queue).await"));
        assert!(production.contains("shutdown_signal().await"));
        assert!(production.contains("work.await"));
        assert_eq!(production.matches("TokenManager::new").count(), 1);
        assert!(production.contains("ProductionReadCoordination::from_env"));
        assert!(production.contains("with_shared_read_coordination"));
        assert!(production.contains("Quota::new(1, 1)"));
        assert!(production.contains("REFERENCE_PATH, REFERENCE_TR_ID"));
        assert!(production.contains("DAILY_BARS_PATH, DAILY_BARS_TR_ID"));
        assert!(!production.contains("response.body"));
    }
}
