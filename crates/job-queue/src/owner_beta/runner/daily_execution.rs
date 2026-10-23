//! Mixed historical/daily queue dispatch with retained sealed input lifetimes.
//! No scheduler, provider, process activation or generic queue settlement here.

use std::{fmt, path::Path, time::Instant};

use market_data::{
    approve_historical_price_only_artifact,
    daily_price_only::{
        DailyPricePinsV1Wire,
        admission::{DailyAdmissionVerificationError, PinnedDailyPolicy},
        artifact::{DailyAdmissionRoot, DailyArtifactError},
        replay::{DailyReplayError, DailyReplayLimits, read_sealed_daily_chain},
    },
};
use sqlx::PgPool;
use tokio::sync::watch;

use super::{
    LeaseMonitorOutcome, OwnerBetaOutcome, OwnerBetaRunnerConfig, OwnerBetaRunnerError,
    OwnerBetaRunnerPaths, compute_slots, join_monitor, malformed_claim, monitor_lease,
    run_historical_claim, validate_artifact_root, validate_lexical_root,
};
use crate::{
    ClaimedJob, ErrorClass, HeartbeatStatus, JobQueue,
    owner_beta::{
        OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPublicationError,
        OwnerBetaPublicationFailure, OwnerBetaPublicationOutcome,
        daily_compute::{OwnerBetaDailyComputationError, compute_owner_beta_daily_recommendation},
        daily_input::{OwnerBetaDailyRecommendationInputV1, OwnerBetaExecutionInput},
        publish::{
            DailyClaimPreflight, preflight_owner_beta_daily_claim,
            publish_owner_beta_daily_success, settle_owner_beta_daily_failure,
        },
    },
};

/// Explicit installed-policy and admission-root constraints. This is configuration,
/// never DB authority or an admitted input. There is no default policy/root/budget.
pub struct DailyExecutionContext<'a> {
    pub admission_root: &'a Path,
    pub trusted_owner: u32,
    pub reader_group: u32,
    pub policy: &'a PinnedDailyPolicy,
    pub replay_limits: DailyReplayLimits,
}
impl fmt::Debug for DailyExecutionContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DailyExecutionContext")
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub enum DailyExecutionMode<'a> {
    #[default]
    Disabled,
    OwnerOnly(DailyExecutionContext<'a>),
}

/// Both formats share the protected job type, so one consumer strictly dispatches
/// after claiming. The legacy run_once API remains historical-only. Deployment
/// must select this mixed consumer when daily scheduling is enabled.
pub async fn run_once_with_daily(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    paths: &OwnerBetaRunnerPaths,
    config: &OwnerBetaRunnerConfig,
    daily: &DailyExecutionMode<'_>,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    if config.lease() != queue.config().lease {
        return Err(OwnerBetaRunnerError::QueueUnavailable);
    }
    if let DailyExecutionMode::OwnerOnly(context) = daily {
        // block_in_place below keeps borrowing capabilities on the calling future.
        // A current-thread runtime must fail before taking any job.
        ensure_multithread_runtime()?;
        if !context.admission_root.is_absolute() || context.admission_root == Path::new("/") {
            return Err(OwnerBetaRunnerError::InvalidArtifactRoot);
        }
        validate_lexical_root(context.admission_root)?;
    }
    validate_artifact_root(&paths.artifact_root)?;
    let Some(claim) = queue
        .claim_next_for(worker_id, OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
        .await?
    else {
        return Ok(OwnerBetaOutcome::Idle);
    };
    let input: OwnerBetaExecutionInput =
        match serde_json::from_value(claim.job.payload_json.clone()) {
            Ok(input) => input,
            Err(_) => return malformed_claim(pool, queue, &claim).await,
        };
    match input {
        OwnerBetaExecutionInput::Historical(input) => {
            run_historical_claim(pool, queue, claim, input, paths, config).await
        }
        OwnerBetaExecutionInput::Daily(input) => match daily {
            DailyExecutionMode::Disabled => {
                settle(
                    pool,
                    queue,
                    &claim,
                    &input,
                    OwnerBetaPublicationFailure::DailyExecutionDisabled,
                )
                .await
            }
            DailyExecutionMode::OwnerOnly(context) => {
                execute_daily(pool, queue, claim, input, paths, config, context).await
            }
        },
    }
}

fn ensure_multithread_runtime() -> Result<(), OwnerBetaRunnerError> {
    match tokio::runtime::Handle::try_current().map(|handle| handle.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => Ok(()),
        _ => Err(OwnerBetaRunnerError::UnsupportedDailyRuntime),
    }
}

async fn execute_daily(
    pool: &PgPool,
    queue: &JobQueue,
    claim: ClaimedJob,
    input: OwnerBetaDailyRecommendationInputV1,
    paths: &OwnerBetaRunnerPaths,
    config: &OwnerBetaRunnerConfig,
    context: &DailyExecutionContext<'_>,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    if input.pins().policy_sha256() != context.policy.sha256() {
        return settle(
            pool,
            queue,
            &claim,
            &input,
            OwnerBetaPublicationFailure::InputInvalid,
        )
        .await;
    }
    let Some(deadline) = Instant::now().checked_add(config.compute_timeout()) else {
        return settle(
            pool,
            queue,
            &claim,
            &input,
            OwnerBetaPublicationFailure::ComputationUnavailable,
        )
        .await;
    };
    let (stop_tx, stop_rx) = watch::channel(false);
    let monitor_queue = queue.clone();
    let monitor_claim = claim.clone();
    let interval = config.heartbeat_interval();
    let mut monitor = tokio::spawn(async move {
        monitor_lease(&monitor_queue, &monitor_claim, interval, stop_rx).await
    });
    // Dropping this future drops stop_tx; a detached monitor then exits rather
    // than extending a canceled computation indefinitely.
    let timeout = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
    tokio::pin!(timeout);
    let authority = tokio::select! {
        result = &mut monitor => return monitored(pool, queue, &claim, &input, join_monitor(result)).await,
        _ = &mut timeout => {
            let _ = stop_tx.send(true);
            return after_stop(pool, queue, &claim, &input, monitor.await,
                OwnerBetaPublicationFailure::ComputationUnavailable).await;
        }
        result = preflight_owner_beta_daily_claim(pool, queue, &claim, &input) => result,
    };
    match authority {
        Ok(DailyClaimPreflight::Ready) => {}
        Ok(DailyClaimPreflight::Canceled) => {
            let _ = stop_tx.send(true);
            let _ = monitor.await;
            return Ok(OwnerBetaOutcome::Canceled {
                job_id: claim.job.id,
                run_id: input.run_id(),
            });
        }
        Err(error) => {
            let _ = stop_tx.send(true);
            let outcome = join_monitor(monitor.await);
            return match outcome {
                LeaseMonitorOutcome::Stopped => {
                    publication_error(pool, queue, &claim, &input, error).await
                }
                outcome => monitored(pool, queue, &claim, &input, outcome).await,
            };
        }
    }
    let permit = tokio::select! {
        result = &mut monitor => return monitored(pool, queue, &claim, &input, join_monitor(result)).await,
        _ = &mut timeout => {
            let _ = stop_tx.send(true);
            return after_stop(pool, queue, &claim, &input, monitor.await,
                OwnerBetaPublicationFailure::ComputationUnavailable).await;
        }
        permit = compute_slots().acquire_owned() =>
            permit.map_err(|_| OwnerBetaRunnerError::PublicationUnavailable)?,
    };
    let base_root = paths.artifact_root.clone();
    // Started read-only loads retain the global permit after timeout/abort.
    // This closure can never publish, settle, or execute a DB operation.
    let mut base_load = tokio::task::spawn_blocking(move || {
        let result = approve_historical_price_only_artifact(&base_root);
        (permit, result)
    });
    let loaded = tokio::select! {
        result = &mut monitor => {
            base_load.abort();
            return monitored(pool, queue, &claim, &input, join_monitor(result)).await;
        }
        _ = &mut timeout => {
            base_load.abort();
            let _ = stop_tx.send(true);
            return after_stop(pool, queue, &claim, &input, monitor.await,
                OwnerBetaPublicationFailure::ComputationUnavailable).await;
        }
        result = &mut base_load => result,
    };
    let (permit, base) = match loaded {
        Ok((permit, Ok(base))) => (permit, base),
        result => {
            let failure = if result.is_err() {
                OwnerBetaPublicationFailure::ComputationUnavailable
            } else {
                OwnerBetaPublicationFailure::EntitlementDenied
            };
            let _ = stop_tx.send(true);
            return after_stop(pool, queue, &claim, &input, monitor.await, failure).await;
        }
    };
    let _permit = permit;
    let expected = DailyPricePinsV1Wire {
        content_sha256: input.pins().content_sha256().clone(),
        manifest_sha256: input.pins().manifest_sha256().clone(),
        admission_sha256: input.pins().admission_sha256().clone(),
        policy_sha256: input.pins().policy_sha256().clone(),
    };
    // Keep the actual base, policy borrow and final file owners in this async
    // future through publication. No self-reference or synthetic capability.
    // Synchronous bounded reads/computation are not individually preemptible.
    let computed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokio::task::block_in_place(|| {
            let keep_working = || Instant::now() < deadline && !monitor.is_finished();
            if !keep_working() {
                return Err(OwnerBetaPublicationFailure::ComputationUnavailable);
            }
            let root = DailyAdmissionRoot::open(
                context.admission_root,
                context.trusted_owner,
                context.reader_group,
            )
            .map_err(|error| replay_failure(DailyReplayError::Artifact(error)))?;
            let chain = read_sealed_daily_chain(
                &root,
                &expected,
                &base,
                context.policy,
                context.replay_limits,
                keep_working,
            )
            .map_err(replay_failure)?;
            if !keep_working() {
                return Err(OwnerBetaPublicationFailure::ComputationUnavailable);
            }
            let admitted = chain.admitted().map_err(replay_failure)?;
            let computation = compute_owner_beta_daily_recommendation(&admitted, &input)
                .map_err(computation_failure)?;
            if !keep_working() {
                return Err(OwnerBetaPublicationFailure::ComputationUnavailable);
            }
            Ok((chain, computation))
        })
    }))
    .unwrap_or(Err(OwnerBetaPublicationFailure::ComputationUnavailable));
    let _ = stop_tx.send(true);
    let monitored_result = join_monitor(monitor.await);
    if !matches!(monitored_result, LeaseMonitorOutcome::Stopped) {
        return monitored(pool, queue, &claim, &input, monitored_result).await;
    }
    if Instant::now() >= deadline {
        return settle(
            pool,
            queue,
            &claim,
            &input,
            OwnerBetaPublicationFailure::ComputationUnavailable,
        )
        .await;
    }
    let (chain, computation) = match computed {
        Ok(value) => value,
        Err(failure) => return settle(pool, queue, &claim, &input, failure).await,
    };
    match queue.heartbeat(&claim).await? {
        HeartbeatStatus::Extended => {}
        HeartbeatStatus::Canceled => {
            return settle(
                pool,
                queue,
                &claim,
                &input,
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
    let admitted = match chain.admitted() {
        Ok(value) => value,
        Err(error) => return settle(pool, queue, &claim, &input, replay_failure(error)).await,
    };
    match publish_owner_beta_daily_success(pool, queue, &claim, &input, &admitted, &computation)
        .await
    {
        Ok(OwnerBetaPublicationOutcome::Published) => Ok(OwnerBetaOutcome::Succeeded {
            job_id: claim.job.id,
            run_id: input.run_id(),
        }),
        Ok(outcome) => outcome_for(
            &claim,
            &input,
            outcome,
            OwnerBetaPublicationFailure::PublicationUnavailable,
        ),
        Err(error) => publication_error(pool, queue, &claim, &input, error).await,
    }
}

fn replay_failure(error: DailyReplayError) -> OwnerBetaPublicationFailure {
    match error {
        DailyReplayError::Interrupted => OwnerBetaPublicationFailure::ComputationUnavailable,
        DailyReplayError::Artifact(DailyArtifactError::Io)
        | DailyReplayError::Verification(DailyAdmissionVerificationError::Artifact(
            DailyArtifactError::Io,
        )) => OwnerBetaPublicationFailure::ComputationUnavailable,
        _ => OwnerBetaPublicationFailure::InputInvalid,
    }
}
fn computation_failure(error: OwnerBetaDailyComputationError) -> OwnerBetaPublicationFailure {
    match error {
        OwnerBetaDailyComputationError::InputInvalid
        | OwnerBetaDailyComputationError::StrategyInvalid => {
            OwnerBetaPublicationFailure::InputInvalid
        }
        OwnerBetaDailyComputationError::FactorDefinitionInvalid => {
            OwnerBetaPublicationFailure::FactorInvalid
        }
        OwnerBetaDailyComputationError::FactorComputeInvalid => {
            OwnerBetaPublicationFailure::ComputationFailed
        }
        OwnerBetaDailyComputationError::TargetInvalid => OwnerBetaPublicationFailure::TargetInvalid,
    }
}
async fn after_stop(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaDailyRecommendationInputV1,
    result: Result<LeaseMonitorOutcome, tokio::task::JoinError>,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match join_monitor(result) {
        LeaseMonitorOutcome::Stopped => settle(pool, queue, claim, input, failure).await,
        outcome => monitored(pool, queue, claim, input, outcome).await,
    }
}
async fn monitored(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaDailyRecommendationInputV1,
    outcome: LeaseMonitorOutcome,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    let failure = match outcome {
        LeaseMonitorOutcome::LeaseLost => {
            return Ok(OwnerBetaOutcome::LeaseLost {
                job_id: claim.job.id,
                run_id: input.run_id(),
            });
        }
        LeaseMonitorOutcome::Canceled => OwnerBetaPublicationFailure::Canceled,
        LeaseMonitorOutcome::Stopped | LeaseMonitorOutcome::Failed(ErrorClass::Transient) => {
            OwnerBetaPublicationFailure::PublicationUnavailable
        }
        LeaseMonitorOutcome::Failed(_) => OwnerBetaPublicationFailure::ComputationUnavailable,
    };
    settle(pool, queue, claim, input, failure).await
}
async fn publication_error(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaDailyRecommendationInputV1,
    error: OwnerBetaPublicationError,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    let failure = match error {
        OwnerBetaPublicationError::CommitUnknown => {
            return Ok(OwnerBetaOutcome::Indeterminate {
                job_id: claim.job.id,
                run_id: input.run_id(),
            });
        }
        OwnerBetaPublicationError::QueueClaimLost => {
            return Ok(OwnerBetaOutcome::LeaseLost {
                job_id: claim.job.id,
                run_id: input.run_id(),
            });
        }
        OwnerBetaPublicationError::DailyAuthorityDenied => {
            OwnerBetaPublicationFailure::DailyAuthorityDenied
        }
        OwnerBetaPublicationError::EntitlementDenied => {
            OwnerBetaPublicationFailure::EntitlementDenied
        }
        OwnerBetaPublicationError::DatabaseTransient
        | OwnerBetaPublicationError::QueueTransient => {
            OwnerBetaPublicationFailure::PublicationUnavailable
        }
        _ => OwnerBetaPublicationFailure::ComputationFailed,
    };
    settle(pool, queue, claim, input, failure).await
}
fn outcome_for(
    claim: &ClaimedJob,
    input: &OwnerBetaDailyRecommendationInputV1,
    outcome: OwnerBetaPublicationOutcome,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    let job_id = claim.job.id;
    let run_id = input.run_id();
    let code = failure.code();
    Ok(match outcome {
        OwnerBetaPublicationOutcome::Retrying => OwnerBetaOutcome::Retrying {
            job_id,
            run_id,
            code,
        },
        OwnerBetaPublicationOutcome::Failed if failure.class() == ErrorClass::DataBlocked => {
            OwnerBetaOutcome::Blocked {
                job_id,
                run_id,
                code,
            }
        }
        OwnerBetaPublicationOutcome::Failed => OwnerBetaOutcome::Failed {
            job_id,
            run_id,
            code,
        },
        OwnerBetaPublicationOutcome::Canceled => OwnerBetaOutcome::Canceled { job_id, run_id },
        OwnerBetaPublicationOutcome::Published => {
            return Err(OwnerBetaRunnerError::PublicationUnavailable);
        }
    })
}
async fn settle(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &OwnerBetaDailyRecommendationInputV1,
    failure: OwnerBetaPublicationFailure,
) -> Result<OwnerBetaOutcome, OwnerBetaRunnerError> {
    match settle_owner_beta_daily_failure(pool, queue, claim, input, failure).await {
        Ok(outcome) => outcome_for(claim, input, outcome, failure),
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
    fn daily_replay_retries_io_or_interruption_but_never_integrity_failures() {
        for error in [
            DailyReplayError::Interrupted,
            DailyReplayError::Artifact(DailyArtifactError::Io),
            DailyReplayError::Verification(DailyAdmissionVerificationError::Artifact(
                DailyArtifactError::Io,
            )),
        ] {
            assert_eq!(replay_failure(error).class(), ErrorClass::Transient);
        }
        for error in [
            DailyReplayError::InvalidLimits,
            DailyReplayError::InvalidChain,
            DailyReplayError::LimitExceeded,
            DailyReplayError::Artifact(DailyArtifactError::SourceChanged),
            DailyReplayError::Artifact(DailyArtifactError::UnsafePath),
            DailyReplayError::Artifact(DailyArtifactError::Conflict),
            DailyReplayError::Verification(DailyAdmissionVerificationError::InvalidPolicy),
            DailyReplayError::Verification(DailyAdmissionVerificationError::InvalidCandidate),
            DailyReplayError::Verification(DailyAdmissionVerificationError::LimitExceeded),
        ] {
            assert_eq!(
                replay_failure(error),
                OwnerBetaPublicationFailure::InputInvalid
            );
            assert_ne!(replay_failure(error).class(), ErrorClass::Transient);
        }
    }
    #[test]
    fn daily_computation_rejections_never_become_retryable_or_success() {
        for (error, failure) in [
            (
                OwnerBetaDailyComputationError::InputInvalid,
                OwnerBetaPublicationFailure::InputInvalid,
            ),
            (
                OwnerBetaDailyComputationError::StrategyInvalid,
                OwnerBetaPublicationFailure::InputInvalid,
            ),
            (
                OwnerBetaDailyComputationError::FactorDefinitionInvalid,
                OwnerBetaPublicationFailure::FactorInvalid,
            ),
            (
                OwnerBetaDailyComputationError::FactorComputeInvalid,
                OwnerBetaPublicationFailure::ComputationFailed,
            ),
            (
                OwnerBetaDailyComputationError::TargetInvalid,
                OwnerBetaPublicationFailure::TargetInvalid,
            ),
        ] {
            assert_eq!(computation_failure(error), failure);
            assert_ne!(computation_failure(error).class(), ErrorClass::Transient);
        }
        for failure in [
            OwnerBetaPublicationFailure::DailyExecutionDisabled,
            OwnerBetaPublicationFailure::DailyAuthorityDenied,
        ] {
            assert_eq!(failure.class(), ErrorClass::DataBlocked);
            assert!(failure.code().starts_with("OWNER_BETA_DAILY_"));
        }
    }
}
