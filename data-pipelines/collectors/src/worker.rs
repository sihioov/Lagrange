use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveTime, TimeZone, Utc};
use domain::{BatchId, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR};
use market_data::ingest::IngestRequest;
use market_data::provider::{EodProvider, KrxProvider, ProviderError, RecordedBundle};
use market_data::storage::RawStore;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::{
    FailureClass, PipelineError, PostgresPublicationSink, PublicationSink, SinkError,
    ingest_and_publish, provider_failure_class, recover_unpublished,
};

const DEFAULT_RUN_AT_KST: &str = "16:30";
const DEFAULT_MAX_PUBLICATION_AGE_SECS: u64 = 4 * 24 * 60 * 60;
const KST_OFFSET_SECS: i32 = 9 * 60 * 60;
const WHOLE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnvironment {
    Development,
    Qa,
    Production,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub user: String,
    pub password: SecretValue,
}

#[derive(Debug, Clone)]
pub struct ResearchWorkerConfig {
    pub app_env: AppEnvironment,
    pub fetch_mode: FetchMode,
    pub run_at_kst: NaiveTime,
    pub max_publication_age: Duration,
    pub raw_root: PathBuf,
    pub database: DatabaseConfig,
    pub provider_credential: Option<SecretValue>,
    pub synthetic_bundle: PathBuf,
}

#[derive(Debug, Clone)]
pub struct HealthcheckConfig {
    pub max_publication_age: Duration,
    pub database: DatabaseConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("missing required configuration {key}")]
    MissingConfig { key: &'static str },
    #[error("invalid configuration {key}")]
    InvalidConfig { key: &'static str },
    #[error("synthetic research fetches are forbidden in {environment}")]
    SyntheticForbidden { environment: &'static str },
    #[error("unable to read nonempty secret from {key}")]
    SecretFile { key: &'static str },
    #[error("worker I/O failed during {phase:?}")]
    Io { phase: WorkerPhase },
    #[error("worker attempt timed out during {phase:?}")]
    Timeout { phase: WorkerPhase },
    #[error("research provider is not configured")]
    ProviderNotConfigured,
    #[error("research provider construction failed")]
    Provider(#[source] ProviderError),
    #[error("database operation failed during {phase:?}")]
    Database {
        phase: WorkerPhase,
        #[source]
        source: SinkError,
    },
    #[error("research worker is unhealthy: {reason}")]
    Unhealthy { reason: HealthFailure },
    #[error("research pipeline failed")]
    Pipeline(#[source] PipelineError),
}

impl WorkerError {
    pub fn failure_class(&self) -> FailureClass {
        match self {
            Self::Io { .. } | Self::Timeout { .. } => FailureClass::Retryable,
            Self::Pipeline(source) => source.failure_class(),
            Self::Provider(source) => provider_failure_class(source),
            Self::Database { source, .. } => {
                if source.is_retryable() {
                    FailureClass::Retryable
                } else {
                    FailureClass::Permanent
                }
            }
            Self::MissingConfig { .. }
            | Self::InvalidConfig { .. }
            | Self::SyntheticForbidden { .. }
            | Self::SecretFile { .. }
            | Self::ProviderNotConfigured
            | Self::Unhealthy { .. } => FailureClass::Permanent,
        }
    }

    pub fn phase(&self) -> WorkerPhase {
        match self {
            Self::MissingConfig { .. }
            | Self::InvalidConfig { .. }
            | Self::SyntheticForbidden { .. }
            | Self::SecretFile { .. } => WorkerPhase::Config,
            Self::ProviderNotConfigured | Self::Provider(_) => WorkerPhase::Provider,
            Self::Database { phase, .. } => *phase,
            Self::Unhealthy { .. } => WorkerPhase::Health,
            Self::Io { phase } | Self::Timeout { phase } => *phase,
            Self::Pipeline(source) => match source.stage() {
                crate::PipelineStage::ReadManifest => WorkerPhase::Recovery,
                crate::PipelineStage::PublicationState
                | crate::PipelineStage::VerifyRaw
                | crate::PipelineStage::Publish => WorkerPhase::Publication,
                crate::PipelineStage::Ingest => WorkerPhase::Ingest,
            },
        }
    }

    pub fn batch_id(&self) -> Option<BatchId> {
        match self {
            Self::Pipeline(source) => source.batch_id(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerPhase {
    Config,
    Provider,
    Recovery,
    DuplicateCheck,
    Ingest,
    Publication,
    Health,
    Database,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthFailure {
    NoEodPublication,
    StaleEodPublication,
}

impl fmt::Display for HealthFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoEodPublication => "no KRX/KR EOD publication",
            Self::StaleEodPublication => "latest KRX/KR EOD publication is stale",
        })
    }
}

impl WorkerPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Provider => "provider",
            Self::Recovery => "recovery",
            Self::DuplicateCheck => "duplicate_check",
            Self::Ingest => "ingest",
            Self::Publication => "publication",
            Self::Health => "health",
            Self::Database => "database",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Elapsed,
    Shutdown,
}

#[async_trait]
pub trait WorkerControl: Send + Sync {
    fn now_utc(&self) -> DateTime<Utc>;
    async fn wait(&self, duration: Option<Duration>) -> WaitOutcome;
}

#[async_trait]
pub trait ResearchBackend: Send + Sync + 'static {
    async fn recover(&self) -> Result<(), WorkerError>;
    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError>;
    async fn ingest(&self, date: TradingDate, now: UtcTimestamp) -> Result<BatchId, WorkerError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerRunOutcome {
    AlreadyPublished,
    Published(BatchId),
    Shutdown,
}

pub struct ResearchWorker {
    config: ResearchWorkerConfig,
    backend: Arc<dyn ResearchBackend>,
}

pub trait WorkerComponentFactory: Send + Sync {
    fn build_provider(
        &self,
        config: &ResearchWorkerConfig,
    ) -> Result<Arc<dyn EodProvider>, WorkerError>;
    fn build_store(&self, config: &ResearchWorkerConfig) -> Result<RawStore, WorkerError>;
    fn build_pool(&self, config: &ResearchWorkerConfig) -> Result<PgPool, WorkerError>;
}

pub struct ProductionWorkerComponentFactory;

impl WorkerComponentFactory for ProductionWorkerComponentFactory {
    fn build_provider(
        &self,
        config: &ResearchWorkerConfig,
    ) -> Result<Arc<dyn EodProvider>, WorkerError> {
        match config.fetch_mode {
            FetchMode::Synthetic => {
                let bundle = RecordedBundle::open(&config.synthetic_bundle)
                    .map_err(WorkerError::Provider)?;
                Ok(Arc::new(KrxProvider::synthetic(bundle)))
            }
            // This is deliberately a permanent, honest failure. The licensed
            // transport has not been implemented and must never be simulated.
            FetchMode::Credentialed => Err(WorkerError::ProviderNotConfigured),
        }
    }

    fn build_store(&self, config: &ResearchWorkerConfig) -> Result<RawStore, WorkerError> {
        Ok(RawStore::new(&config.raw_root))
    }

    fn build_pool(&self, config: &ResearchWorkerConfig) -> Result<PgPool, WorkerError> {
        Ok(build_postgres_pool(&config.database))
    }
}

pub fn bootstrap_worker_with<F>(
    values: &HashMap<String, String>,
    secret_reader: F,
    factory: &dyn WorkerComponentFactory,
) -> Result<ResearchWorker, WorkerError>
where
    F: Fn(&Path) -> io::Result<String>,
{
    // Config parsing performs the production synthetic fence before invoking
    // the secret reader. Component construction occurs only after it returns.
    let config = ResearchWorkerConfig::from_map_with_reader(values, secret_reader)?;
    let provider = factory.build_provider(&config)?;
    let store = factory.build_store(&config)?;
    let pool = factory.build_pool(&config)?;
    let backend = Arc::new(PipelineResearchBackend {
        store,
        provider,
        sink: PostgresPublicationSink::new(pool),
    });
    Ok(ResearchWorker::new(config, backend))
}

pub fn bootstrap_worker(values: &HashMap<String, String>) -> Result<ResearchWorker, WorkerError> {
    bootstrap_worker_with(
        values,
        |path: &Path| std::fs::read_to_string(path),
        &ProductionWorkerComponentFactory,
    )
}

struct PipelineResearchBackend {
    store: RawStore,
    provider: Arc<dyn EodProvider>,
    sink: PostgresPublicationSink,
}

#[async_trait]
impl ResearchBackend for PipelineResearchBackend {
    async fn recover(&self) -> Result<(), WorkerError> {
        recover_unpublished(&self.store, &self.sink)
            .await
            .map(|_| ())
            .map_err(WorkerError::Pipeline)
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError> {
        self.sink
            .has_eod(date)
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::DuplicateCheck,
                source,
            })
    }

    async fn ingest(&self, date: TradingDate, now: UtcTimestamp) -> Result<BatchId, WorkerError> {
        let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
        ingest_and_publish(
            &self.store,
            self.provider.as_ref(),
            &request,
            None,
            &self.sink,
        )
        .await
        .map(|outcome| outcome.manifest.batch_id)
        .map_err(WorkerError::Pipeline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthStatus {
    pub newest_eod_at: DateTime<Utc>,
    pub age: Duration,
}

pub async fn healthcheck(
    pool: &PgPool,
    now_utc: DateTime<Utc>,
    max_age: Duration,
) -> Result<HealthStatus, WorkerError> {
    timeout_query(
        WorkerPhase::Health,
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
    )
    .await?;
    let newest: Option<DateTime<Utc>> = timeout_query(
        WorkerPhase::Health,
        sqlx::query_scalar(
            "SELECT max(retrieved_at) FROM data_batches \
             WHERE provider='KRX' AND market='KR' AND kind='EOD'",
        )
        .fetch_one(pool),
    )
    .await?;
    let newest_eod_at = newest.ok_or(WorkerError::Unhealthy {
        reason: HealthFailure::NoEodPublication,
    })?;
    let age = now_utc
        .signed_duration_since(newest_eod_at)
        .to_std()
        .unwrap_or(Duration::ZERO);
    if age > max_age {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::StaleEodPublication,
        });
    }
    Ok(HealthStatus { newest_eod_at, age })
}

pub fn build_postgres_pool(config: &DatabaseConfig) -> PgPool {
    let options = PgConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .database(&config.name)
        .username(&config.user)
        .password(config.password.expose())
        .application_name("lagrange-research-worker");
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::raw_sql("SET statement_timeout = '15s'; SET lock_timeout = '5s';")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect_lazy_with(options)
}

async fn timeout_query<T, F>(phase: WorkerPhase, future: F) -> Result<T, WorkerError>
where
    F: Future<Output = Result<T, sqlx::Error>>,
{
    tokio::time::timeout(QUERY_TIMEOUT, future)
        .await
        .map_err(|_| WorkerError::Timeout { phase })?
        .map_err(|source| WorkerError::Database {
            phase,
            source: SinkError::from_sqlx(source),
        })
}

impl ResearchWorker {
    pub fn new(config: ResearchWorkerConfig, backend: Arc<dyn ResearchBackend>) -> Self {
        Self { config, backend }
    }

    pub fn config(&self) -> &ResearchWorkerConfig {
        &self.config
    }

    pub async fn run_once(
        &self,
        date: TradingDate,
        control: &dyn WorkerControl,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        if !self.recover_with_retry(control).await? {
            return Ok(WorkerRunOutcome::Shutdown);
        }
        self.run_target_with_retry(date, control).await
    }

    pub async fn run_daemon(
        &self,
        control: &dyn WorkerControl,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        if !self.recover_with_retry(control).await? {
            return Ok(WorkerRunOutcome::Shutdown);
        }
        loop {
            let delay = next_run_delay(control.now_utc(), self.config.run_at_kst);
            if control.wait(Some(delay)).await == WaitOutcome::Shutdown {
                return Ok(WorkerRunOutcome::Shutdown);
            }
            let date = current_kst_date(control.now_utc());
            if self.run_target_with_retry(date, control).await? == WorkerRunOutcome::Shutdown {
                return Ok(WorkerRunOutcome::Shutdown);
            }
        }
    }

    async fn recover_with_retry(&self, control: &dyn WorkerControl) -> Result<bool, WorkerError> {
        let mut failures = 0;
        loop {
            let result = self
                .attempt_or_shutdown(control, WorkerPhase::Recovery, self.backend.recover())
                .await;
            match result {
                AttemptOutcome::Completed(Ok(())) => return Ok(true),
                AttemptOutcome::Shutdown => return Ok(false),
                AttemptOutcome::Completed(Err(error))
                    if error.failure_class() == FailureClass::Retryable =>
                {
                    if control.wait(Some(retry_delay(failures))).await == WaitOutcome::Shutdown {
                        return Ok(false);
                    }
                    failures = failures.saturating_add(1);
                }
                AttemptOutcome::Completed(Err(error)) => return Err(error),
            }
        }
    }

    async fn run_target_with_retry(
        &self,
        date: TradingDate,
        control: &dyn WorkerControl,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        let mut failures = 0;
        loop {
            let operation = async {
                if self.backend.has_eod(date).await? {
                    Ok(WorkerRunOutcome::AlreadyPublished)
                } else {
                    self.backend
                        .ingest(date, UtcTimestamp::from_datetime(control.now_utc()))
                        .await
                        .map(WorkerRunOutcome::Published)
                }
            };
            match self
                .attempt_or_shutdown(control, WorkerPhase::Ingest, operation)
                .await
            {
                AttemptOutcome::Completed(Ok(outcome)) => return Ok(outcome),
                AttemptOutcome::Shutdown => return Ok(WorkerRunOutcome::Shutdown),
                AttemptOutcome::Completed(Err(error))
                    if error.failure_class() == FailureClass::Retryable =>
                {
                    if control.wait(Some(retry_delay(failures))).await == WaitOutcome::Shutdown {
                        return Ok(WorkerRunOutcome::Shutdown);
                    }
                    failures = failures.saturating_add(1);
                }
                AttemptOutcome::Completed(Err(error)) => return Err(error),
            }
        }
    }

    async fn attempt_or_shutdown<T, F>(
        &self,
        control: &dyn WorkerControl,
        phase: WorkerPhase,
        future: F,
    ) -> AttemptOutcome<T>
    where
        T: Send,
        F: Future<Output = Result<T, WorkerError>> + Send,
    {
        tokio::select! {
            result = tokio::time::timeout(WHOLE_ATTEMPT_TIMEOUT, future) => {
                AttemptOutcome::Completed(result.unwrap_or_else(|_| Err(WorkerError::Timeout { phase })))
            }
            _ = control.wait(None) => AttemptOutcome::Shutdown,
        }
    }
}

enum AttemptOutcome<T> {
    Completed(Result<T, WorkerError>),
    Shutdown,
}

impl ResearchWorkerConfig {
    pub fn from_map(values: &HashMap<String, String>) -> Result<Self, WorkerError> {
        Self::from_map_with_reader(values, |path: &Path| std::fs::read_to_string(path))
    }

    pub fn from_map_with_reader<F>(
        values: &HashMap<String, String>,
        reader: F,
    ) -> Result<Self, WorkerError>
    where
        F: Fn(&Path) -> io::Result<String>,
    {
        let app_env = match required(values, "APP_ENV")? {
            "development" => AppEnvironment::Development,
            "qa" => AppEnvironment::Qa,
            "production" => AppEnvironment::Production,
            _ => return Err(WorkerError::InvalidConfig { key: "APP_ENV" }),
        };
        let fetch_mode = match required(values, "RESEARCH_FETCH_MODE")? {
            "synthetic" => FetchMode::Synthetic,
            "credentialed" => FetchMode::Credentialed,
            _ => {
                return Err(WorkerError::InvalidConfig {
                    key: "RESEARCH_FETCH_MODE",
                });
            }
        };

        // This policy check intentionally precedes paths, filesystem reads, and
        // construction of any provider, Raw store, or database pool.
        validate_synthetic_policy(app_env, fetch_mode)?;

        let run_at = values
            .get("RESEARCH_RUN_AT_KST")
            .map(String::as_str)
            .unwrap_or(DEFAULT_RUN_AT_KST);
        let run_at_kst =
            NaiveTime::parse_from_str(run_at, "%H:%M").map_err(|_| WorkerError::InvalidConfig {
                key: "RESEARCH_RUN_AT_KST",
            })?;
        if run_at.len() != 5 {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_RUN_AT_KST",
            });
        }

        let max_age = parse_max_age(values)?;

        let raw_root = nonempty(values, "RESEARCH_RAW_ROOT")?;
        let host = nonempty(values, "DB_HOST")?;
        let port = parse_port(values)?;
        let name = nonempty(values, "DB_NAME")?;
        let user = nonempty(values, "DB_USER")?;
        let password_file = nonempty(values, "DB_PASSWORD_FILE")?;
        let password =
            read_nonempty_secret(&reader, Path::new(&password_file), "DB_PASSWORD_FILE")?;
        let provider_credential = if fetch_mode == FetchMode::Credentialed {
            let path = nonempty(values, "KRX_CREDENTIAL_FILE")?;
            Some(read_nonempty_secret(
                &reader,
                Path::new(&path),
                "KRX_CREDENTIAL_FILE",
            )?)
        } else {
            None
        };

        Ok(Self {
            app_env,
            fetch_mode,
            run_at_kst,
            max_publication_age: max_age,
            raw_root: PathBuf::from(raw_root),
            database: DatabaseConfig {
                host,
                port,
                name,
                user,
                password,
            },
            provider_credential,
            synthetic_bundle: values
                .get("RESEARCH_SYNTHETIC_BUNDLE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("tests/fixtures/kr-etf/contract")),
        })
    }
}

impl HealthcheckConfig {
    pub fn from_map(values: &HashMap<String, String>) -> Result<Self, WorkerError> {
        Self::from_map_with_reader(values, |path: &Path| std::fs::read_to_string(path))
    }

    pub fn from_map_with_reader<F>(
        values: &HashMap<String, String>,
        reader: F,
    ) -> Result<Self, WorkerError>
    where
        F: Fn(&Path) -> io::Result<String>,
    {
        let max_publication_age = parse_max_age(values)?;
        let host = nonempty(values, "DB_HOST")?;
        let port = parse_port(values)?;
        let name = nonempty(values, "DB_NAME")?;
        let user = nonempty(values, "DB_USER")?;
        let password_file = nonempty(values, "DB_PASSWORD_FILE")?;
        let password =
            read_nonempty_secret(&reader, Path::new(&password_file), "DB_PASSWORD_FILE")?;
        Ok(Self {
            max_publication_age,
            database: DatabaseConfig {
                host,
                port,
                name,
                user,
                password,
            },
        })
    }
}

pub fn validate_synthetic_policy(
    environment: AppEnvironment,
    fetch_mode: FetchMode,
) -> Result<(), WorkerError> {
    if fetch_mode != FetchMode::Synthetic {
        return Ok(());
    }
    match environment {
        AppEnvironment::Development | AppEnvironment::Qa => Ok(()),
        AppEnvironment::Production => Err(WorkerError::SyntheticForbidden {
            environment: "production",
        }),
    }
}

pub fn retry_delay(failures: u32) -> Duration {
    let multiplier = 1_u64.checked_shl(failures.min(6)).unwrap_or(64);
    Duration::from_secs((10 * multiplier).min(600))
}

pub fn next_run_delay(now_utc: DateTime<Utc>, run_at_kst: NaiveTime) -> Duration {
    let kst = FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST offset is valid");
    let now_kst = now_utc.with_timezone(&kst);
    let mut target_date = now_kst.date_naive();
    let mut target = kst
        .from_local_datetime(&target_date.and_time(run_at_kst))
        .single()
        .expect("a fixed offset has exactly one local instant")
        .with_timezone(&Utc);
    if target < now_utc {
        target_date = target_date
            .succ_opt()
            .expect("the current civil date has a successor");
        target = kst
            .from_local_datetime(&target_date.and_time(run_at_kst))
            .single()
            .expect("a fixed offset has exactly one local instant")
            .with_timezone(&Utc);
    }
    (target - now_utc).to_std().unwrap_or(Duration::ZERO)
}

pub fn current_kst_date(now_utc: DateTime<Utc>) -> TradingDate {
    let kst = FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST offset is valid");
    TradingDate::parse(&now_utc.with_timezone(&kst).date_naive().to_string())
        .expect("a chrono civil date is a valid trading date")
}

fn required<'a>(
    values: &'a HashMap<String, String>,
    key: &'static str,
) -> Result<&'a str, WorkerError> {
    values
        .get(key)
        .map(String::as_str)
        .ok_or(WorkerError::MissingConfig { key })
}

fn nonempty(values: &HashMap<String, String>, key: &'static str) -> Result<String, WorkerError> {
    let value = required(values, key)?.trim();
    if value.is_empty() {
        Err(WorkerError::InvalidConfig { key })
    } else {
        Ok(value.to_owned())
    }
}

fn parse_max_age(values: &HashMap<String, String>) -> Result<Duration, WorkerError> {
    values
        .get("RESEARCH_MAX_PUBLICATION_AGE_SECS")
        .map_or(Some(DEFAULT_MAX_PUBLICATION_AGE_SECS), |value| {
            value.parse::<u64>().ok()
        })
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .ok_or(WorkerError::InvalidConfig {
            key: "RESEARCH_MAX_PUBLICATION_AGE_SECS",
        })
}

fn parse_port(values: &HashMap<String, String>) -> Result<u16, WorkerError> {
    required(values, "DB_PORT")?
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or(WorkerError::InvalidConfig { key: "DB_PORT" })
}

fn read_nonempty_secret<F>(
    reader: &F,
    path: &Path,
    key: &'static str,
) -> Result<SecretValue, WorkerError>
where
    F: Fn(&Path) -> io::Result<String>,
{
    let value = reader(path).map_err(|_| WorkerError::SecretFile { key })?;
    let value = value.trim();
    if value.is_empty() {
        Err(WorkerError::SecretFile { key })
    } else {
        Ok(SecretValue(value.to_owned()))
    }
}
