mod candidate_pipeline;
mod candidate_sink;
mod pipeline;
mod sink;
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
    FailureClass, PipelineError, PipelineStage, RECOVERY_PAGE_SIZE, RecoveryBatchOutcome,
    RecoveryError, RecoveryPage, RecoveryPosition, RecoveryReport, RunOutcome, ingest_and_publish,
    provider_failure_class, recover_unpublished, recover_unpublished_page_with,
    recover_unpublished_with, store_failure_class,
};
pub use sink::{
    PostgresPublicationSink, PublicationSink, PublicationState, PublishOutcome, SinkError,
};
pub use worker::{
    AppEnvironment, DatabaseConfig, HealthFailure, HealthStatus, HealthcheckConfig,
    ProductionWorkerComponentFactory, RecoveryObserver, ResearchBackend, ResearchWorker,
    ResearchWorkerConfig, SecretValue, WORKER_ENV_KEYS, WaitOutcome, WorkerComponentFactory,
    WorkerControl, WorkerError, WorkerEvent, WorkerEventClass, WorkerEventKind, WorkerObserver,
    WorkerPhase, WorkerRunOutcome, bootstrap_worker, bootstrap_worker_with, build_postgres_pool,
    candidate_healthcheck, current_kst_date, healthcheck, next_run_delay, publication_age,
    retry_delay, run_internal_ingest, run_internal_recovery, run_internal_recovery_page_stream,
    run_internal_recovery_stream, validate_synthetic_policy,
};
