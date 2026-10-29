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
        OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, SimulationCommitState, SimulationExecutionError,
        SimulationPublicationError, SimulationPublicationFailure, SimulationPublicationOutcome,
        VersionedCheckedSimulationPair, VersionedSimulationJobInput,
        publish_versioned_simulation_success, reconcile_versioned_simulation_commit,
        recover_simulation_claims, settle_versioned_simulation_failure,
    },
    types::{AttemptOutcome, ClaimedJob, JobStatus},
};

use super::publish::settle_invalid_simulation_claim;
use super::{
    child::{
        self, BoundedChildResult, ChildCommand, ChildStartError, MAX_CHILD_REQUEST_BYTES,
        StopReason,
    },
    process_parent::{ProgressFailure, ProgressSender},
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
    #[error("simulation child cleanup is unconfirmed")]
    CleanupUnconfirmed,
    #[error("simulation progress or owned task is unavailable")]
    ProgressUnavailable,
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
pub(super) enum LeaseStop {
    Canceled,
    Lost,
    Unavailable,
    Progress(ProgressFailure),
    DrainSuppressed,
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
    shutdown_signal: watch::Receiver<bool>,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    let mut idle_reason = None;
    let mut facts = CompletionFacts::default();
    run_simulation_once_inner(
        queue,
        worker_id,
        runtime,
        shutdown_signal,
        None,
        &mut idle_reason,
        &mut facts,
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdleReason {
    Shutdown,
    SlotRefused,
    NoCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ObservedCycle {
    pub(super) primary: Result<SimulationWorkerOutcome, SimulationWorkerError>,
    pub(super) idle_reason: Option<IdleReason>,
    pub(super) facts: CompletionFacts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CompletionFacts {
    pub(super) shutdown: bool,
    pub(super) progress_error: Option<ProgressFailure>,
    pub(super) heartbeat: Option<HeartbeatCompletion>,
    pub(super) child_error: Option<ChildStartError>,
    pub(super) child_join_failed: bool,
    pub(super) validation_failure: Option<SimulationPublicationFailure>,
    pub(super) provenance_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HeartbeatCompletion {
    pub(super) join_failed: bool,
    pub(super) stop: Option<LeaseStop>,
}

pub(super) enum ConfirmedClaimGate<T> {
    Continue,
    Cleanup { reason: ProgressFailure, result: T },
}

pub(super) async fn confirmed_claim_gate<A, C, F, T>(ack: A, cleanup: C) -> ConfirmedClaimGate<T>
where
    A: std::future::Future<Output = Result<(), ProgressFailure>>,
    C: FnOnce() -> F,
    F: std::future::Future<Output = T>,
{
    match ack.await {
        Ok(()) => ConfirmedClaimGate::Continue,
        Err(reason) => ConfirmedClaimGate::Cleanup {
            reason,
            result: cleanup().await,
        },
    }
}

pub(super) async fn confirmed_claim_outcome<A, C, F>(
    ack: A,
    cleanup: C,
    facts: &mut CompletionFacts,
) -> Option<Result<SimulationWorkerOutcome, SimulationWorkerError>>
where
    A: std::future::Future<Output = Result<(), ProgressFailure>>,
    C: FnOnce() -> F,
    F: std::future::Future<Output = Result<SimulationWorkerOutcome, SimulationWorkerError>>,
{
    match confirmed_claim_gate(ack, cleanup).await {
        ConfirmedClaimGate::Continue => None,
        ConfirmedClaimGate::Cleanup { reason, result } => {
            facts.progress_error = Some(reason);
            Some(result)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeaseProgress {
    Continue,
    StopForDrain,
    StopForFailure(ProgressFailure),
}

pub(super) async fn lease_progress_gate<A>(ack: A) -> LeaseProgress
where
    A: std::future::Future<Output = Result<(), ProgressFailure>>,
{
    match ack.await {
        Ok(()) => LeaseProgress::Continue,
        Err(ProgressFailure::Draining) => LeaseProgress::StopForDrain,
        Err(error) => LeaseProgress::StopForFailure(error),
    }
}

pub(super) fn apply_lease_progress(
    progress: LeaseProgress,
    stop: &watch::Sender<Option<LeaseStop>>,
) -> bool {
    match progress {
        LeaseProgress::Continue => true,
        LeaseProgress::StopForDrain => {
            stop.send_replace(Some(LeaseStop::DrainSuppressed));
            false
        }
        LeaseProgress::StopForFailure(error) => {
            stop.send_replace(Some(LeaseStop::Progress(error)));
            false
        }
    }
}

pub(super) async fn run_simulation_observed(
    queue: &JobQueue,
    worker_id: &str,
    runtime: &TrustedSimulationRuntime,
    shutdown: watch::Receiver<bool>,
    owner: uuid::Uuid,
    progress: ProgressSender,
) -> ObservedCycle {
    let mut idle_reason = None;
    let mut facts = CompletionFacts::default();
    let observed_shutdown = shutdown.clone();
    let primary = run_simulation_once_inner(
        queue,
        worker_id,
        runtime,
        shutdown,
        Some((owner, progress)),
        &mut idle_reason,
        &mut facts,
    )
    .await;
    observed_completion(primary, idle_reason, facts, &observed_shutdown)
}

pub(super) fn observed_completion(
    primary: Result<SimulationWorkerOutcome, SimulationWorkerError>,
    idle_reason: Option<IdleReason>,
    mut facts: CompletionFacts,
    shutdown: &watch::Receiver<bool>,
) -> ObservedCycle {
    facts.shutdown = *shutdown.borrow() || shutdown.has_changed().is_err();
    ObservedCycle {
        primary,
        idle_reason,
        facts,
    }
}

pub(super) fn record_heartbeat_failure(
    stop: &watch::Sender<Option<LeaseStop>>,
    progress: Option<&ProgressSender>,
) {
    if let Some(progress) = progress {
        progress.report_fatal(ProgressFailure::LeaseUnconfirmed);
    }
    stop.send_replace(Some(LeaseStop::Unavailable));
}

async fn run_simulation_once_inner(
    queue: &JobQueue,
    worker_id: &str,
    runtime: &TrustedSimulationRuntime,
    mut shutdown_signal: watch::Receiver<bool>,
    official: Option<(uuid::Uuid, ProgressSender)>,
    idle_reason: &mut Option<IdleReason>,
    facts: &mut CompletionFacts,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    if !linux_execution_supported() {
        return Err(SimulationWorkerError::UnsupportedPlatform);
    }
    runtime.revalidate()?;
    if !valid_worker_id(worker_id) {
        return Err(SimulationWorkerError::WorkerIdInvalid);
    }
    if *shutdown_signal.borrow() || shutdown_signal.has_changed().is_err() {
        *idle_reason = Some(IdleReason::Shutdown);
        return Ok(SimulationWorkerOutcome::Idle);
    }

    if official.is_none() {
        recover_simulation_claims(queue)
            .await
            .map_err(map_publication_error)?;
    }
    let permit = match child_slots().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            *idle_reason = Some(IdleReason::SlotRefused);
            return Ok(SimulationWorkerOutcome::Idle);
        }
    };

    // Provenance is checked again immediately before a claim. An invalid or
    // missing receipt can never cause a claim or recovery pass.
    runtime.revalidate()?;
    let candidate = if let Some((owner, _)) = &official {
        queue
            .claim_next_simulation_for_owner(worker_id, *owner)
            .await
    } else {
        queue
            .claim_next_for(worker_id, OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
            .await
    }
    .map_err(|_| SimulationWorkerError::QueueUnavailable)?;
    let Some(claim) = candidate else {
        *idle_reason = Some(IdleReason::NoCandidate);
        return Ok(SimulationWorkerOutcome::Idle);
    };

    if !claim_is_current(&claim, worker_id) {
        recover_simulation_claims(queue)
            .await
            .map_err(map_publication_error)?;
        return Ok(SimulationWorkerOutcome::ClaimLost);
    }
    if let Some((owner, progress)) = &official {
        if claim.job.owner_user_id != *owner {
            return Err(SimulationWorkerError::QueueUnavailable);
        }
        if let Some(result) = confirmed_claim_outcome(
            progress.claim(&claim),
            || async {
                match parse_claim_input(&claim) {
                    Ok(input) => {
                        settle_interrupted(queue, &claim, &input, Interrupt::Shutdown, Some(*owner))
                            .await
                    }
                    Err(_) => settle_invalid_claim(queue, &claim).await,
                }
            },
            facts,
        )
        .await
        {
            return result;
        }
    }
    let (lease_tx, mut lease_rx) = watch::channel(None);
    let (heartbeat_stop, heartbeat_stop_rx) = watch::channel(false);
    let heartbeat_task = tokio::spawn(heartbeat_loop(
        queue.clone(),
        claim.clone(),
        lease_tx,
        heartbeat_interval(queue),
        official.clone(),
        heartbeat_stop_rx,
    ));
    let mut heartbeat_guard = AbortOnDrop(
        Some(heartbeat_task),
        heartbeat_stop,
        official.is_some(),
        lease_rx.clone(),
    );

    let input = match parse_claim_input(&claim) {
        Ok(input) => input,
        Err(_) => {
            let outcome = settle_invalid_claim(queue, &claim).await;
            return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
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
            return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
        }
    };
    if request.len() > MAX_CHILD_REQUEST_BYTES {
        let outcome = if let Some(interrupted) =
            current_interrupt(&lease_rx, &shutdown_signal, official.is_some())
        {
            settle_interrupted(
                queue,
                &claim,
                &input,
                interrupted,
                official.as_ref().map(|(owner, _)| *owner),
            )
            .await
        } else {
            settle_failure(
                queue,
                &claim,
                &input,
                SimulationPublicationFailure::InputInvalid,
            )
            .await
        };
        return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
    }

    if runtime.revalidate().is_err() {
        if official.is_some() {
            facts.provenance_failed = true;
        }
        let outcome = match current_interrupt(&lease_rx, &shutdown_signal, official.is_some()) {
            Some(interrupted) => {
                settle_interrupted(
                    queue,
                    &claim,
                    &input,
                    interrupted,
                    official.as_ref().map(|(owner, _)| *owner),
                )
                .await
            }
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
        return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
    }

    if let Some(interrupted) = current_interrupt(&lease_rx, &shutdown_signal, official.is_some()) {
        let outcome = settle_interrupted(
            queue,
            &claim,
            &input,
            interrupted,
            official.as_ref().map(|(owner, _)| *owner),
        )
        .await;
        return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
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
                            LeaseStop::Unavailable | LeaseStop::Progress(_) => Interrupt::Unavailable,
                            LeaseStop::DrainSuppressed => Interrupt::Shutdown,
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
            if official.is_some() {
                facts.child_join_failed = true;
                return finish_with_heartbeat(
                    &mut heartbeat_guard,
                    facts,
                    Err(SimulationWorkerError::CleanupUnconfirmed),
                )
                .await;
            }
            let outcome = settle_failure(
                queue,
                &claim,
                &input,
                SimulationPublicationFailure::InfrastructureUnavailable,
            )
            .await;
            return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
        }
    };

    if official.is_some()
        && let Err(error) = &process_result
    {
        facts.child_error = Some(*error);
    }

    if official.is_some() && matches!(&process_result, Err(ChildStartError::CleanupUnconfirmed)) {
        return finish_with_heartbeat(
            &mut heartbeat_guard,
            facts,
            Err(SimulationWorkerError::CleanupUnconfirmed),
        )
        .await;
    }
    if let (Some(_), Err(error)) = (&official, &process_result) {
        let outcome = settle_failure(queue, &claim, &input, start_error_failure(*error)).await;
        return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
    }
    if official.is_some()
        && matches!(
            merge_interrupt(
                interrupted,
                current_interrupt(&lease_rx, &shutdown_signal, official.is_some()),
                official.is_some()
            ),
            Some(Interrupt::Unavailable)
        )
    {
        return finish_with_heartbeat(
            &mut heartbeat_guard,
            facts,
            Ok(SimulationWorkerOutcome::CommitUnresolved),
        )
        .await;
    }
    if let Some(interrupted) = merge_interrupt(
        interrupted,
        current_interrupt(&lease_rx, &shutdown_signal, official.is_some()),
        official.is_some(),
    ) {
        let _ = process_result;
        let outcome = settle_interrupted(
            queue,
            &claim,
            &input,
            interrupted,
            official.as_ref().map(|(owner, _)| *owner),
        )
        .await;
        return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
    }

    let process_result = match process_result {
        Ok(result) => result,
        Err(error) => {
            let failure = start_error_failure(error);
            let outcome = settle_failure(queue, &claim, &input, failure).await;
            return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
        }
    };

    let pair = match validate_child_result(process_result, &input, runtime).await {
        Ok(pair) => pair,
        Err(failure) => {
            if official.is_some() {
                facts.validation_failure = Some(failure);
            }
            let interrupt = current_interrupt(&lease_rx, &shutdown_signal, official.is_some());
            let outcome = match interrupt {
                Some(interrupt) => {
                    settle_interrupted(
                        queue,
                        &claim,
                        &input,
                        interrupt,
                        official.as_ref().map(|(owner, _)| *owner),
                    )
                    .await
                }
                None => settle_failure(queue, &claim, &input, failure).await,
            };
            return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
        }
    };

    if let Some(interrupted) = current_interrupt(&lease_rx, &shutdown_signal, official.is_some()) {
        let outcome = settle_interrupted(
            queue,
            &claim,
            &input,
            interrupted,
            official.as_ref().map(|(owner, _)| *owner),
        )
        .await;
        return finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await;
    }

    let outcome = publish_or_reconcile(queue, &claim, &input, &pair, runtime).await;
    finish_with_heartbeat(&mut heartbeat_guard, facts, outcome).await
}

fn start_error_failure(error: ChildStartError) -> SimulationPublicationFailure {
    match error {
        ChildStartError::InvalidRequest => SimulationPublicationFailure::InputInvalid,
        #[cfg(not(target_os = "linux"))]
        ChildStartError::IsolationUnavailable => {
            SimulationPublicationFailure::InfrastructureUnavailable
        }
        ChildStartError::SpawnFailed
        | ChildStartError::PipeUnavailable
        | ChildStartError::CleanupUnconfirmed => {
            SimulationPublicationFailure::InfrastructureUnavailable
        }
    }
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

fn parse_claim_input(
    claim: &ClaimedJob,
) -> Result<VersionedSimulationJobInput, SimulationWorkerError> {
    let bytes = serde_json::to_vec(&claim.job.payload_json)
        .map_err(|_| SimulationWorkerError::ClaimPayloadInvalid)?;
    VersionedSimulationJobInput::from_json(&bytes)
        .map_err(|_| SimulationWorkerError::ClaimPayloadInvalid)
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
    official: Option<(uuid::Uuid, ProgressSender)>,
    mut finish: watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            changed = finish.changed() => { if changed.is_err() || *finish.borrow() { return; } }
            _ = ticker.tick() => {}
        }
        if *finish.borrow() {
            return;
        }
        let heartbeat = if let Some((owner, _)) = &official {
            queue.heartbeat_simulation_attempt(&claim, *owner).await
        } else {
            queue.heartbeat(&claim).await
        };
        match heartbeat {
            Ok(HeartbeatStatus::Extended) => {
                if let Some((_, progress)) = &official
                    && !apply_lease_progress(
                        lease_progress_gate(progress.lease(&claim)).await,
                        &stop,
                    )
                {
                    return;
                }
            }
            Ok(HeartbeatStatus::Canceled) => {
                stop.send_replace(Some(LeaseStop::Canceled));
                return;
            }
            Ok(HeartbeatStatus::LeaseLost) => {
                stop.send_replace(Some(LeaseStop::Lost));
                return;
            }
            Err(_) => {
                record_heartbeat_failure(&stop, official.as_ref().map(|(_, progress)| progress));
                return;
            }
        }
    }
}

fn current_interrupt(
    lease: &watch::Receiver<Option<LeaseStop>>,
    shutdown: &watch::Receiver<bool>,
    official: bool,
) -> Option<Interrupt> {
    if official
        && matches!(
            *lease.borrow(),
            Some(LeaseStop::Unavailable | LeaseStop::Progress(_))
        )
    {
        return Some(Interrupt::Unavailable);
    }
    if *shutdown.borrow() || shutdown.has_changed().is_err() {
        return Some(Interrupt::Shutdown);
    }
    match *lease.borrow() {
        Some(LeaseStop::Canceled) => Some(Interrupt::Canceled),
        Some(LeaseStop::Lost) => Some(Interrupt::Lost),
        Some(LeaseStop::Unavailable | LeaseStop::Progress(_)) => Some(Interrupt::Unavailable),
        Some(LeaseStop::DrainSuppressed) => Some(Interrupt::Unavailable),
        None => None,
    }
}

fn merge_interrupt(
    first: Option<Interrupt>,
    current: Option<Interrupt>,
    official: bool,
) -> Option<Interrupt> {
    if !official {
        return first.or(current);
    }
    if first == Some(Interrupt::Unavailable) || current == Some(Interrupt::Unavailable) {
        Some(Interrupt::Unavailable)
    } else {
        current.or(first)
    }
}

async fn settle_interrupted(
    queue: &JobQueue,
    claim: &ClaimedJob,
    input: &VersionedSimulationJobInput,
    interrupted: Interrupt,
    owner: Option<uuid::Uuid>,
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
        Interrupt::Unavailable | Interrupt::Shutdown => match if let Some(owner) = owner {
            queue.heartbeat_simulation_attempt(claim, owner).await
        } else {
            queue.heartbeat(claim).await
        } {
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
            Err(_) if owner.is_some() => Ok(SimulationWorkerOutcome::CommitUnresolved),
            Ok(HeartbeatStatus::LeaseLost) | Err(_) => {
                let _ = recover_simulation_claims(queue).await;
                Ok(SimulationWorkerOutcome::ClaimLost)
            }
        },
    }
}

async fn validate_child_result(
    child_result: BoundedChildResult,
    input: &VersionedSimulationJobInput,
    runtime: &TrustedSimulationRuntime,
) -> Result<VersionedCheckedSimulationPair, SimulationPublicationFailure> {
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
        VersionedCheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine)
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
    input: &VersionedSimulationJobInput,
    pair: &VersionedCheckedSimulationPair,
    runtime: &TrustedSimulationRuntime,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    match publish_versioned_simulation_success(queue, claim, input, pair, runtime.engine()).await {
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
    input: &VersionedSimulationJobInput,
    pair: &VersionedCheckedSimulationPair,
    runtime: &TrustedSimulationRuntime,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    for attempt in 0..RECONCILE_ATTEMPTS {
        match reconcile_versioned_simulation_commit(queue, claim, input, pair, runtime.engine())
            .await
        {
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
    input: &VersionedSimulationJobInput,
    failure: SimulationPublicationFailure,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    match settle_versioned_simulation_failure(queue, claim, input, failure).await {
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

pub(super) struct AbortOnDrop(
    pub(super) Option<JoinHandle<()>>,
    pub(super) watch::Sender<bool>,
    pub(super) bool,
    pub(super) watch::Receiver<Option<LeaseStop>>,
);

impl AbortOnDrop {
    async fn abort(&mut self) -> HeartbeatCompletion {
        let mut join_failed = false;
        if let Some(task) = self.0.take() {
            if self.2 {
                self.1.send_replace(true);
                join_failed = task.await.is_err();
            } else {
                task.abort();
                let _ = task.await;
            }
        }
        HeartbeatCompletion {
            join_failed,
            stop: *self.3.borrow(),
        }
    }
}

pub(super) async fn finish_with_heartbeat(
    guard: &mut AbortOnDrop,
    facts: &mut CompletionFacts,
    primary: Result<SimulationWorkerOutcome, SimulationWorkerError>,
) -> Result<SimulationWorkerOutcome, SimulationWorkerError> {
    let completion = guard.abort().await;
    if guard.2 {
        facts.heartbeat = Some(completion);
    }
    primary
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
    fn claim_and_lease_success_are_withheld_when_commit_fails() {
        let claim = crate::queue::confirm_queue_commit(
            Err(sqlx::Error::Protocol("fixture".into())),
            Some(()),
        );
        assert!(claim.is_err());
        let lease = crate::queue::confirm_queue_commit(
            Err(sqlx::Error::Protocol("fixture".into())),
            HeartbeatStatus::Extended,
        );
        assert!(lease.is_err());
        assert!(matches!(
            crate::queue::confirm_queue_commit(Ok(()), HeartbeatStatus::Extended),
            Ok(HeartbeatStatus::Extended)
        ));
    }

    #[tokio::test]
    async fn heartbeat_guard_waits_for_owned_task_before_returning() {
        let (finish, mut finished) = watch::channel(false);
        let (_lease_tx, lease_rx) = watch::channel(None);
        let joined = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let joined_task = Arc::clone(&joined);
        let task = tokio::spawn(async move {
            while !*finished.borrow() {
                if finished.changed().await.is_err() {
                    return;
                }
            }
            joined_task.store(true, Ordering::SeqCst);
        });
        let mut guard = AbortOnDrop(Some(task), finish, true, lease_rx);
        assert!(!guard.abort().await.join_failed);
        assert!(joined.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn legacy_guard_aborts_pending_heartbeat_and_awaits_drop() {
        struct Dropped(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let owned = Dropped(Arc::clone(&dropped));
        let (entered, entered_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _owned = owned;
            let _ = entered.send(());
            std::future::pending::<()>().await;
        });
        assert!(entered_rx.await.is_ok());
        let (finish, _) = watch::channel(false);
        let (_lease_tx, lease_rx) = watch::channel(None);
        let mut guard = AbortOnDrop(Some(task), finish, false, lease_rx);
        let report = guard.abort().await;
        assert!(!report.join_failed);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(guard.0.is_none());
    }

    #[test]
    fn official_shutdown_keeps_later_lease_uncertainty_while_legacy_keeps_priority() {
        let (_lease_tx, lease) = watch::channel(Some(LeaseStop::Unavailable));
        let (_stop_tx, shutdown) = watch::channel(true);
        let official = current_interrupt(&lease, &shutdown, true);
        assert_eq!(official, Some(Interrupt::Unavailable));
        assert_eq!(
            merge_interrupt(Some(Interrupt::Shutdown), official, true),
            Some(Interrupt::Unavailable)
        );
        assert_eq!(
            current_interrupt(&lease, &shutdown, false),
            Some(Interrupt::Shutdown)
        );
        assert_eq!(
            merge_interrupt(
                Some(Interrupt::Shutdown),
                Some(Interrupt::Unavailable),
                false
            ),
            Some(Interrupt::Shutdown)
        );
    }

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
