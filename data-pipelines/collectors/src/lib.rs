mod candidate_pipeline;
mod candidate_sink;
mod pipeline;
mod sink;
mod v3_historical_input;
mod worker;

pub use candidate_pipeline::{
    CandidateDatasetBinding, CandidatePipelineError, PreparedCandidateBatch,
    PreparedCandidateSource, prepare_candidate_batch, publish_candidate_batch,
    recover_candidate_batches,
};
pub use candidate_sink::{
    CandidateInstrumentCatalog, CandidatePricePublication, CandidateSourcePublication,
    PostgresCandidateSourceSink, candidate_raw_manifest_sha256,
};
pub use pipeline::{
    FailureClass, KisNormalizationRecoveryReport, PipelineError, PipelineStage, RECOVERY_PAGE_SIZE,
    RecoveryBatchOutcome, RecoveryError, RecoveryPage, RecoveryPosition, RecoveryReport,
    RecoveryScope, RunOutcome, ingest_and_publish, ingest_normalize_publish_kis,
    normalize_failure_class, provider_failure_class, recover_kis_normalization,
    recover_unpublished, recover_unpublished_normalized_for_date, recover_unpublished_page_with,
    recover_unpublished_page_with_scope, recover_unpublished_scope, recover_unpublished_with,
    recover_unpublished_with_scope, store_failure_class,
};
pub use sink::{
    PostgresPublicationSink, PublicationSink, PublicationState, PublishOutcome, SinkError,
};
pub use v3_historical_input::{
    BATCH_JSON_MAX_BYTES, HistoricalPriceOnlyV3Input, HistoricalPriceOnlyV3InputError,
    MANIFEST_MAX_BYTES, load_historical_price_only_v3_input, verify_historical_price_only_v3_input,
};
pub use worker::{
    AppEnvironment, ChildResponseContext, DailyRangeRawSummary, DatabaseConfig, HealthFailure,
    HealthStatus, HealthcheckConfig, ProductionWorkerComponentFactory, RecoveryObserver,
    ResearchBackend, ResearchWorker, ResearchWorkerConfig, SecretValue, WORKER_ENV_KEYS,
    WaitOutcome, WorkerComponentFactory, WorkerControl, WorkerError, WorkerEvent, WorkerEventClass,
    WorkerEventKind, WorkerObserver, WorkerPhase, WorkerRunOutcome, bootstrap_worker,
    bootstrap_worker_with, build_postgres_pool, candidate_healthcheck, current_kst_date,
    healthcheck, next_run_delay, publication_age, retry_delay,
    run_credentialed_backfill_session_dates_stream, run_credentialed_daily_range_raw_stream,
    run_existing_daily_range_raw_stream, run_internal_ingest, run_internal_recovery,
    run_internal_recovery_page_stream, run_internal_recovery_stream, validate_synthetic_policy,
};
