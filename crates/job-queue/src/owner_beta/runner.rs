//! One-shot, lease-supervised owner-beta price recommendation worker.

use std::{
    fmt,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

use sqlx::PgPool;
use tokio::sync::{Semaphore, watch};
use uuid::Uuid;

use super::publish::settle_malformed_owner_beta_claim;
use super::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaComputation, OwnerBetaComputationError,
    OwnerBetaPriceRecommendationInput, OwnerBetaPublicationError, OwnerBetaPublicationFailure,
    OwnerBetaPublicationOutcome, compute_owner_beta_price_recommendation,
    publish_owner_beta_success, settle_owner_beta_failure,
};
use crate::{
    ClaimedJob, ErrorClass, HeartbeatStatus, JobQueue, QueueError, error::queue_error_class,
};

/// One retained permit bounds detached synchronous artifact reads globally.
/// Timeout or lease-monitor stop aborts unstarted work; started read-only work
/// retains the permit until the closure exits, preventing retry pileups.
static OWNER_BETA_COMPUTE_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn compute_slots() -> Arc<Semaphore> {
    OWNER_BETA_COMPUTE_SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

#[derive(Clone)]
pub struct OwnerBetaRunnerPaths {
    pub artifact_root: PathBuf,
}

impl fmt::Debug for OwnerBetaRunnerPaths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerBetaRunnerPaths")
            .field("artifact_root", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerBetaRunnerConfig {
    heartbeat_interval: Duration,
    lease: Duration,
    compute_timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaRunnerConfigError {
    #[error("owner-beta runner durations must be positive")]
    ZeroDuration,
    #[error("owner-beta runner heartbeat must be shorter than its lease")]
    HeartbeatNotBeforeLease,
}

impl OwnerBetaRunnerConfig {
    pub fn new(
        heartbeat_interval: Duration,
        lease: Duration,
        compute_timeout: Duration,
    ) -> Result<Self, OwnerBetaRunnerConfigError> {
        if heartbeat_interval.is_zero() || lease.is_zero() || compute_timeout.is_zero() {
            return Err(OwnerBetaRunnerConfigError::ZeroDuration);
        }
        if heartbeat_interval >= lease {
            return Err(OwnerBetaRunnerConfigError::HeartbeatNotBeforeLease);
        }
        Ok(Self {
            heartbeat_interval,
            lease,
            compute_timeout,
        })
    }
    pub const fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }
    pub const fn lease(&self) -> Duration {
        self.lease
    }
    pub const fn compute_timeout(&self) -> Duration {
        self.compute_timeout
    }
}

/// Sanitized runner errors never retain payload or filesystem values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaRunnerError {
    #[error("owner-beta artifact root is invalid")]
    InvalidArtifactRoot,
    #[error("owner-beta queue is unavailable")]
    QueueUnavailable,
    #[error("owner-beta publication is unavailable")]
    PublicationUnavailable,
}
impl From<QueueError> for OwnerBetaRunnerError {
    fn from(_: QueueError) -> Self {
        Self::QueueUnavailable
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerBetaOutcome {
    Idle,
    Rejected {
        job_id: Uuid,
    },
    RejectedCanceled {
        job_id: Uuid,
    },
    RejectedIndeterminate {
        job_id: Uuid,
    },
    Succeeded {
        job_id: Uuid,
        run_id: Uuid,
    },
    Retrying {
        job_id: Uuid,
        run_id: Uuid,
        code: &'static str,
    },
    Blocked {
        job_id: Uuid,
        run_id: Uuid,
        code: &'static str,
    },
    Failed {
        job_id: Uuid,
        run_id: Uuid,
        code: &'static str,
    },
    Canceled {
        job_id: Uuid,
        run_id: Uuid,
    },
    LeaseLost {
        job_id: Uuid,
        run_id: Uuid,
    },
    Indeterminate {
        job_id: Uuid,
        run_id: Uuid,
    },
}

/// Claim and execute one exact owner-beta job. Durable transitions stay in
/// the sealed owner-beta publisher; this runner never generic-settles/sweeps.
pub async fn run_once(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    paths: &OwnerBetaRunnerPaths,
    config: &OwnerBetaRunnerConfig,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    if config.lease != queue.config().lease {
        return Err(OwnerBetaRunnerError::QueueUnavailable);
    }
    validate_artifact_root(&paths.artifact_root)?;
    let Some(claim) = queue
        .claim_next_for(worker_id, OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
        .await?
    else {
        return Ok(OwnerBetaOutcome::Idle);
    };
    let input = match serde_json::from_value::<OwnerBetaPriceRecommendationInput>(
        claim.job.payload_json.clone(),
    ) {
        Ok(input) => input,
        Err(_) => return malformed_claim(pool, queue, &claim).await,
    };
    if input.validate_strategy_snapshot().is_err() {
        return settle(
            pool,
            queue,
            &claim,
            &input,
            OwnerBetaPublicationFailure::InputInvalid,
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
    let timeout = tokio::time::sleep(config.compute_timeout);
    tokio::pin!(timeout);
    let permit = tokio::select! {
        monitor_result = &mut monitor => return monitor_outcome(pool, queue, &claim, &input, join_monitor(monitor_result)).await,
        _ = &mut timeout => { let _ = stop_tx.send(true); return after_stop(pool, queue, &claim, &input, monitor.await, OwnerBetaPublicationFailure::ComputationUnavailable).await; }
        permit = compute_slots().acquire_owned() => permit.map_err(|_| OwnerBetaRunnerError::PublicationUnavailable)?,
    };
    let root = paths.artifact_root.clone();
    let compute_input = input.clone();
    let mut compute = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        compute_owner_beta_price_recommendation(&root, &compute_input)
    });
    tokio::select! {
        monitor_result = &mut monitor => { compute.abort(); monitor_outcome(pool, queue, &claim, &input, join_monitor(monitor_result)).await }
        _ = &mut timeout => { compute.abort(); let _ = stop_tx.send(true); after_stop(pool, queue, &claim, &input, monitor.await, OwnerBetaPublicationFailure::ComputationUnavailable).await }
        computation = &mut compute => {
            let _ = stop_tx.send(true);
            match join_monitor(monitor.await) {
                LeaseMonitorOutcome::Stopped => match computation {
                    Ok(Ok(computation)) => finish(pool, queue, &claim, &input, computation).await,
                    Ok(Err(error)) => settle(pool, queue, &claim, &input, computation_failure(error)).await,
                    Err(_) => settle(pool, queue, &claim, &input, OwnerBetaPublicationFailure::ComputationUnavailable).await,
                },
                outcome => monitor_outcome(pool, queue, &claim, &input, outcome).await,
            }
        }
    }
}

fn validate_artifact_root(path: &Path) -> Result<(), OwnerBetaRunnerError> {
    if !path.is_absolute() || path == Path::new("/") {
        return Err(OwnerBetaRunnerError::InvalidArtifactRoot);
    }
    validate_lexical_root(path)?;
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir))
        || components.any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(OwnerBetaRunnerError::InvalidArtifactRoot);
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| OwnerBetaRunnerError::InvalidArtifactRoot)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OwnerBetaRunnerError::InvalidArtifactRoot);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_lexical_root(path: &Path) -> Result<(), OwnerBetaRunnerError> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.ends_with(b"/")
        || bytes.windows(2).any(|window| window == b"//")
        || bytes
            .split(|byte| *byte == b'/')
            .any(|component| component == b"." || component == b"..")
    {
        return Err(OwnerBetaRunnerError::InvalidArtifactRoot);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_lexical_root(_path: &Path) -> Result<(), OwnerBetaRunnerError> {
    Err(OwnerBetaRunnerError::InvalidArtifactRoot)
}

async fn malformed_claim(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match settle_malformed_owner_beta_claim(pool, queue, claim).await {
        Ok(OwnerBetaPublicationOutcome::Failed) => Ok(OwnerBetaOutcome::Rejected {
            job_id: claim.job.id,
        }),
        Ok(OwnerBetaPublicationOutcome::Canceled) => Ok(OwnerBetaOutcome::RejectedCanceled {
            job_id: claim.job.id,
        }),
        Err(OwnerBetaPublicationError::CommitUnknown) => {
            Ok(OwnerBetaOutcome::RejectedIndeterminate {
                job_id: claim.job.id,
            })
        }
        Ok(OwnerBetaPublicationOutcome::Published | OwnerBetaPublicationOutcome::Retrying) => {
            Err(OwnerBetaRunnerError::PublicationUnavailable)
        }
        Err(_) => Err(OwnerBetaRunnerError::PublicationUnavailable),
    }
}

#[derive(Debug, Clone, Copy)]
enum LeaseMonitorOutcome {
    Stopped,
    Canceled,
    LeaseLost,
    Failed(ErrorClass),
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
            changed = stop.changed() => if changed.is_err() || *stop.borrow() { return LeaseMonitorOutcome::Stopped; },
            _ = ticker.tick() => match queue.heartbeat(claim).await {
                Ok(HeartbeatStatus::Extended) => {}, Ok(HeartbeatStatus::Canceled) => return LeaseMonitorOutcome::Canceled,
                Ok(HeartbeatStatus::LeaseLost) => return LeaseMonitorOutcome::LeaseLost,
                Err(error) => return LeaseMonitorOutcome::Failed(queue_error_class(&error)),
            },
        }
    }
}
fn join_monitor(
    result: Result<LeaseMonitorOutcome, tokio::task::JoinError>,
) -> LeaseMonitorOutcome {
    result.unwrap_or(LeaseMonitorOutcome::Failed(ErrorClass::Transient))
}

async fn after_stop(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    monitored: Result<LeaseMonitorOutcome, tokio::task::JoinError>,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match join_monitor(monitored) {
        LeaseMonitorOutcome::Stopped => settle(pool, queue, claim, input, failure).await,
        outcome => monitor_outcome(pool, queue, claim, input, outcome).await,
    }
}
async fn monitor_outcome(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    outcome: LeaseMonitorOutcome,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match outcome {
        LeaseMonitorOutcome::Stopped | LeaseMonitorOutcome::Failed(ErrorClass::Transient) => {
            settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::PublicationUnavailable,
            )
            .await
        }
        LeaseMonitorOutcome::Canceled => {
            settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::Canceled,
            )
            .await
        }
        LeaseMonitorOutcome::LeaseLost => Ok(OwnerBetaOutcome::LeaseLost {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        LeaseMonitorOutcome::Failed(_) => {
            settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::ComputationUnavailable,
            )
            .await
        }
    }
}
async fn finish(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    computation: OwnerBetaComputation,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match queue.heartbeat(claim).await? {
        HeartbeatStatus::Extended => {}
        HeartbeatStatus::Canceled => {
            return settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::Canceled,
            )
            .await;
        }
        HeartbeatStatus::LeaseLost => {
            return Ok(OwnerBetaOutcome::LeaseLost {
                job_id: claim.job.id,
                run_id: input.run_id(),
            });
        }
    }
    match publish_owner_beta_success(
        pool,
        queue,
        claim,
        input,
        computation.factor_snapshot(),
        computation.target_snapshot(),
    )
    .await
    {
        Ok(OwnerBetaPublicationOutcome::Published) => Ok(OwnerBetaOutcome::Succeeded {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        Ok(OwnerBetaPublicationOutcome::Canceled) => Ok(OwnerBetaOutcome::Canceled {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        Ok(OwnerBetaPublicationOutcome::Retrying) => Ok(OwnerBetaOutcome::Retrying {
            job_id: claim.job.id,
            run_id: input.run_id(),
            code: "OWNER_BETA_PUBLICATION_UNAVAILABLE",
        }),
        Ok(OwnerBetaPublicationOutcome::Failed) => Ok(OwnerBetaOutcome::Failed {
            job_id: claim.job.id,
            run_id: input.run_id(),
            code: "OWNER_BETA_COMPUTATION_FAILED",
        }),
        Err(error) => publication_error(pool, queue, claim, input, error).await,
    }
}
async fn publication_error(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    error: OwnerBetaPublicationError,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match error {
        OwnerBetaPublicationError::CommitUnknown => Ok(OwnerBetaOutcome::Indeterminate {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        OwnerBetaPublicationError::QueueClaimLost => Ok(OwnerBetaOutcome::LeaseLost {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        OwnerBetaPublicationError::EntitlementDenied => {
            settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::EntitlementDenied,
            )
            .await
        }
        OwnerBetaPublicationError::DatabaseTransient
        | OwnerBetaPublicationError::QueueTransient => {
            settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::PublicationUnavailable,
            )
            .await
        }
        _ => {
            settle(
                pool,
                queue,
                claim,
                input,
                OwnerBetaPublicationFailure::ComputationFailed,
            )
            .await
        }
    }
}
fn computation_failure(error: OwnerBetaComputationError) -> OwnerBetaPublicationFailure {
    match error {
        OwnerBetaComputationError::StrategyInvalid => OwnerBetaPublicationFailure::InputInvalid,
        OwnerBetaComputationError::ArtifactApprovalRejected => {
            OwnerBetaPublicationFailure::EntitlementDenied
        }
        OwnerBetaComputationError::FactorDefinitionInvalid => {
            OwnerBetaPublicationFailure::FactorInvalid
        }
        OwnerBetaComputationError::FactorComputeInvalid => {
            OwnerBetaPublicationFailure::ComputationFailed
        }
        OwnerBetaComputationError::TargetInvalid => OwnerBetaPublicationFailure::TargetInvalid,
    }
}
async fn settle(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaPriceRecommendationInput,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match settle_owner_beta_failure(pool, queue, claim, input, failure).await {
        Ok(OwnerBetaPublicationOutcome::Retrying) => Ok(OwnerBetaOutcome::Retrying {
            job_id: claim.job.id,
            run_id: input.run_id(),
            code: failure.code(),
        }),
        Ok(OwnerBetaPublicationOutcome::Failed) if failure.class() == ErrorClass::DataBlocked => {
            Ok(OwnerBetaOutcome::Blocked {
                job_id: claim.job.id,
                run_id: input.run_id(),
                code: failure.code(),
            })
        }
        Ok(OwnerBetaPublicationOutcome::Failed) => Ok(OwnerBetaOutcome::Failed {
            job_id: claim.job.id,
            run_id: input.run_id(),
            code: failure.code(),
        }),
        Ok(OwnerBetaPublicationOutcome::Canceled) => Ok(OwnerBetaOutcome::Canceled {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        Ok(OwnerBetaPublicationOutcome::Published) => {
            Err(OwnerBetaRunnerError::PublicationUnavailable)
        }
        Err(OwnerBetaPublicationError::CommitUnknown) => Ok(OwnerBetaOutcome::Indeterminate {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        Err(OwnerBetaPublicationError::QueueClaimLost) => Ok(OwnerBetaOutcome::LeaseLost {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        Err(_) => Err(OwnerBetaRunnerError::PublicationUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_enforces_positive_ordered_durations() {
        assert_eq!(
            OwnerBetaRunnerConfig::new(
                Duration::ZERO,
                Duration::from_secs(2),
                Duration::from_secs(1)
            ),
            Err(OwnerBetaRunnerConfigError::ZeroDuration)
        );
        assert_eq!(
            OwnerBetaRunnerConfig::new(
                Duration::from_secs(2),
                Duration::from_secs(2),
                Duration::from_secs(1)
            ),
            Err(OwnerBetaRunnerConfigError::HeartbeatNotBeforeLease)
        );
    }
    #[test]
    fn paths_errors_and_preflight_are_redacted_and_fail_closed() {
        let paths = OwnerBetaRunnerPaths {
            artifact_root: PathBuf::from("/private/owner-beta"),
        };
        assert!(!format!("{paths:?}").contains("private"));
        assert_eq!(
            validate_artifact_root(Path::new("relative")),
            Err(OwnerBetaRunnerError::InvalidArtifactRoot)
        );
        assert_eq!(
            validate_artifact_root(Path::new("/")),
            Err(OwnerBetaRunnerError::InvalidArtifactRoot)
        );
        assert_eq!(
            validate_artifact_root(Path::new("/tmp/../tmp")),
            Err(OwnerBetaRunnerError::InvalidArtifactRoot)
        );
        assert_eq!(
            validate_artifact_root(Path::new("/tmp//owner-beta")),
            Err(OwnerBetaRunnerError::InvalidArtifactRoot)
        );
    }
    #[test]
    fn computation_errors_map_to_sealed_failures() {
        assert_eq!(
            computation_failure(OwnerBetaComputationError::StrategyInvalid),
            OwnerBetaPublicationFailure::InputInvalid
        );
        assert_eq!(
            computation_failure(OwnerBetaComputationError::ArtifactApprovalRejected),
            OwnerBetaPublicationFailure::EntitlementDenied
        );
        assert_eq!(
            computation_failure(OwnerBetaComputationError::FactorDefinitionInvalid),
            OwnerBetaPublicationFailure::FactorInvalid
        );
        assert_eq!(
            computation_failure(OwnerBetaComputationError::FactorComputeInvalid),
            OwnerBetaPublicationFailure::ComputationFailed
        );
        assert_eq!(
            computation_failure(OwnerBetaComputationError::TargetInvalid),
            OwnerBetaPublicationFailure::TargetInvalid
        );
    }
    #[test]
    fn source_keeps_sealed_bounded_contracts() {
        let source = include_str!("runner.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("validate_artifact_root(&paths.artifact_root)?"));
        assert!(
            production
                .contains("claim_next_for(worker_id, OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)")
        );
        assert!(production.contains("settle_malformed_owner_beta_claim"));
        assert!(production.contains("compute_owner_beta_price_recommendation"));
        assert!(production.contains("publish_owner_beta_success"));
        assert!(production.contains("settle_owner_beta_failure"));
        assert!(production.contains("OWNER_BETA_COMPUTE_SLOTS"));
        assert!(production.contains("let _permit = permit"));
        assert!(!production.contains(".settle_success("));
        assert!(!production.contains(".settle_failure("));
        assert!(!production.contains(".sweep"));
        assert!(!production.contains("std::env"));
        assert!(!production.contains("Command::"));
    }

    #[test]
    fn commit_unknown_never_attempts_a_followup_settlement() {
        let source = include_str!("runner.rs");
        let branch = source
            .split("Err(OwnerBetaPublicationError::CommitUnknown)")
            .nth(1)
            .expect("malformed commit-unknown branch");
        assert!(
            branch.starts_with(" => {\n            Ok(OwnerBetaOutcome::RejectedIndeterminate")
        );
    }
}
