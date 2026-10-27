use super::super::runner::{
    AbortOnDrop, CompletionFacts, HeartbeatCompletion, apply_lease_progress,
    confirmed_claim_outcome, finish_with_heartbeat, lease_progress_gate, observed_completion,
    record_heartbeat_failure,
};
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Sink {
    events: Vec<SimulationHealthEvent>,
    fail: bool,
}
impl HealthSink for Sink {
    fn apply_event(&mut self, event: SimulationHealthEvent) -> Result<(), ()> {
        if self.fail {
            return Err(());
        }
        self.events.push(event);
        Ok(())
    }
}

fn root() -> RootState {
    RootState {
        generation: SimulationProcessGeneration::new(17, 4, Uuid::from_u128(1)).unwrap(),
        epoch: 1,
        draining_epoch: None,
        active: None,
        draining: false,
        fatal: None,
        poisoned: false,
    }
}

fn message(
    root: &RootState,
    attempt: Uuid,
    event: SimulationHealthEvent,
) -> (Progress, oneshot::Receiver<Result<(), ProgressFailure>>) {
    let (ack, rx) = oneshot::channel();
    (
        Progress {
            generation: root.generation,
            epoch: root.epoch,
            attempt,
            event,
            ack,
        },
        rx,
    )
}

fn sender(
    state: &RootState,
) -> (
    ProgressSender,
    mpsc::Receiver<Progress>,
    watch::Receiver<Option<ProgressFailure>>,
) {
    let (tx, rx) = mpsc::channel(1);
    let (fatal, fatal_rx) = watch::channel(None);
    (
        ProgressSender {
            tx,
            fatal,
            generation: state.generation,
            epoch: state.epoch,
        },
        rx,
        fatal_rx,
    )
}

#[test]
fn only_acknowledged_no_candidate_idle_counts_as_cycle() {
    assert!(completed_cycle(
        SimulationWorkerOutcome::Idle,
        Some(IdleReason::NoCandidate)
    ));
    assert!(!completed_cycle(
        SimulationWorkerOutcome::Idle,
        Some(IdleReason::Shutdown)
    ));
    assert!(!completed_cycle(
        SimulationWorkerOutcome::Idle,
        Some(IdleReason::SlotRefused)
    ));
    assert!(!completed_cycle(
        SimulationWorkerOutcome::CommitUnresolved,
        None
    ));
    assert!(!completed_cycle(SimulationWorkerOutcome::ClaimLost, None));
}

#[tokio::test]
async fn failed_startup_never_invokes_claim_path() {
    let result = after_verified_startup(async { Err::<(), ()>(()) }, |()| async {
        panic!("claim path must not start");
        #[allow(unreachable_code)]
        Ok::<(), ()>(())
    })
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn canceled_and_failed_startup_never_admit_the_claim_path() {
    for failure in [
        super::super::process_startup::StartupFailure::Rejected,
        super::super::process_startup::StartupFailure::Step {
            operation: None,
            canceled: true,
            timed_out: false,
        },
        super::super::process_startup::StartupFailure::Step {
            operation: None,
            canceled: false,
            timed_out: true,
        },
    ] {
        let result = after_verified_startup(async { Err::<(), _>(failure) }, |()| async {
            panic!("claim path must not start");
            #[allow(unreachable_code)]
            Ok::<(), ()>(())
        })
        .await;
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn production_ack_controls_attempt_and_rejects_stale_epoch() {
    let mut state = root();
    let mut sink = Sink::default();
    let attempt = Uuid::from_u128(2);
    let (claim, ack) = message(&state, attempt, SimulationHealthEvent::ClaimVerified);
    state.progress(&mut sink, claim);
    assert_eq!(ack.await.unwrap(), Ok(()));
    assert_eq!(state.active, Some(attempt));
    let (mut stale, ack) = message(&state, attempt, SimulationHealthEvent::LeaseVerified);
    stale.epoch += 1;
    state.progress(&mut sink, stale);
    assert_eq!(ack.await.unwrap(), Err(ProgressFailure::Rejected));
    assert_eq!(sink.events, vec![SimulationHealthEvent::ClaimVerified]);
    let (lease, ack) = message(&state, attempt, SimulationHealthEvent::LeaseVerified);
    state.progress(&mut sink, lease);
    assert_eq!(ack.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn sender_full_closed_and_lost_ack_are_failures() {
    let state = root();
    let (tx, mut rx) = mpsc::channel(1);
    let (fatal, _fatal_rx) = watch::channel(None);
    let sender = ProgressSender {
        tx: tx.clone(),
        fatal: fatal.clone(),
        generation: state.generation,
        epoch: state.epoch,
    };
    let (occupied, _ack) = message(
        &state,
        Uuid::from_u128(2),
        SimulationHealthEvent::ClaimVerified,
    );
    assert!(tx.try_send(occupied).is_ok());
    assert_eq!(
        sender
            .send_event(Uuid::from_u128(2), SimulationHealthEvent::ClaimVerified)
            .await,
        Err(ProgressFailure::Full)
    );
    drop(rx.recv().await);
    let handle = tokio::spawn(async move {
        sender
            .send_event(Uuid::from_u128(3), SimulationHealthEvent::ClaimVerified)
            .await
    });
    let item = rx.recv().await.unwrap();
    drop(item.ack);
    assert_eq!(handle.await.unwrap(), Err(ProgressFailure::AckLost));
    drop(rx);
    let sender = ProgressSender {
        tx,
        fatal,
        generation: state.generation,
        epoch: state.epoch,
    };
    assert_eq!(
        sender
            .send_event(Uuid::from_u128(3), SimulationHealthEvent::ClaimVerified)
            .await,
        Err(ProgressFailure::Closed)
    );
}

#[tokio::test]
async fn shutdown_rejects_old_event_and_first_fatal_and_poison_are_sticky() {
    let mut state = root();
    let mut sink = Sink::default();
    let (old, ack) = message(
        &state,
        Uuid::from_u128(2),
        SimulationHealthEvent::ClaimVerified,
    );
    state.close_admission(&mut sink);
    state.progress(&mut sink, old);
    assert_eq!(ack.await.unwrap(), Err(ProgressFailure::Draining));
    state.fail(&mut sink, SimulationHealthFailure::CleanupUnconfirmed);
    state.fail(&mut sink, SimulationHealthFailure::CommitUnresolved);
    assert_eq!(
        state.fatal,
        Some(SimulationHealthFailure::CleanupUnconfirmed)
    );
    assert_eq!(sink.events.len(), 2);

    let mut state = root();
    let mut sink = Sink {
        fail: true,
        ..Sink::default()
    };
    assert!(
        state
            .apply(&mut sink, SimulationHealthEvent::StartupVerified)
            .is_err()
    );
    sink.fail = false;
    assert!(
        state
            .apply(&mut sink, SimulationHealthEvent::Sample)
            .is_err()
    );
    assert!(sink.events.is_empty());
}

#[tokio::test]
async fn lost_ack_after_apply_is_a_sticky_fatal() {
    let mut state = root();
    let mut sink = Sink::default();
    let (item, ack) = message(
        &state,
        Uuid::from_u128(2),
        SimulationHealthEvent::ClaimVerified,
    );
    drop(ack);
    state.progress(&mut sink, item);
    assert_eq!(state.fatal, Some(SimulationHealthFailure::RuntimeFailure));
}

#[test]
fn same_await_signal_and_cleanup_error_keeps_error_and_closes_admission() {
    let mut state = root();
    let mut sink = Sink::default();
    state.close_admission(&mut sink);
    worker_failure(
        &mut state,
        &mut sink,
        SimulationWorkerError::CleanupUnconfirmed,
    );
    assert!(state.draining);
    assert_eq!(
        state.fatal,
        Some(SimulationHealthFailure::CleanupUnconfirmed)
    );
    assert_eq!(
        sink.events,
        vec![
            SimulationHealthEvent::ShutdownRequested,
            SimulationHealthEvent::Fatal(SimulationHealthFailure::CleanupUnconfirmed)
        ]
    );
}

#[test]
fn unconfirmed_commit_or_cleanup_prevents_next_claim() {
    for reason in [
        SimulationHealthFailure::CommitUnresolved,
        SimulationHealthFailure::CleanupUnconfirmed,
    ] {
        let mut state = root();
        let mut sink = Sink::default();
        assert!(state.can_admit());
        state.fail(&mut sink, reason);
        assert!(!state.can_admit());
    }
}

#[test]
fn confirmed_cycle_then_shutdown_is_a_clean_drain() {
    let mut state = root();
    let mut sink = Sink::default();
    assert!(
        state
            .apply(&mut sink, SimulationHealthEvent::StartupVerified)
            .is_ok()
    );
    assert!(
        state
            .apply(&mut sink, SimulationHealthEvent::CycleVerified)
            .is_ok()
    );
    state.close_admission(&mut sink);
    assert!(!state.can_admit());
    assert!(state.draining && state.fatal.is_none() && !state.poisoned);
    assert_eq!(
        sink.events.last(),
        Some(&SimulationHealthEvent::ShutdownRequested)
    );
}

#[tokio::test]
async fn pending_claim_ack_after_signal_runs_owned_cleanup_without_stale_apply() {
    let mut state = root();
    let mut sink = Sink::default();
    let (producer, mut events, fatal) = sender(&state);
    let (stop, shutdown) = watch::channel(false);
    let cleaned = Arc::new(AtomicUsize::new(0));
    let cleanup_count = Arc::clone(&cleaned);
    let task = tokio::spawn(async move {
        let mut facts = CompletionFacts::default();
        let primary = confirmed_claim_outcome(
            producer.send_event(Uuid::from_u128(2), SimulationHealthEvent::ClaimVerified),
            || async move {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
                Ok(SimulationWorkerOutcome::Retrying)
            },
            &mut facts,
        )
        .await
        .expect("draining claim must take cleanup path");
        observed_completion(primary, None, facts, &shutdown)
    });
    let event = events.recv().await.unwrap();
    state.close_admission(&mut sink);
    stop.send_replace(true);
    state.progress(&mut sink, event);
    let completion = task.await.unwrap();
    assert_eq!(
        completion.facts.progress_error,
        Some(ProgressFailure::Draining)
    );
    assert!(completion.facts.shutdown);
    assert_eq!(completion.primary, Ok(SimulationWorkerOutcome::Retrying));
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    assert!(fatal.borrow().is_none());
    apply_completion(&mut state, &mut sink, &completion);
    assert!(state.fatal.is_none() && !state.can_admit());
    assert_eq!(sink.events, vec![SimulationHealthEvent::ShutdownRequested]);
}

#[tokio::test]
async fn pending_lease_ack_after_signal_stops_heartbeat_without_false_uncertainty() {
    let mut state = root();
    let mut sink = Sink::default();
    let (producer, mut events, fatal) = sender(&state);
    let attempt = Uuid::from_u128(2);
    let claim_producer = producer.clone();
    let claim = tokio::spawn(async move {
        claim_producer
            .send_event(attempt, SimulationHealthEvent::ClaimVerified)
            .await
    });
    state.progress(&mut sink, events.recv().await.unwrap());
    assert_eq!(claim.await.unwrap(), Ok(()));
    let lease = tokio::spawn(async move {
        lease_progress_gate(producer.send_event(attempt, SimulationHealthEvent::LeaseVerified))
            .await
    });
    let event = events.recv().await.unwrap();
    let (signal, shutdown) = watch::channel(false);
    state.close_admission(&mut sink);
    signal.send_replace(true);
    state.progress(&mut sink, event);
    let (lease_stop, lease_rx) = watch::channel(None);
    assert!(!apply_lease_progress(lease.await.unwrap(), &lease_stop));
    assert_eq!(*lease_rx.borrow(), Some(LeaseStop::DrainSuppressed));
    let (finish, _) = watch::channel(false);
    let mut guard = AbortOnDrop(Some(tokio::spawn(async {})), finish, true, lease_rx);
    let mut facts = CompletionFacts::default();
    let primary = finish_with_heartbeat(
        &mut guard,
        &mut facts,
        Ok(SimulationWorkerOutcome::Retrying),
    )
    .await;
    let completion = observed_completion(primary, None, facts, &shutdown);
    assert_eq!(
        completion.facts.heartbeat,
        Some(HeartbeatCompletion {
            join_failed: false,
            stop: Some(LeaseStop::DrainSuppressed)
        })
    );
    assert!(fatal.borrow().is_none());
    apply_completion(&mut state, &mut sink, &completion);
    assert!(state.fatal.is_none() && !state.can_admit());
    assert_eq!(
        sink.events,
        vec![
            SimulationHealthEvent::ClaimVerified,
            SimulationHealthEvent::ShutdownRequested
        ]
    );
}

#[tokio::test]
async fn progress_failure_still_cleans_confirmed_claim_and_preserves_primary() {
    let mut state = root();
    let mut sink = Sink::default();
    let (producer, mut events, fatal) = sender(&state);
    let (stop, shutdown) = watch::channel(false);
    let cleaned = Arc::new(AtomicUsize::new(0));
    let cleanup_count = Arc::clone(&cleaned);
    let task = tokio::spawn(async move {
        let mut facts = CompletionFacts::default();
        let primary = confirmed_claim_outcome(
            producer.send_event(Uuid::from_u128(2), SimulationHealthEvent::ClaimVerified),
            || async move {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
                Err(SimulationWorkerError::PublicationUnavailable)
            },
            &mut facts,
        )
        .await
        .unwrap();
        observed_completion(primary, None, facts, &shutdown)
    });
    let mut event = events.recv().await.unwrap();
    event.epoch += 1;
    state.progress(&mut sink, event);
    let completion = task.await.unwrap();
    assert_eq!(
        completion.facts.progress_error,
        Some(ProgressFailure::Rejected)
    );
    assert_eq!(
        completion.primary,
        Err(SimulationWorkerError::PublicationUnavailable)
    );
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    assert_eq!(*fatal.borrow(), Some(ProgressFailure::Rejected));
    stop.send_replace(true);
    state.close_admission(&mut sink);
    apply_completion(&mut state, &mut sink, &completion);
    assert!(state.fatal.is_some() && !state.can_admit());
    assert!(!sink.events.contains(&SimulationHealthEvent::CycleVerified));
}

#[tokio::test]
async fn primary_shutdown_heartbeat_join_and_cleanup_facts_remain_independent() {
    for primary in [
        Ok(SimulationWorkerOutcome::CommitUnresolved),
        Err(SimulationWorkerError::PublicationUnavailable),
        Err(SimulationWorkerError::CleanupUnconfirmed),
    ] {
        let mut state = root();
        let mut sink = Sink::default();
        let (signal, shutdown) = watch::channel(false);
        signal.send_replace(true);
        state.close_admission(&mut sink);
        let (finish, mut finish_rx) = watch::channel(false);
        let (lease_tx, lease_rx) = watch::channel(Some(LeaseStop::Unavailable));
        let joined = Arc::new(AtomicUsize::new(0));
        let joined_task = Arc::clone(&joined);
        let task = tokio::spawn(async move {
            let _ = finish_rx.changed().await;
            joined_task.fetch_add(1, Ordering::SeqCst);
            panic!("fixture heartbeat join failure");
        });
        let mut guard = AbortOnDrop(Some(task), finish, true, lease_rx);
        let mut facts = CompletionFacts::default();
        if primary == Err(SimulationWorkerError::CleanupUnconfirmed) {
            facts.child_error = Some(super::super::child::ChildStartError::CleanupUnconfirmed);
        }
        let retained = finish_with_heartbeat(&mut guard, &mut facts, primary).await;
        let cycle = observed_completion(retained, None, facts, &shutdown);
        assert_eq!(cycle.primary, primary);
        assert!(cycle.facts.shutdown);
        assert_eq!(
            cycle.facts.heartbeat,
            Some(HeartbeatCompletion {
                join_failed: true,
                stop: Some(LeaseStop::Unavailable)
            })
        );
        assert_eq!(joined.load(Ordering::SeqCst), 1);
        assert!(guard.0.is_none());
        apply_completion(&mut state, &mut sink, &cycle);
        assert!(!state.can_admit());
        assert_eq!(
            state.fatal,
            Some(
                if primary == Err(SimulationWorkerError::CleanupUnconfirmed) {
                    SimulationHealthFailure::CleanupUnconfirmed
                } else {
                    SimulationHealthFailure::CommitUnresolved
                }
            )
        );
        assert!(!sink.events.contains(&SimulationHealthEvent::CycleVerified));
        let boundary = ParentFailure::Runtime {
            first_fatal: state.fatal,
            first_progress_failure: Some(ProgressFailure::LeaseUnconfirmed),
            poisoned: state.poisoned,
            draining: state.draining,
            completion: Some(cycle),
            cleanup_deadline_expired: false,
        };
        match boundary {
            ParentFailure::Runtime {
                completion: Some(retained),
                draining,
                first_progress_failure,
                ..
            } => {
                assert!(draining && retained.facts.shutdown);
                assert_eq!(retained.primary, primary);
                assert!(retained.facts.heartbeat.unwrap().join_failed);
                assert_eq!(
                    first_progress_failure,
                    Some(ProgressFailure::LeaseUnconfirmed)
                );
            }
            _ => panic!("completion facts must reach the diagnostic boundary"),
        }
        drop(lease_tx);
    }
}

#[test]
fn fatal_completion_starts_bounded_cleanup_without_resetting_deadline() {
    let mut state = root();
    let mut sink = Sink::default();
    state.fail(&mut sink, SimulationHealthFailure::CommitUnresolved);
    let (stop, stop_rx) = watch::channel(false);
    let mut deadline = None;
    start_fatal_cleanup(&state, &stop, &mut deadline, false, Duration::from_secs(3));
    assert!(*stop_rx.borrow());
    let first = deadline.expect("fatal completion creates a cleanup deadline");
    start_fatal_cleanup(&state, &stop, &mut deadline, false, Duration::from_secs(30));
    assert_eq!(deadline, Some(first));
    deadline = None;
    start_fatal_cleanup(&state, &stop, &mut deadline, true, Duration::from_secs(30));
    assert!(
        deadline.is_none(),
        "expired cleanup cannot receive a fresh deadline"
    );
}

#[tokio::test]
async fn real_lease_error_notifies_root_and_is_not_drain_suppression() {
    let state = root();
    let (producer, _events, mut fatal) = sender(&state);
    let (lease_stop, lease_rx) = watch::channel(None);
    record_heartbeat_failure(&lease_stop, Some(&producer));
    assert!(fatal.changed().await.is_ok());
    assert_eq!(*fatal.borrow(), Some(ProgressFailure::LeaseUnconfirmed));
    assert_eq!(*lease_rx.borrow(), Some(LeaseStop::Unavailable));
}

#[test]
fn validation_error_during_shutdown_is_not_erased_by_settlement_outcome() {
    let mut state = root();
    let mut sink = Sink::default();
    state.close_admission(&mut sink);
    let facts = CompletionFacts {
        shutdown: true,
        validation_failure: Some(super::super::SimulationPublicationFailure::ResultInvalid),
        heartbeat: Some(HeartbeatCompletion {
            join_failed: false,
            stop: None,
        }),
        ..CompletionFacts::default()
    };
    let cycle = ObservedCycle {
        primary: Ok(SimulationWorkerOutcome::Retrying),
        idle_reason: None,
        facts,
    };
    apply_completion(&mut state, &mut sink, &cycle);
    assert_eq!(
        cycle.facts.validation_failure,
        Some(super::super::SimulationPublicationFailure::ResultInvalid)
    );
    assert_eq!(state.fatal, Some(SimulationHealthFailure::RuntimeFailure));
    assert!(!state.can_admit());
    assert!(!sink.events.contains(&SimulationHealthEvent::CycleVerified));
}
