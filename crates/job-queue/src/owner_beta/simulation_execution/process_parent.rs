//! Root-owned health and official execution control.

use sqlx::postgres::PgConnectOptions;
use std::time::Duration;
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use uuid::Uuid;

use super::{
    SimulationHealthEvent, SimulationHealthFailure, SimulationHealthWriter,
    SimulationProcessGeneration, SimulationWorkerError, SimulationWorkerOutcome,
    open_simulation_health_writer,
    process_startup::{StartupEvidence, startup},
    runner::{IdleReason, LeaseStop, ObservedCycle, run_simulation_observed},
};
use crate::types::ClaimedJob;

#[derive(Clone)]
pub(super) struct ProgressSender {
    tx: mpsc::Sender<Progress>,
    fatal: watch::Sender<Option<ProgressFailure>>,
    generation: SimulationProcessGeneration,
    epoch: u64,
}

struct Progress {
    generation: SimulationProcessGeneration,
    epoch: u64,
    attempt: Uuid,
    event: SimulationHealthEvent,
    ack: oneshot::Sender<Result<(), ProgressFailure>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressFailure {
    Full,
    Closed,
    AckLost,
    Rejected,
    Draining,
    WriterPoisoned,
    LeaseUnconfirmed,
}

impl ProgressSender {
    async fn send(
        &self,
        claim: &ClaimedJob,
        event: SimulationHealthEvent,
    ) -> Result<(), ProgressFailure> {
        self.send_event(claim.attempt.id, event).await
    }
    async fn send_event(
        &self,
        attempt: Uuid,
        event: SimulationHealthEvent,
    ) -> Result<(), ProgressFailure> {
        let (ack, received) = oneshot::channel();
        let sent = self
            .tx
            .try_send(Progress {
                generation: self.generation,
                epoch: self.epoch,
                attempt,
                event,
                ack,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ProgressFailure::Full,
                mpsc::error::TrySendError::Closed(_) => ProgressFailure::Closed,
            });
        let result = match sent {
            Ok(()) => received
                .await
                .map_err(|_| ProgressFailure::AckLost)
                .and_then(|ack| ack),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            if error != ProgressFailure::Draining {
                self.report_fatal(error);
            }
        }
        result
    }
    pub(super) async fn claim(&self, claim: &ClaimedJob) -> Result<(), ProgressFailure> {
        self.send(claim, SimulationHealthEvent::ClaimVerified).await
    }
    pub(super) async fn lease(&self, claim: &ClaimedJob) -> Result<(), ProgressFailure> {
        self.send(claim, SimulationHealthEvent::LeaseVerified).await
    }

    pub(super) fn report_fatal(&self, error: ProgressFailure) {
        let _ = self.fatal.send_if_modified(|first| {
            if first.is_some() {
                false
            } else {
                *first = Some(error);
                true
            }
        });
    }
}

struct RootState {
    generation: SimulationProcessGeneration,
    epoch: u64,
    draining_epoch: Option<u64>,
    active: Option<Uuid>,
    draining: bool,
    fatal: Option<SimulationHealthFailure>,
    poisoned: bool,
}

fn completed_cycle(outcome: SimulationWorkerOutcome, idle: Option<IdleReason>) -> bool {
    match outcome {
        SimulationWorkerOutcome::Idle => idle == Some(IdleReason::NoCandidate),
        SimulationWorkerOutcome::Published
        | SimulationWorkerOutcome::Retrying
        | SimulationWorkerOutcome::Failed
        | SimulationWorkerOutcome::Canceled => true,
        SimulationWorkerOutcome::ClaimLost | SimulationWorkerOutcome::CommitUnresolved => false,
    }
}

fn worker_failure(
    root: &mut RootState,
    writer: &mut impl HealthSink,
    error: SimulationWorkerError,
) {
    let failure = match error {
        SimulationWorkerError::CleanupUnconfirmed => SimulationHealthFailure::CleanupUnconfirmed,
        _ => SimulationHealthFailure::CommitUnresolved,
    };
    root.fail(writer, failure);
}

fn completion_failure(cycle: &ObservedCycle) -> Option<SimulationHealthFailure> {
    match cycle.primary {
        Err(SimulationWorkerError::CleanupUnconfirmed) => {
            return Some(SimulationHealthFailure::CleanupUnconfirmed);
        }
        Err(_)
        | Ok(SimulationWorkerOutcome::CommitUnresolved | SimulationWorkerOutcome::ClaimLost) => {
            return Some(SimulationHealthFailure::CommitUnresolved);
        }
        _ => {}
    }
    if cycle.facts.heartbeat.is_some_and(|heartbeat| {
        heartbeat.join_failed
            || matches!(
                heartbeat.stop,
                Some(LeaseStop::Unavailable | LeaseStop::Progress(_))
            )
            || (heartbeat.stop == Some(LeaseStop::DrainSuppressed) && !cycle.facts.shutdown)
    }) {
        return Some(SimulationHealthFailure::CommitUnresolved);
    }
    if cycle.facts.shutdown
        && (cycle.facts.validation_failure.is_some() || cycle.facts.provenance_failed)
    {
        return Some(SimulationHealthFailure::RuntimeFailure);
    }
    if cycle.facts.child_error.is_some() || cycle.facts.child_join_failed {
        return Some(SimulationHealthFailure::RuntimeFailure);
    }
    match cycle.facts.progress_error {
        None => None,
        Some(ProgressFailure::Draining) if cycle.facts.shutdown => None,
        Some(_) => Some(SimulationHealthFailure::RuntimeFailure),
    }
}

fn apply_completion(root: &mut RootState, writer: &mut impl HealthSink, cycle: &ObservedCycle) {
    if let Err(error) = cycle.primary {
        worker_failure(root, writer, error);
        return;
    }
    if let Some(failure) = completion_failure(cycle) {
        root.fail(writer, failure);
        return;
    }
    if let Ok(outcome) = cycle.primary {
        if completed_cycle(outcome, cycle.idle_reason) && !root.draining && root.fatal.is_none() {
            if root
                .apply(writer, SimulationHealthEvent::CycleVerified)
                .is_ok()
            {
                root.active = None;
            } else {
                root.fail(writer, SimulationHealthFailure::RuntimeFailure);
            }
        }
    }
}

fn start_fatal_cleanup(
    root: &RootState,
    stop: &watch::Sender<bool>,
    deadline: &mut Option<Instant>,
    expired: bool,
    timeout: Duration,
) {
    if root.fatal.is_some() || root.poisoned {
        stop.send_replace(true);
        if !expired {
            deadline.get_or_insert_with(|| Instant::now() + timeout);
        }
    }
}

enum ParentFailure {
    Setup,
    Runtime {
        first_fatal: Option<SimulationHealthFailure>,
        first_progress_failure: Option<ProgressFailure>,
        poisoned: bool,
        draining: bool,
        completion: Option<ObservedCycle>,
        cleanup_deadline_expired: bool,
    },
}

struct ParentSuccess {
    draining: bool,
    completion: Option<ObservedCycle>,
}

impl ParentSuccess {
    fn consume(self) {
        let _ = (self.draining, self.completion);
    }
}

impl ParentFailure {
    fn consume(self) {
        if let Self::Runtime {
            first_fatal,
            first_progress_failure,
            poisoned,
            draining,
            completion,
            cleanup_deadline_expired,
        } = self
        {
            let _ = (
                first_fatal,
                first_progress_failure,
                poisoned,
                draining,
                completion,
                cleanup_deadline_expired,
            );
        }
    }
}

trait HealthSink {
    fn apply_event(&mut self, event: SimulationHealthEvent) -> Result<(), ()>;
}

impl HealthSink for SimulationHealthWriter {
    fn apply_event(&mut self, event: SimulationHealthEvent) -> Result<(), ()> {
        self.apply(event).map_err(|_| ())
    }
}

impl RootState {
    fn can_admit(&self) -> bool {
        !self.draining && self.fatal.is_none() && !self.poisoned
    }

    fn apply(
        &mut self,
        writer: &mut impl HealthSink,
        event: SimulationHealthEvent,
    ) -> Result<(), ()> {
        if self.poisoned {
            return Err(());
        }
        if writer.apply_event(event).is_err() {
            self.poisoned = true;
            return Err(());
        }
        Ok(())
    }

    fn close_admission(&mut self, writer: &mut impl HealthSink) {
        if !self.draining {
            self.draining = true;
            if self.fatal.is_none() && !self.poisoned {
                self.draining_epoch = Some(self.epoch);
            }
            self.epoch = self.epoch.saturating_add(1);
            if self.fatal.is_none() {
                let _ = self.apply(writer, SimulationHealthEvent::ShutdownRequested);
            }
        }
    }

    fn fail(&mut self, writer: &mut impl HealthSink, failure: SimulationHealthFailure) {
        if self.fatal.is_none() {
            self.fatal = Some(failure);
            self.epoch = self.epoch.saturating_add(1);
            let _ = self.apply(writer, SimulationHealthEvent::Fatal(failure));
        }
    }

    fn progress(&mut self, writer: &mut impl HealthSink, message: Progress) {
        let matching_attempt = match message.event {
            SimulationHealthEvent::ClaimVerified => self.active.is_none(),
            SimulationHealthEvent::LeaseVerified => self.active == Some(message.attempt),
            _ => false,
        };
        let drain_suppressed = self.draining
            && self.fatal.is_none()
            && !self.poisoned
            && message.generation == self.generation
            && self.draining_epoch == Some(message.epoch)
            && matching_attempt;
        let valid = !self.draining
            && self.fatal.is_none()
            && !self.poisoned
            && message.generation == self.generation
            && message.epoch == self.epoch
            && matching_attempt;
        let result = if valid {
            self.apply(writer, message.event)
                .map_err(|_| ProgressFailure::WriterPoisoned)
                .map(|()| {
                    if message.event == SimulationHealthEvent::ClaimVerified {
                        self.active = Some(message.attempt);
                    }
                })
        } else if drain_suppressed {
            Err(ProgressFailure::Draining)
        } else if self.poisoned {
            Err(ProgressFailure::WriterPoisoned)
        } else {
            Err(ProgressFailure::Rejected)
        };
        if message.ack.send(result).is_err() {
            self.fail(writer, SimulationHealthFailure::RuntimeFailure);
        }
    }
}

#[cfg(test)]
mod tests;

/// Official binary entry. Inputs are independently supplied installer pins;
/// no configuration or path override is accepted by this boundary.
pub async fn run_official_simulation_daemon(
    expected_commit: String,
    expected_config_sha256: String,
    expected_receipt_sha256: String,
    options: PgConnectOptions,
    signal: watch::Receiver<bool>,
) -> Result<(), ()> {
    let startup_signal = signal.clone();
    let pending = async move {
        startup(
            expected_commit,
            expected_config_sha256,
            expected_receipt_sha256,
            options,
            startup_signal,
        )
        .await
        .map_err(|failure| {
            failure.consume();
        })
    };
    after_verified_startup(pending, |ready| async {
        run_ready(ready, signal)
            .await
            .map(|success| success.consume())
            .map_err(|failure| failure.consume())
    })
    .await
}

async fn after_verified_startup<S, T, E, F, Fut, R>(pending: S, run: F) -> Result<R, ()>
where
    S: std::future::Future<Output = Result<T, E>>,
    F: FnOnce(T) -> Fut,
    Fut: std::future::Future<Output = Result<R, ()>>,
{
    let ready = pending.await.map_err(|_| ())?;
    run(ready).await
}

async fn run_ready(
    mut ready: StartupEvidence,
    mut signal: watch::Receiver<bool>,
) -> Result<ParentSuccess, ParentFailure> {
    let mut writer = match open_simulation_health_writer(&ready.config) {
        Ok(writer) => writer,
        Err(_) => {
            ready.pool.close().await;
            return Err(ParentFailure::Setup);
        }
    };
    let mut root = RootState {
        generation: ready.process.generation(),
        epoch: 1,
        draining_epoch: None,
        active: None,
        draining: false,
        fatal: None,
        poisoned: false,
    };
    let observation_ok = ready
        .process
        .revalidate()
        .ok()
        .and_then(|now| now.checked_sub(ready.started_boot_ms))
        .is_some_and(|elapsed| elapsed < ready.limit_ms)
        && ready.process.generation() == root.generation
        && Instant::now() < ready.deadline;
    if !observation_ok || *signal.borrow() || signal.has_changed().is_err() {
        root.close_admission(&mut writer);
        ready.pool.close().await;
        return Err(ParentFailure::Setup);
    }
    if root
        .apply(&mut writer, SimulationHealthEvent::StartupVerified)
        .is_err()
    {
        ready.pool.close().await;
        return Err(ParentFailure::Setup);
    }
    let (progress_tx, mut progress_rx) = mpsc::channel(1);
    let (fatal_tx, mut fatal_rx) = watch::channel(None);
    let (stop_tx, stop_rx) = watch::channel(false);
    let sample_ms = ready
        .config
        .limits()
        .health_max_age_seconds()
        .checked_mul(1000)
        .ok_or(ParentFailure::Setup)?
        / 3;
    if sample_ms == 0 {
        ready.pool.close().await;
        return Err(ParentFailure::Setup);
    }
    let mut sample = tokio::time::interval(Duration::from_millis(sample_ms));
    sample.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let stop_timeout = Duration::from_secs(ready.config.limits().stop_timeout_seconds());
    let mut deadline: Option<Instant> = None;
    let mut last_completion = None;
    let mut cleanup_deadline_expired = false;

    loop {
        if *signal.borrow() || signal.has_changed().is_err() {
            root.close_admission(&mut writer);
            stop_tx.send_replace(true);
            if !cleanup_deadline_expired {
                deadline.get_or_insert_with(|| Instant::now() + stop_timeout);
            }
        }
        if !root.can_admit() {
            break;
        }
        let progress = ProgressSender {
            tx: progress_tx.clone(),
            fatal: fatal_tx.clone(),
            generation: root.generation,
            epoch: root.epoch,
        };
        let work = run_simulation_observed(
            &ready.queue,
            ready.config.worker_id(),
            &ready.runtime,
            stop_rx.clone(),
            ready.config.owner(),
            progress,
        );
        tokio::pin!(work);
        let result = loop {
            tokio::select! {
                biased;
                changed = signal.changed(), if !root.draining => {
                    if changed.is_err() || *signal.borrow() {
                        root.close_admission(&mut writer);
                        stop_tx.send_replace(true);
                        if !cleanup_deadline_expired { deadline.get_or_insert_with(|| Instant::now() + stop_timeout); }
                    }
                }
                changed = fatal_rx.changed(), if root.fatal.is_none() => {
                    if changed.is_err() || fatal_rx.borrow().is_some() {
                        root.fail(&mut writer, SimulationHealthFailure::RuntimeFailure);
                        stop_tx.send_replace(true);
                        if !cleanup_deadline_expired { deadline.get_or_insert_with(|| Instant::now() + stop_timeout); }
                    }
                }
                _ = async { if let Some(at) = deadline { tokio::time::sleep_until(at).await } else { std::future::pending::<()>().await } }, if deadline.is_some() => {
                    cleanup_deadline_expired = true;
                    root.fail(&mut writer, SimulationHealthFailure::CleanupUnconfirmed);
                    deadline = None;
                }
                result = &mut work => break result,
                message = progress_rx.recv() => {
                    if let Some(message) = message {
                        root.progress(&mut writer, message);
                        if root.fatal.is_some() || root.poisoned {
                            stop_tx.send_replace(true);
                            if !cleanup_deadline_expired { deadline.get_or_insert_with(|| Instant::now() + stop_timeout); }
                        }
                    }
                }
                _ = sample.tick(), if !root.poisoned => {
                    if root.apply(&mut writer, SimulationHealthEvent::Sample).is_err() {
                        root.fail(&mut writer, SimulationHealthFailure::RuntimeFailure);
                        stop_tx.send_replace(true);
                        if !cleanup_deadline_expired { deadline.get_or_insert_with(|| Instant::now() + stop_timeout); }
                    }
                }
            }
        };
        if !root.draining && root.fatal.is_none() {
            if let Some(next) = root.epoch.checked_add(1) {
                root.epoch = next;
            } else {
                root.fail(&mut writer, SimulationHealthFailure::RuntimeFailure);
            }
        }
        apply_completion(&mut root, &mut writer, &result);
        start_fatal_cleanup(
            &root,
            &stop_tx,
            &mut deadline,
            cleanup_deadline_expired,
            stop_timeout,
        );
        if matches!(result.primary, Ok(SimulationWorkerOutcome::Idle))
            && !root.draining
            && root.fatal.is_none()
        {
            let idle_until = Instant::now() + Duration::from_secs(2);
            loop {
                tokio::select! {
                    biased;
                        changed = signal.changed() => if changed.is_err() || *signal.borrow() { root.close_admission(&mut writer); stop_tx.send_replace(true); if !cleanup_deadline_expired { deadline.get_or_insert_with(|| Instant::now() + stop_timeout); } break; },
                    _ = tokio::time::sleep_until(idle_until) => break,
                    _ = sample.tick() => if root.apply(&mut writer, SimulationHealthEvent::Sample).is_err() { root.fail(&mut writer, SimulationHealthFailure::RuntimeFailure); break; },
                }
            }
        }
        start_fatal_cleanup(
            &root,
            &stop_tx,
            &mut deadline,
            cleanup_deadline_expired,
            stop_timeout,
        );
        last_completion = Some(result);
    }
    // A finished work future has joined its heartbeat and all owned child work.
    // Keep the writer and lock alive through pool cleanup, even if poisoned.
    let close = ready.pool.close();
    tokio::pin!(close);
    if let Some(at) = deadline {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(at) => {
                cleanup_deadline_expired = true;
                root.fail(&mut writer, SimulationHealthFailure::CleanupUnconfirmed);
                close.await;
            }
            _ = &mut close => {}
        }
    } else {
        close.await;
    }
    if root.fatal.is_some() || root.poisoned || !root.draining {
        Err(ParentFailure::Runtime {
            first_fatal: root.fatal,
            first_progress_failure: *fatal_rx.borrow(),
            poisoned: root.poisoned,
            draining: root.draining,
            completion: last_completion,
            cleanup_deadline_expired,
        })
    } else {
        Ok(ParentSuccess {
            draining: root.draining,
            completion: last_completion,
        })
    }
}
