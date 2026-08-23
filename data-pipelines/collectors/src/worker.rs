use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, FixedOffset, NaiveTime, TimeZone, Utc};
use domain::{BatchId, DatasetId, TradingDate, UtcTimestamp};
use kis_client::clock::Clock;
use kis_client::live_transport::LiveTransport;
use kis_client::secret::SystemCredentialSource;
use kis_client::token_issuer::KisTokenIssuer;
use kis_client::{
    BucketKey, CredentialRef, KisError, KisMarketDataClient, Quota, RateLimiter, SystemClock,
    TokenManager, TokioSleeper,
};
use market_data::contract::{
    FetchMode, MARKET_KR, PROVIDER_KIS_DAILY_RANGE, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX,
    ResponseKind,
};
use market_data::ingest::{IngestRequest, ingest_kis_daily_bars_range_with_batch_id};
use market_data::normalize::NormalizeError;
use market_data::provider::{EodProvider, KrxProvider, ProviderError, RecordedBundle};
use market_data::providers::kis::{KR_ETF_CORE_SYMBOLS, KisProvider};
use market_data::range_normalize::{
    ExpectedRangeSessions, RangeNormalizeError, normalize_kis_daily_range_batch,
};
use market_data::storage::{ManifestEntry, RawStore, StoreError};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CurateError, CurateRequest, CurateStore, curate_generation,
    curation_inputs_from_raw, curation_inputs_from_raw_entries, ingest_bundle_with_kinds,
    price_curation_evidence_for_generation,
};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::{
    CandidateInstrumentCatalog, CandidatePipelineError, CandidatePricePublication, FailureClass,
    KisNormalizationRecoveryReport, PipelineError, PostgresCandidateSourceSink,
    PostgresPublicationSink, RECOVERY_PAGE_SIZE, RecoveryBatchOutcome, RecoveryError, RecoveryPage,
    RecoveryPosition, RecoveryScope, SinkError, ingest_and_publish, ingest_normalize_publish_kis,
    prepare_candidate_batch, provider_failure_class, publish_candidate_batch,
    recover_candidate_batches, recover_kis_normalization, recover_unpublished_normalized_for_date,
    recover_unpublished_page_with_scope, recover_unpublished_with, store_failure_class,
};

const DEFAULT_RUN_AT_KST: &str = "16:30";
const CANDIDATE_CONFIRMED_CLOSE_KST: NaiveTime =
    NaiveTime::from_hms_opt(16, 30, 0).expect("valid candidate close threshold");
const DEFAULT_MAX_PUBLICATION_AGE_SECS: u64 = 4 * 24 * 60 * 60;
const KST_OFFSET_SECS: i32 = 9 * 60 * 60;
const DEFAULT_ATTEMPT_TIMEOUT_SECS: u64 = 15 * 60;
const MAX_ATTEMPT_TIMEOUT_SECS: u64 = 60 * 60;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
const CHILD_OUTPUT_LIMIT: u64 = 4096;
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(5);
const KIS_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const DAILY_RANGE_ENDPOINT: &str =
    "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_RANGE_TR_ID: &str = "FHKST03010100";
const KIS_READ_QUOTA: Quota = Quota {
    capacity: 1,
    refill_per_sec: 1,
};
const KIS_READ_CHANNELS: [(&str, &str); 9] = [
    (
        "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice",
        "FHKST03010100",
    ),
    (
        "/uapi/domestic-stock/v1/quotations/inquire-price",
        "FHKST01010100",
    ),
    (
        "/uapi/domestic-stock/v1/quotations/chk-holiday",
        "CTCA0903R",
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        "HHKDB669100C0",
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
        "HHKDB669101C0",
    ),
    ("/uapi/domestic-stock/v1/ksdinfo/dividend", "HHKDB669102C0"),
    (
        "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        "HHKDB669104C0",
    ),
    ("/uapi/domestic-stock/v1/ksdinfo/rev-split", "HHKDB669105C0"),
    ("/uapi/domestic-stock/v1/ksdinfo/cap-dcrs", "HHKDB669106C0"),
];
const WORKER_PROVIDER_KIS_NORMALIZED: &str = "KIS-NORMALIZED";

fn worker_event_provider(mode: FetchMode) -> &'static str {
    match mode {
        FetchMode::Synthetic => "KRX",
        FetchMode::Credentialed => WORKER_PROVIDER_KIS_NORMALIZED,
    }
}

pub const WORKER_ENV_KEYS: &[&str] = &[
    "APP_ENV",
    "RESEARCH_FETCH_MODE",
    "RESEARCH_RUN_AT_KST",
    "RESEARCH_MAX_PUBLICATION_AGE_SECS",
    "RESEARCH_ATTEMPT_TIMEOUT_SECS",
    "RESEARCH_RAW_ROOT",
    "RESEARCH_CURATED_ROOT",
    "RESEARCH_ENTITLEMENT_REFERENCE",
    "RESEARCH_SYNTHETIC_BUNDLE",
    "RESEARCH_CANDIDATE_ENABLED",
    "RESEARCH_CANDIDATE_RAW_ROOT",
    "RESEARCH_CANDIDATE_SYNTHETIC_BUNDLE",
    "DB_HOST",
    "DB_PORT",
    "DB_NAME",
    "DB_USER",
    "DB_PASSWORD_FILE",
    "KIS_APP_KEY_FILE",
    "KIS_APP_SECRET_FILE",
    "LAGRANGE_CODE_COMMIT",
    "RANGE_RAW_BATCH_ID",
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
    pub attempt_timeout: Duration,
    pub raw_root: PathBuf,
    pub curated_root: PathBuf,
    pub entitlement_reference: String,
    pub database: DatabaseConfig,
    pub kis_app_key_file: Option<PathBuf>,
    pub kis_app_secret_file: Option<PathBuf>,
    pub synthetic_bundle: PathBuf,
    pub candidate_sources_enabled: bool,
    pub candidate_raw_root: PathBuf,
    pub candidate_synthetic_bundle: PathBuf,
}

#[derive(Debug, Clone)]
pub struct HealthcheckConfig {
    pub max_publication_age: Duration,
    pub database: DatabaseConfig,
    pub candidate_sources_enabled: bool,
    pub curated_root: PathBuf,
    pub expected_fetch_mode: FetchMode,
    pub run_at_kst: NaiveTime,
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
    #[error("KIS client construction failed")]
    KisClient(#[source] KisError),
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
        error_code: String,
        endpoint: Option<String>,
        http_status: Option<u16>,
        response_context: Option<Box<ChildResponseContext>>,
    },
    #[error("research worker cycle failed")]
    Cycle {
        target_date: TradingDate,
        #[source]
        source: Box<WorkerError>,
    },
    #[error("research pipeline failed")]
    Pipeline(#[source] PipelineError),
    #[error("KIS daily range normalization failed")]
    RangeNormalize(#[source] RangeNormalizeError),
    #[error("candidate source pipeline failed")]
    CandidatePipeline(#[source] CandidatePipelineError),
    #[error("price curation failed")]
    Curation(#[source] CurateError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildResponseContext {
    pub response_kind: String,
    pub file_name: String,
}

impl WorkerError {
    pub fn failure_class(&self) -> FailureClass {
        match self {
            Self::Io { .. } | Self::Timeout { .. } | Self::ChildIo { .. } => {
                FailureClass::Retryable
            }
            Self::Pipeline(source) => source.failure_class(),
            Self::RangeNormalize(source) => match source {
                RangeNormalizeError::Store(error) => store_failure_class(error),
                _ => FailureClass::Permanent,
            },
            Self::CandidatePipeline(CandidatePipelineError::Publish(source)) => {
                if source.is_retryable() {
                    FailureClass::Retryable
                } else {
                    FailureClass::Permanent
                }
            }
            Self::CandidatePipeline(
                CandidatePipelineError::InvalidRaw(_) | CandidatePipelineError::InvalidDocument(_),
            ) => FailureClass::Permanent,
            Self::CandidatePipeline(CandidatePipelineError::Ingest(source)) => match source {
                market_data::IngestError::Provider(source) => provider_failure_class(source),
                market_data::IngestError::Store(source)
                | market_data::IngestError::Readback { source, .. } => store_failure_class(source),
                market_data::IngestError::MalformedResponse { .. }
                | market_data::IngestError::ResponseShape { .. } => FailureClass::Permanent,
            },
            Self::Curation(source) => match source {
                CurateError::StoreIo { .. } => FailureClass::Retryable,
                CurateError::RawStore { source, .. } => store_failure_class(source),
                _ => FailureClass::Permanent,
            },
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
            | Self::KisClient(_)
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
            Self::ProviderNotConfigured | Self::Provider(_) | Self::KisClient(_) => {
                WorkerPhase::Provider
            }
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
            Self::RangeNormalize(_) => WorkerPhase::Ingest,
            Self::CandidatePipeline(_) | Self::Curation(_) => WorkerPhase::Publication,
        }
    }

    pub fn batch_id(&self) -> Option<BatchId> {
        match self {
            Self::Pipeline(source) => source.batch_id(),
            Self::ChildFailure { batch_id, .. } => *batch_id,
            Self::Cycle { source, .. } => source.batch_id(),
            Self::RangeNormalize(RangeNormalizeError::ExistingBatchConflict {
                batch_id, ..
            }) => Some(*batch_id),
            _ => None,
        }
    }

    pub fn target_date(&self) -> Option<TradingDate> {
        match self {
            Self::Cycle { target_date, .. } => Some(*target_date),
            _ => None,
        }
    }

    /// Safe, structured provider metadata suitable for an operator event.
    /// Free-form provider details and response bodies are intentionally absent.
    pub fn safe_diagnostic(&self) -> Option<WorkerDiagnostic<'_>> {
        match self {
            Self::KisClient(source) => Some(WorkerDiagnostic {
                error_code: source.code(),
                endpoint: kis_error_endpoint(source),
                http_status: kis_error_http_status(source),
                response_kind: None,
                file_name: None,
            }),
            Self::Provider(source) => provider_diagnostic(source),
            Self::Pipeline(PipelineError::Ingest {
                source: market_data::IngestError::Provider(source),
            })
            | Self::CandidatePipeline(CandidatePipelineError::Ingest(
                market_data::IngestError::Provider(source),
            )) => provider_diagnostic(source),
            Self::Pipeline(PipelineError::Ingest {
                source:
                    market_data::IngestError::MalformedResponse {
                        kind,
                        diagnostic: Some(diagnostic),
                        ..
                    },
            }) => Some(WorkerDiagnostic {
                error_code: diagnostic.code,
                endpoint: Some(&diagnostic.endpoint),
                http_status: None,
                response_kind: Some(kind.as_str()),
                file_name: Some(&diagnostic.file_name),
            }),
            Self::Pipeline(PipelineError::Normalize { source, .. }) => normalize_diagnostic(source),
            Self::RangeNormalize(source) => Some(range_normalize_diagnostic(source)),
            Self::ChildFailure {
                error_code,
                endpoint,
                http_status,
                response_context,
                ..
            } => Some(WorkerDiagnostic {
                error_code,
                endpoint: endpoint.as_deref(),
                http_status: *http_status,
                response_kind: response_context
                    .as_deref()
                    .map(|context| context.response_kind.as_str()),
                file_name: response_context
                    .as_deref()
                    .map(|context| context.file_name.as_str()),
            }),
            Self::Cycle { source, .. } => source.safe_diagnostic(),
            _ => None,
        }
    }
}

fn range_normalize_diagnostic(error: &RangeNormalizeError) -> WorkerDiagnostic<'static> {
    let error_code = match error {
        RangeNormalizeError::Store(source) => normalize_store_error_code(source),
        RangeNormalizeError::UnsupportedScope { .. } => "KIS_RANGE_UNSUPPORTED_SCOPE",
        RangeNormalizeError::UnsupportedMode => "KIS_RANGE_UNSUPPORTED_MODE",
        RangeNormalizeError::InvalidExpectedSessions { .. } => {
            "KIS_RANGE_SESSION_SELECTION_INVALID"
        }
        RangeNormalizeError::CalendarArtifact { .. } => "KIS_RANGE_CALENDAR_INVALID",
        RangeNormalizeError::CalendarRangeOutOfBounds { .. } => "KIS_RANGE_CALENDAR_RANGE_INVALID",
        RangeNormalizeError::SourceDateMismatch { .. } => "KIS_RANGE_SOURCE_DATE_MISMATCH",
        RangeNormalizeError::MissingSourceFiles => "KIS_RANGE_SOURCE_FILES_MISSING",
        RangeNormalizeError::UnexpectedSourceKind { .. } => "KIS_RANGE_SOURCE_KIND_INVALID",
        RangeNormalizeError::UnexpectedEndpoint { .. } => "KIS_RANGE_ENDPOINT_INVALID",
        RangeNormalizeError::InvalidQuery { .. } => "KIS_RANGE_QUERY_INVALID",
        RangeNormalizeError::InvalidContinuation { .. } => "KIS_RANGE_CONTINUATION_INVALID",
        RangeNormalizeError::Malformed { .. } => "KIS_RANGE_RESPONSE_MALFORMED",
        RangeNormalizeError::InvalidField { .. } => "KIS_RANGE_FIELD_INVALID",
        RangeNormalizeError::DateOutOfQuery { .. } => "KIS_RANGE_DATE_OUT_OF_QUERY",
        RangeNormalizeError::ReversedOrder { .. } => "KIS_RANGE_ORDER_INVALID",
        RangeNormalizeError::DuplicateRow { .. } => "KIS_RANGE_DUPLICATE_ROW",
        RangeNormalizeError::ConflictingRow { .. } => "KIS_RANGE_CONFLICTING_ROW",
        RangeNormalizeError::EvidenceSizeMismatch { .. } => "KIS_RANGE_EVIDENCE_SIZE_INVALID",
        RangeNormalizeError::OutOfSession { .. } => "KIS_RANGE_OUT_OF_SESSION",
        RangeNormalizeError::SessionCoverage { .. } => "KIS_RANGE_SESSION_COVERAGE_INVALID",
        RangeNormalizeError::ExistingBatchConflict { .. } => "KIS_RANGE_BATCH_CONFLICT",
        RangeNormalizeError::Serialization(_) => "KIS_RANGE_SERIALIZATION_FAILED",
    };
    WorkerDiagnostic {
        error_code,
        endpoint: None,
        http_status: None,
        response_kind: None,
        file_name: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerDiagnostic<'a> {
    pub error_code: &'a str,
    pub endpoint: Option<&'a str>,
    pub http_status: Option<u16>,
    pub response_kind: Option<&'a str>,
    pub file_name: Option<&'a str>,
}

fn provider_diagnostic(error: &ProviderError) -> Option<WorkerDiagnostic<'_>> {
    match error {
        ProviderError::Remote {
            code, diagnostic, ..
        } => Some(WorkerDiagnostic {
            error_code: code,
            endpoint: diagnostic.as_ref().map(|value| value.endpoint.as_str()),
            http_status: diagnostic.as_ref().and_then(|value| value.http_status),
            response_kind: None,
            file_name: None,
        }),
        _ => None,
    }
}

/// Converts a normalization failure to a bounded operator diagnostic.
///
/// NormalizeError retains provider-file names and (for one validation
/// variant) an endpoint supplied by immutable Raw.  Only plain file names and
/// the exact reviewed KIS read endpoints are allowed through this boundary;
/// row values, reasons, paths, hashes, and other free-form fields are never
/// copied into a worker event.
fn normalize_diagnostic(error: &NormalizeError) -> Option<WorkerDiagnostic<'_>> {
    let (error_code, response_kind, file_name, endpoint) = match error {
        NormalizeError::Store(source) => (normalize_store_error_code(source), None, None, None),
        NormalizeError::UnsupportedScope { .. } => {
            ("KIS_NORMALIZE_UNSUPPORTED_SCOPE", None, None, None)
        }
        NormalizeError::UnsupportedMode => ("KIS_NORMALIZE_UNSUPPORTED_MODE", None, None, None),
        NormalizeError::ExistingBatchConflict { .. } => {
            ("KIS_NORMALIZE_BATCH_CONFLICT", None, None, None)
        }
        NormalizeError::EvidenceCountMismatch { .. }
        | NormalizeError::EvidenceMissing { .. }
        | NormalizeError::EvidenceUnexpected { .. }
        | NormalizeError::EvidenceHashMismatch { .. }
        | NormalizeError::EvidenceSizeMismatch { .. } => {
            ("KIS_NORMALIZE_EVIDENCE_INVALID", None, None, None)
        }
        NormalizeError::MissingKind { kind } => (
            "KIS_NORMALIZE_MISSING_RESPONSE",
            Some(kind.as_str()),
            None,
            None,
        ),
        NormalizeError::UnexpectedEndpoint {
            file_name,
            endpoint,
        } => (
            "KIS_NORMALIZE_UNEXPECTED_ENDPOINT",
            None,
            safe_normalize_file_name(file_name),
            safe_kis_read_endpoint(endpoint),
        ),
        NormalizeError::Malformed {
            kind, file_name, ..
        } => (
            "KIS_NORMALIZE_MALFORMED",
            Some(kind.as_str()),
            safe_normalize_file_name(file_name),
            None,
        ),
        NormalizeError::MissingField {
            kind, file_name, ..
        } => (
            "KIS_NORMALIZE_MISSING_FIELD",
            Some(kind.as_str()),
            safe_normalize_file_name(file_name),
            None,
        ),
        NormalizeError::InvalidField {
            kind, file_name, ..
        } => (
            "KIS_NORMALIZE_INVALID_FIELD",
            Some(kind.as_str()),
            safe_normalize_file_name(file_name),
            None,
        ),
        NormalizeError::DuplicateRow { kind, .. } => (
            "KIS_NORMALIZE_DUPLICATE_ROW",
            Some(kind.as_str()),
            None,
            None,
        ),
        NormalizeError::ConflictingRow { kind, .. } => (
            "KIS_NORMALIZE_CONFLICTING_ROW",
            Some(kind.as_str()),
            None,
            None,
        ),
        NormalizeError::UnsupportedAction { file_name, .. } => (
            "KIS_NORMALIZE_UNSUPPORTED_ACTION",
            None,
            safe_normalize_file_name(file_name),
            None,
        ),
        NormalizeError::CanonicalValidation { kind, .. } => (
            "KIS_NORMALIZE_CANONICAL_INVALID",
            Some(kind.as_str()),
            None,
            None,
        ),
        NormalizeError::Serialization { kind, .. } => (
            "KIS_NORMALIZE_SERIALIZATION",
            Some(kind.as_str()),
            None,
            None,
        ),
        NormalizeError::MissingTargetObservation { .. } => {
            ("KIS_NORMALIZE_MISSING_TARGET_OBSERVATION", None, None, None)
        }
        NormalizeError::TargetBarCoverage { .. } => {
            ("KIS_NORMALIZE_BAR_COVERAGE", None, None, None)
        }
    };
    Some(WorkerDiagnostic {
        error_code,
        endpoint,
        http_status: None,
        response_kind,
        file_name,
    })
}

fn normalize_store_error_code(error: &StoreError) -> &'static str {
    match error {
        StoreError::Io { .. } => "KIS_NORMALIZE_STORE_IO",
        StoreError::FileExists { .. } => "KIS_NORMALIZE_STORE_FILE_EXISTS",
        StoreError::CleanupFailed { original, .. }
        | StoreError::IndeterminateBatchCommit {
            source: original, ..
        } => normalize_store_error_code(original),
        StoreError::UnsafeFileName { .. }
        | StoreError::UnsafeScope { .. }
        | StoreError::ScopeMismatch { .. }
        | StoreError::UnsafePath { .. }
        | StoreError::ContentHashMismatch { .. }
        | StoreError::CorruptManifest { .. }
        | StoreError::CorruptBatchMetadata { .. }
        | StoreError::InvalidBatchMetadata { .. }
        | StoreError::MissingEvidence { .. }
        | StoreError::Serialization { .. }
        | StoreError::ManifestConflict { .. } => "KIS_NORMALIZE_INTEGRITY_FAILURE",
    }
}

fn safe_normalize_file_name(file_name: &str) -> Option<&str> {
    valid_file_name(file_name).then_some(file_name)
}

fn safe_kis_read_endpoint(endpoint: &str) -> Option<&str> {
    KIS_READ_CHANNELS
        .iter()
        .find_map(|(allowed, _)| (*allowed == endpoint).then_some(*allowed))
}

fn kis_error_endpoint(error: &KisError) -> Option<&str> {
    match error {
        KisError::RateLimited { endpoint, .. }
        | KisError::Broker { endpoint, .. }
        | KisError::SchemaDrift { endpoint, .. } => Some(endpoint),
        _ => None,
    }
}

fn kis_error_http_status(error: &KisError) -> Option<u16> {
    match error {
        KisError::Broker { status, .. } => Some(*status),
        _ => None,
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
    NoCandidatePublication,
    CandidateUniverseUnavailable,
    StaleCandidatePublication,
    FutureCandidatePublication,
    NoPricePublication,
    StalePricePublication,
    FuturePricePublication,
    PriceManifestMismatch,
}

impl fmt::Display for HealthFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoEodPublication => "no KRX/KR EOD publication",
            Self::StaleEodPublication => "latest KRX/KR EOD publication is stale",
            Self::FutureEodPublication => "latest KRX/KR EOD publication is in the future",
            Self::NoCandidatePublication => "no complete candidate source publication",
            Self::CandidateUniverseUnavailable => {
                "one or more enabled candidate universes are not ready"
            }
            Self::StaleCandidatePublication => "latest candidate source publication is stale",
            Self::FutureCandidatePublication => {
                "latest candidate source publication is in the future"
            }
            Self::NoPricePublication => "no candidate price publication",
            Self::StalePricePublication => "latest candidate price publication is stale",
            Self::FuturePricePublication => "latest candidate price publication is in the future",
            Self::PriceManifestMismatch => {
                "candidate price publication does not match its on-disk manifest"
            }
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
    fn recovered(&self, batch_id: BatchId, date: TradingDate);
    fn skipped(&self, batch_id: BatchId, date: TradingDate);
}

fn notify_recovery_observer(observer: &dyn RecoveryObserver, outcome: RecoveryBatchOutcome) {
    match outcome {
        RecoveryBatchOutcome::Recovered { batch_id, date } => {
            observer.recovered(batch_id, date);
        }
        RecoveryBatchOutcome::Skipped { batch_id, date } => {
            observer.skipped(batch_id, date);
        }
    }
}

struct ContextRecoveryObserver<'a> {
    observer: &'a dyn WorkerObserver,
    provider: &'static str,
}

impl RecoveryObserver for ContextRecoveryObserver<'_> {
    fn recovered(&self, batch_id: BatchId, date: TradingDate) {
        self.emit(WorkerEventKind::Recovered, batch_id, date);
    }

    fn skipped(&self, batch_id: BatchId, date: TradingDate) {
        self.emit(WorkerEventKind::Skipped, batch_id, date);
    }
}

impl ContextRecoveryObserver<'_> {
    fn emit(&self, kind: WorkerEventKind, batch_id: BatchId, date: TradingDate) {
        self.observer.emit(WorkerEvent {
            kind,
            provider: self.provider,
            market: "KR",
            target_date: Some(date),
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

type LiveKisClient = KisMarketDataClient<LiveTransport, TokioSleeper, SystemCredentialSource>;
type LiveKisProvider = KisProvider<LiveKisClient>;

fn kis_system_now_ms() -> i64 {
    SystemClock.now_ms()
}

/// Build the credentialed KIS read path without ever copying a secret value
/// into the worker configuration.  `SystemCredentialSource` resolves both
/// files at token issue/read time, so a mounted-secret rotation takes effect
/// without rebuilding this client.
pub(crate) fn build_production_kis_provider(
    config: &ResearchWorkerConfig,
) -> Result<LiveKisProvider, WorkerError> {
    let app_key_path = config
        .kis_app_key_file
        .as_ref()
        .ok_or(WorkerError::InvalidConfig {
            key: "KIS_APP_KEY_FILE",
        })?;
    let app_secret_path =
        config
            .kis_app_secret_file
            .as_ref()
            .ok_or(WorkerError::InvalidConfig {
                key: "KIS_APP_SECRET_FILE",
            })?;
    build_production_kis_provider_from_files(app_key_path, app_secret_path)
}

/// Build the live KIS read client from mounted credential paths only.
///
/// This intentionally does not accept `ResearchWorkerConfig`: that config
/// carries database settings and is therefore unsuitable for the isolated
/// historical Raw-only command. Credential values remain in
/// `SystemCredentialSource` and are read by the token issuer/client only when
/// the first request is made.
fn build_production_kis_provider_from_files(
    app_key_path: &Path,
    app_secret_path: &Path,
) -> Result<LiveKisProvider, WorkerError> {
    let app_key_ref = CredentialRef::file(app_key_path.to_string_lossy().into_owned());
    let app_secret_ref = CredentialRef::file(app_secret_path.to_string_lossy().into_owned());

    // Token issuance and market reads own separate transports because the
    // live transport is intentionally not Clone.  Both use the same explicit
    // timeout and the same live host selected by `LiveTransport::live`.
    let token_transport = LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(WorkerError::KisClient)?;
    let read_transport = LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(WorkerError::KisClient)?;
    let clock = Arc::new(SystemClock);
    let token_issuer = KisTokenIssuer::new(
        token_transport,
        SystemCredentialSource,
        app_key_ref.clone(),
        app_secret_ref.clone(),
        kis_system_now_ms,
    );
    let tokens = Arc::new(TokenManager::new(clock.clone(), Arc::new(token_issuer)));
    let limiter = KIS_READ_CHANNELS.iter().fold(
        RateLimiter::new(clock, KIS_READ_QUOTA),
        |limiter, (endpoint, tr_id)| {
            limiter.with_quota(BucketKey::new(*endpoint, *tr_id), KIS_READ_QUOTA)
        },
    );
    let client = KisMarketDataClient::new(
        read_transport,
        TokioSleeper,
        tokens,
        Arc::new(limiter),
        SystemCredentialSource,
        app_key_ref,
        app_secret_ref,
    );
    Ok(KisProvider::kr_etf_core(client))
}

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
            // The injected synchronous factory is a fixture/test seam. The
            // production process backend routes credentialed work through the
            // async KIS orchestration instead of erasing it into EodProvider.
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
        entitlement_reference: config.entitlement_reference.clone(),
        expected_fetch_mode: config.fetch_mode,
    });
    Ok(ResearchWorker::new(config, backend))
}

pub fn bootstrap_worker(values: &HashMap<String, String>) -> Result<ResearchWorker, WorkerError> {
    let config = ResearchWorkerConfig::from_map(values)?;
    let executable = std::env::current_exe().map_err(|_| WorkerError::ChildIo {
        phase: WorkerPhase::Config,
    })?;
    let system_root = validated_system_root()?;
    let pool = build_postgres_pool(&config.database);
    let backend = Arc::new(ProcessResearchBackend {
        executable,
        env: helper_environment(values, system_root.as_deref()),
        sink: PostgresPublicationSink::new(pool.clone()),
        candidate_sink: PostgresCandidateSourceSink::new(pool),
        candidate_sources_enabled: config.candidate_sources_enabled,
        expected_fetch_mode: config.fetch_mode,
        attempt_timeout: config.attempt_timeout,
        recovery_position: Mutex::new(RecoveryPosition::default()),
    });
    Ok(ResearchWorker::new(config, backend))
}

struct PipelineResearchBackend {
    store: RawStore,
    provider: Arc<dyn EodProvider>,
    sink: PostgresPublicationSink,
    entitlement_reference: String,
    expected_fetch_mode: FetchMode,
}

#[async_trait]
impl ResearchBackend for PipelineResearchBackend {
    async fn recover(
        &self,
        _control: &dyn WorkerControl,
        observer: &dyn RecoveryObserver,
    ) -> Result<(), WorkerError> {
        let result = recover_unpublished_with(&self.store, &self.sink, |outcome| {
            notify_recovery_observer(observer, outcome);
            Ok::<_, std::convert::Infallible>(())
        })
        .await;
        match result {
            Ok(()) => Ok(()),
            Err(RecoveryError::Pipeline(error)) => Err(WorkerError::Pipeline(error)),
            Err(RecoveryError::Observer { source, .. }) => match source {},
        }
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError> {
        self.sink
            .has_eod_for_mode(date, self.expected_fetch_mode)
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
            Some(&self.entitlement_reference),
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
    candidate_sink: PostgresCandidateSourceSink,
    candidate_sources_enabled: bool,
    expected_fetch_mode: FetchMode,
    attempt_timeout: Duration,
    recovery_position: Mutex<RecoveryPosition>,
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
            self.attempt_timeout,
            phase,
            control,
        )
        .await?
        {
            SupervisedChildOutcome::TimedOut => Err(WorkerError::Timeout { phase }),
            SupervisedChildOutcome::Shutdown => Err(WorkerError::Shutdown),
            SupervisedChildOutcome::Completed { success, stdout } => {
                let decoded = decode_helper_output_with_provider(
                    &stdout,
                    phase,
                    expected_date,
                    worker_event_provider(self.expected_fetch_mode),
                );
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
        loop {
            let position =
                *self
                    .recovery_position
                    .lock()
                    .map_err(|_| WorkerError::ChildContainment {
                        phase: WorkerPhase::Recovery,
                    })?;
            let mut args = vec![OsString::from("__research-internal-recover")];
            if let Some(snapshot_after) = position.snapshot_after {
                args.push(OsString::from("--snapshot-after"));
                args.push(OsString::from(snapshot_after.to_string()));
            }
            if let Some(snapshot_high_water) = position.snapshot_high_water {
                args.push(OsString::from("--snapshot-high-water"));
                args.push(OsString::from(snapshot_high_water.to_string()));
            }
            if let Some(cursor) = position.cursor {
                args.push(OsString::from("--after"));
                args.push(OsString::from(cursor.to_string()));
            }
            let page = supervise_recovery_child_with_provider(
                ChildSpec {
                    executable: self.executable.clone(),
                    args,
                    env: self.env.clone(),
                },
                self.attempt_timeout,
                control,
                observer,
                position,
                &self.recovery_position,
                worker_event_provider(self.expected_fetch_mode),
            )
            .await?;
            if page.has_more {
                continue;
            }
            if page.snapshot_high_water == position.snapshot_after && page.cursor.is_none() {
                *self
                    .recovery_position
                    .lock()
                    .map_err(|_| WorkerError::ChildContainment {
                        phase: WorkerPhase::Recovery,
                    })? = RecoveryPosition::default();
                return Ok(());
            }
            *self
                .recovery_position
                .lock()
                .map_err(|_| WorkerError::ChildContainment {
                    phase: WorkerPhase::Recovery,
                })? = RecoveryPosition {
                snapshot_after: page.snapshot_high_water,
                snapshot_high_water: None,
                cursor: None,
            };
        }
    }

    async fn has_eod(&self, date: TradingDate) -> Result<bool, WorkerError> {
        let eod = self
            .sink
            .has_eod_for_mode(date, self.expected_fetch_mode)
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::DuplicateCheck,
                source,
            })?;
        if !eod || !self.candidate_sources_enabled {
            return Ok(eod);
        }
        let sources = self
            .candidate_sink
            .has_complete_sources(date, self.expected_fetch_mode)
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::DuplicateCheck,
                source,
            })?;
        let price = self
            .candidate_sink
            .has_price(date)
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::DuplicateCheck,
                source,
            })?;
        Ok(sources && price)
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
    run_internal_recovery_stream(values, &mut io::sink())
        .await
        .map(|_| ())
}

pub async fn run_internal_recovery_stream<W>(
    values: &HashMap<String, String>,
    writer: &mut W,
) -> Result<RecoveryPage, WorkerError>
where
    W: io::Write,
{
    run_internal_recovery_page_stream(values, RecoveryPosition::default(), writer).await
}

pub async fn run_internal_recovery_page_stream<W>(
    values: &HashMap<String, String>,
    position: RecoveryPosition,
    writer: &mut W,
) -> Result<RecoveryPage, WorkerError>
where
    W: io::Write,
{
    let config = ResearchWorkerConfig::from_map(values)?;
    let factory = ProductionWorkerComponentFactory;
    let store = factory.build_store(&config)?;
    let pool = factory.build_pool(&config)?;
    let sink = PostgresPublicationSink::new(pool.clone());
    let scope = match config.fetch_mode {
        FetchMode::Synthetic => RecoveryScope::Krx,
        FetchMode::Credentialed => {
            // Normalize every durable wire batch before taking the canonical
            // publication snapshot. This is repeated on each child page so a
            // raw append racing recovery is consumed on the next page.
            recover_kis_normalization(&store).map_err(WorkerError::Pipeline)?;
            RecoveryScope::KisNormalized
        }
    };
    let page = recover_unpublished_page_with_scope(
        &store,
        &sink,
        scope,
        position,
        RECOVERY_PAGE_SIZE,
        |outcome, snapshot_high_water| {
            let (event, batch_id, date) = match outcome {
                RecoveryBatchOutcome::Recovered { batch_id, date } => ("recovered", batch_id, date),
                RecoveryBatchOutcome::Skipped { batch_id, date } => ("skipped", batch_id, date),
            };
            serde_json::to_writer(
                &mut *writer,
                &RecoveryItemWire {
                    status: "event",
                    event,
                    phase: "recovery",
                    batch_id,
                    target_date: date.to_iso(),
                    snapshot_high_water,
                },
            )
            .map_err(io::Error::other)?;
            writer.write_all(b"\n")?;
            writer.flush()
        },
    )
    .await
    .map_err(|error| match error {
        RecoveryError::Pipeline(source) => WorkerError::Pipeline(source),
        RecoveryError::Observer { .. } => WorkerError::Io {
            phase: WorkerPhase::Recovery,
        },
    })?;
    if !page.has_more {
        // KRX synthetic and KIS normalized are both eligible for the
        // provider-neutral Curated price surface.  Candidate source (flow,
        // fundamentals, membership, sector, status) ingestion remains behind
        // its separate feature gate and is not needed for this price replay.
        let price_sink = PostgresCandidateSourceSink::new(pool);
        recover_price_publications(&config, &store, &price_sink).await?;
    }
    Ok(page)
}

pub async fn run_internal_ingest(
    values: &HashMap<String, String>,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<BatchId, WorkerError> {
    let config = ResearchWorkerConfig::from_map(values)?;
    let factory = ProductionWorkerComponentFactory;
    let store = factory.build_store(&config)?;
    let pool = factory.build_pool(&config)?;
    let sink = PostgresPublicationSink::new(pool.clone());
    let price_sink = PostgresCandidateSourceSink::new(pool.clone());
    if config.fetch_mode == FetchMode::Credentialed {
        return run_credentialed_internal_ingest(&config, &store, &sink, &price_sink, date, now)
            .await;
    }
    let provider = factory.build_provider(&config)?;
    let recovered_price_batch = recover_price_publications(&config, &store, &price_sink).await?;
    let eod_batch_id = if sink
        .has_eod_for_mode(date, config.fetch_mode)
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::DuplicateCheck,
            source,
        })? {
        None
    } else {
        let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
        Some(
            ingest_and_publish(
                &store,
                provider.as_ref(),
                &request,
                Some(&config.entitlement_reference),
                &sink,
            )
            .await
            .map(|outcome| outcome.manifest.batch_id)
            .map_err(WorkerError::Pipeline)?,
        )
    };
    let published_price_batch = recover_price_publications(&config, &store, &price_sink).await?;
    let candidate_batch_id = if config.candidate_sources_enabled {
        run_candidate_source_ingest(&config, pool, date, now).await?
    } else {
        None
    };
    candidate_batch_id
        .or(eod_batch_id)
        .or(published_price_batch)
        .or(recovered_price_batch)
        .ok_or(WorkerError::ChildOutput {
            phase: WorkerPhase::Ingest,
        })
}

/// Runs an inclusive credentialed backfill over an exact, sorted session list.
///
/// The scheduler-only XKRX artifact is intentionally consumed by the operator
/// wrapper, not by this provider.  This CLI boundary accepts the already
/// validated civil dates so weekends/closures never enter the worker loop while
/// one process still owns one in-memory TokenManager and cumulative recovery.
pub async fn run_credentialed_backfill_session_dates_stream<W: io::Write>(
    values: &HashMap<String, String>,
    dates: &[TradingDate],
    writer: &mut W,
) -> Result<usize, WorkerError> {
    if dates.is_empty()
        || dates.len() > 10_000
        || dates.windows(2).any(|window| window[0] >= window[1])
    {
        return Err(WorkerError::InvalidConfig {
            key: "--backfill-session-dates",
        });
    }
    let end = *dates.last().expect("validated non-empty session dates");
    let config = ResearchWorkerConfig::from_map(values)?;
    if config.fetch_mode != FetchMode::Credentialed || config.candidate_sources_enabled {
        return Err(WorkerError::InvalidConfig {
            key: "--backfill-session-dates",
        });
    }
    let factory = ProductionWorkerComponentFactory;
    let store = factory.build_store(&config)?;
    let pool = factory.build_pool(&config)?;
    let sink = PostgresPublicationSink::new(pool.clone());
    let price_sink = PostgresCandidateSourceSink::new(pool);
    // Lazy issuance is preserved: construction reads no secret and makes no
    // network call. The first actually missing date obtains the one token.
    let provider = build_production_kis_provider(&config)?.with_calendar_snapshot_cache();
    // Full-scope crash recovery is deliberately once per process. New targets
    // publish only their canonical EOD row here; one cumulative Curated
    // generation is reconciled after the range, avoiding a
    // date_count * manifest_count rescan during a multi-year backfill.
    let normalized = recover_kis_normalization(&store).map_err(WorkerError::Pipeline)?;
    recover_price_publications(&config, &store, &price_sink).await?;
    let context = CredentialedIngestContext {
        config: &config,
        store: &store,
        sink: &sink,
        price_sink: &price_sink,
        provider: &provider,
        normalized: &normalized,
    };
    let mut processed = 0usize;
    for &date in dates {
        let batch_id = tokio::time::timeout(
            config.attempt_timeout,
            run_credentialed_backfill_date(&context, date, UtcTimestamp::now()),
        )
        .await
        .map_err(|_| WorkerError::Timeout {
            phase: WorkerPhase::Ingest,
        })??;
        // One cumulative Curated generation is materialized only after the
        // range is complete. Holding the final canonical event until that
        // succeeds ensures an idempotent rerun still enters this process if
        // the final cumulative recovery fails.
        if date == end {
            recover_price_publications(&config, &store, &price_sink).await?;
        }
        serde_json::to_writer(
            &mut *writer,
            &BackfillItemWire {
                status: "event",
                event: "published",
                phase: "canonical_publication",
                batch_id,
                target_date: date.to_iso(),
            },
        )
        .map_err(|_| WorkerError::Io {
            phase: WorkerPhase::Publication,
        })?;
        writer.write_all(b"\n").map_err(|_| WorkerError::Io {
            phase: WorkerPhase::Publication,
        })?;
        writer.flush().map_err(|_| WorkerError::Io {
            phase: WorkerPhase::Publication,
        })?;
        processed = processed.saturating_add(1);
    }
    Ok(processed)
}

/// Captures and normalizes one bounded historical daily-bars range without
/// opening a database or constructing a publication/Curated sink.
///
/// This is intentionally a separate CLI seam from the production EOD worker.
/// It accepts only the fixed 11-ETF daily-itemchartprice capability, validates
/// the embedded approved XKRX session selection, and leaves the result under
/// the isolated Raw scopes `kis-daily-range` and
/// `kis-daily-range-normalized`. The source remains a current vendor snapshot
/// acquired at the recorded UTC time; this function never makes a strict PIT
/// claim or fabricates adjusted prices/actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyRangeRawSummary {
    pub source_batch_id: BatchId,
    pub normalized_count: usize,
    pub start: TradingDate,
    pub end: TradingDate,
    pub reused_existing_source: bool,
}

pub async fn run_credentialed_daily_range_raw_stream<W: io::Write>(
    values: &HashMap<String, String>,
    start: TradingDate,
    end: TradingDate,
    writer: &mut W,
) -> Result<DailyRangeRawSummary, WorkerError> {
    let config = DailyRangeRawConfig::from_map(values)?;
    let expected =
        ExpectedRangeSessions::approved_xkrx(start, end).map_err(WorkerError::RangeNormalize)?;
    if expected.sessions.is_empty() {
        return Err(WorkerError::InvalidConfig {
            key: "daily-range-session-selection",
        });
    }
    let store = RawStore::new(&config.raw_root);

    // Reconcile the immutable source manifest first. A matching source range
    // is reused verbatim, so a retry after a successful capture never creates
    // another KIS batch or obtains another token. Any malformed matching
    // evidence is passed to the normalizer and fails closed rather than being
    // silently replaced by a fresh network capture.
    let source = find_existing_daily_range_source(
        &store,
        start,
        end,
        config.entitlement_reference.as_str(),
        config.batch_id,
    )?;
    let source = match source {
        Some(source) => source,
        None => {
            let provider = build_production_kis_provider_from_files(
                &config.app_key_file,
                &config.app_secret_file,
            )?;
            ingest_kis_daily_bars_range_with_batch_id(
                &store,
                &provider,
                MARKET_KR,
                start,
                end,
                UtcTimestamp::now(),
                Some(config.entitlement_reference.as_str()),
                config.batch_id,
            )
            .await
            .map_err(|source| WorkerError::Pipeline(PipelineError::Ingest { source }))?
            .entry
        }
    };

    if source.entitlement_reference.as_deref() != Some(config.entitlement_reference.as_str()) {
        return Err(WorkerError::RangeNormalize(
            RangeNormalizeError::ExistingBatchConflict {
                batch_id: source.batch_id,
                reason: "committed range source entitlement does not match the run identity"
                    .to_owned(),
            },
        ));
    }

    let outcomes = normalize_kis_daily_range_batch(&store, &source, &expected)
        .map_err(WorkerError::RangeNormalize)?;
    emit_daily_range_events(writer, &outcomes)?;
    Ok(DailyRangeRawSummary {
        source_batch_id: source.batch_id,
        normalized_count: outcomes.len(),
        start,
        end,
        reused_existing_source: false,
    })
}

/// Re-normalize an explicitly identified immutable Stage3 source batch.
///
/// This path intentionally does not construct a provider or read KIS
/// credentials.  The source batch must already be present in the reconciled
/// Raw manifest and must satisfy the exact range request contract before the
/// Stage4A normalizer is called.  A missing, malformed, or conflicting batch
/// is a permanent error; there is deliberately no fetch fallback.
pub async fn run_existing_daily_range_raw_stream<W: io::Write>(
    values: &HashMap<String, String>,
    start: TradingDate,
    end: TradingDate,
    source_batch_id: BatchId,
    writer: &mut W,
) -> Result<DailyRangeRawSummary, WorkerError> {
    let config = DailyRangeRawRecoveryConfig::from_map(values)?;
    let expected =
        ExpectedRangeSessions::approved_xkrx(start, end).map_err(WorkerError::RangeNormalize)?;
    if expected.sessions.is_empty() {
        return Err(WorkerError::InvalidConfig {
            key: "daily-range-session-selection",
        });
    }
    let store = RawStore::new(&config.raw_root);
    let entries = store
        .read_reconciled_manifest(PROVIDER_KIS_DAILY_RANGE, MARKET_KR)
        .map_err(|source| WorkerError::Pipeline(PipelineError::Manifest { source }))?;
    let source = entries
        .into_iter()
        .find(|entry| entry.batch_id == source_batch_id)
        .ok_or_else(|| {
            WorkerError::RangeNormalize(RangeNormalizeError::ExistingBatchConflict {
                batch_id: source_batch_id,
                reason:
                    "explicit existing source batch is missing from the reconciled Raw manifest"
                        .to_owned(),
            })
        })?;
    validate_explicit_daily_range_source(
        &source,
        start,
        end,
        config.entitlement_reference.as_str(),
    )
    .map_err(|reason| {
        WorkerError::RangeNormalize(RangeNormalizeError::ExistingBatchConflict {
            batch_id: source_batch_id,
            reason,
        })
    })?;
    let outcomes = normalize_kis_daily_range_batch(&store, &source, &expected)
        .map_err(WorkerError::RangeNormalize)?;
    emit_daily_range_events(writer, &outcomes)?;
    Ok(DailyRangeRawSummary {
        source_batch_id: source.batch_id,
        normalized_count: outcomes.len(),
        start,
        end,
        reused_existing_source: true,
    })
}

fn emit_daily_range_events<W: io::Write>(
    writer: &mut W,
    outcomes: &[market_data::range_normalize::RangeNormalizationOutcome],
) -> Result<(), WorkerError> {
    for outcome in outcomes {
        serde_json::to_writer(
            &mut *writer,
            &BackfillItemWire {
                status: "event",
                event: "normalized",
                phase: "raw_only_normalization",
                batch_id: outcome.entry.batch_id,
                target_date: outcome.session_date.to_iso(),
            },
        )
        .map_err(|_| WorkerError::Io {
            phase: WorkerPhase::Ingest,
        })?;
        writer.write_all(b"\n").map_err(|_| WorkerError::Io {
            phase: WorkerPhase::Ingest,
        })?;
        writer.flush().map_err(|_| WorkerError::Io {
            phase: WorkerPhase::Ingest,
        })?;
    }
    Ok(())
}

#[derive(Debug)]
struct DailyRangeRawRecoveryConfig {
    raw_root: PathBuf,
    entitlement_reference: String,
}

impl DailyRangeRawRecoveryConfig {
    fn from_map(values: &HashMap<String, String>) -> Result<Self, WorkerError> {
        if required(values, "APP_ENV")? != "production"
            || required(values, "RESEARCH_FETCH_MODE")? != "credentialed"
        {
            return Err(WorkerError::InvalidConfig {
                key: "daily-range-environment",
            });
        }
        let commit = required(values, "LAGRANGE_CODE_COMMIT")?;
        if !is_git_commit(commit) {
            return Err(WorkerError::InvalidConfig {
                key: "LAGRANGE_CODE_COMMIT",
            });
        }
        if values
            .get("RESEARCH_CANDIDATE_ENABLED")
            .map(String::as_str)
            .unwrap_or("false")
            != "false"
        {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_CANDIDATE_ENABLED",
            });
        }
        let raw_root = PathBuf::from(nonempty(values, "RESEARCH_RAW_ROOT")?);
        if !raw_root.is_absolute() {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_RAW_ROOT",
            });
        }
        let entitlement_reference = nonempty(values, "RESEARCH_ENTITLEMENT_REFERENCE")?.to_owned();
        if entitlement_reference.len() > 256 {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_ENTITLEMENT_REFERENCE",
            });
        }
        Ok(Self {
            raw_root,
            entitlement_reference,
        })
    }
}

#[derive(Debug)]
struct DailyRangeRawConfig {
    raw_root: PathBuf,
    entitlement_reference: String,
    app_key_file: PathBuf,
    app_secret_file: PathBuf,
    batch_id: BatchId,
}

impl DailyRangeRawConfig {
    fn from_map(values: &HashMap<String, String>) -> Result<Self, WorkerError> {
        if required(values, "APP_ENV")? != "production"
            || required(values, "RESEARCH_FETCH_MODE")? != "credentialed"
        {
            return Err(WorkerError::InvalidConfig {
                key: "daily-range-environment",
            });
        }
        let commit = required(values, "LAGRANGE_CODE_COMMIT")?;
        if !is_git_commit(commit) {
            return Err(WorkerError::InvalidConfig {
                key: "LAGRANGE_CODE_COMMIT",
            });
        }
        if values
            .get("RESEARCH_CANDIDATE_ENABLED")
            .map(String::as_str)
            .unwrap_or("false")
            != "false"
        {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_CANDIDATE_ENABLED",
            });
        }
        let raw_root = PathBuf::from(nonempty(values, "RESEARCH_RAW_ROOT")?);
        if !raw_root.is_absolute() {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_RAW_ROOT",
            });
        }
        let entitlement_reference = nonempty(values, "RESEARCH_ENTITLEMENT_REFERENCE")?.to_owned();
        if entitlement_reference.len() > 256 {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_ENTITLEMENT_REFERENCE",
            });
        }
        let app_key = PathBuf::from(nonempty(values, "KIS_APP_KEY_FILE")?);
        let app_secret = PathBuf::from(nonempty(values, "KIS_APP_SECRET_FILE")?);
        if !app_key.is_absolute() || !app_secret.is_absolute() {
            return Err(WorkerError::InvalidConfig {
                key: "KIS_APP_KEY_FILE",
            });
        }
        // Validate shape without exposing values. The live client reads these
        // files lazily through SystemCredentialSource on the first request.
        read_nonempty_secret(
            &|path: &Path| std::fs::read_to_string(path),
            &app_key,
            "KIS_APP_KEY_FILE",
        )?;
        read_nonempty_secret(
            &|path: &Path| std::fs::read_to_string(path),
            &app_secret,
            "KIS_APP_SECRET_FILE",
        )?;
        let batch_id = nonempty(values, "RANGE_RAW_BATCH_ID")?
            .parse()
            .map_err(|_| WorkerError::InvalidConfig {
                key: "RANGE_RAW_BATCH_ID",
            })?;
        Ok(Self {
            raw_root,
            entitlement_reference,
            app_key_file: app_key,
            app_secret_file: app_secret,
            batch_id,
        })
    }
}

fn is_git_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && value.bytes().any(|byte| byte != b'0')
}

fn find_existing_daily_range_source(
    store: &RawStore,
    start: TradingDate,
    end: TradingDate,
    entitlement_reference: &str,
    preferred_batch_id: BatchId,
) -> Result<Option<market_data::storage::ManifestEntry>, WorkerError> {
    let entries = store
        .read_reconciled_manifest(PROVIDER_KIS_DAILY_RANGE, MARKET_KR)
        .map_err(|source| WorkerError::Pipeline(PipelineError::Manifest { source }))?;
    select_existing_daily_range_source(
        entries,
        start,
        end,
        entitlement_reference,
        preferred_batch_id,
    )
}

fn validate_explicit_daily_range_source(
    entry: &ManifestEntry,
    start: TradingDate,
    end: TradingDate,
    entitlement_reference: &str,
) -> Result<(), String> {
    if entry.provider != PROVIDER_KIS_DAILY_RANGE
        || entry.market != MARKET_KR
        || entry.mode != FetchMode::Credentialed
        || entry.date != start
        || entry.entitlement_reference.as_deref() != Some(entitlement_reference)
    {
        return Err(
            "explicit source scope/mode/date/entitlement does not match the requested identity"
                .to_owned(),
        );
    }
    if entry.files.is_empty() {
        return Err("explicit source has no daily-bar response files".to_owned());
    }
    let expected_symbols: BTreeSet<&str> = KR_ETF_CORE_SYMBOLS.iter().copied().collect();
    let expected_query_keys: BTreeSet<&str> = [
        "FID_COND_MRKT_DIV_CODE",
        "FID_INPUT_ISCD",
        "FID_INPUT_DATE_1",
        "FID_INPUT_DATE_2",
        "FID_PERIOD_DIV_CODE",
        "FID_ORG_ADJ_PRC",
    ]
    .into_iter()
    .collect();
    let mut symbols = BTreeSet::new();
    let mut windows = BTreeSet::new();
    let mut maximum_end = None;
    for file in &entry.files {
        if file.kind != ResponseKind::Bars {
            return Err(format!(
                "explicit source file {} is not a daily-bar response",
                file.file_name
            ));
        }
        if file.request.endpoint != DAILY_RANGE_ENDPOINT
            || file.request.mode != FetchMode::Credentialed
            || file.request.query.len() != expected_query_keys.len()
            || file.request.headers.len() != 5
        {
            return Err(format!(
                "explicit source file {} request metadata is invalid",
                file.file_name
            ));
        }
        let query_value = |key: &str| {
            file.request
                .query
                .iter()
                .find(|(actual, _)| actual == key)
                .map(|(_, value)| value.as_str())
        };
        let keys = file
            .request
            .query
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<BTreeSet<_>>();
        if keys != expected_query_keys
            || query_value("FID_COND_MRKT_DIV_CODE") != Some("J")
            || query_value("FID_PERIOD_DIV_CODE") != Some("D")
            || query_value("FID_ORG_ADJ_PRC") != Some("1")
        {
            return Err(format!(
                "explicit source file {} query contract is invalid",
                file.file_name
            ));
        }
        let Some(symbol) = query_value("FID_INPUT_ISCD") else {
            return Err(format!(
                "explicit source file {} has no symbol",
                file.file_name
            ));
        };
        if !expected_symbols.contains(symbol) {
            return Err(format!(
                "explicit source file {} has an unexpected symbol",
                file.file_name
            ));
        }
        let Some(actual_start) = query_value("FID_INPUT_DATE_1").and_then(parse_compact_date)
        else {
            return Err(format!(
                "explicit source file {} has an invalid start date",
                file.file_name
            ));
        };
        let Some(actual_end) = query_value("FID_INPUT_DATE_2").and_then(parse_compact_date) else {
            return Err(format!(
                "explicit source file {} has an invalid end date",
                file.file_name
            ));
        };
        if actual_start != start || actual_end < start || actual_end > end {
            return Err(format!(
                "explicit source file {} date bounds are invalid",
                file.file_name
            ));
        }
        if !windows.insert((symbol.to_owned(), actual_end)) {
            return Err(format!(
                "explicit source file {} duplicates a symbol/window",
                file.file_name
            ));
        }
        symbols.insert(symbol);
        maximum_end =
            Some(maximum_end.map_or(actual_end, |value: TradingDate| value.max(actual_end)));
        let mut authorization = None;
        let mut appkey = None;
        let mut appsecret = None;
        let mut tr_id = None;
        let mut tr_cont = None;
        for (key, value) in &file.request.headers {
            if key.eq_ignore_ascii_case("authorization") {
                if authorization.replace(value.as_str()).is_some() {
                    return Err(format!(
                        "explicit source file {} repeats authorization",
                        file.file_name
                    ));
                }
            } else if key.eq_ignore_ascii_case("appkey") {
                if appkey.replace(value.as_str()).is_some() {
                    return Err(format!(
                        "explicit source file {} repeats appkey",
                        file.file_name
                    ));
                }
            } else if key.eq_ignore_ascii_case("appsecret") {
                if appsecret.replace(value.as_str()).is_some() {
                    return Err(format!(
                        "explicit source file {} repeats appsecret",
                        file.file_name
                    ));
                }
            } else if key.eq_ignore_ascii_case("tr_id") {
                if tr_id.replace(value.as_str()).is_some() {
                    return Err(format!(
                        "explicit source file {} repeats tr_id",
                        file.file_name
                    ));
                }
            } else if key.eq_ignore_ascii_case("tr_cont") {
                if tr_cont.replace(value.as_str()).is_some() {
                    return Err(format!(
                        "explicit source file {} repeats tr_cont",
                        file.file_name
                    ));
                }
            } else {
                return Err(format!(
                    "explicit source file {} has an unexpected header",
                    file.file_name
                ));
            }
        }
        if authorization != Some("[REDACTED]")
            || appkey != Some("[REDACTED]")
            || appsecret != Some("[REDACTED]")
            || tr_id != Some(DAILY_RANGE_TR_ID)
            || tr_cont != Some("")
        {
            return Err(format!(
                "explicit source file {} continuation/header contract is invalid",
                file.file_name
            ));
        }
    }
    if symbols != expected_symbols || maximum_end != Some(end) {
        return Err(
            "explicit source does not contain the exact fixed 11-symbol requested range".to_owned(),
        );
    }
    Ok(())
}

/// Selects an immutable range source before any provider construction. The
/// preferred deterministic batch is authoritative even when its files are
/// malformed; the downstream normalizer must surface that corruption rather
/// than allowing a retry to create a second Raw batch. If state was lost, any
/// other source with the same batch-level identity is a permanent conflict.
fn select_existing_daily_range_source(
    entries: Vec<ManifestEntry>,
    start: TradingDate,
    end: TradingDate,
    entitlement_reference: &str,
    preferred_batch_id: BatchId,
) -> Result<Option<ManifestEntry>, WorkerError> {
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.batch_id == preferred_batch_id)
        .cloned()
    {
        return Ok(Some(entry));
    }

    let candidates: Vec<_> = entries
        .into_iter()
        .filter(|entry| {
            entry.provider == PROVIDER_KIS_DAILY_RANGE
                && entry.market == MARKET_KR
                && entry.mode == FetchMode::Credentialed
                && entry.date == start
                && entry.entitlement_reference.as_deref() == Some(entitlement_reference)
                && range_manifest_entry_matches(entry, start, end)
        })
        .collect();
    if let Some(entry) = candidates.first() {
        return Err(WorkerError::RangeNormalize(
            RangeNormalizeError::ExistingBatchConflict {
                batch_id: entry.batch_id,
                reason: if candidates.len() == 1 {
                    "state identity is absent but an existing range source has another batch id"
                        .to_owned()
                } else {
                    format!(
                        "state identity is absent and {} existing range sources are ambiguous",
                        candidates.len()
                    )
                },
            },
        ));
    }
    Ok(None)
}

fn range_manifest_entry_matches(
    entry: &ManifestEntry,
    start: TradingDate,
    end: TradingDate,
) -> bool {
    // A committed but incomplete batch has no trustworthy request metadata;
    // keep it as a conflict so state loss cannot turn it into a second fetch.
    if entry.files.is_empty() {
        return true;
    }

    let expected_symbols: BTreeSet<&str> = KR_ETF_CORE_SYMBOLS.iter().copied().collect();
    let expected_query_keys: BTreeSet<&str> = [
        "FID_COND_MRKT_DIV_CODE",
        "FID_INPUT_ISCD",
        "FID_INPUT_DATE_1",
        "FID_INPUT_DATE_2",
        "FID_PERIOD_DIV_CODE",
        "FID_ORG_ADJ_PRC",
    ]
    .into_iter()
    .collect();
    let mut requested_symbols = BTreeSet::new();
    let mut requested_windows = BTreeSet::new();
    let mut maximum_end: Option<TradingDate> = None;
    let mut saw_requested_range = false;
    let mut saw_other_range = false;

    for file in &entry.files {
        if file.kind != ResponseKind::Bars {
            // A range batch may contain bars only. An unexpected response is
            // not silently ignored after state loss.
            return true;
        }

        let query_value = |key: &str| {
            file.request
                .query
                .iter()
                .find(|(actual, _)| actual == key)
                .map(|(_, value)| value.as_str())
        };
        let date1 = query_value("FID_INPUT_DATE_1").and_then(parse_compact_date);
        let date2 = query_value("FID_INPUT_DATE_2").and_then(parse_compact_date);

        // A fully identified window outside the requested range is a
        // different source, not a candidate for this state identity. A
        // malformed window whose first bound is the requested start remains
        // a conflict, because refetching could create a second Raw batch.
        match (date1, date2) {
            (Some(actual_start), Some(actual_end)) if actual_start == start => {
                if !(start..=end).contains(&actual_end) {
                    saw_other_range = true;
                    continue;
                }
                saw_requested_range = true;
            }
            (Some(_), Some(_)) => {
                saw_other_range = true;
                continue;
            }
            (Some(actual_start), None) if actual_start != start => {
                saw_other_range = true;
                continue;
            }
            _ => return true,
        }

        // Every requested window must carry exactly the six documented query
        // keys and the original/unadjusted daily-bar values. Extra or
        // duplicate keys are evidence corruption, not a reason to refetch.
        if file.request.endpoint != DAILY_RANGE_ENDPOINT
            || file.request.mode != FetchMode::Credentialed
            || file.request.query.len() != expected_query_keys.len()
        {
            return true;
        }
        let actual_query_keys: BTreeSet<&str> = file
            .request
            .query
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        if actual_query_keys != expected_query_keys {
            return true;
        }
        if query_value("FID_COND_MRKT_DIV_CODE") != Some("J")
            || query_value("FID_PERIOD_DIV_CODE") != Some("D")
            || query_value("FID_ORG_ADJ_PRC") != Some("1")
        {
            return true;
        }

        let Some(symbol) = query_value("FID_INPUT_ISCD") else {
            return true;
        };
        if !expected_symbols.contains(symbol) {
            // The fixed 11-symbol universe must be represented exactly; a
            // symbol outside it is ambiguous and must not trigger a refetch.
            return true;
        }
        requested_symbols.insert(symbol);
        let Some(window_end) = date2 else {
            return true;
        };
        if !requested_windows.insert((symbol, window_end)) {
            return true;
        }
        maximum_end = Some(maximum_end.map_or(window_end, |current| current.max(window_end)));

        let tr_cont_headers: Vec<_> = file
            .request
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("tr_cont"))
            .collect();
        let tr_id_headers: Vec<_> = file
            .request
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("tr_id"))
            .collect();
        if tr_cont_headers.len() != 1
            || !tr_cont_headers[0].1.is_empty()
            || tr_id_headers.len() != 1
            || tr_id_headers[0].1 != DAILY_RANGE_TR_ID
        {
            return true;
        }
    }

    // Do not mistake a different range for this one. If a batch mixes a
    // requested-range window with another range, treat it as ambiguous.
    if !saw_requested_range {
        return false;
    }
    if saw_other_range || requested_symbols != expected_symbols || maximum_end != Some(end) {
        return true;
    }
    true
}

fn parse_compact_date(value: &str) -> Option<TradingDate> {
    if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        TradingDate::parse(&format!(
            "{}-{}-{}",
            &value[0..4],
            &value[4..6],
            &value[6..8]
        ))
        .ok()
    } else {
        TradingDate::parse(value).ok()
    }
}

#[cfg(test)]
mod daily_range_source_selection_tests {
    use super::*;
    use domain::ContentHash;
    use market_data::contract::RequestMetadata;
    use market_data::storage::FileEntry;

    fn entry(batch_id: BatchId, date: TradingDate, entitlement: &str) -> ManifestEntry {
        ManifestEntry {
            batch_id,
            provider: PROVIDER_KIS_DAILY_RANGE.to_owned(),
            market: MARKET_KR.to_owned(),
            date,
            retrieved_at: UtcTimestamp::parse_rfc3339("2026-08-19T00:00:00Z").unwrap(),
            mode: FetchMode::Credentialed,
            entitlement_reference: Some(entitlement.to_owned()),
            files: Vec::new(),
        }
    }

    fn entry_with_bounds(
        batch_id: BatchId,
        date: TradingDate,
        entitlement: &str,
        start: TradingDate,
        end: TradingDate,
    ) -> ManifestEntry {
        let compact = |value: TradingDate| value.to_iso().replace('-', "");
        let mut value = entry(batch_id, date, entitlement);
        value.files.push(FileEntry {
            kind: ResponseKind::Bars,
            file_name: "symbol.json".to_owned(),
            content_hash: ContentHash::from_bytes(b"{}"),
            size_bytes: 2,
            request: RequestMetadata {
                endpoint: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice"
                    .to_owned(),
                query: vec![
                    ("FID_INPUT_DATE_1".to_owned(), compact(start)),
                    ("FID_INPUT_DATE_2".to_owned(), compact(end)),
                ],
                headers: vec![("tr_cont".to_owned(), String::new())],
                mode: FetchMode::Credentialed,
            },
        });
        value
    }

    fn valid_multi_window_entry(
        batch_id: BatchId,
        date: TradingDate,
        entitlement: &str,
        start: TradingDate,
        middle: TradingDate,
        end: TradingDate,
    ) -> ManifestEntry {
        let compact = |value: TradingDate| value.to_iso().replace('-', "");
        let mut value = entry(batch_id, date, entitlement);
        for symbol in KR_ETF_CORE_SYMBOLS {
            for (window, window_end) in [(1, middle), (2, end)] {
                let bytes = format!("{symbol}-{window}").into_bytes();
                value.files.push(FileEntry {
                    kind: ResponseKind::Bars,
                    file_name: format!("daily-bars-range-window-{window}-{symbol}.json"),
                    content_hash: ContentHash::from_bytes(&bytes),
                    size_bytes: bytes.len() as u64,
                    request: RequestMetadata {
                        endpoint: DAILY_RANGE_ENDPOINT.to_owned(),
                        query: vec![
                            ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                            ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
                            ("FID_INPUT_DATE_1".to_owned(), compact(start)),
                            ("FID_INPUT_DATE_2".to_owned(), compact(window_end)),
                            ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
                            ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
                        ],
                        headers: vec![
                            ("authorization".to_owned(), "[REDACTED]".to_owned()),
                            ("appkey".to_owned(), "[REDACTED]".to_owned()),
                            ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                            ("tr_id".to_owned(), DAILY_RANGE_TR_ID.to_owned()),
                            ("tr_cont".to_owned(), String::new()),
                        ],
                        mode: FetchMode::Credentialed,
                    },
                });
            }
        }
        value
    }

    #[test]
    fn preferred_batch_is_reused_even_when_metadata_requires_normalizer_failure() {
        let date = TradingDate::parse("2026-08-18").unwrap();
        let preferred = BatchId::generate();
        let other = BatchId::generate();
        let selected = select_existing_daily_range_source(
            vec![entry(other, date, "ent-1"), entry(preferred, date, "ent-1")],
            date,
            date,
            "ent-1",
            preferred,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.batch_id, preferred);
    }

    #[test]
    fn state_loss_with_one_other_exact_source_blocks_instead_of_refetching() {
        let date = TradingDate::parse("2026-08-18").unwrap();
        let other = BatchId::generate();
        let error = select_existing_daily_range_source(
            vec![entry_with_bounds(other, date, "ent-1", date, date)],
            date,
            date,
            "ent-1",
            BatchId::generate(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WorkerError::RangeNormalize(RangeNormalizeError::ExistingBatchConflict { batch_id, .. })
                if batch_id == other
        ));
    }

    #[test]
    fn state_loss_with_multiple_exact_sources_is_ambiguous_and_blocks() {
        let date = TradingDate::parse("2026-08-18").unwrap();
        let first = BatchId::generate();
        let second = BatchId::generate();
        let error = select_existing_daily_range_source(
            vec![
                entry_with_bounds(first, date, "ent-1", date, date),
                entry_with_bounds(second, date, "ent-1", date, date),
            ],
            date,
            date,
            "ent-1",
            BatchId::generate(),
        )
        .unwrap_err();
        match error {
            WorkerError::RangeNormalize(RangeNormalizeError::ExistingBatchConflict {
                batch_id,
                reason,
            }) => {
                assert_eq!(batch_id, first);
                assert!(reason.contains("2 existing range sources"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn state_loss_with_multi_window_source_blocks_before_any_refetch() {
        let start = TradingDate::parse("2020-01-31").unwrap();
        let middle = TradingDate::parse("2020-02-02").unwrap();
        let end = TradingDate::parse("2020-02-03").unwrap();
        let other = BatchId::generate();
        let error = select_existing_daily_range_source(
            vec![valid_multi_window_entry(
                other, start, "ent-1", start, middle, end,
            )],
            start,
            end,
            "ent-1",
            BatchId::generate(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WorkerError::RangeNormalize(RangeNormalizeError::ExistingBatchConflict {
                batch_id, ..
            }) if batch_id == other
        ));
    }

    #[test]
    fn explicit_existing_source_accepts_recorded_five_header_multi_window_contract() {
        let start = TradingDate::parse("2020-01-31").unwrap();
        let middle = TradingDate::parse("2020-02-02").unwrap();
        let end = TradingDate::parse("2020-02-03").unwrap();
        let source =
            valid_multi_window_entry(BatchId::generate(), start, "ent-1", start, middle, end);
        validate_explicit_daily_range_source(&source, start, end, "ent-1")
            .expect("exact source request contract is accepted");
    }

    #[test]
    fn explicit_existing_source_rejects_scope_and_query_tampering() {
        let start = TradingDate::parse("2020-01-31").unwrap();
        let end = TradingDate::parse("2020-02-03").unwrap();
        let mut source = valid_multi_window_entry(
            BatchId::generate(),
            start,
            "ent-1",
            start,
            TradingDate::parse("2020-02-02").unwrap(),
            end,
        );
        source.entitlement_reference = Some("other-entitlement".to_owned());
        assert!(validate_explicit_daily_range_source(&source, start, end, "ent-1").is_err());
        source.entitlement_reference = Some("ent-1".to_owned());
        source.files[0].request.headers[1].1 = "M".to_owned();
        assert!(validate_explicit_daily_range_source(&source, start, end, "ent-1").is_err());
    }

    #[test]
    fn no_exact_source_allows_the_preferred_batch_to_be_fetched_once() {
        let date = TradingDate::parse("2026-08-18").unwrap();
        assert!(
            select_existing_daily_range_source(
                vec![entry(BatchId::generate(), date, "different-entitlement")],
                date,
                date,
                "ent-1",
                BatchId::generate(),
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn a_different_range_is_not_treated_as_an_exact_state_loss_candidate() {
        let date = TradingDate::parse("2026-08-18").unwrap();
        let other_end = TradingDate::parse("2026-08-19").unwrap();
        assert!(
            select_existing_daily_range_source(
                vec![entry_with_bounds(
                    BatchId::generate(),
                    date,
                    "ent-1",
                    date,
                    other_end,
                )],
                date,
                date,
                "ent-1",
                BatchId::generate(),
            )
            .unwrap()
            .is_none()
        );
    }
}

struct CredentialedIngestContext<'a> {
    config: &'a ResearchWorkerConfig,
    store: &'a RawStore,
    sink: &'a PostgresPublicationSink,
    price_sink: &'a PostgresCandidateSourceSink,
    provider: &'a LiveKisProvider,
    normalized: &'a KisNormalizationRecoveryReport,
}

async fn run_credentialed_backfill_date(
    context: &CredentialedIngestContext<'_>,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<BatchId, WorkerError> {
    run_credentialed_target_ingest(context, date, now, false).await
}

async fn run_credentialed_internal_ingest(
    config: &ResearchWorkerConfig,
    store: &RawStore,
    sink: &PostgresPublicationSink,
    price_sink: &PostgresCandidateSourceSink,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<BatchId, WorkerError> {
    let provider = build_production_kis_provider(config)?;
    run_credentialed_internal_ingest_with_provider(
        config, store, sink, price_sink, &provider, date, now,
    )
    .await
}

async fn run_credentialed_internal_ingest_with_provider(
    config: &ResearchWorkerConfig,
    store: &RawStore,
    sink: &PostgresPublicationSink,
    price_sink: &PostgresCandidateSourceSink,
    provider: &LiveKisProvider,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<BatchId, WorkerError> {
    let normalized = recover_kis_normalization(store).map_err(WorkerError::Pipeline)?;
    recover_price_publications(config, store, price_sink).await?;
    let context = CredentialedIngestContext {
        config,
        store,
        sink,
        price_sink,
        provider,
        normalized: &normalized,
    };
    run_credentialed_target_ingest(&context, date, now, true).await
}

async fn run_credentialed_target_ingest(
    context: &CredentialedIngestContext<'_>,
    date: TradingDate,
    now: UtcTimestamp,
    full_price_recovery_after_ingest: bool,
) -> Result<BatchId, WorkerError> {
    // The range performs full normalization once, then repairs only this
    // target's durable normalized entry before its duplicate/holiday check.
    recover_unpublished_normalized_for_date(context.store, context.sink, context.normalized, date)
        .await
        .map_err(WorkerError::Pipeline)?;

    let has_eod = context
        .sink
        .has_eod_for_mode(date, FetchMode::Credentialed)
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::DuplicateCheck,
            source,
        })?;
    if has_eod {
        return context
            .normalized
            .outcomes
            .iter()
            .rev()
            .find(|outcome| outcome.entry.date == date)
            .map(|outcome| outcome.entry.batch_id)
            .ok_or(WorkerError::ChildOutput {
                phase: WorkerPhase::DuplicateCheck,
            });
    }

    // A KIS calendar-confirmed closure is a durable no-price result, not a
    // transient provider miss.  Keep the normalized batch as the idempotency
    // key and do not fetch the same holiday again on every worker wake-up.
    if let Some(outcome) = context
        .normalized
        .outcomes
        .iter()
        .rev()
        .find(|outcome| outcome.entry.date == date)
    {
        let (calendar, _) = curation_inputs_from_raw(context.store, &outcome.entry)
            .map_err(WorkerError::Curation)?;
        if !calendar.is_session(date) {
            return Ok(outcome.entry.batch_id);
        }
    }

    let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
    let outcome = ingest_normalize_publish_kis(
        context.store,
        context.provider,
        &request,
        Some(&context.config.entitlement_reference),
        context.sink,
    )
    .await
    .map_err(WorkerError::Pipeline)?;
    // Price curation is deliberately after canonical EOD publication.  A
    // crash between these two steps leaves the immutable normalized batch for
    // the next recovery pass, which will replay the exact Curated generation
    // and candidate-price publication without fetching KIS again.
    if full_price_recovery_after_ingest {
        recover_price_publication_for_entry(
            context.config,
            context.store,
            context.price_sink,
            &outcome.manifest,
        )
        .await?;
    }
    Ok(outcome.manifest.batch_id)
}

async fn run_candidate_source_ingest(
    config: &ResearchWorkerConfig,
    pool: PgPool,
    date: TradingDate,
    now: UtcTimestamp,
) -> Result<Option<BatchId>, WorkerError> {
    let sink = PostgresCandidateSourceSink::new(pool);
    let store = RawStore::new(&config.candidate_raw_root);
    recover_candidate_batches(&store, &sink)
        .await
        .map_err(WorkerError::CandidatePipeline)?;
    let missing_by_universe = sink
        .missing_source_kinds_by_universe(date, now.as_datetime(), config.fetch_mode)
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::DuplicateCheck,
            source,
        })?;
    if missing_by_universe.is_empty() || missing_by_universe.values().all(Vec::is_empty) {
        return Ok(None);
    }
    let provider: Arc<dyn EodProvider> = match config.fetch_mode {
        FetchMode::Synthetic => Arc::new(KrxProvider::synthetic(
            RecordedBundle::open(&config.candidate_synthetic_bundle)
                .map_err(WorkerError::Provider)?,
        )),
        FetchMode::Credentialed => return Err(WorkerError::ProviderNotConfigured),
    };
    let request = IngestRequest::new(MARKET_KR.to_owned(), date, now);
    let outcome = ingest_bundle_with_kinds(
        &store,
        provider.as_ref(),
        &request,
        Some(&config.entitlement_reference),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .map_err(|error| WorkerError::CandidatePipeline(CandidatePipelineError::Ingest(error)))?;
    let bindings = sink
        .catalog_candidate_batch(&outcome)
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::Publication,
            source,
        })?;
    let batch = prepare_candidate_batch(&outcome, date, now, &bindings)
        .map_err(WorkerError::CandidatePipeline)?;
    publish_candidate_batch(&sink, &batch)
        .await
        .map_err(WorkerError::CandidatePipeline)?;
    Ok(Some(outcome.batch_id))
}

async fn recover_price_publications(
    config: &ResearchWorkerConfig,
    raw: &RawStore,
    sink: &PostgresCandidateSourceSink,
) -> Result<Option<BatchId>, WorkerError> {
    let dataset_id = DatasetId::parse("krx_eod_bars").map_err(|_| WorkerError::InvalidConfig {
        key: "RESEARCH_CURATED_ROOT",
    })?;
    let curated = CurateStore::new(&config.curated_root);
    let storage_path = config
        .curated_root
        .to_str()
        .ok_or(WorkerError::InvalidConfig {
            key: "RESEARCH_CURATED_ROOT",
        })?;
    let provider = match config.fetch_mode {
        FetchMode::Synthetic => PROVIDER_KRX,
        FetchMode::Credentialed => PROVIDER_KIS_NORMALIZED,
    };
    let entries = raw
        .read_reconciled_manifest(provider, MARKET_KR)
        .map_err(|source| WorkerError::Pipeline(PipelineError::Manifest { source }))?;
    let mut by_date = BTreeMap::<TradingDate, market_data::ManifestEntry>::new();
    for entry in entries {
        if !raw_batch_has_target_bars(raw, &entry)? {
            continue;
        }
        match by_date.get(&entry.date) {
            Some(previous)
                if previous.retrieved_at > entry.retrieved_at
                    || (previous.retrieved_at == entry.retrieved_at
                        && previous.batch_id.to_string() >= entry.batch_id.to_string()) => {}
            _ => {
                by_date.insert(entry.date, entry);
            }
        }
    }
    let entries = by_date.into_values().collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(None);
    }
    let anchor = entries.last().expect("nonempty cumulative source set");
    let contract_reference = anchor
        .entitlement_reference
        .as_deref()
        .filter(|reference| !reference.trim().is_empty())
        .ok_or_else(|| {
            WorkerError::Curation(CurateError::MalformedManifest {
                context: "candidate price entitlement".to_owned(),
                detail: "Raw EOD batch has no governing contract reference".to_owned(),
            })
        })?;
    if entries.iter().any(|entry| {
        entry
            .entitlement_reference
            .as_deref()
            .filter(|reference| !reference.trim().is_empty())
            != Some(contract_reference)
    }) {
        return Err(WorkerError::Curation(CurateError::MalformedManifest {
            context: "cumulative candidate price entitlement".to_owned(),
            detail: "one immutable generation cannot mix Raw contract references".to_owned(),
        }));
    }

    let (calendar, master) =
        curation_inputs_from_raw_entries(raw, &entries).map_err(WorkerError::Curation)?;
    let manifest = match curated
        .latest_manifest(&dataset_id)
        .map_err(WorkerError::Curation)?
    {
        // Legacy manifests predate exact artifact references. They remain
        // parseable for migration, but must never be reused as a production
        // generation: re-curate the same immutable Raw snapshot into the next
        // version so every consumer can verify exact files, hashes and schema.
        Some(manifest)
            if !manifest.artifacts.is_empty() && manifest_matches_entries(&manifest, &entries) =>
        {
            manifest
        }
        _ => {
            curate_generation(
                raw,
                &entries,
                &calendar,
                &master,
                &curated,
                &CurateRequest {
                    dataset_id: &dataset_id,
                    market: MARKET_KR,
                    source: &anchor.provider,
                    now: entries
                        .iter()
                        .map(|entry| entry.retrieved_at)
                        .max()
                        .expect("nonempty cumulative source set"),
                },
            )
            .map_err(WorkerError::Curation)?
            .manifest
        }
    };
    let evidence = price_curation_evidence_for_generation(raw, &entries, &manifest, anchor)
        .map_err(WorkerError::Curation)?;
    if evidence.last_session != anchor.date {
        return Err(WorkerError::Curation(CurateError::MalformedManifest {
            context: "candidate price publication".to_owned(),
            detail: "cumulative Raw EOD source omits its latest target session".to_owned(),
        }));
    }
    let entitlement_id = match sink
        .resolve_price_dataset_entitlement(
            contract_reference,
            evidence.first_session,
            evidence.last_session,
        )
        .await
    {
        Ok(entitlement_id) => entitlement_id,
        Err(original) => {
            let mut pending = Vec::new();
            for entry in &entries {
                if !sink
                    .raw_batch_is_terminal(entry, "price")
                    .await
                    .map_err(|source| WorkerError::Database {
                        phase: WorkerPhase::Recovery,
                        source,
                    })?
                {
                    pending.push(entry.clone());
                }
            }
            for entry in &pending {
                if sink
                    .block_raw_batch_for_inactive_rights(
                        entry,
                        "price",
                        // Persist the exact decision window used for this
                        // cumulative attempt.  Revalidation later may ask
                        // for a wider window, but must include this original
                        // blocked scope.
                        evidence.first_session,
                        evidence.last_session,
                    )
                    .await
                    .is_err()
                {
                    return Err(WorkerError::Database {
                        phase: WorkerPhase::Publication,
                        source: original,
                    });
                }
            }
            return Ok(None);
        }
    };

    // A renewed price entitlement may arrive after one or more exact Raw
    // deliveries were terminally blocked.  Re-open only those exact
    // entitlement-inactive rows; any other terminal reason remains a hard
    // failure in the database procedure.
    for entry in &entries {
        sink.revalidate_price_raw_batch_after_rights(
            entry,
            // Revalidation uses the current cumulative requested window; the
            // database checks that it fully contains the original blocked
            // window stored on this exact Raw ledger row.
            evidence.first_session,
            evidence.last_session,
            entitlement_id,
        )
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::Recovery,
            source,
        })?;
    }
    let mut pending = Vec::new();
    for entry in &entries {
        if !sink
            .raw_batch_is_terminal(entry, "price")
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::Recovery,
                source,
            })?
        {
            pending.push(entry.clone());
        }
    }
    if pending.is_empty() {
        return Ok(None);
    }
    // The canonical instrument master must exist before the price publication
    // writes coverage: `publish_candidate_price_publication` inserts into
    // `candidate_price_instrument_coverage`, whose `instrument_id` is a foreign
    // key onto `instruments`. Commit 84e6ce1 removed this registration from the
    // price path while keeping that insert, and left no other production writer
    // for the table -- `register_candidate_instruments` had zero callers -- so
    // the fixed ETF price publication could never complete against an empty
    // `instruments`. The comment it added, that candidate instrument
    // registration is "not required for the fixed ETF price dataset", is what
    // the database contradicts.
    let reference_sha256 = anchor
        .files
        .iter()
        .find(|file| file.kind == market_data::ResponseKind::Reference)
        .and_then(|file| file.content_hash.as_str().strip_prefix("sha256:"))
        .ok_or_else(|| {
            WorkerError::Curation(CurateError::MalformedManifest {
                context: "candidate instrument catalog".to_owned(),
                detail: "Raw EOD batch has no exact reference hash".to_owned(),
            })
        })?;
    let source_revision = anchor.batch_id.to_string();
    sink.register_candidate_instruments(&CandidateInstrumentCatalog {
        master: &master,
        entitlement_id,
        contract_reference,
        entitlement_date: anchor.date,
        reference_sha256,
        source_revision: &source_revision,
        retrieved_at: anchor.retrieved_at,
        // Not the master's listing date: that is a fallback derived from the
        // sessions in this generation and therefore moves as the cumulative
        // window widens, while the registration can never overwrite what it
        // first stored.  See CandidateInstrumentCatalog::coverage_from.
        coverage_from: TradingDate::parse(market_data::range_normalize::APPROVED_EFFECTIVE_FROM)
            .expect("approved universe coverage floor is a valid date"),
    })
    .await
    .map_err(|source| WorkerError::Database {
        phase: WorkerPhase::Publication,
        source,
    })?;
    let raw_manifest_sha256 = crate::candidate_sink::candidate_raw_manifest_sha256(anchor)
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::Publication,
            source,
        })?;
    let (dataset_version_id, anchor_outcome) = sink
        .publish_price(&CandidatePricePublication {
            raw_batch_id: anchor.batch_id.as_uuid(),
            raw_manifest_sha256: &raw_manifest_sha256,
            fetch_mode: anchor.mode,
            entitlement_date: anchor.date,
            evidence: &evidence,
            dataset_version: &manifest.version.to_string(),
            storage_path,
            provider: &anchor.provider,
            entitlement_id,
            license_ref: contract_reference,
            available_at: anchor.retrieved_at,
            retrieved_at: anchor.retrieved_at,
        })
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::Publication,
            source,
        })?;
    let mut published_batch =
        (anchor_outcome == crate::PublishOutcome::Published).then_some(anchor.batch_id);
    for entry in pending
        .iter()
        .filter(|entry| entry.batch_id != anchor.batch_id)
    {
        let raw_manifest_sha256 = crate::candidate_sink::candidate_raw_manifest_sha256(entry)
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::Publication,
                source,
            })?;
        let outcome = sink
            .bind_price_batch_to_existing_generation(
                entry.batch_id.as_uuid(),
                &raw_manifest_sha256,
                entry.mode,
                contract_reference,
                entry.date,
                dataset_version_id,
            )
            .await
            .map_err(|source| WorkerError::Database {
                phase: WorkerPhase::Publication,
                source,
            })?;
        if outcome == crate::PublishOutcome::Published {
            published_batch = Some(entry.batch_id);
        }
    }
    Ok(published_batch)
}

fn manifest_matches_entries(
    manifest: &market_data::DatasetManifest,
    entries: &[market_data::ManifestEntry],
) -> bool {
    if manifest.source_batches.len() != entries.len() {
        return false;
    }
    entries.iter().all(|entry| {
        let Some(bars) = entry
            .files
            .iter()
            .find(|file| file.kind == market_data::ResponseKind::Bars)
        else {
            return false;
        };
        let Some(actions) = entry
            .files
            .iter()
            .find(|file| file.kind == market_data::ResponseKind::CorporateActions)
        else {
            return false;
        };
        manifest.source_batches.iter().any(|source| {
            source.batch_id == entry.batch_id
                && source.bars_file == bars.file_name
                && source.bars_hash == bars.content_hash
                && source.actions_file == actions.file_name
                && source.actions_hash == actions.content_hash
        })
    })
}

/// The target-ingest path may ask for a narrow post-fetch recovery. The
/// durable Curated contract is cumulative, so the narrow entry is only a
/// trigger; recovery still reconciles the complete Raw history before
/// publishing one immutable generation. This keeps the O(1) target wake-up
/// from silently creating a one-day dataset pin.
async fn recover_price_publication_for_entry(
    config: &ResearchWorkerConfig,
    raw: &RawStore,
    sink: &PostgresCandidateSourceSink,
    _entry: &market_data::ManifestEntry,
) -> Result<(), WorkerError> {
    recover_price_publications(config, raw, sink)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod price_recovery_contract_tests {
    #[test]
    fn cumulative_recovery_revalidates_blocked_price_and_registers_the_instrument_catalog() {
        let source = include_str!("worker.rs");
        let function = source
            .split_once("async fn recover_price_publications(")
            .and_then(|(_, rest)| rest.split_once("fn manifest_matches_entries("))
            .map(|(body, _)| body)
            .expect("price recovery function source");
        assert!(function.contains("resolve_price_dataset_entitlement"));
        assert!(function.contains("revalidate_price_raw_batch_after_rights"));
        assert!(function.contains("!manifest.artifacts.is_empty()"));
        assert!(!function.contains("resolve_contract_entitlement("));
        // The database decides this, not the comment that used to sit here:
        // publish_candidate_price_publication writes
        // candidate_price_instrument_coverage, whose instrument_id is a foreign
        // key onto instruments, and no other production path writes that table.
        assert!(function.contains("register_candidate_instruments("));
        // The floor persisted as instruments.listed_at must not come from the
        // curation master, whose fallback moves as the cumulative window widens
        // while the registration can never overwrite what it first stored.
        assert!(function.contains("coverage_from: TradingDate::parse("));
        assert!(!function.contains("coverage_from: instrument"));
        assert!(!function.contains(
            "the latest source date is already terminal while an older cumulative source is pending"
        ));
        assert!(function.contains("block_raw_batch_for_inactive_rights"));
        assert!(function.contains("return Ok(None)"));
        let resolve = function
            .find("let entitlement_id = match sink")
            .expect("price resolver branch");
        let revalidate = function
            .find("revalidate_price_raw_batch_after_rights")
            .expect("price revalidation branch");
        assert!(resolve < revalidate, "active resolver must precede reopen");
    }
}

fn raw_batch_has_target_bars(
    raw: &RawStore,
    entry: &market_data::storage::ManifestEntry,
) -> Result<bool, WorkerError> {
    let metadata = entry
        .files
        .iter()
        .find(|file| file.kind == market_data::ResponseKind::Bars)
        .ok_or(WorkerError::Curation(CurateError::MissingFile {
            kind: market_data::ResponseKind::Bars,
        }))?;
    let files = raw
        .read_batch_bytes(&entry.provider, &entry.market, entry)
        .map_err(|source| {
            WorkerError::Curation(CurateError::RawStore {
                context: "read price recovery batch".to_owned(),
                source: Box::new(source),
            })
        })?;
    let bytes = files
        .iter()
        .find(|file| file.file_name == metadata.file_name)
        .ok_or(WorkerError::Curation(CurateError::MissingFile {
            kind: market_data::ResponseKind::Bars,
        }))?;
    let document =
        market_data::curate::parse::parse_bars(&bytes.bytes).map_err(WorkerError::Curation)?;
    Ok(document
        .bars
        .iter()
        .any(|bar| bar.date == entry.date.to_iso()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStatus {
    pub newest_eod_at: DateTime<Utc>,
    pub age: Duration,
    pub per_universe: BTreeMap<String, Vec<String>>,
}

pub async fn healthcheck(
    pool: &PgPool,
    now_utc: DateTime<Utc>,
    max_age: Duration,
    expected_fetch_mode: FetchMode,
) -> Result<HealthStatus, WorkerError> {
    timeout_query(
        WorkerPhase::Health,
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
    )
    .await?;
    let kst = FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST offset is valid");
    let current_kst_date = now_utc.with_timezone(&kst).date_naive();
    // Legacy synthetic rows predate the nullable provenance columns. They may
    // satisfy the synthetic compatibility path, but never the credentialed
    // path, which must see an explicitly credentialed EOD publication.
    let newest: Option<(chrono::NaiveDate, DateTime<Utc>)> = timeout_query(
        WorkerPhase::Health,
        sqlx::query_as(
            "SELECT batch_date, retrieved_at FROM data_batches \
             WHERE provider='KRX' AND market='KR' AND kind='EOD' \
               AND (fetch_mode = $2 OR ($2 = 'synthetic' AND fetch_mode IS NULL)) \
               AND batch_date <= $1 \
             ORDER BY batch_date DESC, retrieved_at DESC LIMIT 1",
        )
        .bind(current_kst_date)
        .bind(expected_fetch_mode.as_str())
        .fetch_optional(pool),
    )
    .await?;
    let (batch_date, newest_eod_at) = newest.ok_or(WorkerError::Unhealthy {
        reason: HealthFailure::NoEodPublication,
    })?;
    let (newest_eod_at, age) =
        publication_freshness_for_batch(now_utc, batch_date, newest_eod_at, max_age)
            .map_err(|reason| WorkerError::Unhealthy { reason })?;
    Ok(HealthStatus {
        newest_eod_at,
        age,
        per_universe: BTreeMap::new(),
    })
}

pub async fn candidate_healthcheck(
    pool: &PgPool,
    curated_root: &Path,
    now_utc: DateTime<Utc>,
    max_age: Duration,
    expected_fetch_mode: FetchMode,
    _run_at_kst: NaiveTime,
) -> Result<HealthStatus, WorkerError> {
    let kst = FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST offset is valid");
    let now_kst = now_utc.with_timezone(&kst);
    let current_date = now_kst.date_naive();
    // Daemon wake-up time is operator configurable, but market-data
    // confirmation is a shared product contract with the API: 16:30 KST.
    let include_current_session = now_kst.time() >= CANDIDATE_CONFIRMED_CLOSE_KST;
    let expected_session: Option<chrono::NaiveDate> = timeout_query(
        WorkerPhase::Health,
        sqlx::query_scalar(
            "SELECT calendar.session_date FROM trading_calendars AS calendar
              WHERE calendar.exchange='KRX' AND calendar.session_type='TRADING'
                AND calendar.timezone='Asia/Seoul' AND calendar.session_date <= $1
                AND (calendar.session_date < $1 OR $2)
                AND calendar.source_batch_id IS NOT NULL
                AND calendar.content_sha256 IS NOT NULL AND calendar.retrieved_at IS NOT NULL
              ORDER BY calendar.session_date DESC LIMIT 1",
        )
        .bind(current_date)
        .bind(include_current_session)
        .fetch_optional(pool),
    )
    .await?;
    let expected_session = expected_session.ok_or(WorkerError::Unhealthy {
        reason: HealthFailure::NoEodPublication,
    })?;
    let expected_eod: Option<(chrono::NaiveDate, DateTime<Utc>)> = timeout_query(
        WorkerPhase::Health,
        sqlx::query_as(
            "SELECT batch.batch_date, batch.retrieved_at
               FROM data_batches AS batch
              WHERE batch.provider = 'KRX' AND batch.market = 'KR' AND batch.kind = 'EOD'
                AND batch.fetch_mode = $3
                AND batch.batch_date = $1 AND batch.retrieved_at <= $2
              ORDER BY batch.retrieved_at DESC LIMIT 1",
        )
        .bind(expected_session)
        .bind(now_utc)
        .bind(expected_fetch_mode.as_str())
        .fetch_optional(pool),
    )
    .await?;
    let (source_date, _) = expected_eod.ok_or(WorkerError::Unhealthy {
        reason: HealthFailure::NoEodPublication,
    })?;
    let expected_trading_date = TradingDate::new(
        expected_session.year(),
        expected_session.month(),
        expected_session.day(),
    )
    .map_err(|_| WorkerError::Unhealthy {
        reason: HealthFailure::NoCandidatePublication,
    })?;
    let candidate_source_sink = PostgresCandidateSourceSink::new(pool.clone());
    let missing_by_universe = candidate_source_sink
        .missing_source_kinds_by_universe(expected_trading_date, now_utc, expected_fetch_mode)
        .await
        .map_err(|source| WorkerError::Database {
            phase: WorkerPhase::Health,
            source,
        })?;
    if missing_by_universe.is_empty()
        || missing_by_universe
            .values()
            .any(|missing| !missing.is_empty())
    {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::CandidateUniverseUnavailable,
        });
    }
    let per_universe = missing_by_universe
        .into_iter()
        .map(|(universe, missing)| {
            (
                universe.to_string(),
                missing.into_iter().map(|kind| kind.to_string()).collect(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    #[derive(sqlx::FromRow)]
    struct CandidateSourceHealthRow {
        universe_key: String,
        universe_id: uuid::Uuid,
        flow_dataset_version_id: uuid::Uuid,
        status_dataset_version_id: uuid::Uuid,
        fundamental_dataset_version_id: uuid::Uuid,
        sector_version_id: uuid::Uuid,
        flow_entitlement_id: uuid::Uuid,
        flow_license_ref: String,
        retrieved_at: DateTime<Utc>,
    }
    let source: Vec<CandidateSourceHealthRow> = timeout_query(
        WorkerPhase::Health,
        sqlx::query_as(
            "SELECT registry.universe_key,
                    universe.id AS universe_id,
                    flow.dataset_version_id AS flow_dataset_version_id,
                    status.dataset_version_id AS status_dataset_version_id,
                    fact.dataset_version_id AS fundamental_dataset_version_id,
                    sector.id AS sector_version_id,
                    flow.entitlement_id AS flow_entitlement_id,
                    flow.license_ref AS flow_license_ref,
                    LEAST(flow.retrieved_at, status.retrieved_at) AS retrieved_at
               FROM candidate_universe_registry AS registry
               CROSS JOIN LATERAL (
                    SELECT id, retrieved_at FROM candidate_universe_snapshots
                     WHERE index_id = registry.universe_key
                       AND as_of_date <= $1 AND available_at <= $2
                       AND member_count = (
                           SELECT count(*) FROM candidate_universe_members AS member
                            WHERE member.universe_snapshot_id = candidate_universe_snapshots.id
                              AND member.effective_from <= $1
                              AND (member.effective_until IS NULL OR member.effective_until >= $1))
                       AND public.candidate_source_entitlement_is_valid(
                           entitlement_id, license_ref, registry.membership_dataset_id, $1, $1)
                       AND EXISTS (
                           SELECT 1 FROM candidate_raw_batch_datasets AS binding
                           JOIN candidate_raw_batch_publications AS batch
                             ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                          WHERE binding.dataset_version_id=candidate_universe_snapshots.dataset_version_id
                            AND binding.response_kind='index_membership'
                            AND binding.dataset_id=registry.membership_dataset_id
                            AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
                     ORDER BY as_of_date DESC, available_at DESC, id LIMIT 1
               ) AS universe
               CROSS JOIN LATERAL (
                    SELECT member.dataset_version_id, member.entitlement_id,
                           member.license_ref, member.retrieved_at
                      FROM candidate_investor_flows AS flow
                      JOIN candidate_investor_flow_snapshot_rows AS member
                        ON member.flow_observation_id=flow.id
                     WHERE flow.trade_date = $1 AND member.entitlement_date = $1
                       AND flow.available_at <= $2
                       AND public.candidate_source_entitlement_is_valid(
                           member.entitlement_id, member.license_ref, 'krx_investor_flows', $1, $1)
                       AND EXISTS (
                           SELECT 1 FROM candidate_raw_batch_datasets AS binding
                           JOIN candidate_raw_batch_publications AS batch
                             ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                          WHERE binding.dataset_version_id=member.dataset_version_id
                            AND binding.response_kind='investor_flow'
                            AND binding.dataset_id='krx_investor_flows'
                            AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
                     ORDER BY flow.available_at DESC, flow.id LIMIT 1
               ) AS flow
               CROSS JOIN LATERAL (
                    SELECT dataset_version_id, retrieved_at FROM candidate_market_status_observations
                     WHERE trade_date = $1 AND entitlement_date = $1 AND available_at <= $2
                       AND public.candidate_source_entitlement_is_valid(
                           entitlement_id, license_ref, 'krx_market_status', $1, $1)
                       AND EXISTS (
                           SELECT 1 FROM candidate_raw_batch_datasets AS binding
                           JOIN candidate_raw_batch_publications AS batch
                             ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                          WHERE binding.dataset_version_id=candidate_market_status_observations.dataset_version_id
                            AND binding.response_kind='market_status'
                            AND binding.dataset_id='krx_market_status'
                            AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
                     ORDER BY available_at DESC, id LIMIT 1
               ) AS status
               CROSS JOIN LATERAL (
                    SELECT dataset_version_id, retrieved_at FROM candidate_fundamental_observations
                     WHERE fiscal_period_end <= $1 AND available_at <= $2
                       AND public.candidate_source_entitlement_is_valid(
                           entitlement_id, license_ref, 'krx_fundamentals', $1, $1)
                       AND EXISTS (
                           SELECT 1 FROM candidate_raw_batch_datasets AS binding
                           JOIN candidate_raw_batch_publications AS batch
                             ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                          WHERE binding.dataset_version_id=candidate_fundamental_observations.dataset_version_id
                            AND binding.response_kind='fundamentals'
                            AND binding.dataset_id='krx_fundamentals'
                            AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
                     ORDER BY available_at DESC, id LIMIT 1
               ) AS fact
               CROSS JOIN LATERAL (
                    SELECT id, retrieved_at FROM candidate_sector_versions
                     WHERE effective_from <= $1 AND available_at <= $2
                       AND public.candidate_source_entitlement_is_valid(
                           entitlement_id, license_ref, 'krx_sector_classification', $1, $1)
                       AND EXISTS (
                           SELECT 1 FROM candidate_raw_batch_datasets AS binding
                           JOIN candidate_raw_batch_publications AS batch
                             ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
                          WHERE binding.dataset_version_id=candidate_sector_versions.dataset_version_id
                            AND binding.response_kind='sector_classification'
                            AND binding.dataset_id='krx_sector_classification'
                            AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
                     ORDER BY effective_from DESC, available_at DESC, id LIMIT 1
               ) AS sector
              WHERE registry.enabled",
        )
        .bind(source_date)
        .bind(now_utc)
        .bind(expected_fetch_mode.as_str())
        .fetch_all(pool),
    )
    .await?;
    if source.is_empty() {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::NoCandidatePublication,
        });
    }
    if source.len() != per_universe.len() {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::CandidateUniverseUnavailable,
        });
    }
    let source_universes = source
        .iter()
        .map(|row| row.universe_key.as_str())
        .collect::<BTreeSet<_>>();
    if source_universes.len() != per_universe.len()
        || source_universes
            .iter()
            .any(|universe| !per_universe.contains_key(*universe))
    {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::CandidateUniverseUnavailable,
        });
    }
    let source_retrieved_at = source
        .iter()
        .map(|row| row.retrieved_at)
        .min()
        .expect("nonempty source health rows");
    let (newest_eod_at, age) =
        market_data::freshness::applicable_eod_freshness(now_utc, source_date, source_retrieved_at)
            .ok_or(WorkerError::Unhealthy {
                reason: HealthFailure::FutureCandidatePublication,
            })?;
    if age > max_age {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::StaleCandidatePublication,
        });
    }
    #[derive(sqlx::FromRow)]
    struct PriceHealthRow {
        dataset_version: String,
        manifest_sha256: String,
        curated_generation: i64,
        storage_path: String,
        retrieved_at: DateTime<Utc>,
        available_at: DateTime<Utc>,
    }
    const PRICE_HEALTH_QUERY: &str = "WITH required_sessions AS MATERIALIZED (
             SELECT calendar.session_date
               FROM trading_calendars AS calendar
              WHERE calendar.exchange = 'KRX'
                AND calendar.session_type = 'TRADING'
                AND calendar.timezone = 'Asia/Seoul'
                AND calendar.session_date <= $1
                AND calendar.source_batch_id IS NOT NULL
                AND calendar.content_sha256 IS NOT NULL
                AND calendar.retrieved_at IS NOT NULL
              ORDER BY calendar.session_date DESC LIMIT 60
         )
         SELECT price.dataset_version, price.manifest_sha256,
                price.curated_generation, dataset.storage_path,
                price.retrieved_at, price.available_at
           FROM candidate_price_publications AS price
           JOIN dataset_versions AS dataset ON dataset.id = price.dataset_version_id
          WHERE dataset.dataset_id = 'krx_eod_bars'
            AND dataset.status IN ('READY', 'WARNING')
            AND price.first_session <= $1 AND price.last_session >= $1
            AND (SELECT count(*) FROM required_sessions) = 60
            AND (
                SELECT count(*) FROM candidate_universe_members AS member
                 WHERE member.universe_snapshot_id = $2
                   AND member.effective_from <= $1
                   AND (member.effective_until IS NULL OR member.effective_until >= $1)
                   AND NOT EXISTS (
                       SELECT 1 FROM required_sessions AS required
                        WHERE NOT EXISTS (
                            SELECT 1 FROM candidate_price_instrument_sessions AS coverage_session
                             WHERE coverage_session.dataset_version_id = price.dataset_version_id
                               AND coverage_session.instrument_id = member.instrument_id
                               AND coverage_session.session_date = required.session_date))
                   AND NOT EXISTS (
                       SELECT 1 FROM required_sessions AS required
                       CROSS JOIN (VALUES ('FOREIGN'),('INSTITUTION')) AS class(investor_class)
                        WHERE NOT EXISTS (
                            SELECT 1 FROM candidate_investor_flows AS history
                            JOIN candidate_investor_flow_snapshot_rows AS flow_member
                              ON flow_member.flow_observation_id=history.id
                             WHERE flow_member.dataset_version_id = $3
                               AND history.instrument_id = member.instrument_id
                               AND history.trade_date = required.session_date
                               AND history.investor_class = class.investor_class
                               AND history.available_at <= $4))
                   AND EXISTS (
                       SELECT 1 FROM candidate_market_status_observations AS member_status
                        WHERE member_status.dataset_version_id = $5
                          AND member_status.instrument_id = member.instrument_id
                          AND member_status.trade_date = $1
                          AND member_status.available_at <= $4)
                   AND EXISTS (
                       SELECT 1 FROM candidate_fundamental_observations AS member_fact
                        WHERE member_fact.dataset_version_id = $6
                          AND member_fact.instrument_id = member.instrument_id
                          AND member_fact.fiscal_period_end <= $1
                          AND member_fact.available_at <= $4)
                   AND EXISTS (
                       SELECT 1 FROM candidate_sector_entries AS member_sector
                        WHERE member_sector.sector_version_id = $7
                          AND member_sector.instrument_id = member.instrument_id
                          AND member_sector.effective_from <= $1
                          AND member_sector.available_at <= $4
                          AND (member_sector.effective_until IS NULL
                               OR member_sector.effective_until >= $1))
            ) >= 5
            AND public.price_dataset_entitlement_is_valid(
                price.entitlement_id, price.license_ref,
                price.first_session, price.last_session)
            AND public.candidate_source_entitlement_is_valid(
                $9, $10, 'krx_investor_flows',
                (SELECT min(session_date) FROM required_sessions), $1)
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=price.dataset_version_id
                 AND binding.dataset_id='krx_eod_bars'
                 AND binding.response_kind='bars'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$8)
          ORDER BY price.available_at DESC, dataset.id LIMIT 1";
    let mut prices = Vec::with_capacity(source.len());
    let mut price_age = Duration::ZERO;
    for source in &source {
        let price: Option<PriceHealthRow> = timeout_query(
            WorkerPhase::Health,
            sqlx::query_as(PRICE_HEALTH_QUERY)
                .bind(source_date)
                .bind(source.universe_id)
                .bind(source.flow_dataset_version_id)
                .bind(now_utc)
                .bind(source.status_dataset_version_id)
                .bind(source.fundamental_dataset_version_id)
                .bind(source.sector_version_id)
                .bind(expected_fetch_mode.as_str())
                .bind(source.flow_entitlement_id)
                .bind(&source.flow_license_ref)
                .fetch_optional(pool),
        )
        .await?;
        let Some(price) = price else {
            return Err(WorkerError::Unhealthy {
                reason: HealthFailure::CandidateUniverseUnavailable,
            });
        };
        if price.available_at > now_utc {
            return Err(WorkerError::Unhealthy {
                reason: HealthFailure::FuturePricePublication,
            });
        }
        let (_, age) = market_data::freshness::applicable_eod_freshness(
            now_utc,
            source_date,
            price.retrieved_at,
        )
        .ok_or(WorkerError::Unhealthy {
            reason: HealthFailure::FuturePricePublication,
        })?;
        if age > max_age {
            return Err(WorkerError::Unhealthy {
                reason: HealthFailure::StalePricePublication,
            });
        }
        price_age = price_age.max(age);
        prices.push(price);
    }
    let price = prices
        .first()
        .expect("every enabled universe has a price health row");
    if prices.iter().skip(1).any(|candidate| {
        candidate.dataset_version != price.dataset_version
            || candidate.manifest_sha256 != price.manifest_sha256
            || candidate.curated_generation != price.curated_generation
            || candidate.storage_path != price.storage_path
            || candidate.retrieved_at != price.retrieved_at
            || candidate.available_at != price.available_at
    }) {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::CandidateUniverseUnavailable,
        });
    }
    let generation =
        u32::try_from(price.curated_generation).map_err(|_| WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch,
        })?;
    let configured_root = curated_root.to_str().ok_or(WorkerError::Unhealthy {
        reason: HealthFailure::PriceManifestMismatch,
    })?;
    let dataset_id = DatasetId::parse("krx_eod_bars").map_err(|_| WorkerError::Unhealthy {
        reason: HealthFailure::PriceManifestMismatch,
    })?;
    let store = CurateStore::new(curated_root);
    let manifest = store
        .read_dataset_manifest(&dataset_id, generation)
        .map_err(|_| WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch,
        })?
        .ok_or(WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch,
        })?;
    let computed =
        market_data::dataset_manifest_hash(&manifest).map_err(|_| WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch,
        })?;
    if price.storage_path != configured_root
        || price.dataset_version != generation.to_string()
        || manifest.version != generation
        || manifest.dataset_id != dataset_id
        || manifest.bar_count == 0
        || manifest.content_hash != computed
        || computed.as_str().strip_prefix("sha256:") != Some(price.manifest_sha256.as_str())
    {
        return Err(WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch,
        });
    }
    store
        .verify_artifacts(&manifest)
        .map_err(|_| WorkerError::Unhealthy {
            reason: HealthFailure::PriceManifestMismatch,
        })?;
    Ok(HealthStatus {
        newest_eod_at,
        age: age.max(price_age),
        per_universe,
    })
}

fn publication_freshness_for_batch(
    now_utc: DateTime<Utc>,
    batch_date: chrono::NaiveDate,
    retrieved_at: DateTime<Utc>,
    max_age: Duration,
) -> Result<(DateTime<Utc>, Duration), HealthFailure> {
    let (effective_at, age) =
        market_data::freshness::applicable_eod_freshness(now_utc, batch_date, retrieved_at)
            .ok_or(HealthFailure::FutureEodPublication)?;
    if age > max_age {
        Err(HealthFailure::StaleEodPublication)
    } else {
        Ok((effective_at, age))
    }
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

#[cfg(test)]
mod production_kis_tests {
    use super::*;
    use market_data::storage::StoreError;

    fn config() -> ResearchWorkerConfig {
        ResearchWorkerConfig {
            app_env: AppEnvironment::Development,
            fetch_mode: FetchMode::Credentialed,
            run_at_kst: NaiveTime::from_hms_opt(16, 30, 0).expect("valid time"),
            max_publication_age: Duration::from_secs(60),
            attempt_timeout: Duration::from_secs(60),
            raw_root: PathBuf::from("var/research"),
            curated_root: PathBuf::from("var/curated"),
            entitlement_reference: "fixture://kis".to_owned(),
            database: DatabaseConfig {
                host: "localhost".to_owned(),
                port: 5432,
                name: "research".to_owned(),
                user: "writer".to_owned(),
                password: SecretValue("db-secret".to_owned()),
            },
            kis_app_key_file: Some(PathBuf::from("/run/secrets/kis-app-key")),
            kis_app_secret_file: Some(PathBuf::from("/run/secrets/kis-app-secret")),
            synthetic_bundle: PathBuf::from("fixtures/krx"),
            candidate_sources_enabled: false,
            candidate_raw_root: PathBuf::from("var/research/candidate"),
            candidate_synthetic_bundle: PathBuf::from("fixtures/candidates"),
        }
    }

    #[test]
    fn production_kis_provider_builds_without_network_or_secret_values() {
        let provider = build_production_kis_provider(&config()).expect("live client construction");
        let rendered = format!("{provider:?}");
        assert!(rendered.contains("KisProvider"));
        assert!(rendered.contains("kis-app-key"));
        assert!(rendered.contains("kis-app-secret"));
        assert!(!rendered.contains("db-secret"));
        assert!(!rendered.contains("app-key-value"));
        assert!(!rendered.contains("app-secret-value"));
    }

    #[test]
    fn production_kis_provider_requires_both_file_references() {
        let mut missing_key = config();
        missing_key.kis_app_key_file = None;
        assert!(matches!(
            build_production_kis_provider(&missing_key),
            Err(WorkerError::InvalidConfig {
                key: "KIS_APP_KEY_FILE"
            })
        ));

        let mut missing_secret = config();
        missing_secret.kis_app_secret_file = None;
        assert!(matches!(
            build_production_kis_provider(&missing_secret),
            Err(WorkerError::InvalidConfig {
                key: "KIS_APP_SECRET_FILE"
            })
        ));
    }

    #[test]
    fn curation_raw_store_errors_keep_store_retryability() {
        let retryable = WorkerError::Curation(CurateError::RawStore {
            context: "read normalized raw".to_owned(),
            source: Box::new(StoreError::Io {
                context: "read".to_owned(),
                source: std::io::Error::other("temporary"),
            }),
        });
        assert_eq!(retryable.failure_class(), FailureClass::Retryable);

        let permanent = WorkerError::Curation(CurateError::RawStore {
            context: "read normalized raw".to_owned(),
            source: Box::new(StoreError::ContentHashMismatch {
                path: "bars.json".to_owned(),
                recorded: "a".repeat(64),
                actual: "b".repeat(64),
            }),
        });
        assert_eq!(permanent.failure_class(), FailureClass::Permanent);

        let nested_retryable = WorkerError::Curation(CurateError::RawStore {
            context: "read normalized raw".to_owned(),
            source: Box::new(StoreError::CleanupFailed {
                path: "batch".to_owned(),
                original: Box::new(StoreError::Io {
                    context: "sync".to_owned(),
                    source: std::io::Error::other("temporary"),
                }),
                cleanup: std::io::Error::other("cleanup failed"),
            }),
        });
        assert_eq!(nested_retryable.failure_class(), FailureClass::Retryable);
    }
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
        let startup_now = control.now_utc();
        if at_or_after_run_time(startup_now, self.config.run_at_kst) {
            let date = current_kst_date(startup_now);
            if !self.recover_with_retry(control, Some(date)).await? {
                return Ok(WorkerRunOutcome::Shutdown);
            }
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
        loop {
            let delay = next_run_delay(control.now_utc(), self.config.run_at_kst);
            if control.wait(Some(delay)).await == WaitOutcome::Shutdown {
                return Ok(WorkerRunOutcome::Shutdown);
            }
            let date = current_kst_date(control.now_utc());
            if !self.recover_with_retry(control, Some(date)).await? {
                return Ok(WorkerRunOutcome::Shutdown);
            }
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
            provider: worker_event_provider(self.config.fetch_mode),
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
            provider: worker_event_provider(self.config.fetch_mode),
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
                        provider: worker_event_provider(self.config.fetch_mode),
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
                        provider: worker_event_provider(self.config.fetch_mode),
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
            provider: worker_event_provider(self.config.fetch_mode),
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
            provider: worker_event_provider(self.config.fetch_mode),
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
            result = tokio::time::timeout(self.config.attempt_timeout, future) => {
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
    endpoint: Option<String>,
    #[serde(default)]
    http_status: Option<u16>,
    #[serde(default)]
    response_kind: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    newest_eod_at: Option<String>,
    #[serde(default)]
    age_seconds: Option<u64>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    snapshot_high_water: Option<String>,
    #[serde(default)]
    has_more: Option<bool>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryItemWire<'a> {
    status: &'a str,
    event: &'a str,
    phase: &'a str,
    batch_id: BatchId,
    target_date: String,
    snapshot_high_water: BatchId,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackfillItemWire<'a> {
    status: &'a str,
    event: &'a str,
    phase: &'a str,
    batch_id: BatchId,
    target_date: String,
}

fn helper_environment(
    values: &HashMap<String, String>,
    system_root: Option<&Path>,
) -> HashMap<OsString, OsString> {
    let environment: HashMap<OsString, OsString> = WORKER_ENV_KEYS
        .iter()
        .filter_map(|key| {
            values
                .get(*key)
                .map(|value| (OsString::from(key), OsString::from(value)))
        })
        .collect();
    #[cfg(windows)]
    let mut environment = environment;
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

#[cfg(test)]
fn decode_helper_output(
    output: &[u8],
    default_phase: WorkerPhase,
    expected_date: Option<TradingDate>,
) -> Result<Option<BatchId>, WorkerError> {
    decode_helper_output_with_provider(output, default_phase, expected_date, "KRX")
}

fn decode_helper_output_with_provider(
    output: &[u8],
    default_phase: WorkerPhase,
    expected_date: Option<TradingDate>,
    expected_provider: &str,
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
                || record.endpoint.is_some()
                || record.http_status.is_some()
                || record.response_kind.is_some()
                || record.file_name.is_some()
                || record.newest_eod_at.is_some()
                || record.age_seconds.is_some()
                || record.cursor.is_some()
                || record.snapshot_high_water.is_some()
                || record.has_more.is_some()
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
                || record.cursor.is_some()
                || record.snapshot_high_water.is_some()
                || record.has_more.is_some()
                || record.provider.as_deref() != Some(expected_provider)
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
            let error_code = record.error_code.expect("validated error code");
            if !valid_error_code(&error_code)
                || !record.endpoint.as_deref().is_none_or(valid_endpoint)
                || record
                    .http_status
                    .is_some_and(|status| !(100..=599).contains(&status))
                || (record.http_status.is_some() && record.endpoint.is_none())
                || !record
                    .response_kind
                    .as_deref()
                    .is_none_or(|kind| ResponseKind::parse(kind).is_some())
                || !record.file_name.as_deref().is_none_or(valid_file_name)
                || (record.response_kind.is_some() != record.file_name.is_some())
                || (record.response_kind.is_some()
                    && record.endpoint.is_none()
                    && !error_code.starts_with("KIS_NORMALIZE_"))
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
                error_code,
                endpoint: record.endpoint,
                http_status: record.http_status,
                response_context: record.response_kind.zip(record.file_name).map(
                    |(response_kind, file_name)| {
                        Box::new(ChildResponseContext {
                            response_kind,
                            file_name,
                        })
                    },
                ),
            })
        }
        _ => Err(WorkerError::ChildOutput {
            phase: default_phase,
        }),
    }
}

fn valid_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_endpoint(value: &str) -> bool {
    value.starts_with("/uapi/")
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
}

fn valid_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
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
    Batch {
        outcome: RecoveryBatchOutcome,
        snapshot_high_water: BatchId,
    },
    Terminal(Result<RecoveryPage, WorkerError>),
}

#[cfg(test)]
fn decode_recovery_line(line: &[u8]) -> Result<RecoveryLine, WorkerError> {
    decode_recovery_line_with_provider(line, "KRX")
}

fn decode_recovery_line_with_provider(
    line: &[u8],
    expected_provider: &str,
) -> Result<RecoveryLine, WorkerError> {
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
        let date = TradingDate::parse(&record.target_date)
            .map_err(|_| WorkerError::ChildOutput { phase })?;
        let outcome = match record.event {
            "recovered" => RecoveryBatchOutcome::Recovered {
                batch_id: record.batch_id,
                date,
            },
            "skipped" => RecoveryBatchOutcome::Skipped {
                batch_id: record.batch_id,
                date,
            },
            _ => return Err(WorkerError::ChildOutput { phase }),
        };
        Ok(RecoveryLine::Batch {
            outcome,
            snapshot_high_water: record.snapshot_high_water,
        })
    } else {
        if status.status == "ok" {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct TerminalWire {
                status: String,
                phase: String,
                outcome: String,
                #[serde(default)]
                batch_id: Option<String>,
                #[serde(default)]
                date: Option<String>,
                #[serde(default)]
                newest_eod_at: Option<String>,
                #[serde(default)]
                age_seconds: Option<u64>,
                snapshot_high_water: serde_json::Value,
                #[serde(default)]
                cursor: Option<String>,
                has_more: bool,
            }
            let record: TerminalWire =
                serde_json::from_slice(line).map_err(|_| WorkerError::ChildOutput { phase })?;
            if record.phase != "recovery"
                || record.status != "ok"
                || record.outcome != "recovered"
                || record.batch_id.is_some()
                || record.date.is_some()
                || record.newest_eod_at.is_some()
                || record.age_seconds.is_some()
            {
                return Err(WorkerError::ChildOutput { phase });
            }
            let cursor = record
                .cursor
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|_| WorkerError::ChildOutput { phase })?;
            let snapshot_high_water = match record.snapshot_high_water {
                serde_json::Value::Null => None,
                serde_json::Value::String(value) => Some(
                    value
                        .parse()
                        .map_err(|_| WorkerError::ChildOutput { phase })?,
                ),
                _ => return Err(WorkerError::ChildOutput { phase }),
            };
            Ok(RecoveryLine::Terminal(Ok(RecoveryPage {
                snapshot_high_water,
                cursor,
                has_more: record.has_more,
            })))
        } else {
            let record: HelperWireRecord =
                serde_json::from_slice(line).map_err(|_| WorkerError::ChildOutput { phase })?;
            if record.cursor.is_some()
                || record.snapshot_high_water.is_some()
                || record.has_more.is_some()
            {
                return Err(WorkerError::ChildOutput { phase });
            }
            match decode_helper_output_with_provider(line, phase, None, expected_provider) {
                Err(WorkerError::ChildFailure {
                    class,
                    batch_id,
                    error_code,
                    endpoint,
                    http_status,
                    response_context,
                    ..
                }) => Ok(RecoveryLine::Terminal(Err(WorkerError::ChildFailure {
                    // A nested normalization/publication stage is executed by
                    // the recovery helper. Preserve its bounded diagnostic,
                    // but report the operator-visible phase as recovery.
                    phase,
                    class,
                    batch_id,
                    error_code,
                    endpoint,
                    http_status,
                    response_context,
                }))),
                _ => Err(WorkerError::ChildOutput { phase }),
            }
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

#[cfg(test)]
async fn supervise_recovery_child(
    spec: ChildSpec,
    timeout: Duration,
    control: &dyn WorkerControl,
    observer: &dyn RecoveryObserver,
    position: RecoveryPosition,
    progress: &Mutex<RecoveryPosition>,
) -> Result<RecoveryPage, WorkerError> {
    supervise_recovery_child_with_provider(
        spec, timeout, control, observer, position, progress, "KRX",
    )
    .await
}

async fn supervise_recovery_child_with_provider(
    spec: ChildSpec,
    timeout: Duration,
    control: &dyn WorkerControl,
    observer: &dyn RecoveryObserver,
    position: RecoveryPosition,
    progress: &Mutex<RecoveryPosition>,
    expected_provider: &str,
) -> Result<RecoveryPage, WorkerError> {
    let phase = WorkerPhase::Recovery;
    if *progress
        .lock()
        .map_err(|_| WorkerError::ChildContainment { phase })?
        != position
    {
        return Err(WorkerError::ChildOutput { phase });
    }
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
    let mut seen = Vec::with_capacity(RECOVERY_PAGE_SIZE);

    while !stdout_eof || exit_success.is_none() {
        tokio::select! {
            biased;
            _ = control.wait(None) => {
                terminate_and_reap(&mut child, phase).await?;
                return Err(WorkerError::Shutdown);
            }
            _ = &mut deadline => {
                terminate_and_reap(&mut child, phase).await?;
                return Err(WorkerError::Timeout { phase });
            }
            status = child.wait(), if exit_success.is_none() => {
                exit_success = Some(status.map_err(|_| WorkerError::ChildIo { phase })?.success());
            }
            line = read_bounded_line(&mut reader), if !stdout_eof => {
                match line {
                    Ok(Some(line)) => {
                        if terminal.is_some() {
                            terminate_and_reap(&mut child, phase).await?;
                            return Err(WorkerError::ChildOutput { phase });
                        }
                        match decode_recovery_line_with_provider(&line, expected_provider) {
                            Ok(RecoveryLine::Batch { outcome, snapshot_high_water }) => {
                                let batch_id = outcome.batch_id();
                                if seen.len() >= RECOVERY_PAGE_SIZE
                                    || Some(batch_id) == position.cursor
                                    || seen.contains(&batch_id)
                                {
                                    terminate_and_reap(&mut child, phase).await?;
                                    return Err(WorkerError::ChildOutput { phase });
                                }
                                let high_water_mismatch = {
                                    let mut validated = progress.lock().map_err(|_| {
                                        WorkerError::ChildContainment { phase }
                                    })?;
                                    let mismatch = validated
                                        .snapshot_high_water
                                        .is_some_and(|expected| expected != snapshot_high_water);
                                    if !mismatch {
                                        validated.snapshot_high_water = Some(snapshot_high_water);
                                        validated.cursor = Some(batch_id);
                                    }
                                    mismatch
                                };
                                if high_water_mismatch {
                                    terminate_and_reap(&mut child, phase).await?;
                                    return Err(WorkerError::ChildOutput { phase });
                                }
                                notify_recovery_observer(observer, outcome);
                                seen.push(batch_id);
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
        }
    }

    match (exit_success, terminal) {
        (Some(true), Some(Ok(page))) => {
            let mut validated = progress
                .lock()
                .map_err(|_| WorkerError::ChildContainment { phase })?;
            if validated
                .snapshot_high_water
                .is_some_and(|expected| page.snapshot_high_water != Some(expected))
                || position
                    .snapshot_high_water
                    .is_some_and(|expected| page.snapshot_high_water != Some(expected))
                || page.cursor != validated.cursor
                || (page.has_more && (page.cursor.is_none() || seen.len() != RECOVERY_PAGE_SIZE))
            {
                return Err(WorkerError::ChildOutput { phase });
            }
            validated.snapshot_high_water = page.snapshot_high_water;
            validated.cursor = page.cursor;
            Ok(page)
        }
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
        let attempt_timeout = parse_attempt_timeout(values)?;

        let raw_root = nonempty(values, "RESEARCH_RAW_ROOT")?;
        let curated_root = nonempty(values, "RESEARCH_CURATED_ROOT")?;
        let entitlement_reference = nonempty(values, "RESEARCH_ENTITLEMENT_REFERENCE")?;
        if entitlement_reference.len() > 256 {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_ENTITLEMENT_REFERENCE",
            });
        }
        let host = nonempty(values, "DB_HOST")?;
        let port = parse_port(values)?;
        let name = nonempty(values, "DB_NAME")?;
        let user = nonempty(values, "DB_USER")?;
        let password_file = nonempty(values, "DB_PASSWORD_FILE")?;
        let password =
            read_nonempty_secret(&reader, Path::new(&password_file), "DB_PASSWORD_FILE")?;
        let (kis_app_key_file, kis_app_secret_file) = if fetch_mode == FetchMode::Credentialed {
            let app_key_path = nonempty(values, "KIS_APP_KEY_FILE")?;
            let app_secret_path = nonempty(values, "KIS_APP_SECRET_FILE")?;
            read_nonempty_secret(&reader, Path::new(&app_key_path), "KIS_APP_KEY_FILE")?;
            read_nonempty_secret(&reader, Path::new(&app_secret_path), "KIS_APP_SECRET_FILE")?;
            (
                Some(PathBuf::from(app_key_path)),
                Some(PathBuf::from(app_secret_path)),
            )
        } else {
            (None, None)
        };
        let candidate_sources_enabled = match values
            .get("RESEARCH_CANDIDATE_ENABLED")
            .map(String::as_str)
            .unwrap_or("false")
        {
            "true" => true,
            "false" => false,
            _ => {
                return Err(WorkerError::InvalidConfig {
                    key: "RESEARCH_CANDIDATE_ENABLED",
                });
            }
        };
        if fetch_mode == FetchMode::Credentialed && candidate_sources_enabled {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_CANDIDATE_ENABLED",
            });
        }
        let candidate_raw_root = values
            .get("RESEARCH_CANDIDATE_RAW_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&raw_root).join("candidate"));

        Ok(Self {
            app_env,
            fetch_mode,
            run_at_kst,
            max_publication_age: max_age,
            attempt_timeout,
            raw_root: PathBuf::from(raw_root),
            curated_root: PathBuf::from(curated_root),
            entitlement_reference,
            database: DatabaseConfig {
                host,
                port,
                name,
                user,
                password,
            },
            kis_app_key_file,
            kis_app_secret_file,
            synthetic_bundle: values
                .get("RESEARCH_SYNTHETIC_BUNDLE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("tests/fixtures/kr-etf/contract")),
            candidate_sources_enabled,
            candidate_raw_root,
            candidate_synthetic_bundle: values
                .get("RESEARCH_CANDIDATE_SYNTHETIC_BUNDLE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("tests/fixtures/kr-candidates/contract")),
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
        let app_env = match required(values, "APP_ENV")? {
            "development" => AppEnvironment::Development,
            "qa" => AppEnvironment::Qa,
            "production" => AppEnvironment::Production,
            _ => return Err(WorkerError::InvalidConfig { key: "APP_ENV" }),
        };
        let expected_fetch_mode = match required(values, "RESEARCH_FETCH_MODE")? {
            "synthetic" => FetchMode::Synthetic,
            "credentialed" => FetchMode::Credentialed,
            _ => {
                return Err(WorkerError::InvalidConfig {
                    key: "RESEARCH_FETCH_MODE",
                });
            }
        };
        validate_synthetic_policy(app_env, expected_fetch_mode)?;
        let run_at = values
            .get("RESEARCH_RUN_AT_KST")
            .map(String::as_str)
            .unwrap_or(DEFAULT_RUN_AT_KST);
        let run_at_kst =
            NaiveTime::parse_from_str(run_at, "%H:%M").map_err(|_| WorkerError::InvalidConfig {
                key: "RESEARCH_RUN_AT_KST",
            })?;
        let max_publication_age = parse_max_age(values)?;
        let host = nonempty(values, "DB_HOST")?;
        let port = parse_port(values)?;
        let name = nonempty(values, "DB_NAME")?;
        let user = nonempty(values, "DB_USER")?;
        let password_file = nonempty(values, "DB_PASSWORD_FILE")?;
        let password =
            read_nonempty_secret(&reader, Path::new(&password_file), "DB_PASSWORD_FILE")?;
        let candidate_sources_enabled = match values
            .get("RESEARCH_CANDIDATE_ENABLED")
            .map(String::as_str)
            .unwrap_or("false")
        {
            "true" => true,
            "false" => false,
            _ => {
                return Err(WorkerError::InvalidConfig {
                    key: "RESEARCH_CANDIDATE_ENABLED",
                });
            }
        };
        if expected_fetch_mode == FetchMode::Credentialed && candidate_sources_enabled {
            return Err(WorkerError::InvalidConfig {
                key: "RESEARCH_CANDIDATE_ENABLED",
            });
        }
        let curated_root = PathBuf::from(nonempty(values, "RESEARCH_CURATED_ROOT")?);
        Ok(Self {
            max_publication_age,
            candidate_sources_enabled,
            curated_root,
            expected_fetch_mode,
            run_at_kst,
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

fn at_or_after_run_time(now_utc: DateTime<Utc>, run_at_kst: NaiveTime) -> bool {
    let kst = FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST offset is valid");
    now_utc.with_timezone(&kst).time() >= run_at_kst
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
    if target <= now_utc {
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

fn parse_attempt_timeout(values: &HashMap<String, String>) -> Result<Duration, WorkerError> {
    values
        .get("RESEARCH_ATTEMPT_TIMEOUT_SECS")
        .map_or(Some(DEFAULT_ATTEMPT_TIMEOUT_SECS), |value| {
            value.parse::<u64>().ok()
        })
        .filter(|seconds| (60..=MAX_ATTEMPT_TIMEOUT_SECS).contains(seconds))
        .map(Duration::from_secs)
        .ok_or(WorkerError::InvalidConfig {
            key: "RESEARCH_ATTEMPT_TIMEOUT_SECS",
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
    if value.contains(['\n', '\r']) {
        return Err(WorkerError::SecretFile { key });
    }
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
        WorkerPhase, decode_helper_output, decode_helper_output_with_provider,
        decode_recovery_line, helper_environment, supervise_child, supervise_recovery_child,
    };

    #[derive(Default)]
    struct RecoveryBatches(Mutex<Vec<(domain::BatchId, domain::TradingDate)>>);

    impl RecoveryObserver for RecoveryBatches {
        fn recovered(&self, batch_id: domain::BatchId, date: domain::TradingDate) {
            self.0.lock().unwrap().push((batch_id, date));
        }

        fn skipped(&self, batch_id: domain::BatchId, date: domain::TradingDate) {
            self.0.lock().unwrap().push((batch_id, date));
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

    struct ShutdownAt(tokio::time::Instant);

    #[async_trait]
    impl WorkerControl for ShutdownAt {
        fn now_utc(&self) -> chrono::DateTime<Utc> {
            Utc::now()
        }

        async fn wait(&self, duration: Option<Duration>) -> WaitOutcome {
            if duration.is_none() {
                tokio::time::sleep_until(self.0).await;
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
$batch = '00000000-0000-4000-8000-000000000001'
$date = '2020-01-30'
$highWater = '00000000-0000-4000-8000-000000000099'
if ($env:RESEARCH_TEST_CASE -eq 'complete-second') {
  $batch = '00000000-0000-4000-8000-000000000002'
  $date = '2020-01-31'
}
$event = '{"status":"event","event":"recovered","phase":"recovery","batch_id":"' + $batch + '","target_date":"' + $date + '","snapshot_high_water":"' + $highWater + '"}'
$terminalHighWater = $highWater
if ($env:RESEARCH_TEST_CASE -eq 'mismatched-high-water') {
  $terminalHighWater = '00000000-0000-4000-8000-000000000098'
}
[IO.File]::AppendAllText($env:RESEARCH_TEST_HEARTBEAT, 'x')
[Console]::Out.WriteLine($event)
if ($env:RESEARCH_TEST_CASE -eq 'oversized') {
  [Console]::Out.WriteLine(('x' * 4097))
} elseif ($env:RESEARCH_TEST_CASE -ne 'partial-timeout') {
  [Console]::Out.WriteLine('{"status":"ok","phase":"recovery","outcome":"recovered","batch_id":null,"date":null,"newest_eod_at":null,"age_seconds":null,"snapshot_high_water":"' + $terminalHighWater + '","cursor":"' + $batch + '","has_more":false}')
}
if ($env:RESEARCH_TEST_CASE -eq 'trailing') {
  [Console]::Out.WriteLine('{"unexpected":true}')
}
[Console]::Out.Flush()
if ($env:RESEARCH_TEST_CASE -in @('complete-second', 'mismatched-high-water')) { exit 0 }
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

    #[cfg(windows)]
    fn continuous_recovery_child_spec(heartbeat: PathBuf) -> ChildSpec {
        let script = r#"
$event = '{"status":"event","event":"recovered","phase":"recovery","batch_id":"00000000-0000-4000-8000-000000000001","target_date":"2020-01-30","snapshot_high_water":"00000000-0000-4000-8000-000000000099"}' + "`n"
$chunk = $event * 1024
while ($true) {
  [IO.File]::AppendAllText($env:RESEARCH_TEST_HEARTBEAT, 'x')
  [Console]::Out.Write($chunk)
  [Console]::Out.Flush()
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
    fn continuous_recovery_child_spec(heartbeat: PathBuf) -> ChildSpec {
        let script = r#"
while true; do
  printf x >> "$RESEARCH_TEST_HEARTBEAT"
  i=0
  while [ "$i" -lt 1024 ]; do
    printf '%s\n' '{"status":"event","event":"recovered","phase":"recovery","batch_id":"00000000-0000-4000-8000-000000000001","target_date":"2020-01-30","snapshot_high_water":"00000000-0000-4000-8000-000000000099"}'
    i=$((i + 1))
  done
done
"#;
        ChildSpec {
            executable: PathBuf::from("/bin/sh"),
            args: vec![OsString::from("-c"), OsString::from(script)],
            env: HashMap::from([(
                OsString::from("RESEARCH_TEST_HEARTBEAT"),
                heartbeat.into_os_string(),
            )]),
        }
    }

    #[cfg(unix)]
    fn recovery_protocol_child_spec(heartbeat: PathBuf, case: &str) -> ChildSpec {
        let script = r#"
printf x >> "$RESEARCH_TEST_HEARTBEAT"
batch=00000000-0000-4000-8000-000000000001
date=2020-01-30
high_water=00000000-0000-4000-8000-000000000099
terminal_high_water=$high_water
if [ "$RESEARCH_TEST_CASE" = mismatched-high-water ]; then
  terminal_high_water=00000000-0000-4000-8000-000000000098
fi
if [ "$RESEARCH_TEST_CASE" = complete-second ]; then
  batch=00000000-0000-4000-8000-000000000002
  date=2020-01-31
fi
printf '%s\n' "{\"status\":\"event\",\"event\":\"recovered\",\"phase\":\"recovery\",\"batch_id\":\"$batch\",\"target_date\":\"$date\",\"snapshot_high_water\":\"$high_water\"}"
if [ "$RESEARCH_TEST_CASE" = oversized ]; then
  printf '%04097d\n' 0 | tr '0' x
elif [ "$RESEARCH_TEST_CASE" != partial-timeout ]; then
  printf '%s\n' "{\"status\":\"ok\",\"phase\":\"recovery\",\"outcome\":\"recovered\",\"batch_id\":null,\"date\":null,\"newest_eod_at\":null,\"age_seconds\":null,\"snapshot_high_water\":\"$terminal_high_water\",\"cursor\":\"$batch\",\"has_more\":false}"
fi
if [ "$RESEARCH_TEST_CASE" = trailing ]; then
  printf '%s\n' '{"unexpected":true}'
fi
if [ "$RESEARCH_TEST_CASE" = complete-second ] || [ "$RESEARCH_TEST_CASE" = mismatched-high-water ]; then exit 0; fi
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
            decode_helper_output(
                b"{\"status\":\"ok\",\"phase\":\"publication\",\"outcome\":\"published\",\"batch_id\":\"00000000-0000-4000-8000-000000000001\",\"date\":\"2020-01-31\",\"cursor\":\"00000000-0000-4000-8000-000000000001\",\"has_more\":false}",
                WorkerPhase::Ingest,
                Some(domain::TradingDate::parse("2020-01-31").unwrap()),
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Ingest
            })
        ));
        assert!(matches!(
            decode_recovery_line(
                b"{\"status\":\"event\",\"event\":\"recovered\",\"phase\":\"recovery\",\"batch_id\":\"00000000-0000-4000-8000-000000000001\",\"target_date\":\"2020-01-30\",\"unknown\":true}"
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
        assert!(matches!(
            decode_recovery_line(
                b"{\"status\":\"event\",\"event\":\"recovered\",\"phase\":\"recovery\",\"batch_id\":\"00000000-0000-4000-8000-000000000001\",\"target_date\":\"2020-01-30\"}"
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
        assert!(matches!(
            decode_recovery_line(
                b"{\"status\":\"event\",\"event\":\"recovered\",\"phase\":\"recovery\",\"batch_id\":\"00000000-0000-4000-8000-000000000001\",\"target_date\":\"2020-01-30\",\"snapshot_high_water\":\"00000000-0000-4000-8000-000000000099\"}"
            ),
            Ok(super::RecoveryLine::Batch {
                outcome: crate::RecoveryBatchOutcome::Recovered { date, .. },
                snapshot_high_water: _
            }) if date == domain::TradingDate::parse("2020-01-30").unwrap()
        ));
        assert!(matches!(
            decode_recovery_line(
                b"{\"status\":\"ok\",\"phase\":\"recovery\",\"outcome\":\"recovered\",\"batch_id\":null,\"date\":null,\"newest_eod_at\":null,\"age_seconds\":null,\"snapshot_high_water\":\"00000000-0000-4000-8000-000000000099\",\"cursor\":\"00000000-0000-4000-8000-000000000001\",\"has_more\":true}"
            ),
            Ok(super::RecoveryLine::Terminal(Ok(crate::RecoveryPage {
                cursor: Some(_),
                has_more: true,
                ..
            })))
        ));
        assert!(matches!(
            decode_recovery_line(
                b"{\"status\":\"ok\",\"phase\":\"recovery\",\"outcome\":\"recovered\",\"batch_id\":null,\"date\":null,\"newest_eod_at\":null,\"age_seconds\":null,\"cursor\":null,\"has_more\":false}"
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));

        let normalization_failure = br#"{"status":"error","error_code":"KIS_NORMALIZE_MISSING_FIELD","provider":"KIS-NORMALIZED","market":"KR","target_date":null,"phase":"publication","class":"permanent","batch_id":"00000000-0000-4000-8000-000000000001","message":"operation failed with KIS_NORMALIZE_MISSING_FIELD","response_kind":"reference","file_name":"reference-069500-page-01.json"}"#;
        let decoded = super::decode_recovery_line_with_provider(
            normalization_failure,
            super::WORKER_PROVIDER_KIS_NORMALIZED,
        )
        .expect("bounded normalization diagnostic");
        assert!(matches!(
            decoded,
            super::RecoveryLine::Terminal(Err(super::WorkerError::ChildFailure {
                phase: WorkerPhase::Recovery,
                class: crate::FailureClass::Permanent,
                error_code,
                endpoint: None,
                response_context: Some(context),
                ..
            })) if error_code == "KIS_NORMALIZE_MISSING_FIELD"
                && context.response_kind == "reference"
                && context.file_name == "reference-069500-page-01.json"
        ));

        let unscoped_context = br#"{"status":"error","error_code":"BROKER_REJECTED","provider":"KIS-NORMALIZED","market":"KR","target_date":null,"phase":"publication","class":"permanent","batch_id":null,"message":"redacted","response_kind":"reference","file_name":"reference-069500-page-01.json"}"#;
        assert!(matches!(
            super::decode_recovery_line_with_provider(
                unscoped_context,
                super::WORKER_PROVIDER_KIS_NORMALIZED,
            ),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Recovery
            })
        ));
    }

    #[test]
    fn helper_output_validates_the_credentialed_provider_scope() {
        let batch_id = domain::BatchId::generate();
        let output = format!(
            "{{\"status\":\"error\",\"error_code\":\"PIPELINE_FAILED\",\"provider\":\"KIS-NORMALIZED\",\"market\":\"KR\",\"target_date\":null,\"phase\":\"publication\",\"class\":\"permanent\",\"batch_id\":\"{batch_id}\",\"message\":\"research pipeline failed\"}}"
        );
        let error = decode_helper_output_with_provider(
            output.as_bytes(),
            WorkerPhase::Ingest,
            None,
            "KIS-NORMALIZED",
        )
        .expect_err("credentialed child failure");
        assert!(matches!(
            error,
            super::WorkerError::ChildFailure {
                phase: WorkerPhase::Publication,
                class: crate::FailureClass::Permanent,
                batch_id: Some(id),
                error_code,
                endpoint: None,
                http_status: None,
                response_context: None,
            } if id == batch_id
                && error_code == "PIPELINE_FAILED"
        ));
        assert!(matches!(
            decode_helper_output_with_provider(output.as_bytes(), WorkerPhase::Ingest, None, "KRX"),
            Err(super::WorkerError::ChildOutput {
                phase: WorkerPhase::Ingest
            })
        ));
    }

    #[test]
    fn helper_provider_diagnostic_preserves_only_validated_safe_metadata() {
        let pipeline_error = super::WorkerError::Pipeline(crate::PipelineError::Ingest {
            source: market_data::IngestError::MalformedResponse {
                kind: market_data::ResponseKind::Calendar,
                reason: "fixture detail must not propagate".to_owned(),
                diagnostic: Some(market_data::ingest::ResponseValidationDiagnostic {
                    code: "KIS_RESPONSE_SCHEMA_INVALID",
                    endpoint: "/uapi/domestic-stock/v1/quotations/chk-holiday".to_owned(),
                    file_name: "calendar-page-01.json".to_owned(),
                }),
            },
        });
        let pipeline_diagnostic = pipeline_error.safe_diagnostic().unwrap();
        assert_eq!(
            pipeline_diagnostic.error_code,
            "KIS_RESPONSE_SCHEMA_INVALID"
        );
        assert_eq!(pipeline_diagnostic.response_kind, Some("calendar"));
        assert_eq!(pipeline_diagnostic.file_name, Some("calendar-page-01.json"));
        assert!(!pipeline_error.to_string().contains("fixture detail"));

        let output = br#"{"status":"error","error_code":"BROKER_REJECTED","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"broker body must not propagate: appsecret=fixture-secret","endpoint":"/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice","http_status":403}"#;
        let error = decode_helper_output_with_provider(
            output,
            WorkerPhase::Ingest,
            Some(domain::TradingDate::parse("2026-08-18").unwrap()),
            "KIS-NORMALIZED",
        )
        .expect_err("provider failure");
        let diagnostic = error.safe_diagnostic().expect("safe diagnostic");
        assert_eq!(diagnostic.error_code, "BROKER_REJECTED");
        assert_eq!(
            diagnostic.endpoint,
            Some("/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice")
        );
        assert_eq!(diagnostic.http_status, Some(403));
        assert!(!error.to_string().contains("fixture-secret"));

        let validation_output = br#"{"status":"error","error_code":"KIS_RESPONSE_SCHEMA_INVALID","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"operation failed with KIS_RESPONSE_SCHEMA_INVALID","endpoint":"/uapi/domestic-stock/v1/quotations/chk-holiday","response_kind":"calendar","file_name":"calendar-page-01.json"}"#;
        let validation_error = decode_helper_output_with_provider(
            validation_output,
            WorkerPhase::Ingest,
            Some(domain::TradingDate::parse("2026-08-18").unwrap()),
            "KIS-NORMALIZED",
        )
        .expect_err("validation failure");
        let validation_diagnostic = validation_error.safe_diagnostic().unwrap();
        assert_eq!(
            validation_diagnostic.error_code,
            "KIS_RESPONSE_SCHEMA_INVALID"
        );
        assert_eq!(validation_diagnostic.response_kind, Some("calendar"));
        assert_eq!(
            validation_diagnostic.file_name,
            Some("calendar-page-01.json")
        );

        for invalid in [
            br#"{"status":"error","error_code":"BROKER_REJECTED","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"redacted","endpoint":"https://attacker.invalid/uapi/path","http_status":403}"#.as_slice(),
            br#"{"status":"error","error_code":"bad code","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"redacted","endpoint":"/uapi/path","http_status":403}"#.as_slice(),
            br#"{"status":"error","error_code":"BROKER_REJECTED","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"redacted","endpoint":null,"http_status":403}"#.as_slice(),
            br#"{"status":"error","error_code":"KIS_RESPONSE_SCHEMA_INVALID","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"redacted","endpoint":"/uapi/path","response_kind":"calendar","file_name":"../secret"}"#.as_slice(),
            br#"{"status":"error","error_code":"KIS_RESPONSE_SCHEMA_INVALID","provider":"KIS-NORMALIZED","market":"KR","target_date":"2026-08-18","phase":"ingest","class":"permanent","batch_id":null,"message":"redacted","endpoint":"/uapi/path","response_kind":"calendar"}"#.as_slice(),
        ] {
            assert!(matches!(
                decode_helper_output_with_provider(
                    invalid,
                    WorkerPhase::Ingest,
                    Some(domain::TradingDate::parse("2026-08-18").unwrap()),
                    "KIS-NORMALIZED",
                ),
                Err(super::WorkerError::ChildOutput {
                    phase: WorkerPhase::Ingest
                })
            ));
        }
    }

    #[test]
    fn normalize_diagnostic_is_stable_and_redacts_free_form_fields() {
        let batch_id = domain::BatchId::generate();
        let unsupported = super::WorkerError::Pipeline(crate::PipelineError::Normalize {
            batch_id,
            source: Box::new(market_data::normalize::NormalizeError::UnsupportedAction {
                file_name: "corporate-actions.json".to_owned(),
                reason: "secret row value must not propagate".to_owned(),
            }),
        });
        let diagnostic = unsupported.safe_diagnostic().expect("normalize diagnostic");
        assert_eq!(diagnostic.error_code, "KIS_NORMALIZE_UNSUPPORTED_ACTION");
        assert_eq!(diagnostic.response_kind, None);
        assert_eq!(diagnostic.file_name, Some("corporate-actions.json"));
        assert_eq!(diagnostic.endpoint, None);
        assert!(!format!("{diagnostic:?}").contains("secret"));

        let invalid = super::WorkerError::Pipeline(crate::PipelineError::Normalize {
            batch_id,
            source: Box::new(market_data::normalize::NormalizeError::InvalidField {
                kind: market_data::ResponseKind::Bars,
                file_name: "bars.json".to_owned(),
                field: "stck_clpr".to_owned(),
                value: "secret-value".to_owned(),
            }),
        });
        let diagnostic = invalid.safe_diagnostic().expect("normalize diagnostic");
        assert_eq!(diagnostic.error_code, "KIS_NORMALIZE_INVALID_FIELD");
        assert_eq!(diagnostic.response_kind, Some("bars"));
        assert_eq!(diagnostic.file_name, Some("bars.json"));
        assert_eq!(diagnostic.endpoint, None);
        assert!(!format!("{diagnostic:?}").contains("secret"));

        let integrity = super::WorkerError::Pipeline(crate::PipelineError::Normalize {
            batch_id,
            source: Box::new(market_data::normalize::NormalizeError::Store(
                market_data::storage::StoreError::ContentHashMismatch {
                    path: "/private/secret/path".to_owned(),
                    recorded: "recorded-secret".to_owned(),
                    actual: "actual-secret".to_owned(),
                },
            )),
        });
        let diagnostic = integrity.safe_diagnostic().expect("normalize diagnostic");
        assert_eq!(diagnostic.error_code, "KIS_NORMALIZE_INTEGRITY_FAILURE");
        assert_eq!(diagnostic.response_kind, None);
        assert_eq!(diagnostic.file_name, None);
        assert_eq!(diagnostic.endpoint, None);
        assert!(!format!("{diagnostic:?}").contains("secret"));

        let allowed_endpoint = super::WorkerError::Pipeline(crate::PipelineError::Normalize {
            batch_id,
            source: Box::new(market_data::normalize::NormalizeError::UnexpectedEndpoint {
                file_name: "bars.json".to_owned(),
                endpoint: "/uapi/domestic-stock/v1/quotations/inquire-price".to_owned(),
            }),
        });
        let diagnostic = allowed_endpoint
            .safe_diagnostic()
            .expect("normalize diagnostic");
        assert_eq!(diagnostic.error_code, "KIS_NORMALIZE_UNEXPECTED_ENDPOINT");
        assert_eq!(
            diagnostic.endpoint,
            Some("/uapi/domestic-stock/v1/quotations/inquire-price")
        );

        let untrusted_endpoint = super::WorkerError::Pipeline(crate::PipelineError::Normalize {
            batch_id,
            source: Box::new(market_data::normalize::NormalizeError::UnexpectedEndpoint {
                file_name: "bars.json".to_owned(),
                endpoint: "https://attacker.invalid/secret".to_owned(),
            }),
        });
        let diagnostic = untrusted_endpoint
            .safe_diagnostic()
            .expect("normalize diagnostic");
        assert_eq!(diagnostic.error_code, "KIS_NORMALIZE_UNEXPECTED_ENDPOINT");
        assert_eq!(diagnostic.endpoint, None);
        assert_eq!(diagnostic.file_name, Some("bars.json"));
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
        for case in ["oversized", "trailing", "mismatched-high-water"] {
            let dir = tempfile::tempdir().unwrap();
            let heartbeat = dir.path().join(format!("{case}-heartbeat"));
            let observer = RecoveryBatches::default();
            let progress = Mutex::new(crate::RecoveryPosition::default());
            let started = Instant::now();
            let error = supervise_recovery_child(
                recovery_protocol_child_spec(heartbeat.clone(), case),
                Duration::from_secs(5),
                &NeverShutdown,
                &observer,
                crate::RecoveryPosition::default(),
                &progress,
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
    async fn recovery_timeout_preserves_last_event_cursor_and_resume_advances() {
        let dir = tempfile::tempdir().unwrap();
        let observer = RecoveryBatches::default();
        let progress = Mutex::new(crate::RecoveryPosition::default());
        let first = "00000000-0000-4000-8000-000000000001"
            .parse::<domain::BatchId>()
            .unwrap();
        let second = "00000000-0000-4000-8000-000000000002"
            .parse::<domain::BatchId>()
            .unwrap();

        let timeout = supervise_recovery_child(
            recovery_protocol_child_spec(dir.path().join("partial"), "partial-timeout"),
            Duration::from_secs(1),
            &NeverShutdown,
            &observer,
            crate::RecoveryPosition::default(),
            &progress,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            timeout,
            super::WorkerError::Timeout {
                phase: WorkerPhase::Recovery
            }
        ));
        let high_water = "00000000-0000-4000-8000-000000000099"
            .parse::<domain::BatchId>()
            .unwrap();
        assert_eq!(
            *progress.lock().unwrap(),
            crate::RecoveryPosition {
                snapshot_after: None,
                snapshot_high_water: Some(high_water),
                cursor: Some(first),
            }
        );

        let page = supervise_recovery_child(
            recovery_protocol_child_spec(dir.path().join("resume"), "complete-second"),
            Duration::from_secs(5),
            &NeverShutdown,
            &observer,
            crate::RecoveryPosition {
                snapshot_after: None,
                snapshot_high_water: Some(high_water),
                cursor: Some(first),
            },
            &progress,
        )
        .await
        .unwrap();
        assert_eq!(page.cursor, Some(second));
        assert!(!page.has_more);
        assert_eq!(
            observer
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|(batch, _)| *batch)
                .collect::<Vec<_>>(),
            vec![first, second],
            "resume starts strictly after the last validated event"
        );
    }

    #[tokio::test]
    async fn recovery_output_beyond_one_page_is_contained_without_starvation() {
        for (case, timeout) in [
            ("timeout", Duration::from_millis(500)),
            ("shutdown", Duration::from_secs(5)),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let heartbeat = dir.path().join(format!("{case}-heartbeat"));
            let observer = RecoveryBatches::default();
            let progress = Mutex::new(crate::RecoveryPosition::default());
            let shutdown = ShutdownAt(tokio::time::Instant::now() + Duration::from_millis(500));
            let control: &dyn WorkerControl = if case == "shutdown" {
                &shutdown
            } else {
                &NeverShutdown
            };
            let started = Instant::now();
            let error = tokio::time::timeout(
                Duration::from_secs(3),
                supervise_recovery_child(
                    continuous_recovery_child_spec(heartbeat.clone()),
                    timeout,
                    control,
                    &observer,
                    crate::RecoveryPosition::default(),
                    &progress,
                ),
            )
            .await
            .expect("continuous valid stdout must not starve timeout or shutdown")
            .unwrap_err();

            assert!(matches!(
                error,
                super::WorkerError::ChildOutput {
                    phase: WorkerPhase::Recovery
                }
            ));
            assert!(started.elapsed() < Duration::from_secs(2));
            let records_at_return = observer.0.lock().unwrap().len();
            assert_heartbeat_stops(&heartbeat).await;
            assert_eq!(
                records_at_return,
                observer.0.lock().unwrap().len(),
                "no recovery records may be observed after supervisor return"
            );
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
