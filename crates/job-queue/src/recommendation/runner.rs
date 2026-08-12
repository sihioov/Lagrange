//! Recommendation job orchestration, lease supervision, and failure lifecycle.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use domain::TradingDate;
use market_data::curate::CurateStore;
use serde_json::json;
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::watch;
use uuid::Uuid;

use crate::error::{QueueError, database_error_class};
use crate::queue::JobQueue;
use crate::recommendation::child::{self, TargetChildRequest, TargetProvenance, run_target_child};
use crate::recommendation::compute::{
    AttestedUniverse, RecommendationError, compute_close_async, requirements_for,
};
use crate::recommendation::input::{
    AttestedDataset, AttestedRecommendationInput, RecommendationInputError, RecommendationPayload,
    attest_recommendation_input,
};
use crate::recommendation::publish::{PublicationError, publish_recommendation};
use crate::recommendation::validate::{
    RecommendationValidationError, ValidatedPortfolio, validate_target_output,
};
use crate::types::{ClaimedJob, ErrorClass, HeartbeatStatus, JobStatus, SettleResult};

#[derive(Debug, Clone)]
pub struct RecommendationRunnerPaths {
    /// Absolute deployment root that must contain the DB-attested dataset path.
    pub data_root: PathBuf,
    pub universe_manifest: PathBuf,
    pub child: child::TargetChildPaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecommendationOutcome {
    Idle,
    Succeeded { job_id: Uuid, run_id: Uuid },
    Blocked { job_id: Uuid, code: String },
    Failed { job_id: Uuid, code: String },
    Retrying { job_id: Uuid, code: String },
}

#[derive(Debug, Error)]
pub enum RecommendationRunnerError {
    #[error("recommendation queue unavailable")]
    Queue(#[from] QueueError),
}

#[derive(Debug)]
struct StageFailure {
    class: ErrorClass,
    code: String,
    summary: &'static str,
}

impl StageFailure {
    fn new(class: ErrorClass, code: impl Into<String>, summary: &'static str) -> Self {
        Self {
            class,
            code: code.into(),
            summary,
        }
    }
}

impl From<RecommendationInputError> for StageFailure {
    fn from(error: RecommendationInputError) -> Self {
        Self::new(
            error.class(),
            error.code(),
            match error.class() {
                ErrorClass::DataBlocked => "pinned recommendation data is unavailable",
                ErrorClass::Transient => "recommendation input is temporarily unavailable",
                _ => "recommendation input is invalid",
            },
        )
    }
}

impl From<RecommendationError> for StageFailure {
    fn from(error: RecommendationError) -> Self {
        let class = match error {
            RecommendationError::InvalidUniverse { .. } => ErrorClass::DataBlocked,
            _ => error.class(),
        };
        Self::new(
            class,
            error.code(),
            match class {
                ErrorClass::DataBlocked => "recommendation data or universe is unavailable",
                ErrorClass::Transient => "recommendation computation is temporarily unavailable",
                _ => "recommendation computation failed validation",
            },
        )
    }
}

impl From<child::TargetChildError> for StageFailure {
    fn from(error: child::TargetChildError) -> Self {
        let class = error.class();
        Self::new(
            class,
            error.code(),
            match class {
                ErrorClass::Transient => "target generator is temporarily unavailable",
                _ => "target generator rejected the recommendation",
            },
        )
    }
}

impl From<RecommendationValidationError> for StageFailure {
    fn from(error: RecommendationValidationError) -> Self {
        Self::new(
            error.class(),
            error.code(),
            "target generator output failed validation",
        )
    }
}

impl From<PublicationError> for StageFailure {
    fn from(error: PublicationError) -> Self {
        Self::new(
            error.class(),
            error.code(),
            match error.class() {
                ErrorClass::DataBlocked => "recommendation authorization was revoked",
                ErrorClass::Transient => "recommendation publication is temporarily unavailable",
                _ => "recommendation publication failed validation",
            },
        )
    }
}

/// Timing bounds shared by the one-shot runner and the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecommendationRunnerConfig {
    heartbeat_interval: Duration,
    lease: Duration,
    child_timeout: Duration,
    production: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RecommendationRunnerConfigError {
    #[error("runner durations must be positive")]
    ZeroDuration,
    #[error("heartbeat interval must be shorter than the queue lease")]
    HeartbeatNotBeforeLease,
}

impl RecommendationRunnerConfig {
    pub fn new(
        heartbeat_interval: Duration,
        lease: Duration,
        child_timeout: Duration,
    ) -> Result<Self, RecommendationRunnerConfigError> {
        if heartbeat_interval.is_zero() || lease.is_zero() || child_timeout.is_zero() {
            return Err(RecommendationRunnerConfigError::ZeroDuration);
        }
        if heartbeat_interval >= lease {
            return Err(RecommendationRunnerConfigError::HeartbeatNotBeforeLease);
        }
        Ok(Self {
            heartbeat_interval,
            lease,
            child_timeout,
            production: false,
        })
    }

    pub const fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub const fn lease(&self) -> Duration {
        self.lease
    }

    pub const fn child_timeout(&self) -> Duration {
        self.child_timeout
    }

    pub const fn with_production(mut self, production: bool) -> Self {
        self.production = production;
        self
    }

    pub const fn is_production(&self) -> bool {
        self.production
    }
}

/// Claim and execute at most one recommendation job.
pub async fn run_once(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    paths: &RecommendationRunnerPaths,
    config: &RecommendationRunnerConfig,
) -> Result<RecommendationOutcome, RecommendationRunnerError> {
    if config.lease != queue.config().lease {
        return Err(RecommendationRunnerError::Queue(QueueError::Internal(
            "recommendation runner lease configuration does not match the queue".into(),
        )));
    }
    validate_production_paths(paths, config.production)?;
    let Some(claim) = queue.claim_next_for(worker_id, "recommendation").await? else {
        return Ok(RecommendationOutcome::Idle);
    };
    if claim.job.job_type != "recommendation" {
        return settle_failure_lifecycle(
            pool,
            queue,
            &claim,
            StageFailure::new(
                ErrorClass::Integrity,
                "RECOMMENDATION_WRONG_JOB_TYPE",
                "typed recommendation claim returned an invalid job",
            ),
        )
        .await;
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let monitor_queue = queue.clone();
    let monitor_claim = claim.clone();
    let heartbeat_interval = config.heartbeat_interval;
    let mut monitor = tokio::spawn(async move {
        monitor_lease(&monitor_queue, &monitor_claim, heartbeat_interval, stop_rx).await
    });
    let execution = prepare_claim(pool, &claim, paths, config);
    tokio::pin!(execution);

    let result = tokio::select! {
        biased;
        monitor_result = &mut monitor => {
            match monitor_result {
                Ok(LeaseMonitorOutcome::Canceled) => {
                    settle_canceled_lifecycle(pool, queue, &claim).await
                }
                Ok(LeaseMonitorOutcome::LeaseLost) => lease_lost_outcome(pool, &claim).await,
                Ok(LeaseMonitorOutcome::Unavailable) => settle_failure_lifecycle(
                    pool,
                    queue,
                    &claim,
                    StageFailure::new(
                        ErrorClass::Transient,
                        "RECOMMENDATION_HEARTBEAT_UNAVAILABLE",
                        "recommendation heartbeat is temporarily unavailable",
                    ),
                ).await,
                Ok(LeaseMonitorOutcome::Stopped) => settle_failure_lifecycle(
                    pool,
                    queue,
                    &claim,
                    StageFailure::new(
                        ErrorClass::Transient,
                        "RECOMMENDATION_HEARTBEAT_STOPPED",
                        "recommendation heartbeat stopped unexpectedly",
                    ),
                ).await,
                Err(_) => settle_failure_lifecycle(
                    pool,
                    queue,
                    &claim,
                    StageFailure::new(
                        ErrorClass::Transient,
                        "RECOMMENDATION_HEARTBEAT_UNAVAILABLE",
                        "recommendation heartbeat is temporarily unavailable",
                    ),
                ).await,
            }
        }
        execution_result = &mut execution => {
            let _ = stop_tx.send(true);
            let heartbeat_result = monitor.await;
            match heartbeat_result {
                Ok(LeaseMonitorOutcome::Stopped) => match execution_result {
                    Ok(prepared) => finish_prepared(pool, queue, &claim, prepared).await,
                    Err(failure) => settle_failure_lifecycle(pool, queue, &claim, failure).await,
                },
                Ok(LeaseMonitorOutcome::Canceled) => settle_canceled_lifecycle(pool, queue, &claim).await,
                Ok(LeaseMonitorOutcome::LeaseLost) => lease_lost_outcome(pool, &claim).await,
                Ok(LeaseMonitorOutcome::Unavailable) => settle_failure_lifecycle(
                    pool,
                    queue,
                    &claim,
                    StageFailure::new(
                        ErrorClass::Transient,
                        "RECOMMENDATION_HEARTBEAT_UNAVAILABLE",
                        "recommendation heartbeat is temporarily unavailable",
                    ),
                ).await,
                Err(_) => settle_failure_lifecycle(
                    pool,
                    queue,
                    &claim,
                    StageFailure::new(
                        ErrorClass::Transient,
                        "RECOMMENDATION_HEARTBEAT_UNAVAILABLE",
                        "recommendation heartbeat is temporarily unavailable",
                    ),
                ).await,
            }
        }
    };
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseMonitorOutcome {
    Stopped,
    Canceled,
    LeaseLost,
    Unavailable,
}

async fn monitor_lease(
    queue: &JobQueue,
    claim: &ClaimedJob,
    interval: Duration,
    mut stop: watch::Receiver<bool>,
) -> LeaseMonitorOutcome {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return LeaseMonitorOutcome::Stopped;
                }
            }
            _ = ticker.tick() => {
                match queue.heartbeat(claim).await {
                    Ok(HeartbeatStatus::Extended) => {}
                    Ok(HeartbeatStatus::Canceled) => return LeaseMonitorOutcome::Canceled,
                    Ok(HeartbeatStatus::LeaseLost) => return LeaseMonitorOutcome::LeaseLost,
                    Err(_) => return LeaseMonitorOutcome::Unavailable,
                }
            }
        }
    }
}

async fn lease_lost_outcome(
    pool: &PgPool,
    claim: &ClaimedJob,
) -> Result<RecommendationOutcome, RecommendationRunnerError> {
    let state: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, error_code FROM jobs WHERE id = $1")
            .bind(claim.job.id)
            .fetch_optional(pool)
            .await
            .map_err(QueueError::Database)?;
    match state.as_ref().map(|(status, _)| status.as_str()) {
        Some("QUEUED" | "RUNNING") => Ok(RecommendationOutcome::Retrying {
            job_id: claim.job.id,
            code: "RECOMMENDATION_LEASE_LOST".to_owned(),
        }),
        Some("FAILED") => Ok(RecommendationOutcome::Failed {
            job_id: claim.job.id,
            code: if state.as_ref().and_then(|(_, code)| code.as_deref())
                == Some("attempts_exhausted")
            {
                "RECOMMENDATION_ATTEMPTS_EXHAUSTED".to_owned()
            } else {
                "RECOMMENDATION_LEASE_LOST".to_owned()
            },
        }),
        Some("CANCELED") => Ok(RecommendationOutcome::Failed {
            job_id: claim.job.id,
            code: "RECOMMENDATION_CANCELED".to_owned(),
        }),
        _ => Err(RecommendationRunnerError::Queue(QueueError::StaleClaim(
            claim.job.id,
        ))),
    }
}

struct PreparedRecommendation {
    run_id: Uuid,
    input: AttestedRecommendationInput,
    universe: AttestedUniverse,
    portfolio: ValidatedPortfolio,
}

async fn prepare_claim(
    pool: &PgPool,
    claim: &ClaimedJob,
    paths: &RecommendationRunnerPaths,
    config: &RecommendationRunnerConfig,
) -> Result<PreparedRecommendation, StageFailure> {
    let payload = RecommendationPayload::try_from(claim.job.payload_json.clone())?;
    let input =
        attest_recommendation_input(pool, claim.job.id, claim.job.owner_user_id, payload).await?;
    if input.dataset.dataset_id != "krx_eod_bars" {
        return Err(StageFailure::new(
            ErrorClass::Integrity,
            "RECOMMENDATION_DATASET_NOT_APPROVED",
            "recommendation dataset is not approved for this product",
        ));
    }
    attest_current_entitlement(pool, &input.dataset.dataset_id, input.payload.as_of).await?;
    attest_dataset_path(&paths.data_root, &input.dataset.storage_path).await?;
    if config.production {
        attest_credentialed_sources(pool, &input.dataset).await?;
    }

    let manifest_path = paths.universe_manifest.clone();
    let manifest = tokio::task::spawn_blocking(move || std::fs::read_to_string(manifest_path))
        .await
        .map_err(|_| {
            StageFailure::new(
                ErrorClass::Transient,
                "RECOMMENDATION_UNIVERSE_UNAVAILABLE",
                "recommendation universe is temporarily unavailable",
            )
        })?
        .map_err(|_| {
            StageFailure::new(
                ErrorClass::Transient,
                "RECOMMENDATION_UNIVERSE_UNAVAILABLE",
                "recommendation universe is temporarily unavailable",
            )
        })?;
    let universe = AttestedUniverse::from_manifest_yaml(&manifest)?;
    attest_current_universe(pool, &universe).await?;
    let requirements = requirements_for(&input.resolved_config)?;
    let as_of_text = input.payload.as_of.format("%Y-%m-%d").to_string();
    let as_of = TradingDate::parse(&as_of_text).map_err(|_| {
        StageFailure::new(
            ErrorClass::Input,
            "RECOMMENDATION_INPUT_INVALID",
            "recommendation as-of date is invalid",
        )
    })?;
    let computed =
        compute_close_async(input.dataset.clone(), universe.clone(), as_of, requirements).await?;
    let provenance = TargetProvenance {
        dataset_version_id: input.dataset.id,
        dataset_id: input.dataset.dataset_id.clone(),
        dataset_version: input.dataset.version.clone(),
        curated_version: input.dataset.curated_version,
        dataset_manifest_sha256: input.dataset.manifest_sha256.clone(),
        universe_snapshot_id: universe.snapshot_id().to_owned(),
        factor_snapshot_hash: computed.factor_snapshot_hash.clone(),
    };
    let factors = computed
        .factors
        .into_iter()
        .map(|(instrument, values)| {
            (
                instrument,
                values
                    .into_iter()
                    .map(|(factor, value)| (factor, Some(value)))
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect();
    let request = TargetChildRequest {
        strategy_id: input.resolved_config.strategy_id.clone(),
        strategy_version: input.resolved_config.strategy_version.clone(),
        parameters: input.resolved_config.config.clone(),
        as_of: as_of_text.clone(),
        universe: universe.members().to_vec(),
        factors,
        provenance: provenance.clone(),
    };
    let child =
        run_target_child(&paths.child, claim.job.id, &request, config.child_timeout).await?;
    let portfolio = validate_target_output(
        child,
        &input.resolved_config.strategy_id,
        &input.resolved_config.strategy_version,
        &as_of_text,
        &universe,
        &input.dataset,
        &provenance,
    )?;
    Ok(PreparedRecommendation {
        run_id: input.payload.run_id,
        input,
        universe,
        portfolio,
    })
}

fn validate_production_paths(
    paths: &RecommendationRunnerPaths,
    production: bool,
) -> Result<(), RecommendationRunnerError> {
    if !production {
        return Ok(());
    }
    let configured = [
        (&paths.data_root, true),
        (&paths.universe_manifest, false),
        (&paths.child.repo_root, true),
        (&paths.child.uv_bin, false),
        (&paths.child.temp_root, true),
    ];
    let valid = configured.iter().all(|(path, directory)| {
        path.is_absolute()
            && std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                !metadata.file_type().is_symlink()
                    && if *directory {
                        metadata.is_dir()
                    } else {
                        metadata.is_file()
                    }
            })
    }) && paths
        .universe_manifest
        .file_name()
        .is_some_and(|name| name == "kr-etf-core-v1.yaml");
    if valid {
        Ok(())
    } else {
        Err(RecommendationRunnerError::Queue(QueueError::Internal(
            "production recommendation paths are invalid".into(),
        )))
    }
}

async fn finish_prepared(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    prepared: PreparedRecommendation,
) -> Result<RecommendationOutcome, RecommendationRunnerError> {
    match queue.heartbeat(claim).await {
        Err(_) => {
            return settle_failure_lifecycle(
                pool,
                queue,
                claim,
                StageFailure::new(
                    ErrorClass::Transient,
                    "RECOMMENDATION_HEARTBEAT_UNAVAILABLE",
                    "recommendation heartbeat is temporarily unavailable",
                ),
            )
            .await;
        }
        Ok(HeartbeatStatus::Canceled) => {
            return settle_canceled_lifecycle(pool, queue, claim).await;
        }
        Ok(HeartbeatStatus::LeaseLost) => return lease_lost_outcome(pool, claim).await,
        Ok(HeartbeatStatus::Extended) => {}
    }
    match publish_recommendation(
        pool,
        queue,
        claim,
        &prepared.input,
        &prepared.universe,
        &prepared.portfolio,
    )
    .await
    {
        Ok(_) => Ok(RecommendationOutcome::Succeeded {
            job_id: claim.job.id,
            run_id: prepared.run_id,
        }),
        Err(error) => settle_failure_lifecycle(pool, queue, claim, error.into()).await,
    }
}

async fn attest_dataset_path(
    data_root: &std::path::Path,
    storage_path: &str,
) -> Result<(), StageFailure> {
    let root = data_root.to_path_buf();
    let dataset = PathBuf::from(storage_path);
    let checked = tokio::task::spawn_blocking(move || {
        let root = std::fs::canonicalize(root)?;
        let dataset = std::fs::canonicalize(dataset)?;
        Ok::<_, std::io::Error>(root.is_dir() && dataset.is_dir() && dataset.starts_with(root))
    })
    .await
    .map_err(|_| {
        StageFailure::new(
            ErrorClass::Transient,
            "RECOMMENDATION_DATASET_PATH_UNAVAILABLE",
            "pinned recommendation data is temporarily unavailable",
        )
    })?
    .map_err(|_| {
        StageFailure::new(
            ErrorClass::Transient,
            "RECOMMENDATION_DATASET_PATH_UNAVAILABLE",
            "pinned recommendation data is temporarily unavailable",
        )
    })?;
    if !checked {
        return Err(StageFailure::new(
            ErrorClass::Integrity,
            "RECOMMENDATION_DATASET_PATH_INTEGRITY",
            "pinned recommendation data path is invalid",
        ));
    }
    Ok(())
}

async fn attest_credentialed_sources(
    pool: &PgPool,
    dataset: &AttestedDataset,
) -> Result<(), StageFailure> {
    let storage_path = dataset.storage_path.clone();
    let dataset_id = dataset.dataset_id.clone();
    let curated_version = dataset.curated_version;
    let source_files = tokio::task::spawn_blocking(move || {
        let dataset_id =
            domain::DatasetId::parse(&dataset_id).map_err(|_| ErrorClass::Integrity)?;
        CurateStore::new(storage_path)
            .read_dataset_manifest(&dataset_id, curated_version)
            .map_err(|error| match error {
                market_data::curate::CurateError::StoreIo { .. }
                | market_data::curate::CurateError::RawIo { .. } => ErrorClass::Transient,
                market_data::curate::CurateError::MissingCuratedComponent { .. } => {
                    ErrorClass::DataBlocked
                }
                _ => ErrorClass::Integrity,
            })?
            .ok_or(ErrorClass::DataBlocked)
            .map(|manifest| {
                manifest
                    .source_batches
                    .into_iter()
                    .flat_map(|batch| {
                        let batch_id = batch.batch_id.as_uuid();
                        [
                            (
                                batch_id,
                                batch.bars_file,
                                batch
                                    .bars_hash
                                    .as_str()
                                    .trim_start_matches("sha256:")
                                    .to_owned(),
                            ),
                            (
                                batch_id,
                                batch.actions_file,
                                batch
                                    .actions_hash
                                    .as_str()
                                    .trim_start_matches("sha256:")
                                    .to_owned(),
                            ),
                        ]
                    })
                    .collect::<Vec<_>>()
            })
    })
    .await
    .map_err(|_| {
        StageFailure::new(
            ErrorClass::DataBlocked,
            "RECOMMENDATION_CREDENTIALLED_DATA_REQUIRED",
            "production recommendation data provenance is not attested",
        )
    })?
    .map_err(|class| {
        StageFailure::new(
            class,
            match class {
                ErrorClass::Transient => "RECOMMENDATION_SOURCE_ATTESTATION_UNAVAILABLE",
                ErrorClass::Integrity => "RECOMMENDATION_SOURCE_ATTESTATION_INVALID",
                _ => "RECOMMENDATION_CREDENTIALLED_DATA_REQUIRED",
            },
            match class {
                ErrorClass::Transient => {
                    "production recommendation data provenance is temporarily unavailable"
                }
                ErrorClass::Integrity => {
                    "production recommendation data provenance failed validation"
                }
                _ => "production recommendation data provenance is not attested",
            },
        )
    })?;
    if source_files.is_empty() {
        return Err(StageFailure::new(
            ErrorClass::DataBlocked,
            "RECOMMENDATION_CREDENTIALLED_DATA_REQUIRED",
            "production recommendation data provenance is not attested",
        ));
    }
    let batch_ids = source_files
        .iter()
        .map(|(batch_id, _, _)| *batch_id)
        .collect::<Vec<_>>();
    let file_names = source_files
        .iter()
        .map(|(_, file_name, _)| file_name.as_str())
        .collect::<Vec<_>>();
    let content_hashes = source_files
        .iter()
        .map(|(_, _, content_hash)| content_hash.as_str())
        .collect::<Vec<_>>();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) \
         FROM unnest($1::uuid[], $2::text[], $3::text[]) \
              AS expected(source_batch_id, source_file_name, content_sha256) \
         WHERE EXISTS (SELECT 1 FROM data_batches d \
                        WHERE d.source_batch_id = expected.source_batch_id \
                          AND d.source_file_name = expected.source_file_name \
                          AND d.content_sha256 = expected.content_sha256 \
                          AND d.provider = 'KRX' \
                          AND d.market = 'KR' \
                          AND d.fetch_mode = 'credentialed')",
    )
    .bind(batch_ids)
    .bind(file_names)
    .bind(content_hashes)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        StageFailure::new(
            database_error_class(&error),
            "RECOMMENDATION_SOURCE_ATTESTATION_UNAVAILABLE",
            "production recommendation data provenance is temporarily unavailable",
        )
    })?;
    if count != i64::try_from(source_files.len()).unwrap_or(i64::MAX) {
        return Err(StageFailure::new(
            ErrorClass::DataBlocked,
            "RECOMMENDATION_CREDENTIALLED_DATA_REQUIRED",
            "production recommendation data provenance is not attested",
        ));
    }
    Ok(())
}

async fn attest_current_entitlement(
    pool: &PgPool,
    dataset_id: &str,
    as_of: chrono::NaiveDate,
) -> Result<(), StageFailure> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS (\
            SELECT 1 FROM data_entitlements \
             WHERE status = 'ACTIVE' \
               AND effective_from <= $2 AND effective_until >= $2 \
               AND covered_datasets @> jsonb_build_array($1::text) \
               AND covered_uses @> '[\"recommendation\"]'::jsonb\
         )",
    )
    .bind(dataset_id)
    .bind(as_of)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        StageFailure::new(
            database_error_class(&error),
            "RECOMMENDATION_ENTITLEMENT_UNAVAILABLE",
            "recommendation authorization is temporarily unavailable",
        )
    })?;
    if !allowed {
        return Err(StageFailure::new(
            ErrorClass::DataBlocked,
            "DATA_ENTITLEMENT_REQUIRED",
            "recommendation authorization is inactive",
        ));
    }
    Ok(())
}

async fn attest_current_universe(
    pool: &PgPool,
    universe: &AttestedUniverse,
) -> Result<(), StageFailure> {
    let current: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM universe_snapshots \
         WHERE snapshot_id = $1 AND instruments_json = $2)",
    )
    .bind(universe.snapshot_id())
    .bind(json!(universe.members()))
    .fetch_one(pool)
    .await
    .map_err(|error| {
        StageFailure::new(
            database_error_class(&error),
            "RECOMMENDATION_UNIVERSE_UNAVAILABLE",
            "recommendation universe is temporarily unavailable",
        )
    })?;
    if !current {
        return Err(StageFailure::new(
            ErrorClass::DataBlocked,
            "RECOMMENDATION_UNIVERSE_BLOCKED",
            "recommendation universe is unavailable",
        ));
    }
    Ok(())
}

async fn settle_failure_lifecycle(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    failure: StageFailure,
) -> Result<RecommendationOutcome, RecommendationRunnerError> {
    let mut transaction = pool.begin().await.map_err(QueueError::Database)?;
    let settlement = queue
        .settle_failure_in(
            &mut transaction,
            claim,
            failure.class,
            &failure.code,
            failure.summary,
        )
        .await?;
    let (outcome, run_status, summary_code, summary) = match settlement {
        SettleResult::Canceled(_) => (
            RecommendationOutcome::Failed {
                job_id: claim.job.id,
                code: "RECOMMENDATION_CANCELED".to_owned(),
            },
            Some("FAILED"),
            "RECOMMENDATION_CANCELED",
            "recommendation was canceled",
        ),
        SettleResult::Committed(job) if job.status == JobStatus::Queued => (
            RecommendationOutcome::Retrying {
                job_id: claim.job.id,
                code: failure.code.clone(),
            },
            None,
            failure.code.as_str(),
            failure.summary,
        ),
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            let blocked = failure.class == ErrorClass::DataBlocked;
            (
                if blocked {
                    RecommendationOutcome::Blocked {
                        job_id: claim.job.id,
                        code: failure.code.clone(),
                    }
                } else {
                    RecommendationOutcome::Failed {
                        job_id: claim.job.id,
                        code: failure.code.clone(),
                    }
                },
                Some(if blocked { "BLOCKED" } else { "FAILED" }),
                failure.code.as_str(),
                failure.summary,
            )
        }
        SettleResult::Committed(_) => {
            transaction.rollback().await.map_err(QueueError::Database)?;
            return Err(RecommendationRunnerError::Queue(QueueError::Internal(
                "failure settlement returned an invalid job status".into(),
            )));
        }
    };
    if let Some(run_status) = run_status {
        let summary_json = json!({
            "code": summary_code,
            "message": summary,
        });
        sqlx::query(
            "UPDATE recommendation_runs SET status = $3, summary_json = $4 \
             WHERE job_id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
        )
        .bind(claim.job.id)
        .bind(claim.job.owner_user_id)
        .bind(run_status)
        .bind(summary_json)
        .execute(&mut *transaction)
        .await
        .map_err(QueueError::Database)?;
    }
    transaction.commit().await.map_err(QueueError::Database)?;
    Ok(outcome)
}

async fn settle_canceled_lifecycle(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<RecommendationOutcome, RecommendationRunnerError> {
    let mut transaction = pool.begin().await.map_err(QueueError::Database)?;
    queue
        .settle_aborted_in(&mut transaction, claim, "recommendation canceled")
        .await?;
    sqlx::query(
        "UPDATE recommendation_runs \
         SET status = 'FAILED', summary_json = $3 \
         WHERE job_id = $1 AND owner_user_id = $2 AND status = 'PENDING'",
    )
    .bind(claim.job.id)
    .bind(claim.job.owner_user_id)
    .bind(json!({
        "code": "RECOMMENDATION_CANCELED",
        "message": "recommendation was canceled",
    }))
    .execute(&mut *transaction)
    .await
    .map_err(QueueError::Database)?;
    transaction.commit().await.map_err(QueueError::Database)?;
    Ok(RecommendationOutcome::Failed {
        job_id: claim.job.id,
        code: "RECOMMENDATION_CANCELED".to_owned(),
    })
}
