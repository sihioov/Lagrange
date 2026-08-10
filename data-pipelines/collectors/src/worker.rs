use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
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
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::{
    FailureClass, PipelineError, PostgresPublicationSink, PublicationSink, RecoveryBatchOutcome,
    RecoveryError, SinkError, ingest_and_publish, provider_failure_class, recover_unpublished,
    recover_unpublished_with,
};

const DEFAULT_RUN_AT_KST: &str = "16:30";
const DEFAULT_MAX_PUBLICATION_AGE_SECS: u64 = 4 * 24 * 60 * 60;
const KST_OFFSET_SECS: i32 = 9 * 60 * 60;
const WHOLE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
const CHILD_OUTPUT_LIMIT: u64 = 4096;
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(5);

pub const WORKER_ENV_KEYS: &[&str] = &[
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
    #[error("research helper process failed to start or communicate")]
    ChildIo { phase: WorkerPhase },
    #[error("research helper process could not be contained")]
    ChildContainment { phase: WorkerPhase },
    #[error("research helper process returned invalid output")]
    ChildOutput { phase: WorkerPhase },
    #[error("research worker shutdown requested")]
    Shutdown,
    #[error("research helper process reported failure")]
    ChildFailure {
        phase: WorkerPhase,
        class: FailureClass,
        batch_id: Option<BatchId>,
    },
    #[error("research worker cycle failed")]
    Cycle {
        target_date: TradingDate,
        #[source]
        source: Box<WorkerError>,
    },
    #[error("research pipeline failed")]
    Pipeline(#[source] PipelineError),
}

impl WorkerError {
    pub fn failure_class(&self) -> FailureClass {
        match self {
            Self::Io { .. } | Self::Timeout { .. } | Self::ChildIo { .. } => {
                FailureClass::Retryable
            }
            Self::Pipeline(source) => source.failure_class(),
            Self::Provider(source) => provider_failure_class(source),
            Self::Database { source, .. } => {
                if source.is_retryable() {
                    FailureClass::Retryable
                } else {
                    FailureClass::Permanent
                }
            }
            Self::ChildFailure { class, .. } => *class,
            Self::Cycle { source, .. } => source.failure_class(),
            Self::MissingConfig { .. }
            | Self::InvalidConfig { .. }
            | Self::SyntheticForbidden { .. }
            | Self::SecretFile { .. }
            | Self::ProviderNotConfigured
            | Self::Unhealthy { .. }
            | Self::ChildContainment { .. }
            | Self::ChildOutput { .. }
            | Self::Shutdown => FailureClass::Permanent,
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
            Self::ChildIo { phase }
            | Self::ChildContainment { phase }
            | Self::ChildOutput { phase } => *phase,
            Self::ChildFailure { phase, .. } => *phase,
            Self::Cycle { source, .. } => source.phase(),
            Self::Shutdown => WorkerPhase::Ingest,
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
            Self::ChildFailure { batch_id, .. } => *batch_id,
            Self::Cycle { source, .. } => source.batch_id(),
            _ => None,
        }
    }

    pub fn target_date(&self) -> Option<TradingDate> {
        match self {
            Self::Cycle { target_date, .. } => Some(*target_date),
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
    FutureEodPublication,
}

impl fmt::Display for HealthFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoEodPublication => "no KRX/KR EOD publication",
            Self::StaleEodPublication => "latest KRX/KR EOD publication is stale",
            Self::FutureEodPublication => "latest KRX/KR EOD publication is in the future",
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
    async fn recover(
        &self,
        control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError>;
    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError>;
    async fn ingest(
        &self,
        date: TradingDate,
        now: UtcTimestamp,
        control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerRunOutcome {
    AlreadyPublished,
    Published(BatchId),
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerEventKind {
    Retrying,
    Failed,
    Recovered,
    Completed,
    Skipped,
}

impl WorkerEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retrying => "retrying",
            Self::Failed => "failed",
            Self::Recovered => "recovered",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerEventClass {
    Success,
    Retryable,
    Permanent,
}

impl WorkerEventClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Retryable => "retryable",
            Self::Permanent => "permanent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerEvent {
    pub kind: WorkerEventKind,
    pub provider: &'static str,
    pub market: &'static str,
    pub target_date: Option<TradingDate>,
    pub phase: WorkerPhase,
    pub class: WorkerEventClass,
    pub batch_id: Option<BatchId>,
}

pub trait WorkerObserver: Send + Sync + 'static {
    fn emit(&self, event: WorkerEvent);
}

pub trait RecoveryObserver: Send + Sync {
    fn recovered(&self, batch_id: BatchId);
    fn skipped(&self, batch_id: BatchId);
}

struct ContextRecoveryObserver<'a> {
    observer: &'a dyn WorkerObserver,
    target_date: Option<TradingDate>,
}

impl RecoveryObserver for ContextRecoveryObserver<'_> {
    fn recovered(&self, batch_id: BatchId) {
        self.emit(WorkerEventKind::Recovered, batch_id);
    }

    fn skipped(&self, batch_id: BatchId) {
        self.emit(WorkerEventKind::Skipped, batch_id);
    }
}

impl ContextRecoveryObserver<'_> {
    fn emit(&self, kind: WorkerEventKind, batch_id: BatchId) {
        self.observer.emit(WorkerEvent {
            kind,
            provider: "KRX",
            market: "KR",
            target_date: self.target_date,
            phase: WorkerPhase::Recovery,
            class: WorkerEventClass::Success,
            batch_id: Some(batch_id),
        });
    }
}

struct NoopObserver;

impl WorkerObserver for NoopObserver {
    fn emit(&self, _event: WorkerEvent) {}
}

pub struct ResearchWorker {
    config: ResearchWorkerConfig,
    backend: Arc<dyn ResearchBackend>,
    observer: Arc<dyn WorkerObserver>,
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
    let config = ResearchWorkerConfig::from_map(values)?;
    if config.fetch_mode == FetchMode::Credentialed {
        return Err(WorkerError::ProviderNotConfigured);
    }
    let executable = std::env::current_exe().map_err(|_| WorkerError::ChildIo {
        phase: WorkerPhase::Config,
    })?;
    let system_root = validated_system_root()?;
    let pool = build_postgres_pool(&config.database);
    let backend = Arc::new(ProcessResearchBackend {
        executable,
        env: helper_environment(values, system_root.as_deref()),
        sink: PostgresPublicationSink::new(pool),
    });
    Ok(ResearchWorker::new(config, backend))
}

struct PipelineResearchBackend {
    store: RawStore,
    provider: Arc<dyn EodProvider>,
    sink: PostgresPublicationSink,
}

#[async_trait]
impl ResearchBackend for PipelineResearchBackend {
    async fn recover(
        &self,
        _control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        let report = recover_unpublished(&self.store, &self.sink)
            .await
            .map_err(WorkerError::Pipeline)?;
        for batch_id in report.recovered {
            observer.recovered(batch_id);
        }
        for batch_id in report.skipped {
            observer.skipped(batch_id);
        }
        Ok(())
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

    async fn ingest(
        &self,
        date: TradingDate,
        now: UtcTimestamp,
        _control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError> {
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

struct ProcessResearchBackend {
    executable: PathBuf,
    env: HashMap<OsString, OsString>,
    sink: PostgresPublicationSink,
}

impl ProcessResearchBackend {
    async fn helper(
        &self,
        args: Vec<OsString>,
        phase: WorkerPhase,
        expected_date: Option<TradingDate>,
        control: &dyn WorkerControl,
    ) -> Result<Option<BatchId>, WorkerError> {
        match supervise_child(
            ChildSpec {
                executable: self.executable.clone(),
                args,
                env: self.env.clone(),
            },
            WHOLE_ATTEMPT_TIMEOUT,
            phase,
            control,
        )
        .await?
        {
            SupervisedChildOutcome::TimedOut => Err(WorkerError::Timeout { phase }),
            SupervisedChildOutcome::Shutdown => Err(WorkerError::Shutdown),
            SupervisedChildOutcome::Completed { success, stdout } => {
                let decoded = decode_helper_output(&stdout, phase, expected_date);
                match (success, decoded) {
                    (true, Ok(batch_id)) => Ok(batch_id),
                    (false, Err(error @ WorkerError::ChildFailure { .. })) => Err(error),
                    _ => Err(WorkerError::ChildOutput { phase }),
                }
            }
        }
    }
}

#[async_trait]
impl ResearchBackend for ProcessResearchBackend {
    async fn recover(
        &self,
        control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        supervise_recovery_child(
            ChildSpec {
                executable: self.executable.clone(),
                args: vec![OsString::from("__research-internal-recover")],
                env: self.env.clone(),
            },
            WHOLE_ATTEMPT_TIMEOUT,
            control,
            observer,
        )
        .await
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

    async fn ingest(
        &self,
        date: TradingDate,
        now: UtcTimestamp,
        control: &dyn WorkerControl,
    ) -> Result<BatchId, WorkerError> {
        self.helper(
            vec![
                OsString::from("__research-internal-ingest"),
                OsString::from(date.to_iso()),
                OsString::from(now.to_rfc3339()),
            ],
            WorkerPhase::Ingest,
            Some(date),
            control,
        )
        .await?
        .ok_or(WorkerError::ChildOutput {
            phase: WorkerPhase::Ingest,
        })
    }
}

pub async fn run_internal_recovery(values: &HashMap<String, String>) -> Result<(), WorkerError> {
    run_internal_recovery_stream(values, &mut io::sink()).await
}

pub async fn run_internal_recovery_stream<W>(
    values: &HashMap<String, String>,
    writer: &mut W,
) -> Result<(), WorkerError>
where
    W: io::Write,
{
    let config = ResearchWorkerConfig::from_map(values)?;
    let factory = ProductionWorkerComponentFactory;
    let store = factory.build_store(&config)?;
    let pool = factory.build_pool(&config)?;
    let sink = PostgresPublicationSink::new(pool);
    recover_unpublished_with(&store, &sink, |outcome| {
        let (event, batch_id) = match outcome {
            RecoveryBatchOutcome::Recovered(batch_id) => ("recovered", batch_id),
            RecoveryBatchOutcome::Skipped(batch_id) => ("skipped", batch_id),
        };
        serde_json::to_writer(
            &mut *writer,
            &RecoveryItemWire {
                status: "event",
                event,
                phase: "recovery",
                batch_id,
            },
        )
        .map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
        writer.flush()
    })
    .await
    .map_err(|error| match error {
        RecoveryError::Pipeline(source) => WorkerError::Pipeline(source),
        RecoveryError::Observer { .. } => WorkerError::Io {
            phase: WorkerPhase::Recovery,
        },
    })
}

pub async fn run_internal_ingest(
    values: &HashMap<String, String>,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<BatchId, WorkerError> {
    let config = ResearchWorkerConfig::from_map(values)?;
    let factory = ProductionWorkerComponentFactory;
    let provider = factory.build_provider(&config)?;
    let store = factory.build_store(&config)?;
    let pool = factory.build_pool(&config)?;
    let sink = PostgresPublicationSink::new(pool);
    let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
    ingest_and_publish(&store, provider.as_ref(), &request, None, &sink)
        .await
        .map(|outcome| outcome.manifest.batch_id)
        .map_err(WorkerError::Pipeline)
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
    let age = publication_age(now_utc, newest_eod_at, max_age)
        .map_err(|reason| WorkerError::Unhealthy { reason })?;
    Ok(HealthStatus { newest_eod_at, age })
}

pub fn publication_age(
    now_utc: DateTime<Utc>,
    retrieved_at: DateTime<Utc>,
    max_age: Duration,
) -> Result<Duration, HealthFailure> {
    let age = now_utc
        .signed_duration_since(retrieved_at)
        .to_std()
        .map_err(|_| HealthFailure::FutureEodPublication)?;
    if age > max_age {
        Err(HealthFailure::StaleEodPublication)
    } else {
        Ok(age)
    }
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
        Self {
            config,
            backend,
            observer: Arc::new(NoopObserver),
        }
    }

    pub fn with_observer(mut self, observer: Arc<dyn WorkerObserver>) -> Self {
        self.observer = observer;
        self
    }

    pub fn config(&self) -> &ResearchWorkerConfig {
        &self.config
    }

    pub async fn run_once(
        &self,
        date: TradingDate,
        control: &dyn WorkerControl,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        if !self.recover_with_retry(control, Some(date)).await? {
            return Ok(WorkerRunOutcome::Shutdown);
        }
        self.run_target_with_retry(date, control).await
    }

    pub async fn run_daemon(
        &self,
        control: &dyn WorkerControl,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        if !self.recover_with_retry(control, None).await? {
            return Ok(WorkerRunOutcome::Shutdown);
        }
        loop {
            let delay = next_run_delay(control.now_utc(), self.config.run_at_kst);
            if control.wait(Some(delay)).await == WaitOutcome::Shutdown {
                return Ok(WorkerRunOutcome::Shutdown);
            }
            let date = current_kst_date(control.now_utc());
            match self.run_target_with_retry(date, control).await {
                Ok(WorkerRunOutcome::Shutdown) => return Ok(WorkerRunOutcome::Shutdown),
                Ok(_) => {}
                Err(source) => {
                    return Err(WorkerError::Cycle {
                        target_date: date,
                        source: Box::new(source),
                    });
                }
            }
        }
    }

    async fn recover_with_retry(
        &self,
        control: &dyn WorkerControl,
        target_date: Option<TradingDate>,
    ) -> Result<bool, WorkerError> {
        let mut failures = 0;
        let recovery_observer = ContextRecoveryObserver {
            observer: self.observer.as_ref(),
            target_date,
        };
        loop {
            match self.backend.recover(control, &recovery_observer).await {
                Ok(()) => return Ok(true),
                Err(WorkerError::Shutdown) => return Ok(false),
                Err(error) if error.failure_class() == FailureClass::Retryable => {
                    self.emit_retry(target_date, &error);
                    if control.wait(Some(retry_delay(failures))).await == WaitOutcome::Shutdown {
                        return Ok(false);
                    }
                    failures = failures.saturating_add(1);
                }
                Err(error) => {
                    self.emit_failure(target_date, &error);
                    return Err(error);
                }
            }
        }
    }

    async fn run_target_with_retry(
        &self,
        date: TradingDate,
        control: &dyn WorkerControl,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        let mut failures = 0;
        let mut needs_recovery = false;
        let recovery_observer = ContextRecoveryObserver {
            observer: self.observer.as_ref(),
            target_date: Some(date),
        };
        loop {
            if needs_recovery {
                match self.backend.recover(control, &recovery_observer).await {
                    Ok(()) => {
                        needs_recovery = false;
                        continue;
                    }
                    Err(WorkerError::Shutdown) => return Ok(WorkerRunOutcome::Shutdown),
                    Err(error) if error.failure_class() == FailureClass::Retryable => {
                        self.emit_retry(Some(date), &error);
                        if control.wait(Some(retry_delay(failures))).await == WaitOutcome::Shutdown
                        {
                            return Ok(WorkerRunOutcome::Shutdown);
                        }
                        failures = failures.saturating_add(1);
                        continue;
                    }
                    Err(error) => {
                        self.emit_failure(Some(date), &error);
                        return Err(error);
                    }
                }
            }

            match self
                .attempt_or_shutdown(
                    control,
                    WorkerPhase::DuplicateCheck,
                    self.backend.has_eod(date),
                )
                .await
            {
                AttemptOutcome::Completed(Ok(true)) => {
                    self.observer.emit(WorkerEvent {
                        kind: WorkerEventKind::Skipped,
                        provider: "KRX",
                        market: "KR",
                        target_date: Some(date),
                        phase: WorkerPhase::DuplicateCheck,
                        class: WorkerEventClass::Success,
                        batch_id: None,
                    });
                    return Ok(WorkerRunOutcome::AlreadyPublished);
                }
                AttemptOutcome::Completed(Ok(false)) => {}
                AttemptOutcome::Shutdown => return Ok(WorkerRunOutcome::Shutdown),
                AttemptOutcome::Completed(Err(error))
                    if error.failure_class() == FailureClass::Retryable =>
                {
                    self.emit_retry(Some(date), &error);
                    if control.wait(Some(retry_delay(failures))).await == WaitOutcome::Shutdown {
                        return Ok(WorkerRunOutcome::Shutdown);
                    }
                    failures = failures.saturating_add(1);
                    continue;
                }
                AttemptOutcome::Completed(Err(error)) => {
                    self.emit_failure(Some(date), &error);
                    return Err(error);
                }
            }

            match self
                .backend
                .ingest(
                    date,
                    UtcTimestamp::from_datetime(control.now_utc()),
                    control,
                )
                .await
            {
                Ok(batch_id) => {
                    self.observer.emit(WorkerEvent {
                        kind: WorkerEventKind::Completed,
                        provider: "KRX",
                        market: "KR",
                        target_date: Some(date),
                        phase: WorkerPhase::Publication,
                        class: WorkerEventClass::Success,
                        batch_id: Some(batch_id),
                    });
                    return Ok(WorkerRunOutcome::Published(batch_id));
                }
                Err(WorkerError::Shutdown) => return Ok(WorkerRunOutcome::Shutdown),
                Err(error) if error.failure_class() == FailureClass::Retryable => {
                    needs_recovery = true;
                    self.emit_retry(Some(date), &error);
                    if control.wait(Some(retry_delay(failures))).await == WaitOutcome::Shutdown {
                        return Ok(WorkerRunOutcome::Shutdown);
                    }
                    failures = failures.saturating_add(1);
                }
                Err(error) => {
                    self.emit_failure(Some(date), &error);
                    return Err(error);
                }
            }
        }
    }

    fn emit_retry(&self, target_date: Option<TradingDate>, error: &WorkerError) {
        self.observer.emit(WorkerEvent {
            kind: WorkerEventKind::Retrying,
            provider: "KRX",
            market: "KR",
            target_date,
            phase: error.phase(),
            class: WorkerEventClass::Retryable,
            batch_id: error.batch_id(),
        });
    }

    fn emit_failure(&self, target_date: Option<TradingDate>, error: &WorkerError) {
        self.observer.emit(WorkerEvent {
            kind: WorkerEventKind::Failed,
            provider: "KRX",
            market: "KR",
            target_date,
            phase: error.phase(),
            class: match error.failure_class() {
                FailureClass::Retryable => WorkerEventClass::Retryable,
                FailureClass::Permanent => WorkerEventClass::Permanent,
            },
            batch_id: error.batch_id(),
        });
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

#[derive(Debug)]
struct ChildSpec {
    executable: PathBuf,
    args: Vec<OsString>,
    env: HashMap<OsString, OsString>,
}

#[derive(Debug, PartialEq, Eq)]
enum SupervisedChildOutcome {
    Completed { success: bool, stdout: Vec<u8> },
    TimedOut,
    Shutdown,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperWireRecord {
    status: String,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    target_date: Option<String>,
    phase: String,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    batch_id: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    newest_eod_at: Option<String>,
    #[serde(default)]
    age_seconds: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryItemWire<'a> {
    status: &'a str,
    event: &'a str,
    phase: &'a str,
    batch_id: BatchId,
}

fn helper_environment(
    values: &HashMap<String, String>,
    system_root: Option<&Path>,
) -> HashMap<OsString, OsString> {
    let mut environment: HashMap<OsString, OsString> = WORKER_ENV_KEYS
        .iter()
        .filter_map(|key| {
            values
                .get(*key)
                .map(|value| (OsString::from(key), OsString::from(value)))
        })
        .collect();
    #[cfg(windows)]
    if let Some(system_root) = system_root {
        environment.insert(
            OsString::from("SYSTEMROOT"),
            system_root.as_os_str().to_owned(),
        );
    }
    #[cfg(not(windows))]
    let _ = system_root;
    environment
}

fn validated_system_root() -> Result<Option<PathBuf>, WorkerError> {
    #[cfg(windows)]
    {
        let path = std::env::var_os("SYSTEMROOT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(WorkerError::InvalidConfig { key: "SYSTEMROOT" })?;
        validate_system_root(path).map(Some)
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

#[cfg(windows)]
fn validate_system_root(path: PathBuf) -> Result<PathBuf, WorkerError> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(WorkerError::InvalidConfig { key: "SYSTEMROOT" });
    }
    path.canonicalize()
        .map_err(|_| WorkerError::InvalidConfig { key: "SYSTEMROOT" })
}

fn decode_helper_output(
    output: &[u8],
    default_phase: WorkerPhase,
    expected_date: Option<TradingDate>,
) -> Result<Option<BatchId>, WorkerError> {
    if output.len() as u64 > CHILD_OUTPUT_LIMIT {
        return Err(WorkerError::ChildOutput {
            phase: default_phase,
        });
    }
    let record: HelperWireRecord =
        serde_json::from_slice(output).map_err(|_| WorkerError::ChildOutput {
            phase: default_phase,
        })?;
    let batch_id = record
        .batch_id
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|_| WorkerError::ChildOutput {
            phase: default_phase,
        })?;
    let phase = parse_worker_phase(&record.phase, default_phase)?;
    match record.status.as_str() {
        "ok" => {
            if record.error_code.is_some()
                || record.provider.is_some()
                || record.market.is_some()
                || record.target_date.is_some()
                || record.class.is_some()
                || record.message.is_some()
                || record.newest_eod_at.is_some()
                || record.age_seconds.is_some()
            {
                return Err(WorkerError::ChildOutput {
                    phase: default_phase,
                });
            }
            match default_phase {
                WorkerPhase::Recovery
                    if phase == WorkerPhase::Recovery
                        && record.outcome.as_deref() == Some("recovered")
                        && batch_id.is_none()
                        && record.date.is_none() =>
                {
                    Ok(None)
                }
                WorkerPhase::Ingest
                    if phase == WorkerPhase::Publication
                        && record.outcome.as_deref() == Some("published")
                        && batch_id.is_some()
                        && record
                            .date
                            .as_deref()
                            .and_then(|date| TradingDate::parse(date).ok())
                            == expected_date =>
                {
                    Ok(batch_id)
                }
                _ => Err(WorkerError::ChildOutput {
                    phase: default_phase,
                }),
            }
        }
        "error" => {
            if !record
                .error_code
                .as_deref()
                .is_some_and(|code| !code.is_empty())
                || !record
                    .message
                    .as_deref()
                    .is_some_and(|message| !message.is_empty())
                || record.outcome.is_some()
                || record.date.is_some()
                || record.newest_eod_at.is_some()
                || record.age_seconds.is_some()
                || record.provider.as_deref() != Some("KRX")
                || record.market.as_deref() != Some("KR")
                || record
                    .target_date
                    .as_deref()
                    .map(TradingDate::parse)
                    .transpose()
                    .map_err(|_| WorkerError::ChildOutput {
                        phase: default_phase,
                    })?
                    != expected_date
            {
                return Err(WorkerError::ChildOutput {
                    phase: default_phase,
                });
            }
            let class = match record.class.as_deref() {
                Some("retryable") => FailureClass::Retryable,
                Some("permanent") => FailureClass::Permanent,
                _ => {
                    return Err(WorkerError::ChildOutput {
                        phase: default_phase,
                    });
                }
            };
            Err(WorkerError::ChildFailure {
                phase,
                class,
                batch_id,
            })
        }
        _ => Err(WorkerError::ChildOutput {
            phase: default_phase,
        }),
    }
}

fn parse_worker_phase(
    value: &str,
    invoking_phase: WorkerPhase,
) -> Result<WorkerPhase, WorkerError> {
    match value {
        "config" => Ok(WorkerPhase::Config),
        "provider" => Ok(WorkerPhase::Provider),
        "recovery" => Ok(WorkerPhase::Recovery),
        "duplicate_check" => Ok(WorkerPhase::DuplicateCheck),
        "ingest" => Ok(WorkerPhase::Ingest),
        "publication" => Ok(WorkerPhase::Publication),
        "health" => Ok(WorkerPhase::Health),
        "database" => Ok(WorkerPhase::Database),
        _ => Err(WorkerError::ChildOutput {
            phase: invoking_phase,
        }),
    }
}

enum RecoveryLine {
    Batch(RecoveryBatchOutcome),
    Terminal(Result<(), WorkerError>),
}

fn decode_recovery_line(line: &[u8]) -> Result<RecoveryLine, WorkerError> {
    #[derive(serde::Deserialize)]
    struct Status<'a> {
        status: &'a str,
    }

    let phase = WorkerPhase::Recovery;
    let status: Status<'_> =
        serde_json::from_slice(line).map_err(|_| WorkerError::ChildOutput { phase })?;
    if status.status == "event" {
        let record: RecoveryItemWire<'_> =
            serde_json::from_slice(line).map_err(|_| WorkerError::ChildOutput { phase })?;
        if record.status != "event" || record.phase != "recovery" {
            return Err(WorkerError::ChildOutput { phase });
        }
        let outcome = match record.event {
            "recovered" => RecoveryBatchOutcome::Recovered(record.batch_id),
            "skipped" => RecoveryBatchOutcome::Skipped(record.batch_id),
            _ => return Err(WorkerError::ChildOutput { phase }),
        };
        Ok(RecoveryLine::Batch(outcome))
    } else {
        match decode_helper_output(line, phase, None) {
            Ok(None) => Ok(RecoveryLine::Terminal(Ok(()))),
            Ok(Some(_)) => Err(WorkerError::ChildOutput { phase }),
            Err(error @ WorkerError::ChildFailure { .. }) => Ok(RecoveryLine::Terminal(Err(error))),
            Err(error) => Err(error),
        }
    }
}

async fn read_bounded_line<R>(reader: &mut R) -> Result<Option<Vec<u8>>, WorkerError>
where
    R: AsyncBufRead + Unpin,
{
    let phase = WorkerPhase::Recovery;
    let mut line = Vec::new();
    let read = (&mut *reader)
        .take(CHILD_OUTPUT_LIMIT + 2)
        .read_until(b'\n', &mut line)
        .await
        .map_err(|_| WorkerError::ChildIo { phase })?;
    if read == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return Err(WorkerError::ChildOutput { phase });
    }
    line.pop();
    if line.len() as u64 > CHILD_OUTPUT_LIMIT {
        return Err(WorkerError::ChildOutput { phase });
    }
    Ok(Some(line))
}

async fn supervise_recovery_child(
    spec: ChildSpec,
    timeout: Duration,
    control: &dyn WorkerControl,
    observer: &dyn RecoveryObserver,
) -> Result<(), WorkerError> {
    let phase = WorkerPhase::Recovery;
    let mut child = Command::new(&spec.executable)
        .args(&spec.args)
        .env_clear()
        .envs(spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| WorkerError::ChildIo { phase })?;
    let stdout = child.stdout.take().ok_or(WorkerError::ChildIo { phase })?;
    let mut reader = BufReader::new(stdout);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut terminal = None;
    let mut stdout_eof = false;
    let mut exit_success = None;

    while !stdout_eof || exit_success.is_none() {
        tokio::select! {
            biased;
            line = read_bounded_line(&mut reader), if !stdout_eof => {
                match line {
                    Ok(Some(line)) => {
                        if terminal.is_some() {
                            terminate_and_reap(&mut child, phase).await?;
                            return Err(WorkerError::ChildOutput { phase });
                        }
                        match decode_recovery_line(&line) {
                            Ok(RecoveryLine::Batch(RecoveryBatchOutcome::Recovered(batch_id))) => {
                                observer.recovered(batch_id);
                            }
                            Ok(RecoveryLine::Batch(RecoveryBatchOutcome::Skipped(batch_id))) => {
                                observer.skipped(batch_id);
                            }
                            Ok(RecoveryLine::Terminal(result)) => terminal = Some(result),
                            Err(error) => {
                                terminate_and_reap(&mut child, phase).await?;
                                return Err(error);
                            }
                        }
                    }
                    Ok(None) => stdout_eof = true,
                    Err(error) => {
                        terminate_and_reap(&mut child, phase).await?;
                        return Err(error);
                    }
                }
            }
            status = child.wait(), if exit_success.is_none() => {
                exit_success = Some(status.map_err(|_| WorkerError::ChildIo { phase })?.success());
            }
            _ = &mut deadline => {
                terminate_and_reap(&mut child, phase).await?;
                return Err(WorkerError::Timeout { phase });
            }
            _ = control.wait(None) => {
                terminate_and_reap(&mut child, phase).await?;
                return Err(WorkerError::Shutdown);
            }
        }
    }

    match (exit_success, terminal) {
        (Some(true), Some(Ok(()))) => Ok(()),
        (Some(false), Some(Err(error @ WorkerError::ChildFailure { .. }))) => Err(error),
        (Some(false), None) => Err(WorkerError::ChildIo { phase }),
        _ => Err(WorkerError::ChildOutput { phase }),
    }
}

async fn supervise_child(
    spec: ChildSpec,
    timeout: Duration,
    phase: WorkerPhase,
    control: &dyn WorkerControl,
) -> Result<SupervisedChildOutcome, WorkerError> {
    let mut child = Command::new(&spec.executable)
        .args(&spec.args)
        .env_clear()
        .envs(spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| WorkerError::ChildIo { phase })?;
    let stdout = child.stdout.take().ok_or(WorkerError::ChildIo { phase })?;
    let mut reader = tokio::spawn(async move {
        let mut output = Vec::new();
        stdout
            .take(CHILD_OUTPUT_LIMIT + 1)
            .read_to_end(&mut output)
            .await
            .map(|_| output)
    });
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut finished_output = None;

    loop {
        tokio::select! {
            status = child.wait() => {
                let status = status.map_err(|_| WorkerError::ChildIo { phase })?;
                let output = match finished_output.take() {
                    Some(output) => output,
                    None => reader.await.map_err(|_| WorkerError::ChildIo { phase })?
                        .map_err(|_| WorkerError::ChildIo { phase })?,
                };
                if output.len() as u64 > CHILD_OUTPUT_LIMIT {
                    return Err(WorkerError::ChildOutput { phase });
                }
                return Ok(SupervisedChildOutcome::Completed {
                    success: status.success(),
                    stdout: output,
                });
            }
            read = &mut reader, if finished_output.is_none() => {
                let output = read
                    .map_err(|_| WorkerError::ChildIo { phase })?
                    .map_err(|_| WorkerError::ChildIo { phase })?;
                if output.len() as u64 > CHILD_OUTPUT_LIMIT {
                    terminate_and_reap(&mut child, phase).await?;
                    return Err(WorkerError::ChildOutput { phase });
                }
                finished_output = Some(output);
            }
            _ = &mut deadline => {
                terminate_and_reap(&mut child, phase).await?;
                if finished_output.is_none() {
                    let _ = reader.await;
                }
                return Ok(SupervisedChildOutcome::TimedOut);
            }
            _ = control.wait(None) => {
                terminate_and_reap(&mut child, phase).await?;
                if finished_output.is_none() {
                    let _ = reader.await;
                }
                return Ok(SupervisedChildOutcome::Shutdown);
            }
        }
    }
}

async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    phase: WorkerPhase,
) -> Result<(), WorkerError> {
    match child.start_kill() {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
        Err(_) => return Err(WorkerError::ChildContainment { phase }),
    }
    tokio::time::timeout(CHILD_REAP_TIMEOUT, child.wait())
        .await
        .map_err(|_| WorkerError::ChildContainment { phase })?
        .map_err(|_| WorkerError::ChildContainment { phase })?;
    Ok(())
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

#[cfg(test)]
mod process_tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::{
        ChildSpec, RecoveryObserver, SupervisedChildOutcome, WaitOutcome, WorkerControl,
        WorkerPhase, decode_helper_output, decode_recovery_line, helper_environment,
        supervise_child, supervise_recovery_child,
    };

    #[derive(Default)]
    struct RecoveryBatches(Mutex<Vec<domain::BatchId>>);

    impl RecoveryObserver for RecoveryBatches {
        fn recovered(&self, batch_id: domain::BatchId) {
            self.0.lock().unwrap().push(batch_id);
        }

        fn skipped(&self, batch_id: domain::BatchId) {
            self.0.lock().unwrap().push(batch_id);
        }
    }

    struct NeverShutdown;

    #[async_trait]
    impl WorkerControl for NeverShutdown {
        fn now_utc(&self) -> chrono::DateTime<Utc> {
            Utc::now()
        }

        async fn wait(&self, _duration: Option<Duration>) -> WaitOutcome {
            std::future::pending().await
        }
    }

    struct ShutdownSoon;

    #[async_trait]
    impl WorkerControl for ShutdownSoon {
        fn now_utc(&self) -> chrono::DateTime<Utc> {
            Utc::now()
        }

        async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
            if duration.is_none() {
                tokio::time::sleep(Duration::from_millis(100)).await;
                WaitOutcome::Shutdown
            } else {
                WaitOutcome::Elapsed
            }
        }
    }

    fn blocking_child_spec(heartbeat: PathBuf) -> ChildSpec {
        ChildSpec {
            executable: std::env::current_exe().expect("test executable"),
            args: vec![
                OsString::from("--exact"),
                OsString::from("worker::process_tests::blocking_child"),
                OsString::from("--ignored"),
                OsString::from("--nocapture"),
            ],
            env: HashMap::from([
                (
                    OsString::from("RESEARCH_TEST_BLOCK_CHILD"),
                    OsString::from("1"),
                ),
                (
                    OsString::from("RESEARCH_TEST_HEARTBEAT"),
                    heartbeat.into_os_string(),
                ),
            ]),
        }
    }

    fn oversized_child_spec(heartbeat: PathBuf) -> ChildSpec {
        ChildSpec {
            executable: std::env::current_exe().expect("test executable"),
            args: vec![
                OsString::from("--exact"),
                OsString::from("worker::process_tests::oversized_child"),
                OsString::from("--ignored"),
                OsString::from("--nocapture"),
            ],
            env: HashMap::from([(
                OsString::from("RESEARCH_TEST_HEARTBEAT"),
                heartbeat.into_os_string(),
            )]),
        }
    }

    #[cfg(windows)]
    fn recovery_protocol_child_spec(heartbeat: PathBuf, case: &str) -> ChildSpec {
        let script = r#"
$event = '{"status":"event","event":"recovered","phase":"recovery","batch_id":"00000000-0000-4000-8000-000000000001"}'
[IO.File]::AppendAllText($env:RESEARCH_TEST_HEARTBEAT, 'x')
[Console]::Out.WriteLine($event)
if ($env:RESEARCH_TEST_CASE -eq 'oversized') {
  [Console]::Out.WriteLine(('x' * 4097))
} else {
  [Console]::Out.WriteLine('{"status":"ok","phase":"recovery","outcome":"recovered","batch_id":null,"date":null,"newest_eod_at":null,"age_seconds":null}')
  [Console]::Out.WriteLine('{"unexpected":true}')
}
[Console]::Out.Flush()
while ($true) {
  [IO.File]::AppendAllText($env:RESEARCH_TEST_HEARTBEAT, 'x')
  Start-Sleep -Milliseconds 10
}
"#;
        ChildSpec {
            executable: PathBuf::from("powershell.exe"),
            args: vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-Command"),
                OsString::from(script),
            ],
            env: HashMap::from([
                (OsString::from("RESEARCH_TEST_CASE"), OsString::from(case)),
                (
                    OsString::from("RESEARCH_TEST_HEARTBEAT"),
                    heartbeat.into_os_string(),
                ),
                (
                    OsString::from("SYSTEMROOT"),
                    std::env::var_os("SYSTEMROOT").unwrap(),
                ),
            ]),
        }
    }

    #[cfg(unix)]
    fn recovery_protocol_child_spec(heartbeat: PathBuf, case: &str) -> ChildSpec {
        let script = r#"
printf x >> "$RESEARCH_TEST_HEARTBEAT"
printf '%s\n' '{"status":"event","event":"recovered","phase":"recovery","batch_id":"00000000-0000-4000-8000-000000000001"}'
if [ "$RESEARCH_TEST_CASE" = oversized ]; then
  printf '%04097d\n' 0 | tr '0' x
else
  printf '%s\n' '{"status":"ok","phase":"recovery","outcome":"recovered","batch_id":null,"date":null,"newest_eod_at":null,"age_seconds":null}'
  printf '%s\n' '{"unexpected":true}'
fi
while true; do printf x >> "$RESEARCH_TEST_HEARTBEAT"; sleep 0.01; done
"#;
        ChildSpec {
            executable: PathBuf::from("/bin/sh"),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: HashMap::from([
                (OsString::from("RESEARCH_TEST_CASE"), OsString::from(case)),
                (
                    OsString::from("RESEARCH_TEST_HEARTBEAT"),
                    heartbeat.into_os_string(),
                ),
            ]),
        }
    }

    #[test]
    fn helper_environment_is_an_explicit_allowlist() {
        let system_root = tempfile::tempdir().unwrap();
        let values = HashMap::from([
            ("APP_ENV".to_owned(), "qa".to_owned()),
            ("DB_HOST".to_owned(), "db".to_owned()),
            ("DATABASE_URL".to_owned(), "must-not-cross".to_owned()),
            (
                "AWS_SECRET_ACCESS_KEY".to_owned(),
                "must-not-cross".to_owned(),
            ),
        ]);
        let env = helper_environment(&values, Some(system_root.path()));
        assert_eq!(
            env.get(&OsString::from("APP_ENV")),
            Some(&OsString::from("qa"))
        );
        assert_eq!(
            env.get(&OsString::from("DB_HOST")),
            Some(&OsString::from("db"))
        );
        assert!(!env.contains_key(&OsString::from("DATABASE_URL")));
        assert!(!env.contains_key(&OsString::from("AWS_SECRET_ACCESS_KEY")));
        #[cfg(windows)]
        assert_eq!(
            env.get(&OsString::from("SYSTEMROOT")),
            Some(&system_root.path().as_os_str().to_owned())
        );
        #[cfg(not(windows))]
        assert!(!env.contains_key(&OsString::from("SYSTEMROOT")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_root_validation_requires_an_absolute_existing_directory() {
        let existing = tempfile::tempdir().unwrap();
        let canonical = existing.path().canonicalize().unwrap();
        assert_eq!(
            super::validate_system_root(existing.path().to_path_buf()).unwrap(),
            canonical
        );
        assert!(super::validate_system_root(PathBuf::from("relative")).is_err());
        assert!(super::validate_system_root(existing.path().join("missing")).is_err());
    }

    #[test]
    fn helper_output_is_one_bounded_sanitized_record() {
        let batch_id = domain::BatchId::generate();
        let error = decode_helper_output(
            format!(
                "{{\"status\":\"error\",\"error_code\":\"DATABASE_UNAVAILABLE\",\"provider\":\"KRX\",\"market\":\"KR\",\"target_date\":null,\"phase\":\"publication\",\"class\":\"retryable\",\"batch_id\":\"{batch_id}\",\"message\":\"research pipeline failed\"}}"
            )
            .as_bytes(),
            WorkerPhase::Ingest,
            None,
        )
        .unwrap_err();
        assert_eq!(error.failure_class(), crate::FailureClass::Retryable);
        assert_eq!(error.batch_id(), Some(batch_id));
        assert!(matches!(
            decode_helper_output(b"not-json", WorkerPhase::Recovery, None),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
        assert!(matches!(
            decode_helper_output(&vec![b'x'; 4097], WorkerPhase::Recovery, None),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
        assert!(matches!(
            decode_helper_output(
                b"{\"status\":\"ok\",\"phase\":\"recovery\",\"outcome\":\"recovered\",\"batch_id\":null,\"date\":null,\"newest_eod_at\":null,\"age_seconds\":null,\"unexpected\":true}",
                WorkerPhase::Recovery,
                None,
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
        assert!(matches!(
            decode_recovery_line(
                b"{\"status\":\"event\",\"event\":\"recovered\",\"phase\":\"recovery\",\"batch_id\":\"00000000-0000-4000-8000-000000000001\",\"unknown\":true}"
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
    }

    #[test]
    fn helper_failures_preserve_phase_and_ingest_success_requires_exact_date() {
        for phase in [WorkerPhase::Recovery, WorkerPhase::Ingest] {
            for error in [
                super::WorkerError::ChildIo { phase },
                super::WorkerError::ChildContainment { phase },
                super::WorkerError::ChildOutput { phase },
            ] {
                assert_eq!(error.phase(), phase);
            }
        }

        let expected = domain::TradingDate::parse("2020-01-31").unwrap();
        let wrong = domain::TradingDate::parse("2020-02-03").unwrap();
        let batch_id = domain::BatchId::generate();
        let output = format!(
            "{{\"status\":\"ok\",\"phase\":\"publication\",\"outcome\":\"published\",\"batch_id\":\"{batch_id}\",\"date\":\"{}\",\"newest_eod_at\":null,\"age_seconds\":null}}",
            wrong.to_iso()
        );
        assert!(matches!(
            decode_helper_output(output.as_bytes(), WorkerPhase::Ingest, Some(expected),),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Ingest
            })
        ));
    }

    async fn assert_heartbeat_stops(path: &std::path::Path) {
        let at_return = std::fs::metadata(path)
            .expect("blocking child created heartbeat")
            .len();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let later = std::fs::metadata(path).unwrap().len();
        assert_eq!(at_return, later, "contained child must no longer execute");
    }

    #[tokio::test]
    async fn blocked_child_is_killed_and_reaped_before_timeout_returns() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = dir.path().join("timeout-heartbeat");
        let started = Instant::now();
        let outcome = supervise_child(
            blocking_child_spec(heartbeat.clone()),
            Duration::from_millis(250),
            WorkerPhase::Recovery,
            &NeverShutdown,
        )
        .await
        .unwrap();

        assert_eq!(outcome, SupervisedChildOutcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_heartbeat_stops(&heartbeat).await;
    }

    #[tokio::test]
    async fn blocked_child_is_killed_and_reaped_before_shutdown_returns() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = dir.path().join("shutdown-heartbeat");
        let started = Instant::now();
        let outcome = supervise_child(
            blocking_child_spec(heartbeat.clone()),
            Duration::from_secs(5),
            WorkerPhase::Ingest,
            &ShutdownSoon,
        )
        .await
        .unwrap();

        assert_eq!(outcome, SupervisedChildOutcome::Shutdown);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_heartbeat_stops(&heartbeat).await;
    }

    #[tokio::test]
    async fn oversized_stdout_is_permanent_and_contained_before_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = dir.path().join("oversized-heartbeat");
        let started = Instant::now();
        let error = supervise_child(
            oversized_child_spec(heartbeat.clone()),
            Duration::from_secs(5),
            WorkerPhase::Recovery,
            &NeverShutdown,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            }
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_heartbeat_stops(&heartbeat).await;
    }

    #[tokio::test]
    async fn recovery_stream_rejects_oversized_and_post_terminal_records_after_reap() {
        for case in ["oversized", "trailing"] {
            let dir = tempfile::tempdir().unwrap();
            let heartbeat = dir.path().join(format!("{case}-heartbeat"));
            let observer = RecoveryBatches::default();
            let started = Instant::now();
            let error = supervise_recovery_child(
                recovery_protocol_child_spec(heartbeat.clone(), case),
                Duration::from_secs(5),
                &NeverShutdown,
                &observer,
            )
            .await
            .unwrap_err();

            assert!(matches!(
                error,
                super::WorkerError::ChildOutput {
                    phase: WorkerPhase::Recovery
                }
            ));
            assert_eq!(
                observer.0.lock().unwrap().len(),
                1,
                "the one valid pre-failure record is delivered"
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_heartbeat_stops(&heartbeat).await;
        }
    }

    #[tokio::test]
    async fn helper_spawn_errors_retain_both_invoking_phases() {
        for phase in [WorkerPhase::Recovery, WorkerPhase::Ingest] {
            let error = supervise_child(
                ChildSpec {
                    executable: PathBuf::from("definitely-missing-research-helper"),
                    args: Vec::new(),
                    env: HashMap::new(),
                },
                Duration::from_secs(1),
                phase,
                &NeverShutdown,
            )
            .await
            .unwrap_err();
            assert!(matches!(
                error,
                super::WorkerError::ChildIo { phase: actual } if actual == phase
            ));
        }
    }

    #[test]
    #[ignore = "invoked only as a subprocess by supervisor tests"]
    fn blocking_child() {
        assert_eq!(
            std::env::var("RESEARCH_TEST_BLOCK_CHILD").as_deref(),
            Ok("1")
        );
        let heartbeat = std::env::var_os("RESEARCH_TEST_HEARTBEAT").expect("heartbeat path");
        loop {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&heartbeat)
                .unwrap();
            file.write_all(b"x").unwrap();
            file.sync_all().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "invoked only as a subprocess by supervisor tests"]
    fn oversized_child() {
        use std::io::Write as _;
        let heartbeat = std::env::var_os("RESEARCH_TEST_HEARTBEAT").expect("heartbeat path");
        std::fs::write(&heartbeat, b"started").unwrap();
        std::io::stdout().write_all(&vec![b'x'; 5000]).unwrap();
        std::io::stdout().flush().unwrap();
        loop {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&heartbeat)
                .unwrap();
            file.write_all(b"x").unwrap();
            file.sync_all().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
