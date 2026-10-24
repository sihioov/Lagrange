//! Dedicated queue runner for the owner-beta paired price-only simulation.

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{Semaphore, watch},
    task::JoinHandle,
};

use crate::{
    HeartbeatStatus, JobQueue,
    owner_beta::simulation_execution::{
        CheckedSimulationPair, OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, SimulationCommitState,
        SimulationExecutionError, SimulationJobInput, SimulationPublicationError,
        SimulationPublicationFailure, SimulationPublicationOutcome, publish_simulation_success,
        reconcile_simulation_commit, recover_simulation_claims, settle_simulation_failure,
    },
    types::{AttemptOutcome, ClaimedJob, JobStatus},
};

use super::publish::settle_invalid_simulation_claim;
use super::{
    child::{
        self, BoundedChildResult, ChildCommand, ChildStartError, MAX_CHILD_REQUEST_BYTES,
        StopReason,
    },
    provenance::TrustedSimulationRuntime,
};

const HEARTBEAT_FLOOR: Duration = Duration::from_secs(1);
const RECONCILE_ATTEMPTS: usize = 3;
const RECONCILE_PAUSE: Duration = Duration::from_millis(100);

const fn linux_execution_supported() -> bool {
    cfg!(target_os = "linux")
}

static SIMULATION_CHILD_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn child_slots() -> Arc<Semaphore> {
    SIMULATION_CHILD_SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

/// Static, value-free failures for startup, queue, and publication boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationWorkerError {
    #[error("simulation executable provenance is invalid")]
    ProvenanceInvalid,
    #[error("simulation worker identifier is invalid")]
    WorkerIdInvalid,
    #[error("simulation queue is unavailable")]
    QueueUnavailable,
    #[error("claimed simulation input is invalid")]
    ClaimPayloadInvalid,
    #[error("simulation child output failed validation")]
    ResultInvalid,
    #[error("simulation publication is unavailable")]
    PublicationUnavailable,
    #[error("simulation runner supports Linux only")]
    UnsupportedPlatform,
}

/// Public one-at-a-time result states. These values carry no job payload,
/// provider data, filesystem path, or result bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationWorkerOutcome {
    Idle,
    Published,
    Retrying,
    Failed,
    Canceled,
    ClaimLost,
    CommitUnresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseStop {
    Canceled,
    Lost,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    Canceled,
    Lost,
    Unavailable,
    Shutdown,
}

/// Validates provenance before recovery or claim, claims only the exact
/// simulation type, and holds an independent heartbeat through child work,
/// parent pair validation, and publication.
pub async fn run_simulation_once(
    queue: &JobQueue,
    worker_id: &str,
    runtime: &TrustedSimulationRuntime,
    mut shutdown_signal: watch::Receiver<bool>,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    if !linux_execution_supported() {
        return Err(SimulationWorkerError::UnsupportedPlatform);
    }
    runtime.revalidate()?;
    if !valid_worker_id(worker_id) {
        return Err(SimulationWorkerError::WorkerIdInvalid);
    }
    if *shutdown_signal.borrow() || shutdown_signal.has_changed().is_err() {
        return Ok(SimulationWorkerOutcome::Idle);
    }

    recover_simulation_claims(queue)
        .await
        .map_err(map_publication_error)?;
    let permit = match child_slots().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return Ok(SimulationWorkerOutcome::Idle),
    };

    // Provenance is checked again immediately before a claim. An invalid or
    // missing receipt can never cause a claim or recovery pass.
    runtime.revalidate()?;
    let Some(claim) = queue
        .claim_next_for(worker_id, OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
        .await
        .map_err(|_| SimulationWorkerError::QueueUnavailable)?
    else {
        return Ok(SimulationWorkerOutcome::Idle);
    };

    if !claim_is_current(&claim, worker_id) {
        recover_simulation_claims(queue)
            .await
            .map_err(map_publication_error)?;
        return Ok(SimulationWorkerOutcome::ClaimLost);
    }
    let (lease_tx, mut lease_rx) = watch::channel(None);
    let heartbeat_task = tokio::spawn(heartbeat_loop(
        queue.clone(),
        claim.clone(),
        lease_tx,
        heartbeat_interval(queue),
    ));
    let mut heartbeat_guard = AbortOnDrop(Some(heartbeat_task));

    let input = match parse_claim_input(&claim) {
        Ok(input) => input,
        Err(_) => {
            let outcome = settle_invalid_claim(queue, &claim).await;
            heartbeat_guard.abort().await;
            return outcome;
        }
    };
    let request = match serde_json::to_vec(&input) {
        Ok(request) => request,
        Err(_) => {
            let outcome = settle_failure(
                queue,
                &claim,
                &input,
                SimulationPublicationFailure::InputInvalid,
            )
            .await;
            heartbeat_guard.abort().await;
            return outcome;
        }
    };
    if request.len() > MAX_CHILD_REQUEST_BYTES {
        let outcome = if let Some(interrupted) = current_interrupt(&lease_rx, &shutdown_signal) {
            settle_interrupted(queue, &claim, &input, interrupted).await
        } else {
            settle_failure(
                queue,
                &claim,
                &input,
                SimulationPublicationFailure::InputInvalid,
            )
            .await
        };
        heartbeat_guard.abort().await;
        return outcome;
    }

    if runtime.revalidate().is_err() {
        let outcome = match current_interrupt(&lease_rx, &shutdown_signal) {
            Some(interrupted) => settle_interrupted(queue, &claim, &input, interrupted).await,
            None => {
                settle_failure(
                    queue,
                    &claim,
                    &input,
                    SimulationPublicationFailure::ProvenanceInvalid,
                )
                .await
            }
        };
        heartbeat_guard.abort().await;
        return outcome;
    }

    if let Some(interrupted) = current_interrupt(&lease_rx, &shutdown_signal) {
        let outcome = settle_interrupted(queue, &claim, &input, interrupted).await;
        heartbeat_guard.abort().await;
        return outcome;
    }

    let command = ChildCommand {
        executable: runtime.executable_path().to_path_buf(),
        args: vec![
            "--child".into(),
            "--artifact-root".into(),
            runtime.artifact_root().as_os_str().to_owned(),
            "--build-receipt".into(),
            runtime.receipt_path().as_os_str().to_owned(),
            "--expected-receipt-sha256".into(),
            runtime.expected_receipt_sha256().into(),
        ],
        request,
    };
    let cancel = Arc::new(AtomicU8::new(child::CONTROL_RUNNING));
    let mut process_cancel_guard = CancelOnDrop {
        cancel: cancel.clone(),
        active: true,
    };
    let child_cancel = cancel.clone();
    let mut process_task = tokio::task::spawn_blocking(move || {
        let permit = permit;
        let result = child::run_bounded_child(command, child_cancel);
        if matches!(&result, Err(ChildStartError::CleanupUnconfirmed)) {
            // Cleanup could not be confirmed. Permanently hold this process's
            // single slot so a later daemon iteration cannot start another child.
            std::mem::forget(permit);
        }
        result
    });

    let (process_result, interrupted) = loop {
        tokio::select! {
            result = &mut process_task => break (result, None),
            changed = lease_rx.changed() => {
                if changed.is_ok() {
                    let stop = *lease_rx.borrow();
                    if let Some(stop) = stop {
                        let reason = match stop {
                            LeaseStop::Canceled => Interrupt::Canceled,
                            LeaseStop::Lost => Interrupt::Lost,
                            LeaseStop::Unavailable => Interrupt::Unavailable,
                        };
                        cancel.store(child::CONTROL_CANCEL, Ordering::Release);
                        let result = process_task.await;
                        break (result, Some(reason));
                    }
                }
            }
            changed = shutdown_signal.changed() => {
                if changed.is_err() || *shutdown_signal.borrow() {
                    cancel.store(child::CONTROL_CANCEL, Ordering::Release);
                    let result = process_task.await;
                    break (result, Some(Interrupt::Shutdown));
                }
            }
        }
    };
    let process_result = match process_result {
        Ok(result) => {
            process_cancel_guard.active = false;
            result
        }
        Err(_) => {
            cancel.store(child::CONTROL_CANCEL, Ordering::Release);
            let outcome = settle_failure(
                queue,
                &claim,
                &input,
                SimulationPublicationFailure::InfrastructureUnavailable,
            )
            .await;
            heartbeat_guard.abort().await;
            return outcome;
        }
    };

    if let Some(interrupted) =
        interrupted.or_else(|| current_interrupt(&lease_rx, &shutdown_signal))
    {
        let _ = process_result;
        let outcome = settle_interrupted(queue, &claim, &input, interrupted).await;
        heartbeat_guard.abort().await;
        return outcome;
    }

    let process_result = match process_result {
        Ok(result) => result,
        Err(error) => {
            let failure = match error {
                ChildStartError::InvalidRequest => SimulationPublicationFailure::InputInvalid,
                ChildStartError::SpawnFailed
                | ChildStartError::IsolationUnavailable
                | ChildStartError::PipeUnavailable
                | ChildStartError::CleanupUnconfirmed => {
                    SimulationPublicationFailure::InfrastructureUnavailable
                }
            };
            let outcome = settle_failure(queue, &claim, &input, failure).await;
            heartbeat_guard.abort().await;
            return outcome;
        }
    };

    let pair = match validate_child_result(process_result, &input, runtime).await {
        Ok(pair) => pair,
        Err(failure) => {
            let interrupt = current_interrupt(&lease_rx, &shutdown_signal);
            let outcome = match interrupt {
                Some(interrupt) => settle_interrupted(queue, &claim, &input, interrupt).await,
                None => settle_failure(queue, &claim, &input, failure).await,
            };
            heartbeat_guard.abort().await;
            return outcome;
        }
    };

    if let Some(interrupted) = current_interrupt(&lease_rx, &shutdown_signal) {
        let outcome = settle_interrupted(queue, &claim, &input, interrupted).await;
        heartbeat_guard.abort().await;
        return outcome;
    }

    let outcome = publish_or_reconcile(queue, &claim, &input, &pair, runtime).await;
    heartbeat_guard.abort().await;
    outcome
}

/// Entry used by the dedicated binary's `--child` mode. Only a static exit
/// classification is returned; no error body reaches stderr or stdout.
pub fn run_simulation_child_mode(
    artifact_root: &std::path::Path,
    receipt_path: &std::path::Path,
    expected_receipt_sha256: &str,
) -> u8 {
    child::run_child_mode(artifact_root, receipt_path, expected_receipt_sha256)
        .map(|()| 0)
        .unwrap_or_else(|error| error as u8)
}

fn parse_claim_input(claim: &ClaimedJob) -> Result<SimulationJobInput, SimulationWorkerError> {
    let bytes = serde_json::to_vec(&claim.job.payload_json)
        .map_err(|_| SimulationWorkerError::ClaimPayloadInvalid)?;
    SimulationJobInput::from_json(&bytes).map_err(|_| SimulationWorkerError::ClaimPayloadInvalid)
}

async fn settle_invalid_claim(
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    match settle_invalid_simulation_claim(queue, claim).await {
        Ok(outcome) => Ok(map_publication_outcome(outcome)),
        Err(SimulationPublicationError::QueueClaimLost) => {
            recover_simulation_claims(queue)
                .await
                .map_err(map_publication_error)?;
            Ok(SimulationWorkerOutcome::ClaimLost)
        }
        Err(SimulationPublicationError::CommitUnknown) => {
            Ok(SimulationWorkerOutcome::CommitUnresolved)
        }
        Err(_) => Err(SimulationWorkerError::PublicationUnavailable),
    }
}

fn claim_is_current(claim: &ClaimedJob, worker_id: &str) -> bool {
    claim.job.job_type == OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
        && claim.job.status == JobStatus::Running
        && claim.job.locked_by.as_deref() == Some(worker_id)
        && claim.worker_id == worker_id
        && claim.attempt.job_id == claim.job.id
        && claim.attempt.attempt_no > 0
        && claim.attempt.attempt_no == claim.job.attempt_count
        && claim.job.attempt_count > 0
        && claim.attempt.outcome == AttemptOutcome::Running
        && claim.attempt.claimed_by.as_deref() == Some(worker_id)
}

fn valid_worker_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn heartbeat_interval(queue: &JobQueue) -> Duration {
    queue
        .config()
        .lease
        .checked_div(3)
        .unwrap_or(Duration::from_secs(10))
        .max(HEARTBEAT_FLOOR)
}

async fn heartbeat_loop(
    queue: JobQueue,
    claim: ClaimedJob,
    stop: watch::Sender<Option<LeaseStop>>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match queue.heartbeat(&claim).await {
            Ok(HeartbeatStatus::Extended) => {}
            Ok(HeartbeatStatus::Canceled) => {
                stop.send_replace(Some(LeaseStop::Canceled));
                return;
            }
            Ok(HeartbeatStatus::LeaseLost) => {
                stop.send_replace(Some(LeaseStop::Lost));
                return;
            }
            Err(_) => {
                stop.send_replace(Some(LeaseStop::Unavailable));
                return;
            }
        }
    }
}

fn current_interrupt(
    lease: &watch::Receiver<Option<LeaseStop>>,
    shutdown: &watch::Receiver<bool>,
) -> Option<Interrupt> {
    if *shutdown.borrow() || shutdown.has_changed().is_err() {
        return Some(Interrupt::Shutdown);
    }
    match *lease.borrow() {
        Some(LeaseStop::Canceled) => Some(Interrupt::Canceled),
        Some(LeaseStop::Lost) => Some(Interrupt::Lost),
        Some(LeaseStop::Unavailable) => Some(Interrupt::Unavailable),
        None => None,
    }
}

async fn settle_interrupted(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    interrupted: Interrupt,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    match interrupted {
        Interrupt::Canceled => {
            settle_failure(queue, claim, input, SimulationPublicationFailure::Canceled).await
        }
        Interrupt::Lost => {
            recover_simulation_claims(queue)
                .await
                .map_err(map_publication_error)?;
            Ok(SimulationWorkerOutcome::ClaimLost)
        }
        Interrupt::Unavailable | Interrupt::Shutdown => match queue.heartbeat(claim).await {
            Ok(HeartbeatStatus::Extended) => {
                settle_failure(
                    queue,
                    claim,
                    input,
                    SimulationPublicationFailure::InfrastructureUnavailable,
                )
                .await
            }
            Ok(HeartbeatStatus::Canceled) => {
                settle_failure(queue, claim, input, SimulationPublicationFailure::Canceled).await
            }
            Ok(HeartbeatStatus::LeaseLost) | Err(_) => {
                let _ = recover_simulation_claims(queue).await;
                Ok(SimulationWorkerOutcome::ClaimLost)
            }
        },
    }
}

async fn validate_child_result(
    child_result: BoundedChildResult,
    input: &SimulationJobInput,
    runtime: &TrustedSimulationRuntime,
) -> Result<CheckedSimulationPair, SimulationPublicationFailure> {
    if let Some(reason) = child_result.stop_reason {
        return Err(match reason {
            StopReason::TimedOut => SimulationPublicationFailure::ComputationTimedOut,
            StopReason::Canceled => SimulationPublicationFailure::InfrastructureUnavailable,
            StopReason::OutputTooLarge => SimulationPublicationFailure::ResultInvalid,
        });
    }
    if !child_result.status.success() {
        return Err(child_exit_failure(child_result.status.code()));
    }
    let stdout = child_result.stdout;
    let input = input.clone();
    let engine = runtime.engine().clone();
    tokio::task::spawn_blocking(move || {
        let (illustrative, zero_cost) =
            child::parse_child_pair(&stdout).map_err(|_| SimulationExecutionError::PairInvalid)?;
        CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine)
    })
    .await
    .map_err(|_| SimulationPublicationFailure::ResultInvalid)?
    .map_err(|error| match error {
        SimulationExecutionError::ProvenanceMismatch => {
            SimulationPublicationFailure::ProvenanceInvalid
        }
        SimulationExecutionError::InputInvalid
        | SimulationExecutionError::WindowInvalid
        | SimulationExecutionError::StrategyInvalid => SimulationPublicationFailure::InputInvalid,
        SimulationExecutionError::PairInvalid | SimulationExecutionError::SizeLimitExceeded => {
            SimulationPublicationFailure::ResultInvalid
        }
    })
}

fn child_exit_failure(code: Option<i32>) -> SimulationPublicationFailure {
    match code {
        Some(21) => SimulationPublicationFailure::ProvenanceInvalid,
        Some(22) => SimulationPublicationFailure::InputInvalid,
        Some(24) => SimulationPublicationFailure::ResultInvalid,
        _ => SimulationPublicationFailure::ComputationFailed,
    }
}

async fn publish_or_reconcile(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    pair: &CheckedSimulationPair,
    runtime: &TrustedSimulationRuntime,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    match publish_simulation_success(queue, claim, input, pair, runtime.engine()).await {
        Ok(outcome) => Ok(map_publication_outcome(outcome)),
        Err(SimulationPublicationError::CommitUnknown) => {
            reconcile_unknown_commit(queue, claim, input, pair, runtime).await
        }
        Err(SimulationPublicationError::QueueClaimLost) => {
            let _ = recover_simulation_claims(queue).await;
            Ok(SimulationWorkerOutcome::ClaimLost)
        }
        Err(SimulationPublicationError::EntitlementDenied) => {
            settle_failure(
                queue,
                claim,
                input,
                SimulationPublicationFailure::EntitlementDenied,
            )
            .await
        }
        Err(
            SimulationPublicationError::DatabaseTransient
            | SimulationPublicationError::QueueTransient,
        ) => {
            settle_failure(
                queue,
                claim,
                input,
                SimulationPublicationFailure::InfrastructureUnavailable,
            )
            .await
        }
        Err(
            SimulationPublicationError::DatabaseIntegrity
            | SimulationPublicationError::QueueIntegrity
            | SimulationPublicationError::Integrity,
        ) => {
            settle_failure(
                queue,
                claim,
                input,
                SimulationPublicationFailure::ResultInvalid,
            )
            .await
        }
    }
}

async fn reconcile_unknown_commit(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    pair: &CheckedSimulationPair,
    runtime: &TrustedSimulationRuntime,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    for attempt in 0..RECONCILE_ATTEMPTS {
        match reconcile_simulation_commit(queue, claim, input, pair, runtime.engine()).await {
            Ok(SimulationCommitState::Published) => return Ok(SimulationWorkerOutcome::Published),
            Ok(SimulationCommitState::NotPublished) => {
                return settle_failure(
                    queue,
                    claim,
                    input,
                    SimulationPublicationFailure::InfrastructureUnavailable,
                )
                .await;
            }
            Ok(SimulationCommitState::Unresolved) => {}
            Err(SimulationPublicationError::QueueClaimLost) => {
                let _ = recover_simulation_claims(queue).await;
                return Ok(SimulationWorkerOutcome::ClaimLost);
            }
            Err(_) => {}
        }
        if attempt + 1 < RECONCILE_ATTEMPTS {
            tokio::time::sleep(RECONCILE_PAUSE).await;
        }
    }
    Ok(SimulationWorkerOutcome::CommitUnresolved)
}

async fn settle_failure(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &SimulationJobInput,
    failure: SimulationPublicationFailure,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    match settle_simulation_failure(queue, claim, input, failure).await {
        Ok(outcome) => Ok(map_publication_outcome(outcome)),
        Err(SimulationPublicationError::QueueClaimLost) => {
            let _ = recover_simulation_claims(queue).await;
            Ok(SimulationWorkerOutcome::ClaimLost)
        }
        Err(SimulationPublicationError::CommitUnknown) => {
            Ok(SimulationWorkerOutcome::CommitUnresolved)
        }
        Err(_) => Err(SimulationWorkerError::PublicationUnavailable),
    }
}

fn map_publication_outcome(outcome: SimulationPublicationOutcome) -> SimulationWorkerOutcome {
    match outcome {
        SimulationPublicationOutcome::Published => SimulationWorkerOutcome::Published,
        SimulationPublicationOutcome::Retrying => SimulationWorkerOutcome::Retrying,
        SimulationPublicationOutcome::Failed => SimulationWorkerOutcome::Failed,
        SimulationPublicationOutcome::Canceled => SimulationWorkerOutcome::Canceled,
    }
}

fn map_publication_error(_: SimulationPublicationError) -> SimulationWorkerError {
    SimulationWorkerError::QueueUnavailable
}

struct CancelOnDrop {
    cancel: Arc<AtomicU8>,
    active: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.active {
            self.cancel.store(child::CONTROL_CANCEL, Ordering::Release);
        }
    }
}

struct AbortOnDrop(Option<JoinHandle<()>>);

impl AbortOnDrop {
    async fn abort(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_classifies_only_static_child_exit_codes() {
        assert!(valid_worker_id("worker-1"));
        assert!(!valid_worker_id("worker id"));
        assert_eq!(
            child_exit_failure(Some(21)),
            SimulationPublicationFailure::ProvenanceInvalid
        );
        assert_eq!(
            child_exit_failure(Some(22)),
            SimulationPublicationFailure::InputInvalid
        );
        assert_eq!(
            child_exit_failure(Some(24)),
            SimulationPublicationFailure::ResultInvalid
        );
        assert_eq!(
            child_exit_failure(Some(99)),
            SimulationPublicationFailure::ComputationFailed
        );
    }

    #[test]
    fn runner_uses_one_process_slot_and_fixed_reconciliation_bound() {
        assert_eq!(child_slots().available_permits(), 1);
        assert_eq!(RECONCILE_ATTEMPTS, 3);
        assert_eq!(child::CHILD_DEADLINE, Duration::from_secs(1_200));
    }

    #[test]
    fn runner_runtime_boundary_is_linux_only() {
        assert_eq!(linux_execution_supported(), std::env::consts::OS == "linux");
    }
}
