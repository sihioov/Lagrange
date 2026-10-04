//! Focused tests for the owning C3B facade. The default target exercises only
//! validation boundaries; the opt-in feature runs real roles, migrations, C2
//! loopback sockets, and the exact task-owned disposable PostgreSQL cluster.

use super::market_stream::{DesiredSet, DesiredStreamItem, StreamIdentity};
use super::market_stream_producer::{
    MarketStreamProducerError, normalize_demand, validate_admissions,
};
use uuid::Uuid;

#[test]
fn producer_debug_view_is_closed_metadata_only() {
    // Helper-only control: this formats ProducerDebugView, not an actual
    // OwnerMarketStreamProducer facade. The feature-gated real-role case
    // below covers the facade itself.
    let output = super::market_stream_producer::debug_synthetic_for_test(2);
    assert!(output.contains("OwnerMarketStreamProducer"));
    assert!(output.contains("phase: Ready"));
    assert!(output.contains("subscription_count: 2"));
    for forbidden in [
        "credential_slot_id",
        "fencing_token",
        "owner_user_id",
        "session",
        "epoch",
        "proof",
        "PENDING",
        "c3b-debug-slot-sentinel",
        "9223372036854775807",
    ] {
        assert!(
            !output.contains(forbidden),
            "forbidden Debug field: {forbidden}"
        );
    }
}

#[test]
fn approved_symbol_guard_is_exactly_bounded_and_unique() {
    use super::market_stream_producer::APPROVED_STREAM_SYMBOLS;

    let unique = APPROVED_STREAM_SYMBOLS
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(APPROVED_STREAM_SYMBOLS.len(), 30);
    assert_eq!(unique.len(), 30);
}

#[test]
fn caller_demand_is_bounded_deduplicated_and_exactly_owner_scoped() {
    let owner = Uuid::new_v4();
    let slot = Uuid::new_v4();
    let identity = StreamIdentity::new(
        owner,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "005930.KRX".to_owned(),
        1,
    )
    .unwrap();
    let demand = DesiredSet {
        credential_slot_id: slot,
        owner_user_id: owner,
        items: vec![DesiredStreamItem {
            identity: identity.clone(),
            reference_count: 1,
        }],
    };
    assert_eq!(normalize_demand(&demand, slot, owner).unwrap().len(), 1);

    let duplicate = DesiredSet {
        items: vec![
            DesiredStreamItem {
                identity: identity.clone(),
                reference_count: 1,
            },
            DesiredStreamItem {
                identity: identity.clone(),
                reference_count: 1,
            },
        ],
        ..demand.clone()
    };
    assert_eq!(
        normalize_demand(&duplicate, slot, owner),
        Err(MarketStreamProducerError::InvalidInput)
    );
    assert_eq!(
        normalize_demand(&demand, Uuid::new_v4(), owner),
        Err(MarketStreamProducerError::InvalidInput)
    );
    assert_eq!(
        validate_admissions(&[identity.clone(), identity], owner),
        Err(MarketStreamProducerError::InvalidInput)
    );
}

#[test]
fn invalid_and_unsupported_symbols_never_enter_normalized_demand() {
    let owner = Uuid::new_v4();
    let slot = Uuid::new_v4();
    for (instrument, count) in [("123456.KRX", 1), ("005930.KRX", 0)] {
        let demand = DesiredSet {
            credential_slot_id: slot,
            owner_user_id: owner,
            items: vec![DesiredStreamItem {
                identity: StreamIdentity::new(
                    owner,
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    instrument.to_owned(),
                    1,
                )
                .unwrap(),
                reference_count: count,
            }],
        };
        assert_eq!(
            normalize_demand(&demand, slot, owner),
            Err(MarketStreamProducerError::InvalidInput)
        );
    }
}

#[test]
fn incomplete_reconciliation_cannot_satisfy_strict_apply_success() {
    use super::market_stream_producer::{MarketStreamApplyOutcome, MarketStreamReconcileOutcome};
    let committed = MarketStreamApplyOutcome { active_symbols: 1, transport_commands: 1 };
    let partial = MarketStreamReconcileOutcome::Deferred { committed, not_before_ms: 11_000 };
    assert_eq!(partial.into_complete(), Err(MarketStreamProducerError::NotReady));
    assert_eq!(MarketStreamReconcileOutcome::Complete(committed).into_complete(), Ok(committed));
    // The compatibility adapter uses this same conversion; no error enum or
    // legacy diagnostic fixture is changed to represent incomplete success.
}

#[cfg(feature = "market-stream-db-tests")]
#[path = "market_stream_producer_test_support.rs"]
mod support;

#[cfg(feature = "market-stream-db-tests")]
mod database_cases {
    use super::super::market_stream_producer::{
        MarketStreamApplyOutcome, MarketStreamProducerOutcome, MarketStreamReconcileOutcome,
        OwnerMarketStreamProducer,
        ProducerAwaitPoint, ProducerTestGate,
    };
    use super::*;
    use crate::owner_equity_v2::market_stream::{
        DesiredSet, OwnerMarketStreamRepository, StreamIdentity, StreamLease, StreamLeaseRequest,
        StreamProducerLease,
    };
    use chrono::{DateTime, Utc};
    use sqlx::Row;
    use sqlx::postgres::PgPoolOptions;
    use std::error::Error;
    use std::future::Future;
    use std::io::{self, Write};
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::support::{self, LoopbackHarness, boundary};
    use kis_client::market_stream::MarketSubscriptionOperation;

    type CaseError = Box<dyn Error + Send + Sync>;
    type CaseResult = Result<(), CaseError>;
    type CaseFuture = Pin<Box<dyn Future<Output = CaseResult> + Send>>;

    // Original20-only observer. No values, errors, identifiers or runtime work.
    #[derive(Clone, Copy)]
    enum Original20Site {
        NotStarted, CreateDatabase, DatabaseName, SeedFixture, Scenario,
        InitialApply, InitialFirstCommand, InitialSecondCommand, RemovedLease,
        ReleaseStreamLease, DemandAfterRelease, UnsubscribeApply, UnsubscribeCommand,
        RemovedSnapshot, Admission, PublicationTime, ReadPublish, PublishedRow,
        RequireQuote, RequireOrdinal, RequireTimestamp, PublishedChecks,
        PersistedAppSnapshot, PersistedQuote, RemovedIdentity, FirstTransportFinish,
        FirstTransportChecks, RestoredLease, RenewProducer, RestoredTime,
        RestoredLoopback, RequireOldEpoch, RestoredStart, ReadNewEpoch,
        RequireNewEpoch, RestoredDemand, RestoredApply, ReackFirstCommand,
        ReackSecondCommand, ReackSnapshot, RestoredAppSnapshot, FinalTransportFinish,
        FinalTransportChecks, Complete, DiagnosticUnavailable,
    }

    impl Original20Site {
        fn literal(self) -> &'static str {
            match self {
                Self::NotStarted => "not_started",
                Self::CreateDatabase => "create_database",
                Self::DatabaseName => "database_name",
                Self::SeedFixture => "seed_fixture",
                Self::Scenario => "scenario",
                Self::InitialApply => "initial_apply",
                Self::InitialFirstCommand => "initial_first_command",
                Self::InitialSecondCommand => "initial_second_command",
                Self::RemovedLease => "removed_lease",
                Self::ReleaseStreamLease => "release_stream_lease",
                Self::DemandAfterRelease => "demand_after_release",
                Self::UnsubscribeApply => "unsubscribe_apply",
                Self::UnsubscribeCommand => "unsubscribe_command",
                Self::RemovedSnapshot => "removed_snapshot",
                Self::Admission => "admission",
                Self::PublicationTime => "publication_time",
                Self::ReadPublish => "read_publish",
                Self::PublishedRow => "published_row",
                Self::RequireQuote => "require_quote",
                Self::RequireOrdinal => "require_ordinal",
                Self::RequireTimestamp => "require_timestamp",
                Self::PublishedChecks => "published_checks",
                Self::PersistedAppSnapshot => "persisted_app_snapshot",
                Self::PersistedQuote => "persisted_quote",
                Self::RemovedIdentity => "removed_identity",
                Self::FirstTransportFinish => "first_transport_finish",
                Self::FirstTransportChecks => "first_transport_checks",
                Self::RestoredLease => "restored_lease",
                Self::RenewProducer => "renew_producer",
                Self::RestoredTime => "restored_time",
                Self::RestoredLoopback => "restored_loopback",
                Self::RequireOldEpoch => "require_old_epoch",
                Self::RestoredStart => "restored_start",
                Self::ReadNewEpoch => "read_new_epoch",
                Self::RequireNewEpoch => "require_new_epoch",
                Self::RestoredDemand => "restored_demand",
                Self::RestoredApply => "restored_apply",
                Self::ReackFirstCommand => "reack_first_command",
                Self::ReackSecondCommand => "reack_second_command",
                Self::ReackSnapshot => "reack_snapshot",
                Self::RestoredAppSnapshot => "restored_app_snapshot",
                Self::FinalTransportFinish => "final_transport_finish",
                Self::FinalTransportChecks => "final_transport_checks",
                Self::Complete => "complete",
                Self::DiagnosticUnavailable => "diagnostic_unavailable",
            }
        }
    }

    #[derive(Clone, Copy)]
    struct Original20Record {
        site: Original20Site,
        terminal: Option<Original19Terminal>,
    }

    #[derive(Clone)]
    struct Original20Diagnostic {
        state: Arc<Mutex<Original20Record>>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Original20WriteOutcome { Written, WriteFailed, DiagnosticUnavailable }

    impl Original20Diagnostic {
        fn new() -> Self {
            Self { state: Arc::new(Mutex::new(Original20Record {
                site: Original20Site::NotStarted,
                terminal: None,
            })) }
        }

        fn enter(&self, site: Original20Site) {
            if let Ok(mut state) = self.state.lock() {
                if state.terminal.is_none() { state.site = site; }
            }
        }

        fn freeze(&self, terminal: Original19Terminal) {
            if let Ok(mut state) = self.state.lock() {
                if state.terminal.is_none() { state.terminal = Some(terminal); }
            }
        }

        fn emit_to(&self, writer: &mut impl Write) -> io::Result<bool> {
            // Never recover poisoned values or present an unfrozen record as ready.
            let (site, terminal, available) = match self.state.lock() {
                Ok(state) => match state.terminal {
                    Some(terminal) => (state.site, terminal, true),
                    None => (Original20Site::DiagnosticUnavailable,
                        Original19Terminal::DiagnosticUnavailable, false),
                },
                Err(_) => (Original20Site::DiagnosticUnavailable,
                    Original19Terminal::DiagnosticUnavailable, false),
            };
            const MAX_PHYSICAL_RECORD_BYTES: usize = 4096;
            // The first LF terminates any preceding libtest prefix. The marker
            // starts a new physical line; no prefix or whitespace is stripped.
            let mut frame = [0_u8; MAX_PHYSICAL_RECORD_BYTES + 1];
            let used = {
                let mut cursor = io::Cursor::new(&mut frame[..]);
                writeln!(
                    cursor,
                    "\nC3B_ORIGINAL20_STAGE_V1 site={} terminal={} diagnostic={}",
                    site.literal(), terminal.literal(),
                    if available { "available" } else { "diagnostic_unavailable" },
                )?;
                cursor.position() as usize
            };
            let record = &frame[1..used];
            if record.len() > MAX_PHYSICAL_RECORD_BYTES
                || std::str::from_utf8(record).is_err()
                || !record.is_ascii()
                || record.last() != Some(&b'\n')
                || record.contains(&b'\r')
                || record.iter().filter(|&&byte| byte == b'\n').count() != 1
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
            writer.write_all(&frame[..used])?;
            Ok(available)
        }

        fn emit(&self) -> Original20WriteOutcome {
            // Catch Rust unwinds only; no panic hook, abort/OOM or crash claim.
            match catch_unwind(AssertUnwindSafe(|| {
                let stderr = io::stderr();
                self.emit_to(&mut stderr.lock())
            })) {
                Ok(Ok(true)) => Original20WriteOutcome::Written,
                Ok(Ok(false)) | Err(_) => Original20WriteOutcome::DiagnosticUnavailable,
                Ok(Err(_)) => Original20WriteOutcome::WriteFailed,
            }
        }
    }

    macro_rules! original20_at {
        ($diagnostic:expr, $site:ident, $operation:expr) => {{
            $diagnostic.enter(Original20Site::$site);
            $operation
        }};
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedDiagnosticStage {
        NotStarted,
        Setup,
        AwaitingAckPersistence,
        AckPersistenceReached,
        CommitJoin,
        CommitResult,
        PostCommitChecks,
        Finalization,
        Complete,
    }

    impl ClosedDiagnosticStage {
        fn literal(self) -> &'static str {
            match self {
                Self::NotStarted => "not_started",
                Self::Setup => "setup",
                Self::AwaitingAckPersistence => "awaiting_ack_persistence",
                Self::AckPersistenceReached => "ack_persistence_reached",
                Self::CommitJoin => "commit_join",
                Self::CommitResult => "commit_result",
                Self::PostCommitChecks => "post_commit_checks",
                Self::Finalization => "finalization",
                Self::Complete => "complete",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedDiagnosticResult {
        Unknown,
        InProgress,
        SetupFailure,
        GateReached,
        CommitReturnedBeforeGate,
        OuterJoinTimeout,
        RelayObservationFailure,
        CommitUnknown,
        CommitUnexpectedSuccess,
        CommitUnexpectedFailure,
        LaterFailure,
        FinalizationFailure,
        Complete,
        UnclassifiedFailure,
        DiagnosticStateUnavailable,
    }

    impl ClosedDiagnosticResult {
        fn literal(self) -> &'static str {
            match self {
                Self::Unknown => "unknown",
                Self::InProgress => "in_progress",
                Self::SetupFailure => "setup_failure",
                Self::GateReached => "gate_reached",
                Self::CommitReturnedBeforeGate => "commit_returned_before_gate",
                Self::OuterJoinTimeout => "outer_join_timeout",
                Self::RelayObservationFailure => "relay_observation_failure",
                Self::CommitUnknown => "commit_unknown",
                Self::CommitUnexpectedSuccess => "commit_unexpected_success",
                Self::CommitUnexpectedFailure => "commit_unexpected_failure",
                Self::LaterFailure => "later_failure",
                Self::FinalizationFailure => "finalization_failure",
                Self::Complete => "complete",
                Self::UnclassifiedFailure => "unclassified_failure",
                Self::DiagnosticStateUnavailable => "diagnostic_state_unavailable",
            }
        }

        fn is_explicit_failure(self) -> bool {
            matches!(
                self,
                Self::SetupFailure
                    | Self::CommitReturnedBeforeGate
                    | Self::OuterJoinTimeout
                    | Self::RelayObservationFailure
                    | Self::CommitUnexpectedSuccess
                    | Self::CommitUnexpectedFailure
                    | Self::FinalizationFailure
            )
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedRelayObservation {
        Pending,
        Observed,
        TimedOut,
        SenderClosed,
    }

    impl ClosedRelayObservation {
        fn literal(self) -> &'static str {
            match self {
                Self::Pending => "pending",
                Self::Observed => "observed",
                Self::TimedOut => "timed_out",
                Self::SenderClosed => "sender_closed",
            }
        }
    }

    impl From<support::CommitObservationOutcome> for ClosedRelayObservation {
        fn from(outcome: support::CommitObservationOutcome) -> Self {
            match outcome {
                support::CommitObservationOutcome::Observed => Self::Observed,
                support::CommitObservationOutcome::TimedOut => Self::TimedOut,
                support::CommitObservationOutcome::SenderClosed => Self::SenderClosed,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedProducerResult {
        Pending,
        CommitUnknown,
        ReturnedOk,
        ReturnedOtherError,
    }

    impl ClosedProducerResult {
        fn literal(self) -> &'static str {
            match self {
                Self::Pending => "pending",
                Self::CommitUnknown => "commit_unknown",
                Self::ReturnedOk => "returned_ok",
                Self::ReturnedOtherError => "returned_other_error",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedDiagnosticTermination {
        Running,
        ReturnedOk,
        ReturnedError,
        Panicked,
        Cancelled,
        JoinUnknown,
        TimedOut,
        StateUnavailable,
    }

    impl ClosedDiagnosticTermination {
        fn literal(self) -> &'static str {
            match self {
                Self::Running => "running",
                Self::ReturnedOk => "returned_ok",
                Self::ReturnedError => "returned_error",
                Self::Panicked => "panicked",
                Self::Cancelled => "cancelled",
                Self::JoinUnknown => "join_unknown",
                Self::TimedOut => "timed_out",
                Self::StateUnavailable => "state_unavailable",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedDiagnosticOutput {
        NotAttempted,
        Written,
        WriteFailed,
        StateUnavailable,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ClosedDiagnosticSnapshot {
        stage: ClosedDiagnosticStage,
        gate_reached: bool,
        result: ClosedDiagnosticResult,
        observation: ClosedRelayObservation,
        producer: ClosedProducerResult,
        termination: ClosedDiagnosticTermination,
        output: ClosedDiagnosticOutput,
    }

    struct ClosedDiagnosticState {
        snapshot: ClosedDiagnosticSnapshot,
        terminal: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClosedDiagnosticWriteOutcome {
        Written,
        WriteFailed,
        StateUnavailableWritten,
        StateUnavailableWriteFailed,
    }

    #[derive(Clone)]
    struct ClosedCaseDiagnostic {
        state: Arc<Mutex<ClosedDiagnosticState>>,
    }

    impl ClosedCaseDiagnostic {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(ClosedDiagnosticState {
                    snapshot: ClosedDiagnosticSnapshot {
                        stage: ClosedDiagnosticStage::NotStarted,
                        gate_reached: false,
                        result: ClosedDiagnosticResult::Unknown,
                        observation: ClosedRelayObservation::Pending,
                        producer: ClosedProducerResult::Pending,
                        termination: ClosedDiagnosticTermination::Running,
                        output: ClosedDiagnosticOutput::NotAttempted,
                    },
                    terminal: false,
                })),
            }
        }

        fn lock_state(
            &self,
        ) -> Result<
            std::sync::MutexGuard<'_, ClosedDiagnosticState>,
            std::sync::MutexGuard<'_, ClosedDiagnosticState>,
        > {
            match self.state.lock() {
                Ok(state) => Ok(state),
                Err(poisoned) => {
                    let mut state = poisoned.into_inner();
                    state.snapshot.result = ClosedDiagnosticResult::DiagnosticStateUnavailable;
                    state.snapshot.termination = ClosedDiagnosticTermination::StateUnavailable;
                    state.snapshot.output = ClosedDiagnosticOutput::StateUnavailable;
                    state.terminal = true;
                    Err(state)
                }
            }
        }

        fn snapshot(&self) -> ClosedDiagnosticSnapshot {
            match self.lock_state() {
                Ok(state) | Err(state) => state.snapshot,
            }
        }

        fn update_open(&self, update: impl FnOnce(&mut ClosedDiagnosticSnapshot)) {
            let Ok(mut state) = self.lock_state() else {
                return;
            };
            if state.terminal {
                return;
            }
            update(&mut state.snapshot);
        }

        fn terminate(&self, update: impl FnOnce(&mut ClosedDiagnosticSnapshot)) {
            let Ok(mut state) = self.lock_state() else {
                return;
            };
            if state.terminal {
                return;
            }
            update(&mut state.snapshot);
            state.terminal = true;
        }

        fn enter(&self, stage: ClosedDiagnosticStage) {
            self.update_open(|snapshot| {
                snapshot.stage = stage;
                snapshot.result = ClosedDiagnosticResult::InProgress;
            });
        }

        fn reached_gate(&self) {
            self.update_open(|snapshot| {
                snapshot.stage = ClosedDiagnosticStage::AckPersistenceReached;
                snapshot.gate_reached = true;
                snapshot.result = ClosedDiagnosticResult::GateReached;
            });
        }

        fn result(&self, result: ClosedDiagnosticResult) {
            self.update_open(|snapshot| snapshot.result = result);
        }

        fn observation(&self, observation: support::CommitObservationOutcome) {
            self.update_open(|snapshot| snapshot.observation = observation.into());
        }

        fn producer_result(
            &self,
            result: &Result<MarketStreamApplyOutcome, MarketStreamProducerError>,
        ) {
            self.update_open(|snapshot| {
                snapshot.producer = classify_commit_producer_result(result)
            });
        }

        fn record_commit_join_timeout(&self) {
            self.update_open(|snapshot| {
                snapshot.result = ClosedDiagnosticResult::OuterJoinTimeout;
                if snapshot.observation == ClosedRelayObservation::Pending {
                    snapshot.observation = ClosedRelayObservation::TimedOut;
                }
            });
        }

        fn passthrough<E>(&self, result: ClosedDiagnosticResult, error: E) -> E {
            self.result(result);
            error
        }

        fn record_setup_failure(&self) {
            self.terminate(|snapshot| {
                snapshot.stage = ClosedDiagnosticStage::Setup;
                snapshot.result = ClosedDiagnosticResult::SetupFailure;
                snapshot.termination = ClosedDiagnosticTermination::ReturnedError;
            });
        }

        fn record_body_error(&self) {
            self.terminate(|snapshot| {
                if !snapshot.result.is_explicit_failure() {
                    snapshot.result = match snapshot.stage {
                        ClosedDiagnosticStage::Setup => ClosedDiagnosticResult::SetupFailure,
                        ClosedDiagnosticStage::CommitResult
                        | ClosedDiagnosticStage::PostCommitChecks => {
                            ClosedDiagnosticResult::LaterFailure
                        }
                        ClosedDiagnosticStage::Finalization => {
                            ClosedDiagnosticResult::FinalizationFailure
                        }
                        _ => ClosedDiagnosticResult::UnclassifiedFailure,
                    };
                }
                snapshot.termination = ClosedDiagnosticTermination::ReturnedError;
            });
        }

        fn record_join_error(&self, is_panic: bool, is_cancelled: bool) {
            self.terminate(|snapshot| {
                snapshot.termination = classify_join_termination(is_panic, is_cancelled);
            });
        }

        fn record_body_timeout(&self) {
            self.terminate(|snapshot| {
                snapshot.termination = ClosedDiagnosticTermination::TimedOut;
            });
        }

        fn record_body_success(&self) {
            self.terminate(|snapshot| {
                if snapshot.stage != ClosedDiagnosticStage::Complete
                    || snapshot.result != ClosedDiagnosticResult::Complete
                {
                    snapshot.result = ClosedDiagnosticResult::UnclassifiedFailure;
                }
                snapshot.termination = ClosedDiagnosticTermination::ReturnedOk;
            });
        }

        fn record_output(&self, output: ClosedDiagnosticOutput) -> bool {
            let Ok(mut state) = self.lock_state() else {
                return false;
            };
            state.snapshot.output = output;
            true
        }

        fn emit_to(&self, writer: &mut impl Write) -> ClosedDiagnosticWriteOutcome {
            let (snapshot, available_before) = match self.lock_state() {
                Ok(state) => (state.snapshot, true),
                Err(state) => (state.snapshot, false),
            };
            let written = writeln!(
                writer,
                "C3B_CLOSED_DIAGNOSTIC stage={} gate={} result={} termination={} observation={} producer={}",
                snapshot.stage.literal(),
                if snapshot.gate_reached {
                    "reached"
                } else {
                    "not_reached"
                },
                snapshot.result.literal(),
                snapshot.termination.literal(),
                snapshot.observation.literal(),
                snapshot.producer.literal(),
            )
            .is_ok();
            let recorded = self.record_output(if written {
                ClosedDiagnosticOutput::Written
            } else {
                ClosedDiagnosticOutput::WriteFailed
            });
            match (available_before && recorded, written) {
                (true, true) => ClosedDiagnosticWriteOutcome::Written,
                (true, false) => ClosedDiagnosticWriteOutcome::WriteFailed,
                (false, true) => ClosedDiagnosticWriteOutcome::StateUnavailableWritten,
                (false, false) => ClosedDiagnosticWriteOutcome::StateUnavailableWriteFailed,
            }
        }

        fn emit(&self) -> ClosedDiagnosticWriteOutcome {
            let stderr = io::stderr();
            self.emit_to(&mut stderr.lock())
        }
    }

    fn classify_join_termination(
        is_panic: bool,
        is_cancelled: bool,
    ) -> ClosedDiagnosticTermination {
        if is_panic {
            ClosedDiagnosticTermination::Panicked
        } else if is_cancelled {
            ClosedDiagnosticTermination::Cancelled
        } else {
            ClosedDiagnosticTermination::JoinUnknown
        }
    }

    fn classify_commit_producer_result(
        result: &Result<MarketStreamApplyOutcome, MarketStreamProducerError>,
    ) -> ClosedProducerResult {
        match result {
            Err(MarketStreamProducerError::Storage(
                crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown,
            )) => ClosedProducerResult::CommitUnknown,
            Ok(_) => ClosedProducerResult::ReturnedOk,
            Err(_) => ClosedProducerResult::ReturnedOtherError,
        }
    }

    async fn track_commit_join<F>(
        operation: F,
        observation: &mut support::CommitObservation,
        diagnostic: &ClosedCaseDiagnostic,
    ) -> Option<(
        Result<MarketStreamApplyOutcome, MarketStreamProducerError>,
        support::CommitObservationOutcome,
    )>
    where
        F: Future<Output = Result<MarketStreamApplyOutcome, MarketStreamProducerError>>,
    {
        let producer_diagnostic = diagnostic.clone();
        let observation_diagnostic = diagnostic.clone();
        let tracked_operation = async {
            let result = operation.await;
            producer_diagnostic.producer_result(&result);
            result
        };
        let tracked_observation = async {
            let outcome = observation.receive().await;
            observation_diagnostic.observation(outcome);
            outcome
        };
        match tokio::time::timeout(Duration::from_secs(8), async {
            tokio::join!(tracked_operation, tracked_observation)
        })
        .await
        {
            Ok(joined) => Some(joined),
            Err(_) => {
                diagnostic.record_commit_join_timeout();
                None
            }
        }
    }

    async fn emit_diagnostic_then_cleanup<Emit, Cleanup, CleanupFuture, CleanupOutput>(
        diagnostic: Option<&ClosedCaseDiagnostic>,
        emit: Emit,
        cleanup: Cleanup,
    ) -> (Option<ClosedDiagnosticWriteOutcome>, CleanupOutput)
    where
        Emit: FnOnce(&ClosedCaseDiagnostic) -> ClosedDiagnosticWriteOutcome,
        Cleanup: FnOnce() -> CleanupFuture,
        CleanupFuture: Future<Output = CleanupOutput>,
    {
        let diagnostic_write = diagnostic.map(emit);
        let cleanup_output = cleanup().await;
        (diagnostic_write, cleanup_output)
    }


    // Original19-only companion. No diagnostic operation owns or retries work.
    use super::super::market_stream_producer::{
        MarketStreamControlOutcome, MarketStreamTransportStatus,
        ProducerTransportErrorObservation, ProducerTransportErrorProbe,
    };
    use crate::owner_equity_v2::market_stream::MarketStreamStorageError;
    use kis_client::market_stream_approval::ApprovalError;
    use kis_client::market_stream_state::{BudgetError, StateError};
    use kis_client::market_stream_wire::WireError;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    macro_rules! original19_labels {
        ($name:ident { $($variant:ident => $literal:literal),+ $(,)? }) => {
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            enum $name { $($variant),+ }
            impl $name {
                fn literal(self) -> &'static str {
                    match self { $(Self::$variant => $literal),+ }
                }
            }
        };
    }

    original19_labels!(Original19Site {
        NotStarted => "not_started",
        CreateDatabase => "create_database",
        DatabaseName => "database_name",
        Seed => "seed",
        PrimaryLease => "primary_lease",
        Claim => "claim",
        InitialDbTime => "initial_db_time",
        LoopbackSetup => "loopback_setup",
        Start => "start",
        InitialDemand => "initial_demand",
        InitialApply => "initial_apply",
        FirstCommandReceipt => "first_command_receipt",
        SecondCommandReceipt => "second_command_receipt",
        SnapshotBefore => "snapshot_before",
        ExtraLease => "extra_lease",
        DemandTwice => "demand_twice",
        ApplyTwice => "apply_twice",
        SnapshotTwice => "snapshot_twice",
        ApplyIdentical => "apply_identical",
        SnapshotIdentical => "snapshot_identical",
        ReleaseExtra => "release_extra",
        DemandOnce => "demand_once",
        ApplyOnce => "apply_once",
        SnapshotOnce => "snapshot_once",
        PublicationDbTime => "publication_db_time",
        ReadQuote => "read_quote",
        RequirePublished => "require_published",
        RequireQuote => "require_quote",
        AppSnapshot => "app_snapshot",
        TransportFinish => "transport_finish",
        FinalAssertions => "final_assertions",
        Complete => "complete",
    });
    original19_labels!(Original19Iteration {
        None => "none", One => "one", Two => "two", Three => "three",
    });
    original19_labels!(Original19Step {
        NotStarted => "not_started", Entered => "entered",
        ReturnedOk => "returned_ok", ReturnedError => "returned_error",
        SemanticNonPublished => "semantic_non_published",
        SemanticMissingQuote => "semantic_missing_quote", Complete => "complete",
    });
    original19_labels!(Original19Terminal {
        ReturnedOk => "returned_ok", ReturnedError => "returned_error",
        JoinPanic => "join_panic", JoinCancelled => "join_cancelled",
        JoinOther => "join_other", TimeoutJoinedOk => "timeout_joined_ok",
        TimeoutJoinedBodyError => "timeout_joined_body_error",
        TimeoutJoinedPanic => "timeout_joined_panic",
        TimeoutJoinedCancelled => "timeout_joined_cancelled",
        TimeoutJoinedOther => "timeout_joined_other",
        DiagnosticUnavailable => "diagnostic_unavailable",
    });
    original19_labels!(Original19Availability {
        Available => "available", DiagnosticUnavailable => "diagnostic_unavailable",
    });

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Original19ErrorClass {
        kind: &'static str,
        producer: &'static str,
        storage: &'static str,
        transport: &'static str,
        sqlx: &'static str,
    }

    impl Original19ErrorClass {
        const NONE: Self = Self {
            kind: "none", producer: "none", storage: "none",
            transport: "none", sqlx: "none",
        };
    }

    fn original19_transport(error: &ProducerTransportErrorObservation) -> &'static str {
        use ProducerTransportErrorObservation as O;
        match error {
            O::Approval(error) => match error {
                ApprovalError::EndpointNotAllowed => "approval_endpoint_not_allowed",
                ApprovalError::HttpRejected { .. } => "approval_http_rejected",
                ApprovalError::ResponseInvalid => "approval_response_invalid",
                ApprovalError::OutcomeAmbiguous => "approval_outcome_ambiguous",
                ApprovalError::AttemptTooSoon => "approval_attempt_too_soon",
                ApprovalError::AttemptBudgetExhausted => "approval_attempt_budget_exhausted",
                ApprovalError::CredentialGenerationMismatch => "approval_credential_generation_mismatch",
                ApprovalError::Expired => "approval_expired",
                ApprovalError::TransportAmbiguous => "approval_transport_ambiguous",
                ApprovalError::State => "approval_state",
            },
            O::EndpointNotAllowed => "endpoint_not_allowed",
            O::Io => "io",
            O::Handshake => "handshake",
            O::FrameInvalid => "frame_invalid",
            O::Protocol => "protocol",
            O::Closed => "closed",
            O::CommandPending => "command_pending",
            O::CommandCapabilityMismatch => "command_capability_mismatch",
            O::CommandTimingInvalid => "command_timing_invalid",
            O::CommandTimingOverflow => "command_timing_overflow",
            O::AckTimeout => "ack_timeout",
            O::ControlWriteTimeout => "control_write_timeout",
            O::AckMismatch => "ack_mismatch",
            O::DuplicateAck => "duplicate_ack",
            O::CommandRejected => "command_rejected",
            O::DataBeforeAck => "data_before_ack",
            O::SymbolNotAllowed => "symbol_not_allowed",
            O::ResubscribeRequiresNewEpoch => "resubscribe_requires_new_epoch",
            O::SubscriptionLimit => "subscription_limit",
            O::CommandBudget(error) => match error {
                BudgetError::MinimumSpacing => "command_budget_minimum_spacing",
                BudgetError::RollingWindow => "command_budget_rolling_window",
                BudgetError::DailyLimit => "command_budget_daily_limit",
            },
            O::ReconnectBudget(error) => match error {
                BudgetError::MinimumSpacing => "reconnect_budget_minimum_spacing",
                BudgetError::RollingWindow => "reconnect_budget_rolling_window",
                BudgetError::DailyLimit => "reconnect_budget_daily_limit",
            },
            O::ReconnectExhausted => "reconnect_exhausted",
            O::Wire(error) => match error {
                WireError::MessageTooLarge => "wire_message_too_large",
                WireError::InvalidUtf8 => "wire_invalid_utf8",
                WireError::ControlCharacter => "wire_control_character",
                WireError::FramingInvalid => "wire_framing_invalid",
                WireError::RecordCountInvalid => "wire_record_count_invalid",
                WireError::RecordCountExceeded => "wire_record_count_exceeded",
                WireError::FieldCountMismatch => "wire_field_count_mismatch",
                WireError::FieldTooLarge => "wire_field_too_large",
                WireError::SchemaMismatch => "wire_schema_mismatch",
                WireError::SymbolInvalid => "wire_symbol_invalid",
                WireError::DateInvalid => "wire_date_invalid",
                WireError::TimeInvalid => "wire_time_invalid",
                WireError::NumericInvalid => "wire_numeric_invalid",
                WireError::DirectionInvalid => "wire_direction_invalid",
                WireError::DirectionContradiction => "wire_direction_contradiction",
                WireError::VolumeInvalid => "wire_volume_invalid",
                WireError::HaltFlagInvalid => "wire_halt_flag_invalid",
                WireError::MarketClassUnsupported => "wire_market_class_unsupported",
                WireError::FutureObservation => "wire_future_observation",
                WireError::OutsideSessionWindow => "wire_outside_session_window",
                WireError::StaleEpochObservation => "wire_stale_epoch_observation",
                WireError::UnsupportedMessage => "wire_unsupported_message",
            },
            O::State(error) => match error {
                StateError::Io => "state_io",
                StateError::UnsafePath => "state_unsafe_path",
                StateError::InvalidState => "state_invalid_state",
                StateError::LockBusy => "state_lock_busy",
                StateError::PriorSessionUncertain => "state_prior_session_uncertain",
                StateError::ReconnectNotReady => "state_reconnect_not_ready",
                StateError::Serialization => "state_serialization",
                StateError::Budget(error) => match error {
                    BudgetError::MinimumSpacing => "state_budget_minimum_spacing",
                    BudgetError::RollingWindow => "state_budget_rolling_window",
                    BudgetError::DailyLimit => "state_budget_daily_limit",
                },
            },
            O::Cancelled => "cancelled",
            O::SessionProofRequired => "session_proof_required",
            O::SessionProofInvalid => "session_proof_invalid",
        }
    }

    // Only TransportFailure calls read, once; a poisoned producer probe may
    // unwind internally. Catch only that observer unwind, never producer work.
    fn original19_producer_error(
        error: &MarketStreamProducerError,
        read: impl FnOnce() -> Option<ProducerTransportErrorObservation>,
    ) -> Original19ErrorClass {
        use MarketStreamProducerError as E;
        let producer = match error {
            E::Storage(_) => "storage",
            E::InvalidInput => "invalid_input",
            E::NotReady => "not_ready",
            E::Terminal => "terminal",
            E::TransportFailure => "transport_failure",
            E::SubscriptionOutOfSync => "subscription_out_of_sync",
            E::SessionUnavailable => "session_unavailable",
        };
        let storage = match error {
            E::Storage(error) => error.code(),
            _ => "none",
        };
        let transport = if matches!(error, E::TransportFailure) {
            match catch_unwind(AssertUnwindSafe(read)) {
                Ok(Some(error)) => original19_transport(&error),
                Ok(None) | Err(_) => "diagnostic_unavailable",
            }
        } else {
            "none"
        };
        Original19ErrorClass {
            kind: "producer", producer, storage, transport, sqlx: "none",
        }
    }

    fn original19_sqlx(error: &sqlx::Error) -> &'static str {
        match error {
            sqlx::Error::Configuration(_) => "configuration",
            sqlx::Error::InvalidArgument(_) => "invalid_argument",
            sqlx::Error::Database(_) => "database",
            sqlx::Error::Io(_) => "io",
            sqlx::Error::Tls(_) => "tls",
            sqlx::Error::Protocol(_) => "protocol",
            sqlx::Error::RowNotFound => "row_not_found",
            sqlx::Error::TypeNotFound { .. } => "type_not_found",
            sqlx::Error::ColumnIndexOutOfBounds { .. } => "column_index_out_of_bounds",
            sqlx::Error::ColumnNotFound(_) => "column_not_found",
            sqlx::Error::ColumnDecode { .. } => "column_decode",
            sqlx::Error::Encode(_) => "encode",
            sqlx::Error::Decode(_) => "decode",
            sqlx::Error::AnyDriverError(_) => "any_driver_error",
            sqlx::Error::PoolTimedOut => "pool_timed_out",
            sqlx::Error::PoolClosed => "pool_closed",
            sqlx::Error::WorkerCrashed => "worker_crashed",
            sqlx::Error::Migrate(_) => "migrate",
            sqlx::Error::InvalidSavePointStatement => "invalid_save_point_statement",
            sqlx::Error::BeginFailed => "begin_failed",
            sqlx::Error::ConfigFile(_) => "config_file",
            _ => "other_non_exhaustive",
        }
    }

    fn original19_fixture_error(error: &(dyn Error + 'static)) -> Original19ErrorClass {
        if let Some(error) = error.downcast_ref::<MarketStreamProducerError>() {
            original19_producer_error(error, || None)
        } else if let Some(error) = error.downcast_ref::<MarketStreamStorageError>() {
            Original19ErrorClass {
                kind: "storage", storage: error.code(), ..Original19ErrorClass::NONE
            }
        } else if let Some(error) = error.downcast_ref::<sqlx::Error>() {
            Original19ErrorClass {
                kind: "sqlx", sqlx: original19_sqlx(error), ..Original19ErrorClass::NONE
            }
        } else {
            Original19ErrorClass {
                kind: "fixture_returned_error", ..Original19ErrorClass::NONE
            }
        }
    }

    fn original19_control(outcome: MarketStreamControlOutcome) -> &'static str {
        match outcome {
            MarketStreamControlOutcome::ApplicationHeartbeat => "application_heartbeat",
            MarketStreamControlOutcome::ControlPong => "control_pong",
            MarketStreamControlOutcome::NoCurrentAdmission => "no_current_admission",
            MarketStreamControlOutcome::TransportStatus(status) => match status {
                MarketStreamTransportStatus::Connected => "status_connected",
                MarketStreamTransportStatus::Closed => "status_closed",
                MarketStreamTransportStatus::Gap => "status_gap",
                MarketStreamTransportStatus::DataBeforeAck => "status_data_before_ack",
                MarketStreamTransportStatus::DuplicateObservation => "status_duplicate_observation",
                MarketStreamTransportStatus::StaleObservation => "status_stale_observation",
            },
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Original19Joined {
        BodyOk, BodyError, Panic, Cancelled, Other,
    }

    fn original19_join_error(panic: bool, cancelled: bool) -> Original19Joined {
        if panic { Original19Joined::Panic }
        else if cancelled { Original19Joined::Cancelled }
        else { Original19Joined::Other }
    }

    fn original19_joined<E>(
        joined: &Result<Result<(), E>, tokio::task::JoinError>,
    ) -> Original19Joined {
        match joined {
            Ok(Ok(())) => Original19Joined::BodyOk,
            Ok(Err(_)) => Original19Joined::BodyError,
            Err(error) => original19_join_error(error.is_panic(), error.is_cancelled()),
        }
    }

    fn original19_terminal(timeout: bool, joined: Original19Joined) -> Original19Terminal {
        use Original19Joined as J;
        use Original19Terminal as T;
        match (timeout, joined) {
            (false, J::BodyOk) => T::ReturnedOk,
            (false, J::BodyError) => T::ReturnedError,
            (false, J::Panic) => T::JoinPanic,
            (false, J::Cancelled) => T::JoinCancelled,
            (false, J::Other) => T::JoinOther,
            (true, J::BodyOk) => T::TimeoutJoinedOk,
            (true, J::BodyError) => T::TimeoutJoinedBodyError,
            (true, J::Panic) => T::TimeoutJoinedPanic,
            (true, J::Cancelled) => T::TimeoutJoinedCancelled,
            (true, J::Other) => T::TimeoutJoinedOther,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Original19Record {
        site: Original19Site,
        iteration: Original19Iteration,
        step: Original19Step,
        terminal: Option<Original19Terminal>,
        error: Original19ErrorClass,
        control: &'static str,
    }

    #[derive(Clone)]
    struct Original19Diagnostic {
        state: Arc<Mutex<Original19Record>>,
    }

    impl Original19Diagnostic {
        fn new() -> Self {
            Self { state: Arc::new(Mutex::new(Original19Record {
                site: Original19Site::NotStarted,
                iteration: Original19Iteration::None,
                step: Original19Step::NotStarted,
                terminal: None,
                error: Original19ErrorClass::NONE,
                control: "none",
            })) }
        }

        fn update(&self, update: impl FnOnce(&mut Original19Record)) {
            // A poisoned lock is never unwrapped or repaired by the observer.
            // Snapshot reports unavailable; subsequent updates are ignored.
            if let Ok(mut state) = self.state.lock() {
                if state.terminal.is_none() {
                    update(&mut state);
                }
            }
        }

        fn snapshot(&self) -> (Original19Record, Original19Availability) {
            match self.state.lock() {
                Ok(state) => (*state, Original19Availability::Available),
                Err(poison) => (*poison.into_inner(), Original19Availability::DiagnosticUnavailable),
            }
        }

        fn enter(&self, site: Original19Site) {
            self.update(|state| {
                state.site = site;
                state.step = Original19Step::Entered;
                state.error = Original19ErrorClass::NONE;
                state.control = "none";
            });
        }

        fn iteration(&self, iteration: Original19Iteration) {
            self.update(|state| state.iteration = iteration);
        }

        fn observe<T, E>(
            &self,
            result: Result<T, E>,
            classify: impl FnOnce(&E) -> Original19ErrorClass,
        ) -> Result<T, E> {
            match &result {
                Ok(_) => self.update(|state| state.step = Original19Step::ReturnedOk),
                Err(error) => {
                    let error = classify(error);
                    self.update(|state| {
                        state.step = Original19Step::ReturnedError;
                        state.error = error;
                    });
                }
            }
            // Preserve the original object, including boxed fixture errors.
            result
        }

        fn require_published(&self, outcome: &MarketStreamProducerOutcome) {
            self.enter(Original19Site::RequirePublished);
            self.update(|state| match outcome {
                MarketStreamProducerOutcome::Published(_) => {
                    state.step = Original19Step::ReturnedOk;
                }
                MarketStreamProducerOutcome::Control(control) => {
                    state.step = Original19Step::SemanticNonPublished;
                    state.control = original19_control(*control);
                }
            });
        }

        fn require_quote<T, E>(&self, result: Result<T, E>) -> Result<T, E> {
            self.update(|state| {
                state.step = if result.is_ok() {
                    Original19Step::ReturnedOk
                } else {
                    Original19Step::SemanticMissingQuote
                };
            });
            result
        }

        fn freeze(&self, terminal: Original19Terminal) {
            self.update(|state| state.terminal = Some(terminal));
        }

        fn complete(&self) {
            self.update(|state| {
                state.site = Original19Site::Complete;
                state.iteration = Original19Iteration::None;
                state.step = Original19Step::Complete;
            });
        }

        fn emit_to(&self, writer: &mut impl Write) -> io::Result<()> {
            let (record, availability) = self.snapshot();
            // All arguments originate exclusively from finite static matches.
            // Emission never mutates the terminal record.
            writeln!(
                writer,
                "C3B_ORIGINAL19_DIAGNOSTIC_V1 site={} iteration={} step={} terminal={} error={} producer={} storage={} transport={} sqlx={} control={} diagnostic={}",
                record.site.literal(),
                record.iteration.literal(),
                record.step.literal(),
                record.terminal.unwrap_or(Original19Terminal::DiagnosticUnavailable).literal(),
                record.error.kind,
                record.error.producer,
                record.error.storage,
                record.error.transport,
                record.error.sqlx,
                record.control,
                availability.literal(),
            )
        }

        fn emit(&self) -> io::Result<()> {
            let stderr = io::stderr();
            self.emit_to(&mut stderr.lock())
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Original19WriteOutcome { Written, WriteFailed, DiagnosticUnavailable }

    // Synchronous no-unwind boundary, before invoking the existing cleanup
    // closure. It catches Rust unwinds only, not abort/OOM/process termination.
    // No output bookkeeping writes into the frozen diagnostic state.
    fn original19_emit_then_cleanup<Emit, Cleanup, Output>(
        diagnostic: Option<&Original19Diagnostic>,
        emit: Emit,
        cleanup: Cleanup,
    ) -> (Option<Original19WriteOutcome>, Output)
    where
        Emit: FnOnce(&Original19Diagnostic) -> io::Result<()>,
        Cleanup: FnOnce() -> Output,
    {
        let write = diagnostic.map(|diagnostic| {
            match catch_unwind(AssertUnwindSafe(|| emit(diagnostic))) {
                Ok(Ok(())) => Original19WriteOutcome::Written,
                Ok(Err(_)) => Original19WriteOutcome::WriteFailed,
                Err(_) => Original19WriteOutcome::DiagnosticUnavailable,
            }
        });
        (write, cleanup())
    }

    macro_rules! original19_observe {
        ($diagnostic:expr, $site:ident, $operation:expr, $classify:expr) => {{
            let diagnostic = &$diagnostic;
            diagnostic.enter(Original19Site::$site);
            diagnostic.observe($operation, $classify)
        }};
    }

    macro_rules! original19_quote {
        ($diagnostic:expr, $operation:expr) => {{
            let diagnostic = &$diagnostic;
            diagnostic.enter(Original19Site::RequireQuote);
            diagnostic.require_quote($operation)
        }};
    }

    async fn run_original19_database_case<F>(body: F) -> Result<CleanupReceipt, CaseError>
    where
        F: FnOnce(boundary::DisposableDatabase, Original19Diagnostic) -> CaseFuture
            + Send
            + 'static,
    {
        let diagnostic = Original19Diagnostic::new();
        let body_diagnostic = diagnostic.clone();
        run_database_case_inner(
            move |database| body(database, body_diagnostic),
            None,
            Some(diagnostic),
            None,
        ).await
    }


    #[cfg(test)]
    mod original19_closed_diagnostic_unit_tests {
        use super::*;
        use std::cell::Cell;

        fn failed_record() -> Original19Diagnostic {
            let diagnostic = Original19Diagnostic::new();
            diagnostic.enter(Original19Site::InitialApply);
            let result: Result<(), MarketStreamProducerError> =
                Err(MarketStreamProducerError::TransportFailure);
            let unchanged = diagnostic.observe(result, |error| {
                original19_producer_error(error, || Some(
                    ProducerTransportErrorObservation::CommandBudget(BudgetError::MinimumSpacing)
                ))
            });
            assert_eq!(unchanged, result);
            diagnostic.freeze(Original19Terminal::ReturnedError);
            diagnostic
        }

        fn render(diagnostic: &Original19Diagnostic) -> String {
            let mut bytes = Vec::new();
            diagnostic.emit_to(&mut bytes).expect("private in-memory writer");
            String::from_utf8(bytes).expect("closed ASCII grammar")
        }

        fn original19_grammar_value_is_valid(field: &str, value: &str) -> bool {
            !value.is_empty()
                && (value.bytes().all(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                    || (field == "transport" && value == "wire_invalid_utf8"))
        }

        fn assert_grammar(line: &str) {
            assert!(line.ends_with('\n'));
            assert_eq!(line.bytes().filter(|byte| *byte == b'\n').count(), 1);
            let fields: Vec<_> = line.trim_end_matches('\n').split(' ').collect();
            assert_eq!(fields.len(), 12);
            assert_eq!(fields[0], "C3B_ORIGINAL19_DIAGNOSTIC_V1");
            for (field, key) in fields[1..].iter().zip([
                "site", "iteration", "step", "terminal", "error", "producer",
                "storage", "transport", "sqlx", "control", "diagnostic",
            ]) {
                let (actual_key, value) = field.split_once('=').expect("fixed field");
                assert_eq!(actual_key, key);
                assert!(original19_grammar_value_is_valid(key, value));
            }
        }

        #[tokio::test(flavor = "current_thread")]
        async fn emitter_unwind_error_success_and_absence_keep_cleanup_once() {
            struct ErrorWriter;
            impl Write for ErrorWriter {
                fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                    Err(io::Error::new(io::ErrorKind::BrokenPipe, "R71_SYNTHETIC_WRITER"))
                }
                fn flush(&mut self) -> io::Result<()> { Ok(()) }
            }
            for mode in 0..4 {
                let diagnostic = failed_record();
                let frozen = diagnostic.snapshot();
                let emit_calls = Cell::new(0);
                let cleanup_calls = Cell::new(0);
                let cleanup_runs = Cell::new(0);
                let present = if mode == 3 { None } else { Some(&diagnostic) };
                // This is the same synchronous helper and deferred cleanup
                // future used by the real optional wrapper path.
                let (write, cleanup) = original19_emit_then_cleanup(
                    present,
                    |diagnostic| {
                        emit_calls.set(emit_calls.get() + 1);
                        assert_eq!(cleanup_calls.get(), 0);
                        match mode {
                            0 => panic!("R71_SYNTHETIC_EMITTER_UNWIND"),
                            1 => diagnostic.emit_to(&mut ErrorWriter),
                            2 => diagnostic.emit_to(&mut Vec::new()),
                            _ => panic!("absent companion must not emit"),
                        }
                    },
                    || {
                        assert_eq!(emit_calls.get(), usize::from(mode != 3));
                        cleanup_calls.set(cleanup_calls.get() + 1);
                        async {
                            cleanup_runs.set(cleanup_runs.get() + 1);
                            17_u8
                        }
                    },
                );
                assert_eq!(cleanup.await, 17);
                assert_eq!(cleanup_calls.get(), 1);
                assert_eq!(cleanup_runs.get(), 1);
                assert_eq!(diagnostic.snapshot(), frozen);
                assert_eq!(write, match mode {
                    0 => Some(Original19WriteOutcome::DiagnosticUnavailable),
                    1 => Some(Original19WriteOutcome::WriteFailed),
                    2 => Some(Original19WriteOutcome::Written),
                    _ => None,
                });
            }
        }

        #[test]
        fn probe_some_none_unwind_preserve_public_error_and_take_once() {
            for mode in 0..3 {
                let diagnostic = Original19Diagnostic::new();
                diagnostic.enter(Original19Site::InitialApply);
                let calls = Cell::new(0);
                let error = MarketStreamProducerError::TransportFailure;
                let result: Result<(), _> = diagnostic.observe(Err(error), |error| {
                    original19_producer_error(error, || {
                        calls.set(calls.get() + 1);
                        match mode {
                            0 => Some(ProducerTransportErrorObservation::CommandBudget(
                                BudgetError::MinimumSpacing,
                            )),
                            1 => None,
                            _ => panic!("R71_SYNTHETIC_PROBE_UNWIND"),
                        }
                    })
                });
                assert_eq!(result, Err(error));
                assert_eq!(calls.get(), 1);
                assert_eq!(diagnostic.snapshot().0.error.transport, if mode == 0 {
                    "command_budget_minimum_spacing"
                } else { "diagnostic_unavailable" });
            }
            // Real per-instance probes: observing one empty slot never reads
            // or changes the other. Private poison is exercised by injection
            // above; this test makes no claim to poison P's private mutex.
            let first = ProducerTransportErrorProbe::default();
            let second = ProducerTransportErrorProbe::default();
            let calls = Cell::new(0);
            let first_class = original19_producer_error(
                &MarketStreamProducerError::TransportFailure,
                || { calls.set(calls.get() + 1); first.take() },
            );
            assert_eq!(first_class.transport, "diagnostic_unavailable");
            assert_eq!(calls.get(), 1);
            assert_eq!(second.take(), None);
        }

        #[test]
        fn every_producer_variant_preserves_error_without_unneeded_probe_read() {
            use MarketStreamProducerError as E;
            for (error, expected) in [
                (E::Storage(MarketStreamStorageError::CommitUnknown), "storage"),
                (E::InvalidInput, "invalid_input"),
                (E::NotReady, "not_ready"),
                (E::Terminal, "terminal"),
                (E::SubscriptionOutOfSync, "subscription_out_of_sync"),
                (E::SessionUnavailable, "session_unavailable"),
            ] {
                let diagnostic = Original19Diagnostic::new();
                diagnostic.enter(Original19Site::Start);
                let result: Result<(), _> = diagnostic.observe(Err(error), |error| {
                    original19_producer_error(error, || panic!("non-transport must not read"))
                });
                assert_eq!(result, Err(error));
                assert_eq!(diagnostic.snapshot().0.error.producer, expected);
                assert_eq!(diagnostic.snapshot().0.error.transport, "none");
            }
        }

        #[test]
        fn storage_variants_remain_exact_through_box_and_producer() {
            use MarketStreamStorageError as S;
            for (error, expected) in [
                (S::InvalidInput, "MARKET_STREAM_INPUT_INVALID"),
                (S::LeaseNotFound, "MARKET_STREAM_LEASE_NOT_FOUND"),
                (S::LeaseReleased, "MARKET_STREAM_LEASE_RELEASED"),
                (S::LeaseExpired, "MARKET_STREAM_LEASE_EXPIRED"),
                (S::LeaseSessionMismatch, "MARKET_STREAM_LEASE_SESSION_MISMATCH"),
                (S::IdempotencyMismatch, "MARKET_STREAM_IDEMPOTENCY_MISMATCH"),
                (S::SequenceConflict, "MARKET_STREAM_SEQUENCE_CONFLICT"),
                (S::LeaseCapacity, "MARKET_STREAM_LEASE_CAPACITY"),
                (S::IdentityCapacity, "MARKET_STREAM_IDENTITY_CAPACITY"),
                (S::MembershipNotReady, "MARKET_STREAM_MEMBERSHIP_NOT_READY"),
                (S::RightsInvalid, "MARKET_STREAM_RIGHTS_INVALID"),
                (S::SessionInvalid, "MARKET_STREAM_SESSION_INVALID"),
                (S::ProducerHeld, "MARKET_STREAM_PRODUCER_HELD"),
                (S::ProducerLost, "MARKET_STREAM_PRODUCER_LOST"),
                (S::FenceExhausted, "MARKET_STREAM_FENCE_EXHAUSTED"),
                (S::EpochInvalid, "MARKET_STREAM_EPOCH_INVALID"),
                (S::SubscriptionInvalid, "MARKET_STREAM_SUBSCRIPTION_INVALID"),
                (S::SubscriptionNotAcked, "MARKET_STREAM_SUBSCRIPTION_NOT_ACKED"),
                (S::ReceiptInvalid, "MARKET_STREAM_RECEIPT_INVALID"),
                (S::ReceiptStale, "MARKET_STREAM_RECEIPT_STALE"),
                (S::PipelineLag, "MARKET_STREAM_PIPELINE_LAG"),
                (S::CacheNotFound, "MARKET_STREAM_CACHE_NOT_FOUND"),
                (S::VersionExhausted, "MARKET_STREAM_VERSION_EXHAUSTED"),
                (S::StatusInvalid, "MARKET_STREAM_STATUS_INVALID"),
                (S::DatabaseUnavailable, "MARKET_STREAM_DATABASE_UNAVAILABLE"),
                (S::DatabaseIntegrity, "MARKET_STREAM_DATABASE_INTEGRITY"),
                (S::PermissionDenied, "MARKET_STREAM_PERMISSION_DENIED"),
                (S::CommitUnknown, "MARKET_STREAM_COMMIT_OUTCOME_UNKNOWN"),
            ] {
                let boxed: CaseError = Box::new(error);
                let pointer = boxed.as_ref() as *const (dyn Error + Send + Sync);
                let diagnostic = Original19Diagnostic::new();
                diagnostic.enter(Original19Site::PrimaryLease);
                let result: Result<(), CaseError> = diagnostic.observe(Err(boxed), |error| {
                    original19_fixture_error(error.as_ref())
                });
                let same = result.err().expect("unchanged boxed error");
                assert!(std::ptr::eq(pointer, same.as_ref() as *const (dyn Error + Send + Sync)));
                assert_eq!(diagnostic.snapshot().0.error.storage, expected);
                assert_eq!(diagnostic.snapshot().0.error.kind, "storage");
                let producer = original19_producer_error(
                    &MarketStreamProducerError::Storage(error),
                    || panic!("storage must not read probe"),
                );
                assert_eq!(producer.storage, expected);
                assert_eq!(producer.producer, "storage");
                diagnostic.freeze(Original19Terminal::ReturnedError);
                assert_grammar(&render(&diagnostic));
            }
        }

        #[test]
        fn transport_nested_variants_render_closed_and_suppress_payload() {
            use ProducerTransportErrorObservation as O;
            // Exact finite rows are independently reviewable against P/C2;
            // every row runs through the observer, probe and output boundary.
            let cases = vec![
                (O::Approval(ApprovalError::EndpointNotAllowed), "approval_endpoint_not_allowed"),
                (O::Approval(ApprovalError::HttpRejected { status: 59999 }), "approval_http_rejected"),
                (O::Approval(ApprovalError::ResponseInvalid), "approval_response_invalid"),
                (O::Approval(ApprovalError::OutcomeAmbiguous), "approval_outcome_ambiguous"),
                (O::Approval(ApprovalError::AttemptTooSoon), "approval_attempt_too_soon"),
                (O::Approval(ApprovalError::AttemptBudgetExhausted), "approval_attempt_budget_exhausted"),
                (O::Approval(ApprovalError::CredentialGenerationMismatch), "approval_credential_generation_mismatch"),
                (O::Approval(ApprovalError::Expired), "approval_expired"),
                (O::Approval(ApprovalError::TransportAmbiguous), "approval_transport_ambiguous"),
                (O::Approval(ApprovalError::State), "approval_state"),
                (O::EndpointNotAllowed, "endpoint_not_allowed"),
                (O::Io, "io"),
                (O::Handshake, "handshake"),
                (O::FrameInvalid, "frame_invalid"),
                (O::Protocol, "protocol"),
                (O::Closed, "closed"),
                (O::CommandPending, "command_pending"),
                (O::CommandCapabilityMismatch, "command_capability_mismatch"),
                (O::CommandTimingInvalid, "command_timing_invalid"),
                (O::CommandTimingOverflow, "command_timing_overflow"),
                (O::AckTimeout, "ack_timeout"),
                (O::ControlWriteTimeout, "control_write_timeout"),
                (O::AckMismatch, "ack_mismatch"),
                (O::DuplicateAck, "duplicate_ack"),
                (O::CommandRejected, "command_rejected"),
                (O::DataBeforeAck, "data_before_ack"),
                (O::SymbolNotAllowed, "symbol_not_allowed"),
                (O::ResubscribeRequiresNewEpoch, "resubscribe_requires_new_epoch"),
                (O::SubscriptionLimit, "subscription_limit"),
                (O::CommandBudget(BudgetError::MinimumSpacing), "command_budget_minimum_spacing"),
                (O::CommandBudget(BudgetError::RollingWindow), "command_budget_rolling_window"),
                (O::CommandBudget(BudgetError::DailyLimit), "command_budget_daily_limit"),
                (O::ReconnectBudget(BudgetError::MinimumSpacing), "reconnect_budget_minimum_spacing"),
                (O::ReconnectBudget(BudgetError::RollingWindow), "reconnect_budget_rolling_window"),
                (O::ReconnectBudget(BudgetError::DailyLimit), "reconnect_budget_daily_limit"),
                (O::ReconnectExhausted, "reconnect_exhausted"),
                (O::Wire(WireError::MessageTooLarge), "wire_message_too_large"),
                (O::Wire(WireError::InvalidUtf8), "wire_invalid_utf8"),
                (O::Wire(WireError::ControlCharacter), "wire_control_character"),
                (O::Wire(WireError::FramingInvalid), "wire_framing_invalid"),
                (O::Wire(WireError::RecordCountInvalid), "wire_record_count_invalid"),
                (O::Wire(WireError::RecordCountExceeded), "wire_record_count_exceeded"),
                (O::Wire(WireError::FieldCountMismatch), "wire_field_count_mismatch"),
                (O::Wire(WireError::FieldTooLarge), "wire_field_too_large"),
                (O::Wire(WireError::SchemaMismatch), "wire_schema_mismatch"),
                (O::Wire(WireError::SymbolInvalid), "wire_symbol_invalid"),
                (O::Wire(WireError::DateInvalid), "wire_date_invalid"),
                (O::Wire(WireError::TimeInvalid), "wire_time_invalid"),
                (O::Wire(WireError::NumericInvalid), "wire_numeric_invalid"),
                (O::Wire(WireError::DirectionInvalid), "wire_direction_invalid"),
                (O::Wire(WireError::DirectionContradiction), "wire_direction_contradiction"),
                (O::Wire(WireError::VolumeInvalid), "wire_volume_invalid"),
                (O::Wire(WireError::HaltFlagInvalid), "wire_halt_flag_invalid"),
                (O::Wire(WireError::MarketClassUnsupported), "wire_market_class_unsupported"),
                (O::Wire(WireError::FutureObservation), "wire_future_observation"),
                (O::Wire(WireError::OutsideSessionWindow), "wire_outside_session_window"),
                (O::Wire(WireError::StaleEpochObservation), "wire_stale_epoch_observation"),
                (O::Wire(WireError::UnsupportedMessage), "wire_unsupported_message"),
                (O::State(StateError::Io), "state_io"),
                (O::State(StateError::UnsafePath), "state_unsafe_path"),
                (O::State(StateError::InvalidState), "state_invalid_state"),
                (O::State(StateError::LockBusy), "state_lock_busy"),
                (O::State(StateError::PriorSessionUncertain), "state_prior_session_uncertain"),
                (O::State(StateError::ReconnectNotReady), "state_reconnect_not_ready"),
                (O::State(StateError::Serialization), "state_serialization"),
                (O::State(StateError::Budget(BudgetError::MinimumSpacing)), "state_budget_minimum_spacing"),
                (O::State(StateError::Budget(BudgetError::RollingWindow)), "state_budget_rolling_window"),
                (O::State(StateError::Budget(BudgetError::DailyLimit)), "state_budget_daily_limit"),
                (O::Cancelled, "cancelled"),
                (O::SessionProofRequired, "session_proof_required"),
                (O::SessionProofInvalid, "session_proof_invalid"),
            ];
            assert!(original19_grammar_value_is_valid("transport", "wire_invalid_utf8"));
            for value in [
                "wire_invalid_utf7",
                "wire_invalid_utf80",
                "wire_invalid_utf8_suffix",
                "wire_invalid_utf8 ",
                "wire_invalid_utf8\n",
                "",
                "arbitrary2",
            ] {
                assert!(!original19_grammar_value_is_valid("transport", value));
            }
            for field in [
                "site", "iteration", "step", "terminal", "error", "producer",
                "storage", "sqlx", "control", "diagnostic",
            ] {
                assert!(!original19_grammar_value_is_valid(field, "wire_invalid_utf8"));
                assert!(!original19_grammar_value_is_valid(field, "value2"));
            }
            for (observation, expected) in cases {
                let diagnostic = Original19Diagnostic::new();
                diagnostic.enter(Original19Site::InitialApply);
                let calls = Cell::new(0);
                let result: Result<(), _> = diagnostic.observe(
                    Err(MarketStreamProducerError::TransportFailure),
                    |error| original19_producer_error(error, || {
                        calls.set(calls.get() + 1);
                        Some(observation)
                    }),
                );
                assert_eq!(result, Err(MarketStreamProducerError::TransportFailure));
                assert_eq!(calls.get(), 1);
                diagnostic.freeze(Original19Terminal::ReturnedError);
                assert_eq!(diagnostic.snapshot().0.error.transport, expected);
                let line = render(&diagnostic);
                assert_grammar(&line);
                assert!(line.contains("error=producer producer=transport_failure storage=none "));
                assert!(line.contains(&format!("transport={expected} sqlx=none")));
                assert!(!line.contains("59999")); // artificial HttpRejected payload
                assert!(!line.contains("R71_SYNTHETIC_SECRET"));
            }
        }

        #[test]
        fn fixture_sqlx_and_unknown_payloads_are_structural_and_site_bound() {
            const SENTINEL: &str = "R71_SYNTHETIC_SECRET";
            let cases: Vec<(CaseError, &str, &str)> = vec![
                (Box::new(sqlx::Error::Protocol(SENTINEL.into())), "sqlx", "protocol"),
                (Box::new(sqlx::Error::InvalidArgument(SENTINEL.into())), "sqlx", "invalid_argument"),
                (Box::new(sqlx::Error::ColumnNotFound(SENTINEL.into())), "sqlx", "column_not_found"),
                (Box::new(sqlx::Error::RowNotFound), "sqlx", "row_not_found"),
                (Box::new(sqlx::Error::PoolTimedOut), "sqlx", "pool_timed_out"),
                (Box::new(sqlx::Error::PoolClosed), "sqlx", "pool_closed"),
                (Box::new(sqlx::Error::Io(io::Error::other(SENTINEL))), "sqlx", "io"),
                (Box::new(sqlx::Error::Decode(Box::new(io::Error::other(SENTINEL)))), "sqlx", "decode"),
                (SENTINEL.into(), "fixture_returned_error", "none"),
            ];
            for (error, expected_kind, expected_sqlx) in cases {
                let diagnostic = Original19Diagnostic::new();
                diagnostic.enter(Original19Site::LoopbackSetup);
                let pointer = error.as_ref() as *const (dyn Error + Send + Sync);
                let result: Result<(), CaseError> = diagnostic.observe(Err(error), |error| {
                    original19_fixture_error(error.as_ref())
                });
                let returned = result.err().expect("fixture error unchanged");
                assert!(std::ptr::eq(pointer, returned.as_ref() as *const (dyn Error + Send + Sync)));
                diagnostic.freeze(Original19Terminal::ReturnedError);
                let record = diagnostic.snapshot().0;
                assert_eq!(record.site, Original19Site::LoopbackSetup);
                assert_eq!(record.error.kind, expected_kind);
                assert_eq!(record.error.sqlx, expected_sqlx);
                let output = render(&diagnostic);
                assert_grammar(&output);
                assert!(!output.contains(SENTINEL));
            }
        }

        #[tokio::test(flavor = "current_thread")]
        async fn terminal_mapping_uses_actual_join_results_and_timeout_races() {
            use Original19Joined as J;
            use Original19Terminal as T;
            for (joined, ordinary, timed_out) in [
                (J::BodyOk, T::ReturnedOk, T::TimeoutJoinedOk),
                (J::BodyError, T::ReturnedError, T::TimeoutJoinedBodyError),
                (J::Panic, T::JoinPanic, T::TimeoutJoinedPanic),
                (J::Cancelled, T::JoinCancelled, T::TimeoutJoinedCancelled),
                (J::Other, T::JoinOther, T::TimeoutJoinedOther),
            ] {
                assert_eq!(original19_terminal(false, joined), ordinary);
                assert_eq!(original19_terminal(true, joined), timed_out);
            }
            let ok = tokio::spawn(async { Ok::<(), ()>(()) }).await;
            let error = tokio::spawn(async { Err::<(), ()>(()) }).await;
            let panic: Result<Result<(), ()>, _> = tokio::spawn(async {
                panic!("R71_SYNTHETIC_BODY_UNWIND");
            }).await;
            let pending = tokio::spawn(std::future::pending::<Result<(), ()>>());
            pending.abort();
            let cancelled = pending.await;
            assert_eq!(original19_joined(&ok), J::BodyOk);
            assert_eq!(original19_joined(&error), J::BodyError);
            assert_eq!(original19_joined(&panic), J::Panic);
            assert_eq!(original19_joined(&cancelled), J::Cancelled);
            // Tokio offers no public construction of an "other" JoinError;
            // the closed fallback uses these exact flags in the real adapter.
            assert_eq!(original19_join_error(false, false), J::Other);
            assert_eq!(original19_join_error(true, true), J::Panic);
            assert_eq!(original19_terminal(true, original19_joined(&ok)), T::TimeoutJoinedOk);
            assert_eq!(original19_terminal(true, original19_joined(&error)), T::TimeoutJoinedBodyError);
        }

        #[test]
        fn late_updates_and_poison_never_reopen_terminal_or_skip_cleanup() {
            let diagnostic = failed_record();
            let frozen = diagnostic.snapshot();
            diagnostic.enter(Original19Site::TransportFinish);
            diagnostic.iteration(Original19Iteration::Three);
            let _: Result<(), ()> = diagnostic.observe(Ok(()), |_| unreachable!());
            diagnostic.complete();
            diagnostic.freeze(Original19Terminal::ReturnedOk);
            assert_eq!(diagnostic.snapshot(), frozen);
            let state = diagnostic.state.clone();
            assert!(catch_unwind(move || {
                let _guard = state.lock().expect("fresh synthetic diagnostic state");
                panic!("R71_SYNTHETIC_STATE_POISON");
            }).is_err());
            diagnostic.enter(Original19Site::Seed);
            diagnostic.freeze(Original19Terminal::JoinPanic);
            let (record, availability) = diagnostic.snapshot();
            assert_eq!(record, frozen.0);
            assert_eq!(availability, Original19Availability::DiagnosticUnavailable);
            let cleanup = Cell::new(0);
            let mut output = Vec::new();
            let (write, ()) = original19_emit_then_cleanup(
                Some(&diagnostic),
                |diagnostic| diagnostic.emit_to(&mut output),
                || cleanup.set(cleanup.get() + 1),
            );
            assert_eq!(write, Some(Original19WriteOutcome::Written));
            assert_eq!(cleanup.get(), 1);
            let line = String::from_utf8(output).expect("closed ASCII");
            assert_grammar(&line);
            assert!(line.contains("terminal=returned_error"));
            assert!(line.ends_with("diagnostic=diagnostic_unavailable\n"));
        }

        #[test]
        fn semantic_controls_missing_quote_and_iterations_keep_fixed_grammar() {
            use MarketStreamControlOutcome as C;
            use MarketStreamTransportStatus as S;
            for (control, label) in [
                (C::ApplicationHeartbeat, "application_heartbeat"),
                (C::ControlPong, "control_pong"),
                (C::NoCurrentAdmission, "no_current_admission"),
                (C::TransportStatus(S::Connected), "status_connected"),
                (C::TransportStatus(S::Closed), "status_closed"),
                (C::TransportStatus(S::Gap), "status_gap"),
                (C::TransportStatus(S::DataBeforeAck), "status_data_before_ack"),
                (C::TransportStatus(S::DuplicateObservation), "status_duplicate_observation"),
                (C::TransportStatus(S::StaleObservation), "status_stale_observation"),
            ] {
                let diagnostic = Original19Diagnostic::new();
                diagnostic.iteration(Original19Iteration::Two);
                diagnostic.require_published(&MarketStreamProducerOutcome::Control(control));
                diagnostic.freeze(Original19Terminal::ReturnedError);
                let record = diagnostic.snapshot().0;
                assert_eq!(record.site, Original19Site::RequirePublished);
                assert_eq!(record.step, Original19Step::SemanticNonPublished);
                assert_eq!(record.control, label);
                assert_eq!(record.iteration, Original19Iteration::Two);
                assert_grammar(&render(&diagnostic));
            }
            for iteration in [Original19Iteration::One, Original19Iteration::Two, Original19Iteration::Three] {
                let diagnostic = Original19Diagnostic::new();
                diagnostic.iteration(iteration);
                let missing: Result<(), &str> = original19_quote!(diagnostic, Err("R71_SYNTHETIC_SECRET"));
                assert_eq!(missing, Err("R71_SYNTHETIC_SECRET"));
                diagnostic.freeze(Original19Terminal::ReturnedError);
                let record = diagnostic.snapshot().0;
                assert_eq!(record.site, Original19Site::RequireQuote);
                assert_eq!(record.step, Original19Step::SemanticMissingQuote);
                assert_eq!(record.iteration, iteration);
                assert!(!render(&diagnostic).contains("R71_SYNTHETIC_SECRET"));
            }
        }

        #[test]
        fn macro_records_entered_before_operation_and_success_only_after_ok() {
            let diagnostic = Original19Diagnostic::new();
            let value: Result<u8, ()> = original19_observe!(
                diagnostic, InitialDemand,
                {
                    let record = diagnostic.snapshot().0;
                    assert_eq!(record.site, Original19Site::InitialDemand);
                    assert_eq!(record.step, Original19Step::Entered);
                    Ok(17)
                },
                |_| unreachable!("success must not classify an error")
            );
            assert_eq!(value, Ok(17));
            assert_eq!(diagnostic.snapshot().0.step, Original19Step::ReturnedOk);
            let quote: Result<u8, ()> = original19_quote!(diagnostic, Ok(23));
            assert_eq!(quote, Ok(23));
            assert_eq!(diagnostic.snapshot().0.step, Original19Step::ReturnedOk);
        }


        #[test]
        fn only_original19_source_body_opts_in() {
            let source = include_str!("market_stream_producer_tests.rs");
            let call = concat!("let receipt = ", "run_original19_database_case(");
            assert_eq!(source.matches(call).count(), 1);
            let name = concat!("async fn producer_real_role_", "publishes_and_keeps_positive_count_ack()");
            let start = source.find(name).expect("original19 source definition");
            let next = source[start..].find("\n    #[tokio::test").expect("next existing case");
            assert!(source[start..start + next].contains(call));
        }

        #[test]
        fn exact_companion_and_legacy_lines_remain_separate() {
            assert_eq!(
                render(&failed_record()),
                "C3B_ORIGINAL19_DIAGNOSTIC_V1 site=initial_apply iteration=none step=returned_error terminal=returned_error error=producer producer=transport_failure storage=none transport=command_budget_minimum_spacing sqlx=none control=none diagnostic=available\n"
            );
            let legacy = ClosedCaseDiagnostic::new();
            let mut bytes = Vec::new();
            assert_eq!(legacy.emit_to(&mut bytes), ClosedDiagnosticWriteOutcome::Written);
            assert_eq!(
                bytes,
                b"C3B_CLOSED_DIAGNOSTIC stage=not_started gate=not_reached result=unknown termination=running observation=pending producer=pending\n"
            );
            let calls = Cell::new(0);
            let (write, ()) = original19_emit_then_cleanup(
                None,
                |_| { calls.set(calls.get() + 1); Ok(()) },
                || (),
            );
            assert_eq!(write, None);
            assert_eq!(calls.get(), 0);
        }
    }

    struct CleanupReceipt {
        body_ok: bool,
        body_panicked: bool,
        cleanup_ok: bool,
        absent: bool,
        connections: i64,
    }

    async fn run_database_case<F>(body: F) -> Result<CleanupReceipt, CaseError>
    where
        F: FnOnce(boundary::DisposableDatabase) -> CaseFuture + Send + 'static,
    {
        run_database_case_inner(body, None, None, None).await
    }

    async fn run_database_case_with_diagnostic<F>(body: F) -> Result<CleanupReceipt, CaseError>
    where
        F: FnOnce(boundary::DisposableDatabase, ClosedCaseDiagnostic) -> CaseFuture
            + Send
            + 'static,
    {
        let diagnostic = ClosedCaseDiagnostic::new();
        let body_diagnostic = diagnostic.clone();
        run_database_case_inner(
            move |database| body(database, body_diagnostic),
            Some(diagnostic),
            None,
            None,
        )
        .await
    }

    async fn run_original20_database_case<F>(body: F) -> Result<CleanupReceipt, CaseError>
    where
        F: FnOnce(boundary::DisposableDatabase, Original20Diagnostic) -> CaseFuture
            + Send
            + 'static,
    {
        let diagnostic = Original20Diagnostic::new();
        let body_diagnostic = diagnostic.clone();
        run_database_case_inner(
            move |database| body(database, body_diagnostic),
            None,
            None,
            Some(diagnostic),
        ).await
    }

    async fn run_database_case_inner<F>(
        body: F,
        diagnostic: Option<ClosedCaseDiagnostic>,
        original19: Option<Original19Diagnostic>,
        original20: Option<Original20Diagnostic>,
    ) -> Result<CleanupReceipt, CaseError>
    where
        F: FnOnce(boundary::DisposableDatabase) -> CaseFuture + Send + 'static,
    {
        if let Some(companion) = &original20 { companion.enter(Original20Site::CreateDatabase); }
        if let Some(companion) = &original19 { companion.enter(Original19Site::CreateDatabase); }
        let database = match boundary::DisposableDatabase::create().await {
            Ok(database) => {
                if let Some(companion) = &original19 {
                    let _: Result<(), ()> = companion.observe(Ok(()), |_| unreachable!());
                }
                database
            },
            Err(error) => {
                if let Some(diagnostic) = &diagnostic {
                    diagnostic.record_setup_failure();
                    let _ = diagnostic.emit();
                }
                if let Some(companion) = &original19 {
                    let _: Result<(), &CaseError> = companion.observe(Err(&error), |error| original19_fixture_error(error.as_ref()));
                    companion.freeze(Original19Terminal::ReturnedError);
                    let _ = original19_emit_then_cleanup(Some(companion), Original19Diagnostic::emit, || ());
                }
                if let Some(companion) = &original20 {
                    companion.freeze(Original19Terminal::ReturnedError);
                    let _ = companion.emit();
                }
                return Err(error.into());
            }
        };
        if let Some(companion) = &original20 { companion.enter(Original20Site::DatabaseName); }
        if let Some(companion) = &original19 { companion.enter(Original19Site::DatabaseName); }
        let database_name: String = match sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&database.migration_owner)
            .await
        {
            Ok(database_name) => {
                if let Some(companion) = &original19 {
                    let _: Result<(), ()> = companion.observe(Ok(()), |_| unreachable!());
                }
                database_name
            },
            Err(error) => {
                if let Some(diagnostic) = &diagnostic {
                    diagnostic.record_setup_failure();
                    let _ = diagnostic.emit();
                }
                if let Some(companion) = &original19 {
                    let _: Result<(), &sqlx::Error> = companion.observe(Err(&error), |error| original19_fixture_error(*error));
                    companion.freeze(Original19Terminal::ReturnedError);
                    let _ = original19_emit_then_cleanup(Some(companion), Original19Diagnostic::emit, || ());
                }
                if let Some(companion) = &original20 {
                    companion.freeze(Original19Terminal::ReturnedError);
                    let _ = companion.emit();
                }
                return Err(error.into());
            }
        };
        let test_database = database.clone();
        let fixture_owner = support::FixtureOwner::new();
        let mut task = tokio::spawn(support::with_fixture_owner(
            fixture_owner.clone(),
            body(test_database),
        ));
        let (body_ok, body_panicked) =
            match tokio::time::timeout(Duration::from_secs(120), &mut task).await {
                Ok(Ok(Ok(()))) => {
                    if let Some(companion) = &original20 {
                        companion.freeze(original19_terminal(false, Original19Joined::BodyOk));
                    }
                    if let Some(companion) = &original19 {
                        companion.freeze(original19_terminal(false, Original19Joined::BodyOk));
                    }
                    if let Some(diagnostic) = &diagnostic {
                        diagnostic.record_body_success();
                    }
                    (true, false)
                }
                Ok(Ok(Err(_))) => {
                    if let Some(companion) = &original20 {
                        companion.freeze(original19_terminal(false, Original19Joined::BodyError));
                    }
                    if let Some(companion) = &original19 {
                        companion.freeze(original19_terminal(false, Original19Joined::BodyError));
                    }
                    if let Some(diagnostic) = &diagnostic {
                        diagnostic.record_body_error();
                    }
                    (false, false)
                }
                Ok(Err(error)) => {
                    if let Some(companion) = &original20 {
                        companion.freeze(original19_terminal(false, original19_join_error(error.is_panic(), error.is_cancelled())));
                    }
                    if let Some(companion) = &original19 {
                        companion.freeze(original19_terminal(false, original19_join_error(error.is_panic(), error.is_cancelled())));
                    }
                    if let Some(diagnostic) = &diagnostic {
                        diagnostic.record_join_error(error.is_panic(), error.is_cancelled());
                    }
                    (false, error.is_panic())
                }
                Err(_) => {
                    task.abort();
                    let joined = task.await;
                    if let Some(companion) = &original20 {
                        companion.freeze(original19_terminal(true, original19_joined(&joined)));
                    }
                    if let Some(companion) = &original19 {
                        companion.freeze(original19_terminal(true, original19_joined(&joined)));
                    }
                    if let Some(diagnostic) = &diagnostic {
                        diagnostic.record_body_timeout();
                    }
                    (false, joined.is_err_and(|error| error.is_panic()))
                }
            };
        if let Some(companion) = &original20 { let _ = companion.emit(); }
        let (_companion_write, cleanup_future) = original19_emit_then_cleanup(
            original19.as_ref(),
            Original19Diagnostic::emit,
            || emit_diagnostic_then_cleanup(
                diagnostic.as_ref(),
                ClosedCaseDiagnostic::emit,
                || async {
                    let fixture_cleanup = fixture_owner.cleanup().await;
                    let cleanup_result = database.cleanup().await;
                    let catalog_result = catalog_state(&database_name).await;
                    (fixture_cleanup, cleanup_result, catalog_result)
                },
            ),
        );
        let (_diagnostic_write, (fixture_cleanup, cleanup_result, catalog_result)) =
            cleanup_future.await;
        let (absent, connections) = catalog_result?;
        let cleanup_ok =
            fixture_cleanup.is_ok() && cleanup_result.is_ok() && absent && connections == 0;
        eprintln!(
            "C3B_DB_CLEANUP database={database_name} body_ok={body_ok} body_panicked={body_panicked} fixture_cleanup_ok={} cleanup_ok={cleanup_ok} present={} connections={connections}",
            fixture_cleanup.is_ok(),
            !absent,
        );
        Ok(CleanupReceipt {
            body_ok,
            body_panicked,
            cleanup_ok,
            absent,
            connections,
        })
    }

    async fn catalog_state(database_name: &str) -> Result<(bool, i64), CaseError> {
        if !database_name.starts_with("lagrange_ws3a_")
            || !database_name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("generated database name is outside this test scope".into());
        }
        let url = std::env::var(boundary::SUPERVISOR_ENV)?;
        let supervisor = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&url)
            .await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1)",
        )
        .bind(database_name)
        .fetch_one(&supervisor)
        .await?;
        let connections: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity WHERE datname = $1",
        )
        .bind(database_name)
        .fetch_one(&supervisor)
        .await?;
        supervisor.close().await;
        Ok((!exists, connections))
    }

    fn assert_clean(receipt: CleanupReceipt) {
        assert!(
            receipt.cleanup_ok,
            "exact generated database cleanup failed"
        );
        assert!(receipt.absent, "exact generated database remains");
        assert_eq!(
            receipt.connections, 0,
            "generated database connections remain"
        );
    }

    fn assert_case_ok(receipt: CleanupReceipt) {
        assert!(receipt.body_ok, "actual-role C3B body did not complete");
        assert!(!receipt.body_panicked, "actual-role C3B body panicked");
        assert_clean(receipt);
    }

    #[cfg(test)]
    mod closed_failure_diagnostic_unit_tests {
        use super::*;

        struct FailingWriter {
            phase: Arc<std::sync::atomic::AtomicUsize>,
        }

        impl Write for FailingWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                let _ = self.phase.compare_exchange(
                    0,
                    1,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                );
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "closed diagnostic writer control",
                ))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        #[tokio::test(flavor = "current_thread")]
        async fn failing_writer_does_not_prevent_shared_cleanup_sequence() {
            let diagnostic = ClosedCaseDiagnostic::new();
            diagnostic.enter(ClosedDiagnosticStage::Setup);
            diagnostic.record_body_error();
            let phase = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut writer = FailingWriter {
                phase: phase.clone(),
            };
            let cleanup_phase = phase.clone();
            let (write, cleanup_value) = emit_diagnostic_then_cleanup(
                Some(&diagnostic),
                |diagnostic| diagnostic.emit_to(&mut writer),
                || async move {
                    assert_eq!(
                        cleanup_phase.load(std::sync::atomic::Ordering::SeqCst),
                        1,
                        "cleanup must run after the failed diagnostic write"
                    );
                    cleanup_phase.store(2, std::sync::atomic::Ordering::SeqCst);
                    17_u8
                },
            )
            .await;
            assert_eq!(write, Some(ClosedDiagnosticWriteOutcome::WriteFailed));
            assert_eq!(cleanup_value, 17);
            assert_eq!(phase.load(std::sync::atomic::Ordering::SeqCst), 2);
            let snapshot = diagnostic.snapshot();
            assert_eq!(snapshot.result, ClosedDiagnosticResult::SetupFailure);
            assert_eq!(
                snapshot.termination,
                ClosedDiagnosticTermination::ReturnedError
            );
            assert_eq!(snapshot.output, ClosedDiagnosticOutput::WriteFailed);

            let none_cleanup_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let none_cleanup_flag = none_cleanup_ran.clone();
            let (none_write, none_value) = emit_diagnostic_then_cleanup(
                None,
                |_| panic!("None diagnostic must not call its emitter"),
                || async move {
                    none_cleanup_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    23_u8
                },
            )
            .await;
            assert_eq!(none_write, None);
            assert_eq!(none_value, 23);
            assert!(none_cleanup_ran.load(std::sync::atomic::Ordering::SeqCst));
        }

        #[test]
        fn poisoned_state_is_non_panicking_and_closed() {
            let diagnostic = ClosedCaseDiagnostic::new();
            let state = diagnostic.state.clone();
            let poisoned = std::panic::catch_unwind(move || {
                let _guard = state.lock().expect("fresh diagnostic lock");
                panic!("closed diagnostic poison control");
            });
            assert!(poisoned.is_err());

            let mut marker = Vec::new();
            let emitted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                diagnostic.emit_to(&mut marker)
            }));
            assert!(matches!(
                emitted,
                Ok(ClosedDiagnosticWriteOutcome::StateUnavailableWritten)
            ));
            assert_eq!(
                String::from_utf8(marker).expect("fixed marker is UTF-8"),
                "C3B_CLOSED_DIAGNOSTIC stage=not_started gate=not_reached result=diagnostic_state_unavailable termination=state_unavailable observation=pending producer=pending\n"
            );
            diagnostic.enter(ClosedDiagnosticStage::Finalization);
            let snapshot = diagnostic.snapshot();
            assert_eq!(snapshot.stage, ClosedDiagnosticStage::NotStarted);
            assert_eq!(
                snapshot.result,
                ClosedDiagnosticResult::DiagnosticStateUnavailable
            );
            assert_eq!(
                snapshot.termination,
                ClosedDiagnosticTermination::StateUnavailable
            );
            assert_eq!(snapshot.output, ClosedDiagnosticOutput::StateUnavailable);
        }

        #[test]
        fn terminal_timeout_rejects_late_body_updates() {
            let diagnostic = ClosedCaseDiagnostic::new();
            diagnostic.enter(ClosedDiagnosticStage::AwaitingAckPersistence);
            diagnostic.reached_gate();
            diagnostic.enter(ClosedDiagnosticStage::CommitJoin);
            let ready = Arc::new(std::sync::Barrier::new(2));
            let release = Arc::new(std::sync::Barrier::new(2));
            let late = diagnostic.clone();
            let late_ready = ready.clone();
            let late_release = release.clone();
            let writer = std::thread::spawn(move || {
                late_ready.wait();
                late_release.wait();
                late.enter(ClosedDiagnosticStage::Finalization);
                late.result(ClosedDiagnosticResult::Complete);
            });
            ready.wait();
            diagnostic.record_body_timeout();
            release.wait();
            writer.join().expect("late diagnostic writer");

            assert_eq!(
                diagnostic.snapshot(),
                ClosedDiagnosticSnapshot {
                    stage: ClosedDiagnosticStage::CommitJoin,
                    gate_reached: true,
                    result: ClosedDiagnosticResult::InProgress,
                    observation: ClosedRelayObservation::Pending,
                    producer: ClosedProducerResult::Pending,
                    termination: ClosedDiagnosticTermination::TimedOut,
                    output: ClosedDiagnosticOutput::NotAttempted,
                }
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn join_error_classification_is_panic_cancelled_or_unknown() {
            let panic_error = tokio::spawn(async {
                panic!("closed diagnostic panic control");
            })
            .await
            .expect_err("panic task must return JoinError");
            assert!(panic_error.is_panic());
            let panicked = ClosedCaseDiagnostic::new();
            panicked.enter(ClosedDiagnosticStage::PostCommitChecks);
            panicked.record_join_error(panic_error.is_panic(), panic_error.is_cancelled());
            assert_eq!(
                panicked.snapshot().termination,
                ClosedDiagnosticTermination::Panicked
            );

            let cancelled_task = tokio::spawn(std::future::pending::<()>());
            cancelled_task.abort();
            let cancelled_error = cancelled_task
                .await
                .expect_err("aborted task must return JoinError");
            assert!(cancelled_error.is_cancelled());
            let cancelled = ClosedCaseDiagnostic::new();
            cancelled.enter(ClosedDiagnosticStage::CommitJoin);
            cancelled.record_join_error(cancelled_error.is_panic(), cancelled_error.is_cancelled());
            assert_eq!(
                cancelled.snapshot().termination,
                ClosedDiagnosticTermination::Cancelled
            );

            let unknown = ClosedCaseDiagnostic::new();
            unknown.record_join_error(false, false);
            assert_eq!(
                unknown.snapshot().termination,
                ClosedDiagnosticTermination::JoinUnknown
            );

            let returned_error = ClosedCaseDiagnostic::new();
            returned_error.record_body_error();
            let returned_snapshot = returned_error.snapshot();
            assert_eq!(
                returned_snapshot.result,
                ClosedDiagnosticResult::UnclassifiedFailure
            );
            assert_eq!(
                returned_snapshot.termination,
                ClosedDiagnosticTermination::ReturnedError
            );
        }

        #[tokio::test(start_paused = true)]
        async fn commit_join_tracks_observation_and_producer_outcomes_independently() {
            let observed_diagnostic = ClosedCaseDiagnostic::new();
            observed_diagnostic.enter(ClosedDiagnosticStage::CommitJoin);
            let (observed_tx, observed_rx) = tokio::sync::oneshot::channel();
            let mut observed = support::CommitObservation::from_receiver(observed_rx);
            observed_tx
                .send(())
                .expect("observation receiver remains owned");
            let observed_join = track_commit_join(
                async {
                    Err(MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown,
                    ))
                },
                &mut observed,
                &observed_diagnostic,
            )
            .await
            .expect("observed COMMIT and producer result complete");
            assert_eq!(observed_join.1, support::CommitObservationOutcome::Observed);
            assert_eq!(
                observed_diagnostic.snapshot().observation,
                ClosedRelayObservation::Observed
            );
            assert_eq!(
                observed_diagnostic.snapshot().producer,
                ClosedProducerResult::CommitUnknown
            );

            let closed_diagnostic = ClosedCaseDiagnostic::new();
            closed_diagnostic.enter(ClosedDiagnosticStage::CommitJoin);
            let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
            let mut closed = support::CommitObservation::from_receiver(closed_rx);
            drop(closed_tx);
            let closed_join = track_commit_join(
                async {
                    Ok(MarketStreamApplyOutcome {
                        active_symbols: 1,
                        transport_commands: 1,
                    })
                },
                &mut closed,
                &closed_diagnostic,
            )
            .await
            .expect("sender closure and producer result complete");
            assert_eq!(
                closed_join.1,
                support::CommitObservationOutcome::SenderClosed
            );
            assert_eq!(
                closed_diagnostic.snapshot().observation,
                ClosedRelayObservation::SenderClosed
            );
            assert_eq!(
                closed_diagnostic.snapshot().producer,
                ClosedProducerResult::ReturnedOk
            );

            let timeout_diagnostic = ClosedCaseDiagnostic::new();
            timeout_diagnostic.enter(ClosedDiagnosticStage::CommitJoin);
            let (_pending_tx, pending_rx) = tokio::sync::oneshot::channel();
            let mut pending = support::CommitObservation::from_receiver(pending_rx);
            let timed_out = track_commit_join(
                std::future::pending::<Result<MarketStreamApplyOutcome, MarketStreamProducerError>>(
                ),
                &mut pending,
                &timeout_diagnostic,
            )
            .await;
            assert!(timed_out.is_none());
            let timeout_snapshot = timeout_diagnostic.snapshot();
            assert_eq!(
                timeout_snapshot.result,
                ClosedDiagnosticResult::OuterJoinTimeout
            );
            assert_eq!(
                timeout_snapshot.observation,
                ClosedRelayObservation::TimedOut
            );
            assert_eq!(timeout_snapshot.producer, ClosedProducerResult::Pending);

            assert_eq!(
                classify_commit_producer_result(&Err(MarketStreamProducerError::Terminal)),
                ClosedProducerResult::ReturnedOtherError
            );
        }

        #[test]
        fn fixed_marker_and_independent_instances_remain_closed() {
            let first = ClosedCaseDiagnostic::new();
            let first_clone = first.clone();
            let second = ClosedCaseDiagnostic::new();

            first_clone.reached_gate();
            first_clone.enter(ClosedDiagnosticStage::CommitJoin);
            first_clone.result(ClosedDiagnosticResult::OuterJoinTimeout);
            let mut marker = Vec::new();
            assert_eq!(
                first.emit_to(&mut marker),
                ClosedDiagnosticWriteOutcome::Written
            );
            let marker = String::from_utf8(marker).expect("fixed marker is UTF-8");
            assert_eq!(
                marker,
                "C3B_CLOSED_DIAGNOSTIC stage=commit_join gate=reached result=outer_join_timeout termination=running observation=pending producer=pending\n"
            );
            for forbidden in [
                "provider",
                "payload",
                "credential",
                "SELECT",
                "secret-sentinel",
                "Debug",
            ] {
                assert!(!marker.contains(forbidden));
            }
            assert!(marker.is_ascii());

            assert_eq!(first.snapshot(), first_clone.snapshot());
            assert_eq!(first.snapshot().output, ClosedDiagnosticOutput::Written);
            assert_eq!(
                second.snapshot(),
                ClosedDiagnosticSnapshot {
                    stage: ClosedDiagnosticStage::NotStarted,
                    gate_reached: false,
                    result: ClosedDiagnosticResult::Unknown,
                    observation: ClosedRelayObservation::Pending,
                    producer: ClosedProducerResult::Pending,
                    termination: ClosedDiagnosticTermination::Running,
                    output: ClosedDiagnosticOutput::NotAttempted,
                }
            );
        }
    }

    async fn snapshot_subscription(
        database: &boundary::DisposableDatabase,
        owner: Uuid,
        slot: Uuid,
        symbol: &str,
    ) -> Result<(String, i32, Uuid, Option<DateTime<Utc>>, Option<Uuid>), sqlx::Error> {
        snapshot_subscription_optional(database, owner, slot, symbol)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    async fn snapshot_subscription_optional(
        database: &boundary::DisposableDatabase,
        owner: Uuid,
        slot: Uuid,
        symbol: &str,
    ) -> Result<Option<(String, i32, Uuid, Option<DateTime<Utc>>, Option<Uuid>)>, sqlx::Error> {
        let mut tx = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            "SELECT state, desired_reference_count, subscription_revision, acked_at, epoch
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(slot)
        .bind(symbol)
        .fetch_optional(&mut *tx)
        .await?;
        let snapshot = row
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    row.try_get("state")?,
                    row.try_get("desired_reference_count")?,
                    row.try_get("subscription_revision")?,
                    row.try_get("acked_at")?,
                    row.try_get("epoch")?,
                ))
            })
            .transpose()?;
        tx.rollback().await?;
        Ok(snapshot)
    }

    async fn pending_fields(
        database: &boundary::DisposableDatabase,
        owner: Uuid,
        slot: Uuid,
        symbol: &str,
    ) -> Result<
        Option<(
            Option<String>,
            Option<i64>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        )>,
        sqlx::Error,
    > {
        let mut tx = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            "SELECT pending_operation::text AS pending_operation, pending_ordinal,
                    pending_reserved_at, pending_deadline, acked_at
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2",
        )
        .bind(slot)
        .bind(symbol)
        .fetch_optional(&mut *tx)
        .await?;
        let fields = row
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    row.try_get("pending_operation")?,
                    row.try_get("pending_ordinal")?,
                    row.try_get("pending_reserved_at")?,
                    row.try_get("pending_deadline")?,
                    row.try_get("acked_at")?,
                ))
            })
            .transpose()?;
        tx.rollback().await?;
        Ok(fields)
    }

    async fn producer_epoch(
        database: &boundary::DisposableDatabase,
        owner: Uuid,
        slot: Uuid,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let mut tx = database.worker.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        let epoch = sqlx::query_scalar(
            "SELECT current_epoch FROM public.owner_market_stream_producers
              WHERE credential_slot_id = $1 AND owner_user_id = $2",
        )
        .bind(slot)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await?;
        tx.rollback().await?;
        Ok(epoch.flatten())
    }

    async fn make_lease(
        database: &boundary::DisposableDatabase,
        fixture: &boundary::Fixture,
        identities: &[StreamIdentity],
        consumer_id: Uuid,
        sequence: u64,
        key: &str,
    ) -> Result<crate::owner_equity_v2::market_stream::StreamLease, CaseError> {
        let request = StreamLeaseRequest::new(
            consumer_id,
            sequence,
            identities
                .iter()
                .map(|identity| {
                    crate::owner_equity_v2::market_stream::StreamLeaseIdentity::new(
                        identity.membership_id,
                        identity.instrument_id.clone(),
                        identity.generation,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            key.to_owned(),
        )?;
        let app = OwnerMarketStreamRepository::new(database.app.clone());
        Ok(app
            .replace_stream_lease(fixture.owner_user_id, &fixture.owner_session_hash, &request)
            .await?)
    }

    struct ProducerScenario {
        leases: Vec<StreamLease>,
        repository: OwnerMarketStreamRepository,
        producer_lease: StreamProducerLease,
        producer: OwnerMarketStreamProducer,
        transport: LoopbackHarness,
        desired: DesiredSet,
    }

    async fn producer_scenario(
        database: &boundary::DisposableDatabase,
        fixture: &boundary::Fixture,
        repository: OwnerMarketStreamRepository,
        symbols: &[&str],
        plans: Vec<support::CommandPlan>,
        separate_leases: bool,
    ) -> Result<ProducerScenario, CaseError> {
        let identities = symbols
            .iter()
            .map(|symbol| {
                fixture
                    .identities
                    .iter()
                    .find(|identity| identity.symbol() == *symbol)
                    .cloned()
                    .ok_or_else(|| format!("fixture lacks approved symbol {symbol}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut leases = Vec::new();
        if separate_leases {
            for (index, identity) in identities.iter().enumerate() {
                leases.push(
                    make_lease(
                        database,
                        fixture,
                        std::slice::from_ref(identity),
                        Uuid::new_v4(),
                        0,
                        &format!("c3b-split-lease-{index}"),
                    )
                    .await?,
                );
            }
        } else {
            leases.push(
                make_lease(
                    database,
                    fixture,
                    &identities,
                    Uuid::new_v4(),
                    0,
                    "c3b-scenario-lease",
                )
                .await?,
            );
        }
        let producer_lease = repository
            .claim_stream_producer(
                fixture.credential_slot_id,
                Uuid::new_v4(),
                fixture.grant_revision,
            )
            .await?;
        let now_ms = boundary::now(database).await?.timestamp_millis();
        let mut transport = support::loopback_session(
            fixture.session_date,
            fixture.credential_slot_id,
            now_ms,
            plans,
            true,
        )
        .await?;
        let producer = OwnerMarketStreamProducer::start(
            repository.clone(),
            producer_lease.clone(),
            fixture.session.clone(),
            transport.take_session(),
        )
        .await?;
        let desired = repository
            .read_stream_demand(fixture.credential_slot_id)
            .await?;
        Ok(ProducerScenario {
            leases,
            repository,
            producer_lease,
            producer,
            transport,
            desired,
        })
    }

    fn deferred_reconciliation(
        progress: MarketStreamReconcileOutcome,
    ) -> Result<(MarketStreamApplyOutcome, i64), CaseError> {
        match progress {
            MarketStreamReconcileOutcome::Deferred {
                committed,
                not_before_ms,
            } => Ok((committed, not_before_ms)),
            MarketStreamReconcileOutcome::Complete(_) => {
                Err("expected one bounded pacing deferral".into())
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pacing_deferral_preserves_second_subscribe_and_one_continuation_completes() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plans = vec![
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                ];
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["000660", "005930"],
                    plans,
                    false,
                )
                .await?;

                let (committed, not_before_ms) = deferred_reconciliation(
                    scenario
                        .producer
                        .reconcile_desired(&scenario.desired)
                        .await?,
                )?;
                assert_eq!(committed.active_symbols, 1);
                assert_eq!(committed.transport_commands, 1);
                assert_eq!(scenario.transport.next_command().await?.0, "000660");
                let first_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                assert_eq!(first_ack.0, "ACKED");
                assert!(first_ack.3.is_some());
                assert!(snapshot_subscription_optional(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .is_none());
                let state_before_cancelled_wait = scenario.producer.command_state_snapshot()?;
                assert_eq!(state_before_cancelled_wait.command_attempts_ms.len(), 1);
                assert_eq!(state_before_cancelled_wait.next_command_ordinal, 1);
                assert!(state_before_cancelled_wait.pending_command.is_none());

                {
                    let wait = scenario
                        .transport
                        .wait_for_command_eligibility(not_before_ms);
                    tokio::pin!(wait);
                    tokio::select! {
                        biased;
                        result = &mut wait => {
                            return Err(format!("deferred caller wait completed before cancellation: {result:?}").into());
                        }
                        _ = tokio::task::yield_now() => {}
                    }
                }
                assert_eq!(
                    scenario.producer.command_state_snapshot()?,
                    state_before_cancelled_wait,
                    "dropping the caller-owned wait cannot reserve a command",
                );
                assert!(scenario.transport.try_next_command().is_none());
                scenario
                    .transport
                    .wait_for_command_eligibility(not_before_ms)
                    .await?;
                let fresh = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let resumed = scenario
                    .producer
                    .reconcile_desired(&fresh)
                    .await?
                    .into_complete()?;
                assert_eq!(resumed.active_symbols, 2);
                assert_eq!(resumed.transport_commands, 1);
                assert_eq!(scenario.transport.next_command().await?.0, "005930");
                let first_after = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                assert_eq!(first_after, first_ack);
                let second_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(second_ack.0, "ACKED");
                assert!(second_ack.3.is_some());
                let final_state = scenario.producer.command_state_snapshot()?;
                assert_eq!(final_state.command_attempts_ms.len(), 2);
                assert_eq!(final_state.next_command_ordinal, 2);
                assert!(final_state.pending_command.is_none());
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 2);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("two-subscribe pacing deferral case and cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pacing_deferral_preserves_acked_removal_until_fresh_continuation() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plans = vec![
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Unsubscribe,
                        Vec::new(),
                    ),
                ];
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["000660", "005930"],
                    plans,
                    true,
                )
                .await?;

                let (_, subscribe_not_before) = deferred_reconciliation(
                    scenario
                        .producer
                        .reconcile_desired(&scenario.desired)
                        .await?,
                )?;
                assert_eq!(scenario.transport.next_command().await?.0, "000660");
                scenario
                    .transport
                    .wait_for_command_eligibility(subscribe_not_before)
                    .await?;
                let fresh = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let completed = scenario
                    .producer
                    .reconcile_desired(&fresh)
                    .await?
                    .into_complete()?;
                assert_eq!(completed.transport_commands, 1);
                assert_eq!(scenario.transport.next_command().await?.0, "005930");
                let retained_before = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                let removed_before = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(retained_before.0, "ACKED");
                assert_eq!(removed_before.0, "ACKED");
                assert!(removed_before.3.is_some());

                let removed_lease = &scenario.leases[1];
                OwnerMarketStreamRepository::new(database.app.clone())
                    .release_stream_lease(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        removed_lease.lease_id,
                        removed_lease.renewal_sequence,
                    )
                    .await?;
                let current = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                assert_eq!(current.items.len(), 1);
                let state_before = scenario.producer.command_state_snapshot()?;
                let (committed, unsubscribe_not_before) = deferred_reconciliation(
                    scenario.producer.reconcile_desired(&current).await?,
                )?;
                assert_eq!(committed.active_symbols, 2);
                assert_eq!(committed.transport_commands, 0);
                assert_eq!(scenario.producer.command_state_snapshot()?, state_before);
                assert_eq!(
                    snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?,
                    removed_before,
                    "deferred removal must not clear its prior ACK or revision",
                );
                assert_eq!(
                    snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "000660",
                    )
                    .await?,
                    retained_before,
                );
                assert!(scenario.transport.try_next_command().is_none());

                scenario
                    .transport
                    .wait_for_command_eligibility(unsubscribe_not_before)
                    .await?;
                let fresh = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let completed = scenario
                    .producer
                    .reconcile_desired(&fresh)
                    .await?
                    .into_complete()?;
                assert_eq!(completed.active_symbols, 1);
                assert_eq!(completed.transport_commands, 1);
                assert_eq!(
                    scenario.transport.next_command().await?.1,
                    MarketSubscriptionOperation::Unsubscribe,
                );
                let removed_after = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(removed_after.0, "ABSENT");
                assert_eq!(removed_after.1, 0);
                assert!(removed_after.3.is_none());
                assert_ne!(removed_after.2, removed_before.2);
                assert_eq!(
                    snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "000660",
                    )
                    .await?,
                    retained_before,
                );
                let final_state = scenario.producer.command_state_snapshot()?;
                assert_eq!(final_state.command_attempts_ms.len(), 3);
                assert_eq!(final_state.next_command_ordinal, 3);
                assert!(final_state.pending_command.is_none());
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 3);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("ACKed removal pacing deferral case and cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pacing_continuation_cancellation_before_desired_persistence_is_terminal() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plans = vec![
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                ];
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["000660", "005930"],
                    plans,
                    false,
                )
                .await?;
                let (_, not_before_ms) = deferred_reconciliation(
                    scenario
                        .producer
                        .reconcile_desired(&scenario.desired)
                        .await?,
                )?;
                assert_eq!(scenario.transport.next_command().await?.0, "000660");
                let first_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                scenario
                    .transport
                    .wait_for_command_eligibility(not_before_ms)
                    .await?;
                let fresh = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let state_before = scenario.transport.command_state_snapshot()?;
                let gate = ProducerTestGate::new(ProducerAwaitPoint::DesiredPersistence);
                scenario.producer.install_test_gate(gate.clone());
                {
                    let continuation = scenario.producer.reconcile_desired(&fresh);
                    tokio::pin!(continuation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => {}
                        result = &mut continuation => {
                            return Err(format!("continuation escaped desired-persistence gate: {result:?}").into());
                        }
                    }
                }
                assert_eq!(
                    scenario.producer.reconcile_desired(&fresh).await.unwrap_err(),
                    MarketStreamProducerError::Terminal,
                );
                assert_eq!(scenario.transport.command_state_snapshot()?, state_before);
                assert_eq!(
                    snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "000660",
                    )
                    .await?,
                    first_ack,
                );
                assert!(snapshot_subscription_optional(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .is_none());
                assert!(scenario.transport.try_next_command().is_none());
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("cancelled pacing continuation and cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pacing_continuation_lease_loss_fails_before_c1_write_or_wire_send() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plans = vec![
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                ];
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["000660", "005930"],
                    plans,
                    false,
                )
                .await?;
                let (_, not_before_ms) = deferred_reconciliation(
                    scenario
                        .producer
                        .reconcile_desired(&scenario.desired)
                        .await?,
                )?;
                assert_eq!(scenario.transport.next_command().await?.0, "000660");
                let first_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                scenario
                    .transport
                    .wait_for_command_eligibility(not_before_ms)
                    .await?;
                let fresh = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let state_before = scenario.transport.command_state_snapshot()?;
                scenario.producer.make_lease_stale_for_test()?;
                assert_eq!(
                    scenario.producer.reconcile_desired(&fresh).await.unwrap_err(),
                    MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::ProducerLost,
                    ),
                );
                assert_eq!(scenario.transport.command_state_snapshot()?, state_before);
                assert_eq!(
                    snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "000660",
                    )
                    .await?,
                    first_ack,
                );
                assert!(snapshot_subscription_optional(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .is_none());
                assert!(scenario.transport.try_next_command().is_none());
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("stale-lease pacing continuation and cleanup must succeed");
        assert_case_ok(receipt);
    }

    async fn start_task_relay(
        database: &boundary::DisposableDatabase,
    ) -> Result<support::PgRelay, CaseError> {
        let database_name: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&database.migration_owner)
            .await?;
        Ok(support::PgRelay::start(&database_name).await?)
    }

    async fn worker_repository_via_relay(
        database: &boundary::DisposableDatabase,
        relay: &support::PgRelay,
    ) -> Result<OwnerMarketStreamRepository, CaseError> {
        let database_name: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&database.migration_owner)
            .await?;
        Ok(OwnerMarketStreamRepository::new(
            relay.worker_pool(&database_name).await?,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_actual_facade_debug_is_closed_metadata_only() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let holder_id = Uuid::new_v4();
                let producer_lease = repository
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        holder_id,
                        fixture.grant_revision,
                    )
                    .await?;
                assert!(!fixture.owner_user_id.is_nil());
                assert!(!fixture.credential_slot_id.is_nil());
                assert!(!fixture.grant_id.is_nil());
                assert!(!fixture.grant_revision.is_nil());
                assert!(!fixture.session.session_proof_id.is_nil());
                assert!(!fixture.session.calendar_source_batch_id.is_nil());
                assert!(!fixture.session.session_proof_sha256.is_empty());
                assert!(!fixture.session.calendar_content_sha256.is_empty());
                assert!(!fixture.session.window_contract_sha256.is_empty());
                assert!(!holder_id.is_nil());
                assert!(producer_lease.fencing_token > 0);
                assert_eq!(producer_lease.owner_user_id, fixture.owner_user_id);
                assert_eq!(
                    producer_lease.credential_slot_id,
                    fixture.credential_slot_id
                );
                assert_eq!(producer_lease.grant_id, fixture.grant_id);
                assert_eq!(producer_lease.grant_revision, fixture.grant_revision);
                assert_eq!(producer_lease.holder_id, holder_id);

                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    Vec::new(),
                    true,
                )
                .await?;
                let producer = OwnerMarketStreamProducer::start(
                    repository.clone(),
                    producer_lease.clone(),
                    fixture.session.clone(),
                    transport.take_session(),
                )
                .await?;
                let epoch =
                    producer_epoch(&database, fixture.owner_user_id, fixture.credential_slot_id)
                        .await?
                        .ok_or("actual facade start did not persist an epoch")?;
                assert!(!epoch.is_nil());

                let output = format!("{producer:?}");
                let pretty = format!("{producer:#?}");
                assert_eq!(
                    output,
                    "OwnerMarketStreamProducer { phase: Ready, subscription_count: 0 }"
                );
                assert!(pretty.contains("phase: Ready"));
                assert!(pretty.contains("subscription_count: 0"));
                for forbidden in [
                    "credential_slot_id",
                    "fencing_token",
                    "owner_user_id",
                    "holder_id",
                    "grant_id",
                    "grant_revision",
                    "session",
                    "epoch",
                    "proof",
                    "repository",
                    "lease",
                    "subscriptions",
                ] {
                    assert!(
                        !output.contains(forbidden) && !pretty.contains(forbidden),
                        "actual facade Debug exposed forbidden label: {forbidden}"
                    );
                }
                let stored_sentinels = [
                    fixture.owner_user_id.to_string(),
                    fixture.credential_slot_id.to_string(),
                    fixture.grant_id.to_string(),
                    fixture.grant_revision.to_string(),
                    holder_id.to_string(),
                    fixture.session.session_proof_id.to_string(),
                    fixture.session.calendar_source_batch_id.to_string(),
                    fixture.session.session_proof_sha256.clone(),
                    fixture.session.calendar_content_sha256.clone(),
                    fixture.session.window_contract_sha256.clone(),
                    epoch.to_string(),
                    // This facade has zero subscriptions, so its sole allowed
                    // numeric Debug value is 0; the positive fence is a
                    // non-colliding stored numeric sentinel.
                    producer_lease.fencing_token.to_string(),
                ];
                for sentinel in stored_sentinels {
                    assert!(
                        !output.contains(&sentinel) && !pretty.contains(&sentinel),
                        "actual facade Debug exposed a stored sentinel"
                    );
                }

                drop(producer);
                drop(repository);
                let server = transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("actual facade Debug case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_real_role_publishes_and_keeps_positive_count_ack() {
        let receipt = run_original19_database_case(|database, diagnostic| {
            Box::pin(async move {
                let fixture = original19_observe!(diagnostic, Seed, boundary::seed_fixture(&database).await, |error| original19_fixture_error(error.as_ref()))?;
                let identities = fixture.identities[..2].to_vec();
                let lease = original19_observe!(diagnostic, PrimaryLease, make_lease(
                    &database,
                    &fixture,
                    &identities,
                    Uuid::new_v4(),
                    0,
                    "c3b-primary-lease",
                )
                .await, |error| original19_fixture_error(error.as_ref()))?;
                let worker = OwnerMarketStreamRepository::new(database.worker.clone());
                let producer_lease = original19_observe!(diagnostic, Claim, worker
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        Uuid::new_v4(),
                        fixture.grant_revision,
                    )
                    .await, |error| original19_fixture_error(error))?;
                let now_ms = original19_observe!(diagnostic, InitialDbTime, boundary::now(&database).await, |error| original19_fixture_error(error))?.timestamp_millis();
                let mut transport = original19_observe!(diagnostic, LoopbackSetup, support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    vec![
                        support::CommandPlan::new(
                            "000660",
                            MarketSubscriptionOperation::Subscribe,
                            vec![("000660".to_owned(), 1)],
                        ),
                        support::CommandPlan::new(
                            "005930",
                            MarketSubscriptionOperation::Subscribe,
                            vec![("005930".to_owned(), 2), ("000660".to_owned(), 3)],
                        ),
                    ],
                    false,
                )
                .await, |error| original19_fixture_error(error.as_ref()))?;
                let mut producer = original19_observe!(diagnostic, Start, OwnerMarketStreamProducer::start(
                    worker.clone(),
                    producer_lease,
                    fixture.session.clone(),
                    transport.take_session(),
                )
                .await, |error| original19_producer_error(error, || None))?;
                let transport_probe: ProducerTransportErrorProbe = producer.transport_error_probe();
                let demand = original19_observe!(diagnostic, InitialDemand, worker
                    .read_stream_demand(fixture.credential_slot_id)
                    .await, |error| original19_fixture_error(error))?;
                // Preserve the existing received_at >= exact DB sample assertion.
                // Only advance via real elapsed time from the conservative socket floor.
                original19_observe!(diagnostic, InitialApply, transport.wait_for_command_eligibility(now_ms).await, |error| original19_fixture_error(error.as_ref()))?;
                let progress = original19_observe!(diagnostic, InitialApply, producer.reconcile_desired(&demand).await, |error| original19_producer_error(error, || transport_probe.take()))?;
                let applied = match progress {
                    MarketStreamReconcileOutcome::Complete(applied) => applied,
                    MarketStreamReconcileOutcome::Deferred { committed, not_before_ms } => {
                        assert_eq!(committed.active_symbols, 1);
                        assert_eq!(committed.transport_commands, 1);
                        let ack_before = original19_observe!(diagnostic, SnapshotBefore, snapshot_subscription(
                            &database, fixture.owner_user_id, fixture.credential_slot_id, "000660",
                        ).await, |error| original19_fixture_error(error))?;
                        assert_eq!(ack_before.0, "ACKED");
                        assert!(ack_before.3.is_some());
                        original19_observe!(diagnostic, InitialApply, transport.wait_for_command_eligibility(not_before_ms).await, |error| original19_fixture_error(error.as_ref()))?;
                        let fresh = original19_observe!(diagnostic, InitialDemand, worker.read_stream_demand(fixture.credential_slot_id).await, |error| original19_fixture_error(error))?;
                        // Exactly one explicit continuation, no retry loop.
                        let resumed = original19_observe!(diagnostic, InitialApply, producer.reconcile_desired(&fresh).await, |error| original19_producer_error(error, || transport_probe.take()))?;
                        let resumed = original19_observe!(diagnostic, InitialApply, resumed.into_complete(), |error| original19_producer_error(error, || transport_probe.take()))?;
                        assert_eq!(resumed.transport_commands, 1);
                        let ack_after = original19_observe!(diagnostic, SnapshotBefore, snapshot_subscription(
                            &database, fixture.owner_user_id, fixture.credential_slot_id, "000660",
                        ).await, |error| original19_fixture_error(error))?;
                        assert_eq!(ack_after, ack_before, "continuation must preserve the first ACK");
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed.transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("two bounded command counts"),
                        }
                    }
                };
                assert_eq!(applied.active_symbols, 2);
                assert_eq!(applied.transport_commands, 2);
                assert_eq!(original19_observe!(diagnostic, FirstCommandReceipt, transport.next_command().await, |error| original19_fixture_error(error.as_ref()))?.0, "000660");
                assert_eq!(original19_observe!(diagnostic, SecondCommandReceipt, transport.next_command().await, |error| original19_fixture_error(error.as_ref()))?.0, "005930");

                let before = original19_observe!(diagnostic, SnapshotBefore, snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await, |error| original19_fixture_error(error))?;
                assert_eq!(before.0, "ACKED");
                assert_eq!(before.1, 1);
                assert!(before.3.is_some());

                let extra_lease = original19_observe!(diagnostic, ExtraLease, make_lease(
                    &database,
                    &fixture,
                    &[identities
                        .iter()
                        .find(|identity| identity.symbol() == "000660")
                        .unwrap()
                        .clone()],
                    Uuid::new_v4(),
                    0,
                    "c3b-extra-lease",
                )
                .await, |error| original19_fixture_error(error.as_ref()))?;
                let twice = original19_observe!(diagnostic, DemandTwice, worker
                    .read_stream_demand(fixture.credential_slot_id)
                    .await, |error| original19_fixture_error(error))?;
                assert_eq!(
                    twice
                        .items
                        .iter()
                        .find(|item| item.identity.symbol() == "000660")
                        .unwrap()
                        .reference_count,
                    2
                );
                assert_eq!(original19_observe!(diagnostic, ApplyTwice, producer.apply_desired(&twice).await, |error| original19_producer_error(error, || transport_probe.take()))?.transport_commands, 0);
                let after_two = original19_observe!(diagnostic, SnapshotTwice, snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await, |error| original19_fixture_error(error))?;
                assert_eq!(after_two.0, before.0);
                assert_eq!(after_two.1, 2);
                assert_eq!(after_two.2, before.2);
                assert_eq!(after_two.3, before.3);
                assert_eq!(after_two.4, before.4);
                assert_eq!(
                    original19_observe!(diagnostic, ApplyIdentical, producer.apply_desired(&twice).await, |error| original19_producer_error(error, || transport_probe.take()))?.transport_commands,
                    0,
                    "reapplying the identical positive count must reserve no command"
                );
                let after_identical = original19_observe!(diagnostic, SnapshotIdentical, snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await, |error| original19_fixture_error(error))?;
                assert_eq!(after_identical, after_two);

                let app = OwnerMarketStreamRepository::new(database.app.clone());
                original19_observe!(diagnostic, ReleaseExtra, app.release_stream_lease(
                    fixture.owner_user_id,
                    &fixture.owner_session_hash,
                    extra_lease.lease_id,
                    extra_lease.renewal_sequence,
                )
                .await, |error| original19_fixture_error(error))?;
                let once_more = original19_observe!(diagnostic, DemandOnce, worker
                    .read_stream_demand(fixture.credential_slot_id)
                    .await, |error| original19_fixture_error(error))?;
                assert_eq!(
                    original19_observe!(diagnostic, ApplyOnce, producer.apply_desired(&once_more).await, |error| original19_producer_error(error, || transport_probe.take()))?.transport_commands,
                    0
                );
                let after_one = original19_observe!(diagnostic, SnapshotOnce, snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await, |error| original19_fixture_error(error))?;
                assert_eq!(after_one.1, 1);
                assert_eq!(after_one.2, before.2);
                assert_eq!(after_one.3, before.3);
                assert_eq!(after_one.4, before.4);
                assert!(transport.try_next_command().is_none());

                let admissions = demand
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                let publish_now_ms = original19_observe!(diagnostic, PublicationDbTime, boundary::now(&database).await, |error| original19_fixture_error(error))?.timestamp_millis();
                let mut committed_symbols = Vec::new();
                let mut published_rows = Vec::new();
                for iteration in [Original19Iteration::One, Original19Iteration::Two, Original19Iteration::Three] {
                    diagnostic.iteration(iteration);
                    let outcome = original19_observe!(diagnostic, ReadQuote, producer.read_and_publish(&admissions).await, |error| original19_producer_error(error, || transport_probe.take()))?;
                    diagnostic.require_published(&outcome);
                    let MarketStreamProducerOutcome::Published(result) = outcome else {
                        return Err("expected committed quote publication".into());
                    };
                    assert!(result.changed);
                    assert_eq!(result.rows.len(), 1);
                    let row = &result.rows[0];
                    let quote = original19_quote!(diagnostic, row.quote.as_ref().ok_or("committed row has no quote"))?;
                    assert_eq!(quote.base_price, None);
                    assert_eq!(quote.halted, false);
                    assert_eq!(row.epoch, after_one.4);
                    assert!(row.epoch.is_some_and(|epoch| !epoch.is_nil()));
                    assert!(row.receive_ordinal.is_some_and(|ordinal| ordinal > 0));
                    let received_at_ms = row.received_at.unwrap().timestamp_millis();
                    assert!(received_at_ms >= now_ms);
                    assert!(publish_now_ms - received_at_ms <= 3_000);
                    committed_symbols.push(row.identity.symbol().to_owned());
                    published_rows.push(row.clone());
                }
                diagnostic.iteration(Original19Iteration::None);
                committed_symbols.sort();
                assert_eq!(committed_symbols, vec!["000660", "000660", "005930"]);
                assert!(
                    published_rows[1].receive_ordinal.unwrap()
                        < published_rows[2].receive_ordinal.unwrap(),
                    "both records from the packed WebSocket data frame retain ordered ordinals"
                );
                assert_eq!(
                    published_rows[1].received_at, published_rows[2].received_at,
                    "packed records retain their single immutable frame timestamp"
                );
                let snapshot = original19_observe!(diagnostic, AppSnapshot, app
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        lease.lease_id,
                    )
                    .await, |error| original19_fixture_error(error))?;
                assert_eq!(snapshot.rows.len(), 2);
                assert!(snapshot.rows.iter().all(|row| row.quote.is_some()));
                assert_eq!(
                    snapshot
                        .rows
                        .iter()
                        .find(|row| row.identity.symbol() == "000660")
                        .unwrap()
                        .quote
                        .as_ref()
                        .unwrap()
                        .price,
                    // The public snapshot reads numeric(20,8) through price::text,
                    // so the persisted storage scale is part of this String contract.
                    "70003.00000000"
                );
                drop(producer);
                let server = original19_observe!(diagnostic, TransportFinish, transport.finish().await, |error| original19_fixture_error(error.as_ref()))?;
                diagnostic.enter(Original19Site::FinalAssertions);
                assert_eq!(server.commands.len(), 2);
                assert_eq!(server.market_record_batches, vec![1, 2]);
                assert!(server.client_closed);
                diagnostic.complete();
                Ok(())
            })
        })
        .await
        .expect("actual-role producer publication case and cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn demand_read_cancellation_owns_session_before_actual_pool_wait() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let identity = fixture
                    .identities
                    .iter()
                    .find(|identity| identity.symbol() == "005930")
                    .cloned()
                    .ok_or("fixture lacks demand-read identity")?;
                let lease = make_lease(
                    &database,
                    &fixture,
                    std::slice::from_ref(&identity),
                    Uuid::new_v4(),
                    0,
                    "c3b-demand-read-cancel",
                )
                .await?;
                let worker_pool = support::single_connection_worker_pool(&database).await?;
                let repository = OwnerMarketStreamRepository::new(worker_pool.clone());
                let producer_lease = repository
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        Uuid::new_v4(),
                        fixture.grant_revision,
                    )
                    .await?;
                let desired = OwnerMarketStreamRepository::new(database.worker.clone())
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    vec![support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    )],
                    true,
                )
                .await?;
                let mut producer = OwnerMarketStreamProducer::start(
                    repository.clone(),
                    producer_lease,
                    fixture.session.clone(),
                    transport.take_session(),
                )
                .await?;
                let demand_gate = ProducerTestGate::new(ProducerAwaitPoint::DemandRead);
                producer.install_test_gate(demand_gate.clone());
                let held_connection = worker_pool.acquire().await?;
                {
                    let operation = producer.apply_desired(&desired);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = demand_gate.wait_until_reached() => {},
                        result = &mut operation => panic!("demand read did not reach its private boundary: {result:?}"),
                    }
                    demand_gate.resume();
                    assert!(
                        tokio::time::timeout(Duration::from_millis(250), &mut operation)
                            .await
                            .is_err(),
                        "demand read completed while the sole worker connection was held"
                    );
                }
                drop(held_connection);
                worker_pool.close().await;
                assert_eq!(
                    producer.apply_desired(&desired).await.unwrap_err(),
                    MarketStreamProducerError::Terminal,
                    "cancelling the actual demand-read wait must be terminal"
                );
                assert!(transport.try_next_command().is_none());
                drop(producer);
                drop(repository);
                let server = transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                drop(lease);
                Ok(())
            })
        })
        .await
        .expect("demand-read cancellation case and exact cleanup must compile as a real-role case");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resumed_pending_and_ack_db_phases_finish_before_wire_and_publication() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let (first_plan, second_plan) = (
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    ),
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        vec![("000660".to_owned(), 11)],
                    ),
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![first_plan, second_plan],
                    false,
                )
                .await?;
                let pending_gate = ProducerTestGate::new(ProducerAwaitPoint::PendingPersistence);
                scenario.producer.install_test_gate(pending_gate.clone());
                {
                    let first_operation = scenario.producer.apply_desired(&scenario.desired);
                    tokio::pin!(first_operation);
                    tokio::select! {
                        _ = pending_gate.wait_until_reached() => {},
                        result = &mut first_operation => panic!("pending phase did not reach its gate: {result:?}"),
                    }
                    let before_pending = snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?;
                    assert_eq!(before_pending.0, "DESIRED");
                    assert!(scenario.transport.try_next_command().is_none());
                    pending_gate.resume();
                    assert_eq!(first_operation.await?.transport_commands, 1);
                }
                assert_eq!(scenario.transport.next_command().await?.0, "005930");

                let second_identity = fixture
                    .identities
                    .iter()
                    .find(|identity| identity.symbol() == "000660")
                    .cloned()
                    .ok_or("fixture lacks second success identity")?;
                let _second_lease = make_lease(
                    &database,
                    &fixture,
                    std::slice::from_ref(&second_identity),
                    Uuid::new_v4(),
                    0,
                    "c3b-success-second-lease",
                )
                .await?;
                let second_desired = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                let ack_gate = ProducerTestGate::new(ProducerAwaitPoint::AckPersistence);
                scenario.producer.install_test_gate(ack_gate.clone());
                let (gate_seen, progress) = {
                    let operation = scenario.producer.reconcile_desired(&second_desired);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = ack_gate.wait_until_reached() => {
                            assert_eq!(scenario.transport.next_command().await?.0, "000660");
                            let before_ack = snapshot_subscription(
                                &database,
                                fixture.owner_user_id,
                                fixture.credential_slot_id,
                                "000660",
                            )
                            .await?;
                            assert_eq!(before_ack.0, "PENDING_SUBSCRIBE");
                            assert!(before_ack.3.is_none());
                            ack_gate.resume();
                            (true, operation.await?)
                        }
                        result = &mut operation => (false, result?),
                    }
                };
                let applied = match (gate_seen, progress) {
                    (true, MarketStreamReconcileOutcome::Complete(applied)) => {
                        assert_eq!(applied.active_symbols, 2);
                        assert_eq!(applied.transport_commands, 1);
                        applied
                    }
                    (true, MarketStreamReconcileOutcome::Deferred { .. }) => {
                        return Err("ACK gate was reached but second reconciliation deferred".into());
                    }
                    (false, MarketStreamReconcileOutcome::Complete(_)) => {
                        return Err("second reconciliation completed before ACK gate".into());
                    }
                    (
                        false,
                        MarketStreamReconcileOutcome::Deferred {
                            committed,
                            not_before_ms,
                        },
                    ) => {
                        assert_eq!(committed.active_symbols, 1);
                        assert_eq!(committed.transport_commands, 0);
                        scenario
                            .transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = {
                            let continuation =
                                scenario.producer.reconcile_desired(&fresh);
                            tokio::pin!(continuation);
                            tokio::select! {
                                _ = ack_gate.wait_until_reached() => {
                                    assert_eq!(scenario.transport.next_command().await?.0, "000660");
                                    let before_ack = snapshot_subscription(
                                        &database,
                                        fixture.owner_user_id,
                                        fixture.credential_slot_id,
                                        "000660",
                                    )
                                    .await?;
                                    assert_eq!(before_ack.0, "PENDING_SUBSCRIBE");
                                    assert!(before_ack.3.is_none());
                                    ack_gate.resume();
                                    continuation.await?
                                }
                                result = &mut continuation => {
                                    return Err(format!(
                                        "continuation returned before ACK gate: {result:?}"
                                    ).into());
                                }
                            }
                        };
                        let resumed = match resumed {
                            MarketStreamReconcileOutcome::Complete(applied) => applied,
                            MarketStreamReconcileOutcome::Deferred { .. } => {
                                return Err("continuation deferred twice".into());
                            }
                        };
                        assert_eq!(resumed.active_symbols, 2);
                        assert_eq!(resumed.transport_commands, 1);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("one bounded resumed command"),
                        }
                    }
                };
                assert_eq!(applied.transport_commands, 1);
                let after_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                assert_eq!(after_ack.0, "ACKED");
                assert!(after_ack.3.is_some());
                let published = scenario
                    .producer
                    .read_and_publish(&[second_identity])
                    .await?;
                let MarketStreamProducerOutcome::Published(committed) = published else {
                    return Err("buffered quote did not publish after ACK commit".into());
                };
                assert_eq!(committed.rows.len(), 1);
                assert_eq!(committed.rows[0].identity.symbol(), "000660");
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 2);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("resumed DB-phase controls and exact cleanup must compile as a real-role case");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn same_epoch_resubscribe_is_terminal_and_new_epoch_requires_fresh_ack() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plans = vec![
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        vec![("000660".to_owned(), 1)],
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        vec![("005930".to_owned(), 2)],
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Unsubscribe,
                        vec![("005930".to_owned(), 3), ("000660".to_owned(), 4)],
                    ),
                ];
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["000660", "005930"],
                    plans,
                    true,
                )
                .await?;
                let applied = match scenario
                    .producer
                    .reconcile_desired(&scenario.desired)
                    .await?
                {
                    MarketStreamReconcileOutcome::Complete(applied) => applied,
                    MarketStreamReconcileOutcome::Deferred {
                        committed,
                        not_before_ms,
                    } => {
                        assert_eq!(committed.active_symbols, 1);
                        assert_eq!(committed.transport_commands, 1);
                        let first_ack = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_ack.0, "ACKED");
                        assert!(first_ack.3.is_some());
                        assert!(snapshot_subscription_optional(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "005930",
                        )
                        .await?
                        .is_none());
                        scenario
                            .transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = scenario
                            .producer
                            .reconcile_desired(&fresh)
                            .await?
                            .into_complete()?;
                        assert_eq!(resumed.active_symbols, 2);
                        assert_eq!(resumed.transport_commands, 1);
                        let first_after = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_after, first_ack);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("two bounded command counts"),
                        }
                    }
                };
                assert_eq!(applied.active_symbols, 2);
                assert_eq!(applied.transport_commands, 2);
                assert_eq!(scenario.transport.next_command().await?.0, "000660");
                assert_eq!(scenario.transport.next_command().await?.0, "005930");
                let old_epoch =
                    producer_epoch(&database, fixture.owner_user_id, fixture.credential_slot_id)
                        .await?
                        .ok_or("producer epoch missing")?;
                let other_before = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                let target_before = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(other_before.0, "ACKED");
                assert_eq!(other_before.1, 1);
                assert!(other_before.3.is_some());
                assert_eq!(other_before.4, Some(old_epoch));
                assert_eq!(target_before.0, "ACKED");
                assert_eq!(target_before.1, 1);
                assert!(target_before.3.is_some());
                assert_eq!(target_before.4, Some(old_epoch));

                let initial_admissions = scenario
                    .desired
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                let mut initial_symbols = Vec::new();
                for _ in 0..2 {
                    let MarketStreamProducerOutcome::Published(result) = scenario
                        .producer
                        .read_and_publish(&initial_admissions)
                        .await?
                    else {
                        return Err("initial ACKed data returned a control outcome".into());
                    };
                    assert_eq!(result.rows.len(), 1);
                    initial_symbols.push(result.rows[0].identity.symbol().to_owned());
                }
                assert_eq!(initial_symbols, vec!["000660", "005930"]);
                let target_initial_snapshot =
                    OwnerMarketStreamRepository::new(database.app.clone())
                        .read_stream_snapshot(
                            fixture.owner_user_id,
                            &fixture.owner_session_hash,
                            scenario.leases[1].lease_id,
                        )
                        .await?;
                assert_eq!(target_initial_snapshot.rows.len(), 1);
                let initial_target_row = target_initial_snapshot.rows[0].clone();
                assert_eq!(initial_target_row.identity, scenario.leases[1].identities[0]);
                assert_eq!(
                    initial_target_row
                        .quote
                        .as_ref()
                        .ok_or("initial target quote missing")?
                        .price,
                    "70002.00000000"
                );
                assert_eq!(initial_target_row.epoch, Some(old_epoch));
                assert!(initial_target_row.quote_version > 0);
                assert!(initial_target_row.state_version > 0);
                assert!(initial_target_row
                    .receive_ordinal
                    .is_some_and(|ordinal| ordinal > 0));
                assert!(initial_target_row.received_at.is_some());
                assert!(initial_target_row.committed_at.is_some());
                let assert_retained_target =
                    |row: &crate::owner_equity_v2::market_stream::StreamCacheRow| {
                        assert_eq!(row, &initial_target_row);
                        assert_eq!(
                            row.quote
                                .as_ref()
                                .expect("retained target quote missing")
                                .price,
                            "70002.00000000"
                        );
                        assert_ne!(
                            row.quote
                                .as_ref()
                                .expect("retained target quote missing")
                                .price,
                            "70003.00000000"
                        );
                        assert_eq!(row.epoch, Some(old_epoch));
                        assert_eq!(row.quote_version, initial_target_row.quote_version);
                        assert_eq!(row.state_version, initial_target_row.state_version);
                        assert_eq!(row.receive_ordinal, initial_target_row.receive_ordinal);
                        assert_eq!(row.received_at, initial_target_row.received_at);
                        assert_eq!(row.committed_at, initial_target_row.committed_at);
                    };

                let removed_lease = &scenario.leases[1];
                OwnerMarketStreamRepository::new(database.app.clone())
                    .release_stream_lease(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        removed_lease.lease_id,
                        removed_lease.renewal_sequence,
                    )
                    .await?;
                let current = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                assert_eq!(current.items.len(), 1);
                assert_eq!(current.items[0].identity.symbol(), "000660");
                let applied = match scenario.producer.reconcile_desired(&current).await? {
                    MarketStreamReconcileOutcome::Complete(applied) => applied,
                    MarketStreamReconcileOutcome::Deferred {
                        committed,
                        not_before_ms,
                    } => {
                        assert_eq!(committed.active_symbols, 2);
                        assert_eq!(committed.transport_commands, 0);
                        scenario
                            .transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = scenario
                            .producer
                            .reconcile_desired(&fresh)
                            .await?
                            .into_complete()?;
                        assert_eq!(resumed.active_symbols, 1);
                        assert_eq!(resumed.transport_commands, 1);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("one bounded unsubscribe command"),
                        }
                    }
                };
                assert_eq!(applied.active_symbols, 1);
                assert_eq!(applied.transport_commands, 1);
                assert_eq!(
                    scenario.transport.next_command().await?.1,
                    MarketSubscriptionOperation::Unsubscribe
                );
                let absent = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(absent.0, "ABSENT");
                assert_eq!(absent.1, 0);
                assert!(absent.3.is_none());
                assert_eq!(absent.4, Some(old_epoch));
                assert_ne!(absent.2, target_before.2);
                let other_after_zero = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                assert_eq!(other_after_zero.0, "ACKED");
                assert_eq!(other_after_zero.1, 1);
                assert_eq!(other_after_zero.2, other_before.2);
                assert_eq!(other_after_zero.3, other_before.3);
                assert_eq!(other_after_zero.4, Some(old_epoch));

                let current_admissions = current
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                let mut unrelated_after_zero = None;
                for _ in 0..8 {
                    match scenario.producer.read_and_publish(&current_admissions).await? {
                        MarketStreamProducerOutcome::Published(result) => {
                            assert_eq!(result.rows.len(), 1);
                            assert_eq!(result.rows[0].identity.symbol(), "000660");
                            unrelated_after_zero = Some(
                                result.rows[0]
                                    .quote
                                    .as_ref()
                                    .ok_or("unrelated quote missing after unsubscribe")?
                                    .price
                                    .clone(),
                            );
                            break;
                        }
                        MarketStreamProducerOutcome::Control(_) => {}
                    }
                }
                assert_eq!(unrelated_after_zero.as_deref(), Some("70004.00000000"));

                let target_identity = scenario.leases[1].identities[0].clone();
                let restored_lease = make_lease(
                    &database,
                    &fixture,
                    std::slice::from_ref(&target_identity),
                    Uuid::new_v4(),
                    0,
                    "c3b-same-epoch-restored",
                )
                .await?;
                let restored_demand = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                assert_eq!(restored_demand.items.len(), 2);
                let restored_snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        restored_lease.lease_id,
                    )
                    .await?;
                assert_eq!(restored_snapshot.rows.len(), 1);
                // A restored lease exposes authorized last-known data; it does
                // not make the old quote current subscription admission.
                assert_retained_target(&restored_snapshot.rows[0]);

                assert_eq!(
                    scenario
                        .producer
                        .apply_desired(&restored_demand)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::TransportFailure
                );
                assert_eq!(
                    scenario
                        .producer
                        .apply_desired(&restored_demand)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                assert_eq!(
                    scenario
                        .producer
                        .read_and_publish(&initial_admissions)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                assert!(scenario.transport.try_next_command().is_none());
                assert_eq!(
                    producer_epoch(&database, fixture.owner_user_id, fixture.credential_slot_id)
                        .await?,
                    Some(old_epoch)
                );
                let same_epoch_desired = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(same_epoch_desired, absent);
                let still_suppressed = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        restored_lease.lease_id,
                    )
                    .await?;
                assert_eq!(still_suppressed.rows.len(), 1);
                // Same-epoch rejection sent no target frame: packed sequence 3
                // remains suppressed and cannot advance the retained row.
                assert_retained_target(&still_suppressed.rows[0]);

                drop(scenario.producer);
                let old_server = scenario.transport.finish().await?;
                assert_eq!(
                    old_server.commands,
                    vec![
                        (
                            "000660".to_owned(),
                            MarketSubscriptionOperation::Subscribe,
                        ),
                        (
                            "005930".to_owned(),
                            MarketSubscriptionOperation::Subscribe,
                        ),
                        (
                            "005930".to_owned(),
                            MarketSubscriptionOperation::Unsubscribe,
                        ),
                    ]
                );
                assert!(old_server.client_closed);

                let renewed_producer_lease = scenario
                    .repository
                    .renew_stream_producer(&scenario.producer_lease)
                    .await?;
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut next_transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    vec![
                        support::CommandPlan::new(
                            "000660",
                            MarketSubscriptionOperation::Subscribe,
                            Vec::new(),
                        ),
                        support::CommandPlan::new(
                            "005930",
                            MarketSubscriptionOperation::Subscribe,
                            Vec::new(),
                        ),
                    ],
                    true,
                )
                .await?;
                let mut next_producer = OwnerMarketStreamProducer::start(
                    scenario.repository.clone(),
                    renewed_producer_lease,
                    fixture.session.clone(),
                    next_transport.take_session(),
                )
                .await?;
                let new_epoch =
                    producer_epoch(&database, fixture.owner_user_id, fixture.credential_slot_id)
                        .await?
                        .ok_or("new producer epoch missing")?;
                assert_ne!(new_epoch, old_epoch);
                let target_before_new_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(target_before_new_ack.0, "ABSENT");
                assert_eq!(target_before_new_ack.1, 0);
                assert!(target_before_new_ack.3.is_none());
                assert_eq!(target_before_new_ack.4, Some(new_epoch));
                assert_ne!(target_before_new_ack.2, same_epoch_desired.2);
                let other_before_new_ack = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                assert_eq!(other_before_new_ack.0, "DESIRED");
                assert!(other_before_new_ack.3.is_none());
                assert_eq!(other_before_new_ack.4, Some(new_epoch));
                assert_ne!(other_before_new_ack.2, other_before.2);

                let reset_snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        restored_lease.lease_id,
                    )
                    .await?;
                assert_eq!(reset_snapshot.rows.len(), 1);
                assert_retained_target(&reset_snapshot.rows[0]);
                assert_ne!(reset_snapshot.rows[0].epoch, Some(new_epoch));

                let ack_gate = ProducerTestGate::new(ProducerAwaitPoint::AckPersistence);
                next_producer.install_test_gate(ack_gate.clone());
                let mut pending_target_revision = None;
                let (second_gate_seen, progress) = {
                    let operation = next_producer.reconcile_desired(&restored_demand);
                    tokio::pin!(operation);
                    let first_progress = tokio::time::timeout(Duration::from_secs(8), async {
                        tokio::select! {
                            _ = ack_gate.wait_until_reached() => {
                                assert_eq!(next_transport.next_command().await?.0, "000660");
                                let target_while_other_pending = snapshot_subscription(
                                    &database,
                                    fixture.owner_user_id,
                                    fixture.credential_slot_id,
                                    "005930",
                                )
                                .await?;
                                assert_eq!(target_while_other_pending, target_before_new_ack);
                                ack_gate.resume();
                                Ok::<Option<MarketStreamReconcileOutcome>, CaseError>(None)
                            }
                            result = &mut operation => {
                                Ok::<Option<MarketStreamReconcileOutcome>, CaseError>(Some(result?))
                            }
                        }
                    })
                    .await
                    .map_err(|_| "unrelated fresh ACK commit gate timed out")??;
                    if let Some(progress) = first_progress {
                        return Err(format!(
                            "fresh epoch reconciliation returned before first ACK gate: {progress:?}"
                        )
                        .into());
                    }
                    tokio::time::timeout(Duration::from_secs(8), async {
                        tokio::select! {
                            _ = ack_gate.wait_until_reached() => {
                                assert_eq!(next_transport.next_command().await?.0, "005930");
                                let pending_target = snapshot_subscription(
                                    &database,
                                    fixture.owner_user_id,
                                    fixture.credential_slot_id,
                                    "005930",
                                )
                                .await?;
                                assert_eq!(pending_target.0, "PENDING_SUBSCRIBE");
                                assert_eq!(pending_target.1, 1);
                                assert!(pending_target.3.is_none());
                                assert_eq!(pending_target.4, Some(new_epoch));
                                assert_ne!(pending_target.2, target_before_new_ack.2);
                                pending_target_revision = Some(pending_target.2);
                                ack_gate.resume();
                                Ok::<_, CaseError>((true, operation.await?))
                            }
                            result = &mut operation => Ok::<_, CaseError>((false, result?)),
                        }
                    })
                    .await
                    .map_err(|_| "target fresh ACK commit gate timed out")??
                };
                let applied = match (second_gate_seen, progress) {
                    (true, MarketStreamReconcileOutcome::Complete(applied)) => {
                        assert_eq!(applied.active_symbols, 2);
                        assert_eq!(applied.transport_commands, 2);
                        applied
                    }
                    (true, MarketStreamReconcileOutcome::Deferred { .. }) => {
                        return Err("second ACK gate was reached but reconciliation deferred".into());
                    }
                    (false, MarketStreamReconcileOutcome::Complete(_)) => {
                        return Err("fresh target reconciliation completed before second ACK gate".into());
                    }
                    (
                        false,
                        MarketStreamReconcileOutcome::Deferred {
                            committed,
                            not_before_ms,
                        },
                    ) => {
                        assert_eq!(committed.active_symbols, 1);
                        assert_eq!(committed.transport_commands, 1);
                        let first_ack = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_ack.0, "ACKED");
                        assert!(first_ack.3.is_some());
                        let target_before_wait = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "005930",
                        )
                        .await?;
                        assert_eq!(target_before_wait, target_before_new_ack);
                        next_transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = tokio::time::timeout(Duration::from_secs(8), async {
                            let continuation = next_producer.reconcile_desired(&fresh);
                            tokio::pin!(continuation);
                            tokio::select! {
                                _ = ack_gate.wait_until_reached() => {
                                    assert_eq!(next_transport.next_command().await?.0, "005930");
                                    let pending_target = snapshot_subscription(
                                        &database,
                                        fixture.owner_user_id,
                                        fixture.credential_slot_id,
                                        "005930",
                                    )
                                    .await?;
                                    assert_eq!(pending_target.0, "PENDING_SUBSCRIBE");
                                    assert_eq!(pending_target.1, 1);
                                    assert!(pending_target.3.is_none());
                                    assert_eq!(pending_target.4, Some(new_epoch));
                                    assert_ne!(pending_target.2, target_before_new_ack.2);
                                    pending_target_revision = Some(pending_target.2);
                                    ack_gate.resume();
                                    let progress = continuation.await?;
                                    match progress {
                                        MarketStreamReconcileOutcome::Complete(applied) => {
                                            Ok::<MarketStreamApplyOutcome, CaseError>(applied)
                                        }
                                        MarketStreamReconcileOutcome::Deferred { .. } => {
                                            Err("fresh target continuation deferred twice".into())
                                        }
                                    }
                                }
                                result = &mut continuation => {
                                    let progress = result?;
                                    Err(format!(
                                        "fresh target continuation returned before ACK gate: {progress:?}"
                                    )
                                    .into())
                                }
                            }
                        })
                        .await
                        .map_err(|_| "fresh target ACK continuation timed out")??;
                        assert_eq!(resumed.active_symbols, 2);
                        assert_eq!(resumed.transport_commands, 1);
                        let first_after = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_after, first_ack);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("two bounded fresh-epoch commands"),
                        }
                    }
                };
                assert_eq!(applied.transport_commands, 2);
                let reacked = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(reacked.0, "ACKED");
                assert_eq!(reacked.1, 1);
                assert!(reacked.3.is_some());
                assert_eq!(reacked.4, Some(new_epoch));
                let pending_target_revision = pending_target_revision
                    .ok_or("fresh target pending revision was not captured")?;
                assert_ne!(reacked.2, pending_target_revision);
                assert_ne!(reacked.2, target_before_new_ack.2);
                assert_ne!(reacked.2, target_before.2);
                let reacked_other = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "000660",
                )
                .await?;
                assert_eq!(reacked_other.0, "ACKED");
                assert_eq!(reacked_other.1, 1);
                assert!(reacked_other.3.is_some());
                assert_eq!(reacked_other.4, Some(new_epoch));
                assert_ne!(reacked_other.2, other_before_new_ack.2);
                let final_snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        restored_lease.lease_id,
                    )
                    .await?;
                assert_eq!(final_snapshot.rows.len(), 1);
                // Fresh subscription ACK establishes admission in new_epoch,
                // but without a fresh trade the cache stays old-epoch last-known.
                assert_retained_target(&final_snapshot.rows[0]);
                assert_ne!(final_snapshot.rows[0].epoch, Some(new_epoch));
                drop(next_producer);
                let new_server = next_transport.finish().await?;
                assert_eq!(
                    new_server.commands,
                    vec![
                        (
                            "000660".to_owned(),
                            MarketSubscriptionOperation::Subscribe,
                        ),
                        (
                            "005930".to_owned(),
                            MarketSubscriptionOperation::Subscribe,
                        ),
                    ]
                );
                assert!(new_server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("canonical same-epoch rejection and fresh-epoch ACK case must complete cleanup");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_unsubscribe_suppresses_target_and_retains_other_acked_wire_order() {
        let receipt = run_original20_database_case(|database, diagnostic| {
            Box::pin(async move {
                diagnostic.enter(Original20Site::SeedFixture);
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plans = vec![
                    support::CommandPlan::new(
                        "000660",
                        MarketSubscriptionOperation::Subscribe,
                        vec![("000660".to_owned(), 1)],
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        vec![("005930".to_owned(), 2)],
                    ),
                    support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Unsubscribe,
                        vec![("005930".to_owned(), 3), ("000660".to_owned(), 4)],
                    ),
                ];
                diagnostic.enter(Original20Site::Scenario);
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["000660", "005930"],
                    plans,
                    true,
                )
                .await?;
                diagnostic.enter(Original20Site::InitialApply);
                let applied = match scenario
                    .producer
                    .reconcile_desired(&scenario.desired)
                    .await?
                {
                    MarketStreamReconcileOutcome::Complete(applied) => applied,
                    MarketStreamReconcileOutcome::Deferred {
                        committed,
                        not_before_ms,
                    } => {
                        assert_eq!(committed.active_symbols, 1);
                        assert_eq!(committed.transport_commands, 1);
                        let first_ack = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_ack.0, "ACKED");
                        assert!(first_ack.3.is_some());
                        scenario
                            .transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = scenario
                            .producer
                            .reconcile_desired(&fresh)
                            .await?
                            .into_complete()?;
                        assert_eq!(resumed.active_symbols, 2);
                        assert_eq!(resumed.transport_commands, 1);
                        let first_after = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_after, first_ack);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("two bounded command counts"),
                        }
                    }
                };
                assert_eq!(applied.active_symbols, 2);
                assert_eq!(applied.transport_commands, 2);
                diagnostic.enter(Original20Site::InitialFirstCommand);
                assert_eq!(scenario.transport.next_command().await?.0, "000660");
                diagnostic.enter(Original20Site::InitialSecondCommand);
                assert_eq!(scenario.transport.next_command().await?.0, "005930");

                let target_identity = scenario
                    .desired
                    .items
                    .iter()
                    .find(|item| item.identity.symbol() == "005930")
                    .map(|item| item.identity.clone())
                    .ok_or("scenario lacks 005930 target identity")?;
                // Fresh ACKs retain subscription proof but do not seed cache; seed via the real status path before release.
                let seeded = scenario
                    .producer
                    .record_awaiting_first_trade_for_test(&target_identity)
                    .await?;
                assert!(seeded.changed);
                assert_eq!(seeded.rows.len(), 1);
                assert_eq!(seeded.rows[0].identity, target_identity);
                assert!(seeded.rows[0].quote.is_none());
                let target_snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        scenario.leases[1].lease_id,
                    )
                    .await?;
                assert_eq!(target_snapshot.rows.len(), 1);
                assert_eq!(target_snapshot.rows[0].identity, target_identity);
                assert!(target_snapshot.rows[0].quote.is_none());

                diagnostic.enter(Original20Site::RemovedLease);
                let removed_lease = &scenario.leases[1];
                diagnostic.enter(Original20Site::ReleaseStreamLease);
                OwnerMarketStreamRepository::new(database.app.clone())
                    .release_stream_lease(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        removed_lease.lease_id,
                        removed_lease.renewal_sequence,
                    )
                    .await?;
                diagnostic.enter(Original20Site::DemandAfterRelease);
                let current = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                assert_eq!(current.items.len(), 1);
                assert_eq!(current.items[0].identity.symbol(), "000660");
                diagnostic.enter(Original20Site::UnsubscribeApply);
                let applied = match scenario.producer.reconcile_desired(&current).await? {
                    MarketStreamReconcileOutcome::Complete(applied) => applied,
                    MarketStreamReconcileOutcome::Deferred {
                        committed,
                        not_before_ms,
                    } => {
                        assert_eq!(committed.active_symbols, 2);
                        assert_eq!(committed.transport_commands, 0);
                        scenario
                            .transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = scenario
                            .producer
                            .reconcile_desired(&fresh)
                            .await?
                            .into_complete()?;
                        assert_eq!(resumed.active_symbols, 1);
                        assert_eq!(resumed.transport_commands, 1);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("one bounded unsubscribe command"),
                        }
                    }
                };
                assert_eq!(applied.active_symbols, 1);
                assert_eq!(applied.transport_commands, 1);
                diagnostic.enter(Original20Site::UnsubscribeCommand);
                assert_eq!(
                    scenario.transport.next_command().await?.1,
                    MarketSubscriptionOperation::Unsubscribe
                );

                diagnostic.enter(Original20Site::RemovedSnapshot);
                let removed = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(removed.0, "ABSENT");
                assert_eq!(removed.1, 0);
                assert!(removed.3.is_none());
                diagnostic.enter(Original20Site::Admission);
                let admission = current
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                diagnostic.enter(Original20Site::PublicationTime);
                let publish_now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut published = Vec::new();
                for _ in 0..8 {
                    diagnostic.enter(Original20Site::ReadPublish);
                    match scenario.producer.read_and_publish(&admission).await? {
                        MarketStreamProducerOutcome::Published(result) => {
                            diagnostic.enter(Original20Site::PublishedRow);
                            assert_eq!(result.rows.len(), 1);
                            let row = &result.rows[0];
                            assert_eq!(row.identity.symbol(), "000660");
                            diagnostic.enter(Original20Site::RequireQuote);
                            let quote = row.quote.as_ref().ok_or("committed quote missing")?;
                            assert_eq!(quote.base_price, None);
                            published.push((
                                quote.price.clone(),
                                original20_at!(diagnostic, RequireOrdinal,
                                    row.receive_ordinal.ok_or("receipt ordinal missing"))?,
                                original20_at!(diagnostic, RequireTimestamp,
                                    row.received_at.ok_or("receipt timestamp missing"))?
                                    .timestamp_millis(),
                            ));
                            if published.len() == 2 {
                                break;
                            }
                        }
                        MarketStreamProducerOutcome::Control(_) => {}
                    }
                }
                diagnostic.enter(Original20Site::PublishedChecks);
                assert_eq!(published.len(), 2, "unrelated ACKed data was not preserved");
                assert_eq!(published[0].0, "70001.00000000");
                assert_eq!(published[1].0, "70004.00000000");
                assert!(published[0].1 < published[1].1);
                assert!(published[0].2 <= published[1].2);
                assert!(
                    published
                        .iter()
                        .all(|(_, _, received_at)| { publish_now_ms - received_at <= 3_000 })
                );

                diagnostic.enter(Original20Site::PersistedAppSnapshot);
                let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        scenario.leases[0].lease_id,
                    )
                    .await?;
                assert_eq!(snapshot.rows.len(), 1);
                assert_eq!(snapshot.rows[0].identity.symbol(), "000660");
                diagnostic.enter(Original20Site::PersistedQuote);
                assert_eq!(snapshot.rows[0].quote.as_ref().unwrap().price, "70004.00000000");
                diagnostic.enter(Original20Site::RemovedIdentity);
                let removed_identity = scenario.leases[1].identities[0].clone();
                drop(scenario.producer);
                diagnostic.enter(Original20Site::FirstTransportFinish);
                let server = scenario.transport.finish().await?;
                diagnostic.enter(Original20Site::FirstTransportChecks);
                assert_eq!(server.commands.len(), 3);
                assert_eq!(server.market_record_batches, vec![1, 1, 2]);
                assert!(server.client_closed);

                diagnostic.enter(Original20Site::RestoredLease);
                let restored_lease = make_lease(
                    &database,
                    &fixture,
                    std::slice::from_ref(&removed_identity),
                    Uuid::new_v4(),
                    0,
                    "c3b-restored-lease",
                )
                .await?;
                diagnostic.enter(Original20Site::RenewProducer);
                let new_producer_lease = scenario
                    .repository
                    .renew_stream_producer(&scenario.producer_lease)
                    .await?;
                diagnostic.enter(Original20Site::RestoredTime);
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                diagnostic.enter(Original20Site::RestoredLoopback);
                let mut next_transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    vec![
                        support::CommandPlan::new(
                            "000660",
                            MarketSubscriptionOperation::Subscribe,
                            Vec::new(),
                        ),
                        support::CommandPlan::new(
                            "005930",
                            MarketSubscriptionOperation::Subscribe,
                            Vec::new(),
                        ),
                    ],
                    true,
                )
                .await?;
                diagnostic.enter(Original20Site::RequireOldEpoch);
                let old_epoch = removed.4.ok_or("ABSENT row did not retain epoch")?;
                diagnostic.enter(Original20Site::RestoredStart);
                let mut next_producer = OwnerMarketStreamProducer::start(
                    scenario.repository.clone(),
                    new_producer_lease,
                    fixture.session.clone(),
                    next_transport.take_session(),
                )
                .await?;
                diagnostic.enter(Original20Site::ReadNewEpoch);
                let new_epoch =
                    producer_epoch(&database, fixture.owner_user_id, fixture.credential_slot_id)
                        .await?;
                diagnostic.enter(Original20Site::RequireNewEpoch);
                let new_epoch = new_epoch.ok_or("new producer epoch missing")?;
                assert_ne!(new_epoch, old_epoch);
                diagnostic.enter(Original20Site::RestoredDemand);
                let restored_demand = scenario
                    .repository
                    .read_stream_demand(fixture.credential_slot_id)
                    .await?;
                assert_eq!(restored_demand.items.len(), 2);
                diagnostic.enter(Original20Site::RestoredApply);
                let applied = match next_producer
                    .reconcile_desired(&restored_demand)
                    .await?
                {
                    MarketStreamReconcileOutcome::Complete(applied) => applied,
                    MarketStreamReconcileOutcome::Deferred {
                        committed,
                        not_before_ms,
                    } => {
                        assert_eq!(committed.active_symbols, 1);
                        assert_eq!(committed.transport_commands, 1);
                        let first_ack = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_ack.0, "ACKED");
                        assert!(first_ack.3.is_some());
                        next_transport
                            .wait_for_command_eligibility(not_before_ms)
                            .await?;
                        let fresh = scenario
                            .repository
                            .read_stream_demand(fixture.credential_slot_id)
                            .await?;
                        let resumed = next_producer
                            .reconcile_desired(&fresh)
                            .await?
                            .into_complete()?;
                        assert_eq!(resumed.active_symbols, 2);
                        assert_eq!(resumed.transport_commands, 1);
                        let first_after = snapshot_subscription(
                            &database,
                            fixture.owner_user_id,
                            fixture.credential_slot_id,
                            "000660",
                        )
                        .await?;
                        assert_eq!(first_after, first_ack);
                        MarketStreamApplyOutcome {
                            active_symbols: resumed.active_symbols,
                            transport_commands: committed
                                .transport_commands
                                .checked_add(resumed.transport_commands)
                                .expect("two bounded restored command counts"),
                        }
                    }
                };
                assert_eq!(
                    applied.active_symbols,
                    2,
                    "positive demand after unsubscribe requires fresh subscribe ACKs"
                );
                assert_eq!(applied.transport_commands, 2);
                diagnostic.enter(Original20Site::ReackFirstCommand);
                assert_eq!(next_transport.next_command().await?.0, "000660");
                diagnostic.enter(Original20Site::ReackSecondCommand);
                assert_eq!(next_transport.next_command().await?.0, "005930");
                diagnostic.enter(Original20Site::ReackSnapshot);
                let reacked = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(reacked.0, "ACKED");
                assert_eq!(reacked.1, 1);
                assert_eq!(reacked.4, Some(new_epoch));
                assert!(reacked.3.is_some());
                assert_ne!(reacked.2, removed.2);
                diagnostic.enter(Original20Site::RestoredAppSnapshot);
                let restored_snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        restored_lease.lease_id,
                    )
                    .await?;
                assert_eq!(restored_snapshot.rows.len(), 1);
                assert!(restored_snapshot.rows[0].quote.is_none());
                drop(next_producer);
                diagnostic.enter(Original20Site::FinalTransportFinish);
                let next_server = next_transport.finish().await?;
                diagnostic.enter(Original20Site::FinalTransportChecks);
                assert_eq!(next_server.commands.len(), 2);
                assert!(next_server.client_closed);
                diagnostic.enter(Original20Site::Complete);
                Ok(())
            })
        })
        .await
        .expect("unsubscribe publication case and cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn cancellation_before_desired_and_pending_commits_is_terminal_and_sends_no_bytes() {
        for pause_point in [
            ProducerAwaitPoint::DesiredPersistence,
            ProducerAwaitPoint::PendingPersistence,
        ] {
            let receipt = run_database_case(move |database| {
                Box::pin(async move {
                    let fixture = boundary::seed_fixture(&database).await?;
                    let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                    let plan = support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    );
                    let mut scenario = producer_scenario(
                        &database,
                        &fixture,
                        repository,
                        &["005930"],
                        vec![plan],
                        false,
                    )
                    .await?;
                    let gate = ProducerTestGate::new(pause_point);
                    scenario.producer.install_test_gate(gate.clone());
                    {
                        let operation = scenario.producer.apply_desired(&scenario.desired);
                        tokio::pin!(operation);
                        tokio::select! {
                            _ = gate.wait_until_reached() => {},
                            result = &mut operation => panic!("operation escaped its private await gate: {result:?}"),
                        }
                    }
                    assert_eq!(
                        scenario
                            .producer
                            .apply_desired(&scenario.desired)
                            .await
                            .unwrap_err(),
                        MarketStreamProducerError::Terminal
                    );
                    let row = snapshot_subscription_optional(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?;
                    match pause_point {
                        ProducerAwaitPoint::DesiredPersistence => assert!(row.is_none()),
                        ProducerAwaitPoint::PendingPersistence => {
                            let row = row.ok_or("desired row missing before prepared command")?;
                            assert_eq!(row.0, "DESIRED");
                            assert_eq!(row.1, 1);
                            let pending = pending_fields(
                                &database,
                                fixture.owner_user_id,
                                fixture.credential_slot_id,
                                "005930",
                            )
                            .await?
                            .ok_or("pending row missing")?;
                            assert_eq!(pending.0, None);
                            assert_eq!(pending.1, None);
                            assert_eq!(pending.2, None);
                            assert_eq!(pending.3, None);
                        }
                        _ => unreachable!(),
                    }
                    drop(scenario.producer);
                    let server = scenario.transport.finish().await?;
                    assert!(server.commands.is_empty());
                    assert!(server.client_closed);
                    Ok(())
                })
            })
            .await
            .expect("pre-commit cancellation case and exact cleanup must succeed");
            assert_case_ok(receipt);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn cancellation_during_real_ack_wait_preserves_pending_and_never_replays() {
        let (plan, mut ack_pause) = support::CommandPlan::paused_ack(
            "005930",
            MarketSubscriptionOperation::Subscribe,
            vec![("005930".to_owned(), 1)],
        );
        let receipt = run_database_case(move |database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                {
                    let operation = scenario.producer.apply_desired(&scenario.desired);
                    tokio::pin!(operation);
                    tokio::select! {
                        result = &mut operation => panic!("ACK wait unexpectedly completed: {result:?}"),
                        result = ack_pause.wait_until_reached() => result?,
                    }
                    assert_eq!(scenario.transport.next_command().await?.0, "005930");
                }
                assert_eq!(
                    scenario
                        .producer
                        .apply_desired(&scenario.desired)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                ack_pause.release();
                let pending = pending_fields(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .ok_or("pending row missing")?;
                assert_eq!(pending.0.as_deref(), Some("SUBSCRIBE"));
                assert_eq!(pending.1, Some(1));
                assert!(pending.2.is_some_and(|reserved| {
                    pending.3.is_some_and(|deadline| reserved < deadline)
                }));
                assert!(pending.4.is_none());
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("ACK-wait cancellation case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancellation_while_pending_and_ack_commit_results_are_withheld_is_terminal() {
        for await_point in [
            ProducerAwaitPoint::PendingPersistence,
            ProducerAwaitPoint::AckPersistence,
        ] {
            let receipt = run_database_case(move |database| {
                Box::pin(async move {
                    let fixture = boundary::seed_fixture(&database).await?;
                    let relay = start_task_relay(&database).await?;
                    let repository = worker_repository_via_relay(&database, &relay).await?;
                    let plan = support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        if await_point == ProducerAwaitPoint::AckPersistence {
                            vec![("005930".to_owned(), 1)]
                        } else {
                            Vec::new()
                        },
                    );
                    let mut scenario = producer_scenario(
                        &database,
                        &fixture,
                        repository,
                        &["005930"],
                        vec![plan],
                        false,
                    )
                    .await?;
                    let gate = ProducerTestGate::new(await_point);
                    scenario.producer.install_test_gate(gate.clone());
                    let mut held_commit;
                    {
                        let operation = scenario.producer.apply_desired(&scenario.desired);
                        tokio::pin!(operation);
                        tokio::select! {
                            _ = gate.wait_until_reached() => {},
                            result = &mut operation => panic!("producer did not reach the selected DB phase: {result:?}"),
                        }
                        held_commit = relay.hold_next_commit_response();
                        gate.resume();
                        tokio::time::timeout(Duration::from_secs(8), async {
                            tokio::select! {
                                result = &mut operation => Err::<(), CaseError>(
                                    format!("held DB phase completed before COMMIT observation: {result:?}").into(),
                                ),
                                result = held_commit.wait() => result,
                            }
                        })
                        .await
                        .map_err(|_| "held DB phase did not reach the real COMMIT observation")??;
                    }
                    held_commit.release();
                    assert_eq!(
                        scenario
                            .producer
                            .apply_desired(&scenario.desired)
                            .await
                            .unwrap_err(),
                        MarketStreamProducerError::Terminal
                    );
                    let row = snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?;
                    match await_point {
                        ProducerAwaitPoint::PendingPersistence => {
                            assert_eq!(row.0, "PENDING_SUBSCRIBE");
                            assert!(row.3.is_none());
                        }
                        ProducerAwaitPoint::AckPersistence => {
                            assert_eq!(row.0, "ACKED");
                            assert!(row.3.is_some());
                        }
                        _ => unreachable!(),
                    }
                    if await_point == ProducerAwaitPoint::AckPersistence {
                        let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                            .read_stream_snapshot(
                                fixture.owner_user_id,
                                &fixture.owner_session_hash,
                                scenario.leases[0].lease_id,
                            )
                            .await?;
                        assert!(snapshot.rows.iter().all(|row| row.quote.is_none()));
                    }
                    drop(scenario.producer);
                    drop(scenario.repository);
                    relay.close().await?;
                    let server = scenario.transport.finish().await?;
                    assert_eq!(
                        server.commands.len(),
                        usize::from(await_point == ProducerAwaitPoint::AckPersistence)
                    );
                    assert!(server.client_closed);
                    Ok(())
                })
            })
            .await
            .expect("DB-phase cancellation case and exact cleanup must succeed");
            assert_case_ok(receipt);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn cancellation_after_authentic_ack_before_db_ack_commit_keeps_pending() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    vec![("005930".to_owned(), 1)],
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                let gate = ProducerTestGate::new(ProducerAwaitPoint::AckPersistence);
                scenario.producer.install_test_gate(gate.clone());
                {
                    let operation = scenario.producer.apply_desired(&scenario.desired);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => {},
                        result = &mut operation => panic!("authentic ACK did not reach private DB gate: {result:?}"),
                    }
                }
                assert_eq!(
                    scenario
                        .producer
                        .apply_desired(&scenario.desired)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                let row = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(row.0, "PENDING_SUBSCRIBE");
                assert!(row.3.is_none());
                let pending = pending_fields(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .ok_or("pending row missing")?;
                assert_eq!(pending.0.as_deref(), Some("SUBSCRIBE"));
                assert_eq!(pending.1, Some(1));
                assert!(pending.4.is_none());
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("post-ACK cancellation case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn panic_after_session_take_is_caught_and_drops_the_live_socket() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    Vec::new(),
                );
                let scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                let mut producer = scenario.producer;
                producer.panic_after_next_session_take();
                let desired = scenario.desired;
                let task = tokio::spawn(async move { producer.apply_desired(&desired).await });
                let joined = task.await;
                assert!(joined.is_err_and(|error| error.is_panic()));
                assert!(
                    snapshot_subscription_optional(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?
                    .is_none()
                );
                drop(scenario.repository);
                let server = scenario.transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("caught facade panic case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn canceled_next_event_and_post_dequeue_publication_are_terminal() {
        for pause_publication in [false, true] {
            let receipt = run_database_case(move |database| {
                Box::pin(async move {
                    let fixture = boundary::seed_fixture(&database).await?;
                    let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                    let events = if pause_publication {
                        vec![("005930".to_owned(), 1)]
                    } else {
                        Vec::new()
                    };
                    let plan = support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        events,
                    );
                    let mut scenario = producer_scenario(
                        &database,
                        &fixture,
                        repository,
                        &["005930"],
                        vec![plan],
                        false,
                    )
                    .await?;
                    scenario.producer.apply_desired(&scenario.desired).await?;
                    let admission = scenario
                        .desired
                        .items
                        .iter()
                        .map(|item| item.identity.clone())
                        .collect::<Vec<_>>();
                    if pause_publication {
                        let gate = ProducerTestGate::new(ProducerAwaitPoint::Publication);
                        scenario.producer.install_test_gate(gate.clone());
                        {
                            let operation = scenario.producer.read_and_publish(&admission);
                            tokio::pin!(operation);
                            tokio::select! {
                                _ = gate.wait_until_reached() => {},
                                result = &mut operation => panic!("receipt failed to reach private publication gate: {result:?}"),
                            }
                        }
                    } else {
                        {
                            let operation = scenario.producer.read_and_publish(&admission);
                            tokio::pin!(operation);
                            assert!(tokio::time::timeout(
                                Duration::from_millis(100),
                                &mut operation,
                            )
                            .await
                            .is_err(), "next_event did not remain blocked on the real local socket");
                        }
                    }
                    assert_eq!(
                        scenario
                            .producer
                            .read_and_publish(&admission)
                            .await
                            .unwrap_err(),
                        MarketStreamProducerError::Terminal
                    );
                    let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                        .read_stream_snapshot(
                            fixture.owner_user_id,
                            &fixture.owner_session_hash,
                            scenario.leases[0].lease_id,
                        )
                        .await?;
                    assert!(snapshot.rows.iter().all(|row| row.quote.is_none()));
                    drop(scenario.producer);
                    let server = scenario.transport.finish().await?;
                    assert_eq!(server.commands.len(), 1);
                    assert!(server.client_closed);
                    Ok(())
                })
            })
            .await
            .expect("read cancellation case and exact cleanup must succeed");
            assert_case_ok(receipt);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn start_epoch_actual_commit_failure_returns_no_facade_and_rolls_back() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let producer_lease = repository
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        Uuid::new_v4(),
                        fixture.grant_revision,
                    )
                    .await?;
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    Vec::new(),
                    true,
                )
                .await?;
                let observation = relay.drop_next_commit_before_forward();
                let task = tokio::spawn(OwnerMarketStreamProducer::start(
                    repository.clone(),
                    producer_lease,
                    fixture.session.clone(),
                    transport.take_session(),
                ));
                let mut observation = observation;
                observation.wait().await?;
                let result = tokio::time::timeout(Duration::from_secs(8), task)
                    .await
                    .map_err(|_| "start failure future did not terminate")??;
                assert_eq!(
                    result.unwrap_err(),
                    MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown
                    )
                );
                assert_eq!(
                    producer_epoch(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                    )
                    .await?,
                    None,
                    "COMMIT was not forwarded and the epoch must roll back"
                );
                drop(repository);
                relay.close().await?;
                let server = transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("start rollback case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn start_epoch_committed_but_lost_response_returns_no_facade() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let producer_lease = repository
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        Uuid::new_v4(),
                        fixture.grant_revision,
                    )
                    .await?;
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    Vec::new(),
                    true,
                )
                .await?;
                let observation = relay.drop_next_commit_response();
                let task = tokio::spawn(OwnerMarketStreamProducer::start(
                    repository.clone(),
                    producer_lease,
                    fixture.session.clone(),
                    transport.take_session(),
                ));
                let mut observation = observation;
                observation.wait().await?;
                let result = tokio::time::timeout(Duration::from_secs(8), task)
                    .await
                    .map_err(|_| "start unknown-outcome future did not terminate")??;
                assert_eq!(
                    result.unwrap_err(),
                    MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown
                    )
                );
                assert!(
                    producer_epoch(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                    )
                    .await?
                    .is_some(),
                    "server committed the epoch before response loss"
                );
                drop(repository);
                relay.close().await?;
                let server = transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("start lost-response case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn start_epoch_future_cancellation_drops_owned_socket_without_resume() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let producer_lease = repository
                    .claim_stream_producer(
                        fixture.credential_slot_id,
                        Uuid::new_v4(),
                        fixture.grant_revision,
                    )
                    .await?;
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let mut transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    Vec::new(),
                    true,
                )
                .await?;
                let mut held_commit = relay.hold_next_commit_response();
                let task = tokio::spawn(OwnerMarketStreamProducer::start(
                    repository.clone(),
                    producer_lease,
                    fixture.session.clone(),
                    transport.take_session(),
                ));
                held_commit.wait().await?;
                task.abort();
                let joined = task.await;
                assert!(joined.is_err_and(|error| error.is_cancelled()));
                held_commit.release();
                assert!(
                    producer_epoch(&database, fixture.owner_user_id, fixture.credential_slot_id,)
                        .await?
                        .is_some(),
                    "the server committed while the start result was withheld"
                );
                drop(repository);
                relay.close().await?;
                let server = transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("start cancellation case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn desired_commit_response_loss_is_terminal_before_command_preparation() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    Vec::new(),
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository.clone(),
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                let gate = ProducerTestGate::new(ProducerAwaitPoint::DesiredPersistence);
                scenario.producer.install_test_gate(gate.clone());
                let mut held_commit;
                {
                    let operation = scenario.producer.apply_desired(&scenario.desired);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => {},
                        result = &mut operation => panic!("desired persistence did not reach its private gate: {result:?}"),
                    }
                    held_commit = relay.hold_next_commit_response();
                    gate.resume();
                    tokio::time::timeout(Duration::from_secs(8), async {
                        tokio::select! {
                            result = &mut operation => Err::<(), CaseError>(
                                format!("desired persistence completed before COMMIT observation: {result:?}").into(),
                            ),
                            result = held_commit.wait() => result,
                        }
                    })
                    .await
                    .map_err(|_| "desired persistence did not reach the real COMMIT observation")??;
                }
                held_commit.release();
                assert_eq!(
                    scenario
                        .producer
                        .apply_desired(&scenario.desired)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                let desired = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(desired.0, "DESIRED");
                assert_eq!(desired.1, 1);
                assert!(pending_fields(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .is_some_and(|pending| pending.0.is_none() && pending.1.is_none()));
                drop(scenario.producer);
                drop(scenario.repository);
                relay.close().await?;
                let server = scenario.transport.finish().await?;
                assert!(server.commands.is_empty());
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("desired lost-response case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    async fn runtime_claim_commit_fault(lose_response_after_commit: bool) {
        let receipt = run_database_case(move |database| {
            Box::pin(async move {
                use crate::owner_equity_v2::market_stream::{
                    MarketStreamStorageError, RuntimeMarketStreamStorageError as RuntimeError,
                };
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let storage = worker_repository_via_relay(&database, &relay).await?;
                let repository = storage.runtime_repository();
                let shared = repository.clone();
                let holder = Uuid::new_v4();
                let mut observation = if lose_response_after_commit {
                    relay.drop_next_commit_response()
                } else {
                    relay.drop_next_commit_before_forward()
                };
                let started = tokio::time::Instant::now();
                let (call, observed) = tokio::join!(
                    async {
                        let result = repository
                            .claim_stream_producer(
                                fixture.credential_slot_id,
                                holder,
                                fixture.grant_revision,
                            )
                            .await;
                        (result, started.elapsed())
                    },
                    observation.observe_with_timeout(Duration::from_secs(2)),
                );
                let followup = tokio::time::timeout(
                    Duration::from_millis(100),
                    shared.claim_stream_producer(
                        fixture.credential_slot_id,
                        Uuid::new_v4(),
                        fixture.grant_revision,
                    ),
                )
                .await;
                drop(shared);
                drop(repository);
                drop(storage);
                // Join all owned relay children before inspecting the independent database state.
                relay.close().await?;
                let stored: Vec<Uuid> = tokio::time::timeout(
                    Duration::from_secs(2),
                    sqlx::query_scalar(
                        "SELECT holder_id FROM public.owner_market_stream_producers
                        WHERE credential_slot_id = $1",
                    )
                    .bind(fixture.credential_slot_id)
                    .fetch_all(&database.migration_owner),
                )
                .await??;
                assert_eq!(observed, support::CommitObservationOutcome::Observed);
                assert!(matches!(
                    call.0,
                    Err(RuntimeError::Storage(
                        MarketStreamStorageError::CommitUnknown
                    ))
                ));
                assert!(call.1 < Duration::from_millis(1250));
                assert!(matches!(followup, Ok(Err(RuntimeError::Terminal))));
                if lose_response_after_commit {
                    assert_eq!(stored, vec![holder]);
                } else {
                    assert!(stored.is_empty());
                }
                Ok(())
            })
        })
        .await
        .expect("runtime claim ambiguity and owned relay cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_claim_commit_before_forward_is_terminal_and_rolls_back() {
        runtime_claim_commit_fault(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runtime_claim_lost_commit_response_is_terminal_and_does_not_retry() {
        runtime_claim_commit_fault(true).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn real_wire_pre_ack_frame_cannot_publish_or_consume_receipt_ordinal() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let repository = OwnerMarketStreamRepository::new(database.worker.clone());
                let identity = fixture.identities.iter()
                    .find(|identity| identity.symbol() == "005930")
                    .cloned().ok_or("approved pre-ACK identity missing")?;
                let (plan, mut pause) = support::CommandPlan::paused_ack_with_pre_ack(
                    "005930", vec![("005930".to_owned(), 1)],
                    vec![("005930".to_owned(), 2)],
                );
                let mut scenario = producer_scenario(
                    &database, &fixture, repository, &["005930"], vec![plan], false,
                ).await?;
                let app = OwnerMarketStreamRepository::new(database.app.clone());
                let checked: Result<_, CaseError> = async {
                    let (before, pending, applied) = {
                        let operation = scenario.producer.apply_desired(&scenario.desired);
                        tokio::pin!(operation);
                        tokio::select! {
                            result = pause.wait_until_reached() => result?,
                            _ = &mut operation => return Err("producer completed before causal pre-ACK barrier".into()),
                        }
                        // The mock saw the client's Pong after the real pre-ACK data frame.
                        // The genuine ACK is still withheld, so no fabricated capture is needed.
                        let before = tokio::time::timeout(Duration::from_secs(2), app.read_stream_snapshot(
                            fixture.owner_user_id, &fixture.owner_session_hash, scenario.leases[0].lease_id,
                        )).await??;
                        let pending = tokio::time::timeout(Duration::from_secs(2), snapshot_subscription(
                            &database, fixture.owner_user_id, fixture.credential_slot_id, "005930",
                        )).await??;
                        pause.release();
                        (before, pending, operation.await?)
                    };
                    let control = tokio::time::timeout(Duration::from_secs(2),
                        scenario.producer.read_and_publish(std::slice::from_ref(&identity)),
                    ).await??;
                    let after_control = tokio::time::timeout(Duration::from_secs(2), app.read_stream_snapshot(
                        fixture.owner_user_id, &fixture.owner_session_hash, scenario.leases[0].lease_id,
                    )).await??;
                    let published = tokio::time::timeout(Duration::from_secs(2),
                        scenario.producer.read_and_publish(std::slice::from_ref(&identity)),
                    ).await??;
                    let final_snapshot = tokio::time::timeout(Duration::from_secs(2), app.read_stream_snapshot(
                        fixture.owner_user_id, &fixture.owner_session_hash, scenario.leases[0].lease_id,
                    )).await??;
                    Ok((before, pending, applied, control, after_control, published, final_snapshot))
                }.await;
                drop(scenario.producer);
                let server = scenario.transport.finish().await?;
                let (before, pending, applied, control, after_control, published, final_snapshot) = checked?;
                assert!(before.rows.is_empty());
                assert_eq!(before.delivery_rows.len(), 1);
                assert!(before.delivery_rows[0].cache.is_none());
                assert!(!before.delivery_rows[0].live);
                assert_eq!(before.delivery_rows[0].state_version, 0);
                assert_eq!(before.delivery_rows[0].identity, identity);
                assert_eq!(pending.0, "PENDING_SUBSCRIBE");
                assert!(pending.3.is_none());
                assert_eq!(applied.transport_commands, 1);
                assert!(matches!(control, MarketStreamProducerOutcome::Control(
                    MarketStreamControlOutcome::TransportStatus(
                        MarketStreamTransportStatus::DataBeforeAck,
                    ),
                )));
                assert!(after_control.rows.is_empty());
                assert_eq!(after_control.delivery_rows.len(), 1);
                assert!(after_control.delivery_rows[0].cache.is_none());
                assert!(!after_control.delivery_rows[0].live);
                let MarketStreamProducerOutcome::Published(committed) = published else {
                    return Err("authentic post-ACK record did not publish".into());
                };
                assert_eq!(committed.rows.len(), 1);
                assert_eq!(committed.rows[0].quote_version, 1);
                assert_eq!(committed.rows[0].receive_ordinal, Some(1));
                assert_eq!(final_snapshot.rows[0].quote_version, 1);
                assert_eq!(final_snapshot.rows[0].receive_ordinal, Some(1));
                assert!(final_snapshot.rows[0].quote.is_some());
                assert_eq!(server.market_record_batches, vec![1, 1]);
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        }).await.expect("real pre-ACK wire case and owned cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pending_commit_failure_rolls_back_and_lost_response_preserves_ambiguity() {
        for lose_response_after_commit in [false, true] {
            let receipt = run_database_case(move |database| {
                Box::pin(async move {
                    let fixture = boundary::seed_fixture(&database).await?;
                    let relay = start_task_relay(&database).await?;
                    let repository = worker_repository_via_relay(&database, &relay).await?;
                    let plan = support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    );
                    let mut scenario = producer_scenario(
                        &database,
                        &fixture,
                        repository,
                        &["005930"],
                        vec![plan],
                        false,
                    )
                    .await?;
                    let gate = ProducerTestGate::new(ProducerAwaitPoint::PendingPersistence);
                    scenario.producer.install_test_gate(gate.clone());
                    let result = {
                        let operation = scenario.producer.apply_desired(&scenario.desired);
                        tokio::pin!(operation);
                        tokio::select! {
                            _ = gate.wait_until_reached() => {},
                            result = &mut operation => panic!("pending commit did not reach private gate: {result:?}"),
                        }
                        let mut observation = if lose_response_after_commit {
                            relay.drop_next_commit_response()
                        } else {
                            relay.drop_next_commit_before_forward()
                        };
                        gate.resume();
                        let (result, observed) = tokio::time::timeout(
                            Duration::from_secs(8),
                            async { tokio::join!(operation, observation.wait()) },
                        )
                        .await
                        .map_err(|_| "pending COMMIT result/observation did not complete")?;
                        observed?;
                        result
                    };
                    assert_eq!(
                        result.unwrap_err(),
                        MarketStreamProducerError::Storage(
                            crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown
                        )
                    );
                    assert_eq!(
                        scenario
                            .producer
                            .apply_desired(&scenario.desired)
                            .await
                            .unwrap_err(),
                        MarketStreamProducerError::Terminal
                    );
                    let row = snapshot_subscription(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?;
                    let pending = pending_fields(
                        &database,
                        fixture.owner_user_id,
                        fixture.credential_slot_id,
                        "005930",
                    )
                    .await?
                    .ok_or("subscription row missing")?;
                    if lose_response_after_commit {
                        assert_eq!(row.0, "PENDING_SUBSCRIBE");
                        assert_eq!(pending.0.as_deref(), Some("SUBSCRIBE"));
                        assert_eq!(pending.1, Some(1));
                        assert!(pending.2.is_some() && pending.3.is_some());
                    } else {
                        assert_eq!(row.0, "DESIRED");
                        assert_eq!(pending.0, None);
                        assert_eq!(pending.1, None);
                        assert_eq!(pending.2, None);
                        assert_eq!(pending.3, None);
                    }
                    drop(scenario.producer);
                    drop(scenario.repository);
                    relay.close().await?;
                    let server = scenario.transport.finish().await?;
                    assert!(server.commands.is_empty());
                    assert!(server.client_closed);
                    Ok(())
                })
            })
            .await
            .expect("pending commit ambiguity case and exact cleanup must succeed");
            assert_case_ok(receipt);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ack_commit_lost_response_never_publishes_buffered_quote_or_returns_proof() {
        let receipt = run_database_case_with_diagnostic(|database, diagnostic| {
            Box::pin(async move {
                diagnostic.enter(ClosedDiagnosticStage::Setup);
                let fixture = boundary::seed_fixture(&database)
                    .await
                    .map_err(|error| {
                        diagnostic.passthrough(ClosedDiagnosticResult::SetupFailure, error)
                    })?;
                let relay = start_task_relay(&database).await.map_err(|error| {
                    diagnostic.passthrough(ClosedDiagnosticResult::SetupFailure, error)
                })?;
                let repository = worker_repository_via_relay(&database, &relay)
                    .await
                    .map_err(|error| {
                        diagnostic.passthrough(ClosedDiagnosticResult::SetupFailure, error)
                    })?;
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    vec![("005930".to_owned(), 1)],
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await
                .map_err(|error| {
                    diagnostic.passthrough(ClosedDiagnosticResult::SetupFailure, error)
                })?;
                let gate = ProducerTestGate::new(ProducerAwaitPoint::AckPersistence);
                scenario.producer.install_test_gate(gate.clone());
                let transport_error = scenario.producer.transport_error_probe();
                diagnostic.enter(ClosedDiagnosticStage::AwaitingAckPersistence);
                let result = {
                    let operation = scenario.producer.apply_desired(&scenario.desired);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => diagnostic.reached_gate(),
                        result = &mut operation => {
                            diagnostic.result(ClosedDiagnosticResult::CommitReturnedBeforeGate);
                            panic!(
                                "authentic ACK did not reach DB commit gate: public={result:?}; private={:?}",
                                transport_error.take()
                            )
                        },
                    }
                    assert_eq!(transport_error.take(), None);
                    let mut observation = relay.drop_next_commit_response();
                    gate.resume();
                    diagnostic.enter(ClosedDiagnosticStage::CommitJoin);
                    let Some((result, observed)) = track_commit_join(
                        &mut operation,
                        &mut observation,
                        &diagnostic,
                    )
                    .await
                    else {
                        return Err("ACK COMMIT result/observation did not complete".into());
                    };
                    if observed != support::CommitObservationOutcome::Observed {
                        diagnostic.result(ClosedDiagnosticResult::RelayObservationFailure);
                        return Err(match observed {
                            support::CommitObservationOutcome::TimedOut => {
                                "relay COMMIT observation timed out"
                            }
                            support::CommitObservationOutcome::SenderClosed => {
                                "relay COMMIT observation sender closed"
                            }
                            support::CommitObservationOutcome::Observed => {
                                "relay COMMIT observation state invalid"
                            }
                        }
                        .into());
                    }
                    result
                };
                diagnostic.enter(ClosedDiagnosticStage::CommitResult);
                diagnostic.result(match &result {
                    Err(MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown,
                    )) => ClosedDiagnosticResult::CommitUnknown,
                    Ok(_) => ClosedDiagnosticResult::CommitUnexpectedSuccess,
                    Err(_) => ClosedDiagnosticResult::CommitUnexpectedFailure,
                });
                assert_eq!(
                    result.unwrap_err(),
                    MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown
                    )
                );
                diagnostic.enter(ClosedDiagnosticStage::PostCommitChecks);
                assert_eq!(
                    scenario
                        .producer
                        .apply_desired(&scenario.desired)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                let row = snapshot_subscription(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?;
                assert_eq!(row.0, "ACKED");
                assert!(row.3.is_some());
                let pending = pending_fields(
                    &database,
                    fixture.owner_user_id,
                    fixture.credential_slot_id,
                    "005930",
                )
                .await?
                .ok_or("ACKed subscription row missing")?;
                assert!(pending.0.is_none() && pending.1.is_none());
                assert!(pending.2.is_none() && pending.3.is_none());
                assert!(pending.4.is_some());
                let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        scenario.leases[0].lease_id,
                    )
                    .await?;
                assert!(snapshot.rows.iter().all(|row| row.quote.is_none()));
                diagnostic.enter(ClosedDiagnosticStage::Finalization);
                drop(scenario.producer);
                drop(scenario.repository);
                relay.close().await.map_err(|error| {
                    diagnostic.passthrough(ClosedDiagnosticResult::FinalizationFailure, error)
                })?;
                let server = scenario.transport.finish().await.map_err(|error| {
                    diagnostic.passthrough(ClosedDiagnosticResult::FinalizationFailure, error)
                })?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                diagnostic.enter(ClosedDiagnosticStage::Complete);
                diagnostic.result(ClosedDiagnosticResult::Complete);
                Ok(())
            })
        })
        .await
        .expect("ACK lost-response case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn committed_quote_lost_response_is_returned_once_by_exact_reread() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    vec![("005930".to_owned(), 1)],
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                scenario.producer.apply_desired(&scenario.desired).await?;
                assert_eq!(scenario.transport.next_command().await?.0, "005930");
                let gate = ProducerTestGate::new(ProducerAwaitPoint::Publication);
                scenario.producer.install_test_gate(gate.clone());
                let admissions = scenario
                    .desired
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                let result = {
                    let operation = scenario.producer.read_and_publish(&admissions);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => {},
                        result = &mut operation => panic!("authentic receipt did not reach publication gate: {result:?}"),
                    }
                    let mut observation = relay.drop_next_commit_response();
                    gate.resume();
                    let (result, observed) = tokio::time::timeout(
                        Duration::from_secs(8),
                        async { tokio::join!(operation, observation.wait()) },
                    )
                    .await
                    .map_err(|_| "publication COMMIT result/observation did not complete")?;
                    observed?;
                    result?
                };
                let MarketStreamProducerOutcome::Published(committed) = result else {
                    return Err("committed exact reread did not return one publication result".into());
                };
                assert!(!committed.changed, "reread result must identify recovered commit");
                assert_eq!(committed.rows.len(), 1);
                let row = &committed.rows[0];
                assert_eq!(row.identity.symbol(), "005930");
                assert_eq!(row.quote.as_ref().unwrap().base_price, None);
                assert!(row.receive_ordinal.is_some_and(|ordinal| ordinal > 0));
                assert!(row.received_at.is_some());
                let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        scenario.leases[0].lease_id,
                    )
                    .await?;
                assert_eq!(snapshot.rows.len(), 1);
                assert_eq!(snapshot.rows[0].quote, row.quote);
                assert_eq!(snapshot.rows[0].receive_ordinal, row.receive_ordinal);
                assert!(scenario.transport.try_next_command().is_none());
                drop(scenario.producer);
                drop(scenario.repository);
                relay.close().await?;
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("quote reread case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancellation_during_actual_quote_commit_returns_no_result_and_is_terminal() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    vec![("005930".to_owned(), 1)],
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                scenario.producer.apply_desired(&scenario.desired).await?;
                let admissions = scenario
                    .desired
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                let gate = ProducerTestGate::new(ProducerAwaitPoint::Publication);
                scenario.producer.install_test_gate(gate.clone());
                let mut held_commit;
                {
                    let operation = scenario.producer.read_and_publish(&admissions);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => {},
                        result = &mut operation => panic!("receipt did not reach publication commit boundary: {result:?}"),
                    }
                    held_commit = relay.hold_next_commit_response();
                    gate.resume();
                    tokio::time::timeout(Duration::from_secs(8), async {
                        tokio::select! {
                            result = &mut operation => Err::<(), CaseError>(
                                format!("publication completed before held COMMIT observation: {result:?}").into(),
                            ),
                            result = held_commit.wait() => result,
                        }
                    })
                    .await
                    .map_err(|_| "publication did not reach the real held COMMIT observation")??;
                }
                held_commit.release();
                assert_eq!(
                    scenario
                        .producer
                        .read_and_publish(&admissions)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        scenario.leases[0].lease_id,
                    )
                    .await?;
                assert_eq!(snapshot.rows.len(), 1);
                assert!(snapshot.rows[0].quote.is_some());
                assert!(snapshot.rows[0].receive_ordinal.is_some());
                drop(scenario.producer);
                drop(scenario.repository);
                relay.close().await?;
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("quote-commit cancellation case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn quote_commit_failure_rolls_back_and_returns_no_publication() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let repository = worker_repository_via_relay(&database, &relay).await?;
                let plan = support::CommandPlan::new(
                    "005930",
                    MarketSubscriptionOperation::Subscribe,
                    vec![("005930".to_owned(), 1)],
                );
                let mut scenario = producer_scenario(
                    &database,
                    &fixture,
                    repository,
                    &["005930"],
                    vec![plan],
                    false,
                )
                .await?;
                scenario.producer.apply_desired(&scenario.desired).await?;
                let gate = ProducerTestGate::new(ProducerAwaitPoint::Publication);
                scenario.producer.install_test_gate(gate.clone());
                let admissions = scenario
                    .desired
                    .items
                    .iter()
                    .map(|item| item.identity.clone())
                    .collect::<Vec<_>>();
                let result = {
                    let operation = scenario.producer.read_and_publish(&admissions);
                    tokio::pin!(operation);
                    tokio::select! {
                        _ = gate.wait_until_reached() => {},
                        result = &mut operation => panic!("receipt did not reach commit-failure gate: {result:?}"),
                    }
                    let mut observation = relay.drop_next_commit_before_forward();
                    gate.resume();
                    let (result, observed) = tokio::time::timeout(
                        Duration::from_secs(8),
                        async { tokio::join!(operation, observation.wait()) },
                    )
                    .await
                    .map_err(|_| "rollback COMMIT result/observation did not complete")?;
                    observed?;
                    result
                };
                assert_eq!(
                    result.unwrap_err(),
                    MarketStreamProducerError::Storage(
                        crate::owner_equity_v2::market_stream::MarketStreamStorageError::CommitUnknown
                    )
                );
                assert_eq!(
                    scenario
                        .producer
                        .read_and_publish(&admissions)
                        .await
                        .unwrap_err(),
                    MarketStreamProducerError::Terminal
                );
                let snapshot = OwnerMarketStreamRepository::new(database.app.clone())
                    .read_stream_snapshot(
                        fixture.owner_user_id,
                        &fixture.owner_session_hash,
                        scenario.leases[0].lease_id,
                    )
                    .await?;
                assert!(snapshot.rows.iter().all(|row| row.quote.is_none()));
                drop(scenario.producer);
                drop(scenario.repository);
                relay.close().await?;
                let server = scenario.transport.finish().await?;
                assert_eq!(server.commands.len(), 1);
                assert!(server.client_closed);
                Ok(())
            })
        })
        .await
        .expect("quote rollback case and exact cleanup must succeed");
        assert_case_ok(receipt);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn database_cleanup_runs_after_intentional_test_panic() {
        let receipt = run_database_case(|database| {
            Box::pin(async move {
                let fixture = boundary::seed_fixture(&database).await?;
                let relay = start_task_relay(&database).await?;
                let _worker = worker_repository_via_relay(&database, &relay).await?;
                let now_ms = boundary::now(&database).await?.timestamp_millis();
                let transport = support::loopback_session(
                    fixture.session_date,
                    fixture.credential_slot_id,
                    now_ms,
                    vec![support::CommandPlan::new(
                        "005930",
                        MarketSubscriptionOperation::Subscribe,
                        Vec::new(),
                    )],
                    true,
                )
                .await?;
                let _owned_fixtures = (fixture, relay, transport);
                panic!("intentional private C3B cleanup control");
            })
        })
        .await
        .expect("intentional panic control and exact cleanup must succeed");
        assert!(!receipt.body_ok);
        assert!(receipt.body_panicked);
        assert_clean(receipt);
    }

    #[cfg(test)]
    mod original20_stage_diagnostic_unit_tests {
        use super::*;
        use std::cell::Cell;

        fn render(diagnostic: &Original20Diagnostic) -> (bool, Vec<u8>) {
            let mut bytes = Vec::new();
            let available = diagnostic
                .emit_to(&mut bytes)
                .expect("private in-memory diagnostic writer");
            (available, bytes)
        }

        fn assert_physical_frame(frame: &[u8]) {
            assert!(!frame.is_empty());
            assert_eq!(frame[0], b'\n');
            let record = &frame[1..];
            assert!(std::str::from_utf8(record).is_ok());
            assert!(record.is_ascii());
            assert_eq!(record.last(), Some(&b'\n'));
            assert!(!record.contains(&b'\r'));
            assert_eq!(record.iter().filter(|&&byte| byte == b'\n').count(), 1);
            assert!(record.len() <= 4096);
        }

        #[test]
        fn finite_tokens_and_physical_framing_are_closed() {
            use Original19Terminal as T;
            use Original20Site as S;
            const SITES: [(S, &str); 45] = [
                (S::NotStarted, "not_started"),
                (S::CreateDatabase, "create_database"),
                (S::DatabaseName, "database_name"),
                (S::SeedFixture, "seed_fixture"),
                (S::Scenario, "scenario"),
                (S::InitialApply, "initial_apply"),
                (S::InitialFirstCommand, "initial_first_command"),
                (S::InitialSecondCommand, "initial_second_command"),
                (S::RemovedLease, "removed_lease"),
                (S::ReleaseStreamLease, "release_stream_lease"),
                (S::DemandAfterRelease, "demand_after_release"),
                (S::UnsubscribeApply, "unsubscribe_apply"),
                (S::UnsubscribeCommand, "unsubscribe_command"),
                (S::RemovedSnapshot, "removed_snapshot"),
                (S::Admission, "admission"),
                (S::PublicationTime, "publication_time"),
                (S::ReadPublish, "read_publish"),
                (S::PublishedRow, "published_row"),
                (S::RequireQuote, "require_quote"),
                (S::RequireOrdinal, "require_ordinal"),
                (S::RequireTimestamp, "require_timestamp"),
                (S::PublishedChecks, "published_checks"),
                (S::PersistedAppSnapshot, "persisted_app_snapshot"),
                (S::PersistedQuote, "persisted_quote"),
                (S::RemovedIdentity, "removed_identity"),
                (S::FirstTransportFinish, "first_transport_finish"),
                (S::FirstTransportChecks, "first_transport_checks"),
                (S::RestoredLease, "restored_lease"),
                (S::RenewProducer, "renew_producer"),
                (S::RestoredTime, "restored_time"),
                (S::RestoredLoopback, "restored_loopback"),
                (S::RequireOldEpoch, "require_old_epoch"),
                (S::RestoredStart, "restored_start"),
                (S::ReadNewEpoch, "read_new_epoch"),
                (S::RequireNewEpoch, "require_new_epoch"),
                (S::RestoredDemand, "restored_demand"),
                (S::RestoredApply, "restored_apply"),
                (S::ReackFirstCommand, "reack_first_command"),
                (S::ReackSecondCommand, "reack_second_command"),
                (S::ReackSnapshot, "reack_snapshot"),
                (S::RestoredAppSnapshot, "restored_app_snapshot"),
                (S::FinalTransportFinish, "final_transport_finish"),
                (S::FinalTransportChecks, "final_transport_checks"),
                (S::Complete, "complete"),
                (S::DiagnosticUnavailable, "diagnostic_unavailable"),
            ];
            const TERMINALS: [(T, &str); 11] = [
                (T::ReturnedOk, "returned_ok"),
                (T::ReturnedError, "returned_error"),
                (T::JoinPanic, "join_panic"),
                (T::JoinCancelled, "join_cancelled"),
                (T::JoinOther, "join_other"),
                (T::TimeoutJoinedOk, "timeout_joined_ok"),
                (T::TimeoutJoinedBodyError, "timeout_joined_body_error"),
                (T::TimeoutJoinedPanic, "timeout_joined_panic"),
                (T::TimeoutJoinedCancelled, "timeout_joined_cancelled"),
                (T::TimeoutJoinedOther, "timeout_joined_other"),
                (T::DiagnosticUnavailable, "diagnostic_unavailable"),
            ];
            assert_eq!(SITES.iter().map(|(_, token)| *token).collect::<std::collections::BTreeSet<_>>().len(), 45);
            assert_eq!(TERMINALS.iter().map(|(_, token)| *token).collect::<std::collections::BTreeSet<_>>().len(), 11);

            let mut max_frame = 0;
            let mut max_record = 0;
            for (site, site_token) in SITES {
                let diagnostic = Original20Diagnostic::new();
                diagnostic.enter(site);
                diagnostic.freeze(T::ReturnedOk);
                let (available, frame) = render(&diagnostic);
                assert!(available);
                assert_physical_frame(&frame);
                assert_eq!(
                    frame,
                    format!("\nC3B_ORIGINAL20_STAGE_V1 site={site_token} terminal=returned_ok diagnostic=available\n").as_bytes(),
                );
                max_frame = max_frame.max(frame.len());
                max_record = max_record.max(frame.len() - 1);
            }
            for (terminal, terminal_token) in TERMINALS {
                let diagnostic = Original20Diagnostic::new();
                diagnostic.enter(S::Scenario);
                diagnostic.freeze(terminal);
                let (available, frame) = render(&diagnostic);
                assert!(available);
                assert_physical_frame(&frame);
                assert_eq!(
                    frame,
                    format!("\nC3B_ORIGINAL20_STAGE_V1 site=scenario terminal={terminal_token} diagnostic=available\n").as_bytes(),
                );
                max_frame = max_frame.max(frame.len());
                max_record = max_record.max(frame.len() - 1);
            }

            let unavailable = Original20Diagnostic::new();
            let (available, frame) = render(&unavailable);
            assert!(!available);
            assert_physical_frame(&frame);
            assert_eq!(frame, b"\nC3B_ORIGINAL20_STAGE_V1 site=diagnostic_unavailable terminal=diagnostic_unavailable diagnostic=diagnostic_unavailable\n");
            max_frame = max_frame.max(frame.len());
            max_record = max_record.max(frame.len() - 1);
            assert_eq!((max_record, max_frame), (118, 119));

            let diagnostic = Original20Diagnostic::new();
            diagnostic.enter(S::FinalTransportChecks);
            diagnostic.freeze(T::ReturnedOk);
            let prefix = vec![b'P'; 8192];
            let mut capture = prefix.clone();
            assert!(diagnostic.emit_to(&mut capture).expect("bounded Vec write"));
            assert_eq!(&capture[..prefix.len()], prefix.as_slice());
            let appended = &capture[prefix.len()..];
            assert_physical_frame(appended);
            assert_eq!(appended[0], b'\n');
            assert_eq!(appended, b"\nC3B_ORIGINAL20_STAGE_V1 site=final_transport_checks terminal=returned_ok diagnostic=available\n");
        }

        #[tokio::test(flavor = "current_thread")]
        async fn terminal_mapping_retains_timeout_join_outcomes() {
            use Original19Joined as J;
            use Original19Terminal as T;

            fn assert_mapping(joined: Original19Joined, ordinary: T, timed_out: T) {
                for (timeout, expected) in [(false, ordinary), (true, timed_out)] {
                    let actual = original19_terminal(timeout, joined);
                    assert_eq!(actual, expected);
                    let diagnostic = Original20Diagnostic::new();
                    diagnostic.enter(Original20Site::Scenario);
                    diagnostic.freeze(actual);
                    let (available, frame) = render(&diagnostic);
                    assert!(available);
                    assert_physical_frame(&frame);
                    assert_eq!(
                        frame,
                        format!("\nC3B_ORIGINAL20_STAGE_V1 site=scenario terminal={} diagnostic=available\n", expected.literal()).as_bytes(),
                    );
                }
            }

            let ok = tokio::spawn(async { Ok::<(), ()>(()) }).await;
            let error = tokio::spawn(async { Err::<(), ()>(()) }).await;
            let panic: Result<Result<(), ()>, tokio::task::JoinError> = tokio::spawn(async {
                panic!("R110D7_SYNTHETIC_JOIN_PANIC");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            }).await;
            let pending = tokio::spawn(std::future::pending::<Result<(), ()>>());
            pending.abort();
            let cancelled = pending.await;

            assert_mapping(original19_joined(&ok), T::ReturnedOk, T::TimeoutJoinedOk);
            assert_mapping(original19_joined(&error), T::ReturnedError, T::TimeoutJoinedBodyError);
            assert_mapping(original19_joined(&panic), T::JoinPanic, T::TimeoutJoinedPanic);
            assert_mapping(original19_joined(&cancelled), T::JoinCancelled, T::TimeoutJoinedCancelled);
            // Tokio has no public constructor for an arbitrary non-panic,
            // non-cancelled JoinError; this tests the existing pure flag fallback.
            assert_eq!(original19_join_error(false, false), J::Other);
            assert_mapping(J::Other, T::JoinOther, T::TimeoutJoinedOther);
        }

        #[test]
        fn terminal_freeze_ignores_late_site_updates() {
            let diagnostic = Original20Diagnostic::new();
            let shared = diagnostic.clone();
            assert!(Arc::ptr_eq(&diagnostic.state, &shared.state));
            diagnostic.enter(Original20Site::InitialApply);
            diagnostic.freeze(Original19Terminal::ReturnedError);
            let frozen = render(&diagnostic);
            assert!(frozen.0);
            assert_eq!(frozen.1, b"\nC3B_ORIGINAL20_STAGE_V1 site=initial_apply terminal=returned_error diagnostic=available\n");

            shared.enter(Original20Site::Complete);
            shared.freeze(Original19Terminal::ReturnedOk);
            diagnostic.enter(Original20Site::FinalTransportFinish);
            diagnostic.freeze(Original19Terminal::JoinPanic);
            assert_eq!(render(&shared), frozen);
            assert_eq!(render(&diagnostic), frozen);
        }

        #[test]
        fn poisoned_and_unfrozen_state_are_unavailable() {
            let unfrozen = Original20Diagnostic::new();
            unfrozen.enter(Original20Site::Scenario);
            let (available, frame) = render(&unfrozen);
            assert!(!available);
            assert_eq!(frame, b"\nC3B_ORIGINAL20_STAGE_V1 site=diagnostic_unavailable terminal=diagnostic_unavailable diagnostic=diagnostic_unavailable\n");

            let poisoned = Original20Diagnostic::new();
            poisoned.enter(Original20Site::InitialApply);
            let state = poisoned.state.clone();
            assert!(catch_unwind(AssertUnwindSafe(move || {
                let _guard = state.lock().expect("fresh synthetic diagnostic state");
                panic!("R110D7_SYNTHETIC_STATE_POISON");
            })).is_err());
            poisoned.enter(Original20Site::Complete);
            poisoned.freeze(Original19Terminal::ReturnedOk);
            let (available, frame) = render(&poisoned);
            assert!(!available);
            assert_physical_frame(&frame);
            assert_eq!(frame, b"\nC3B_ORIGINAL20_STAGE_V1 site=diagnostic_unavailable terminal=diagnostic_unavailable diagnostic=diagnostic_unavailable\n");
        }

        #[test]
        fn write_error_preserves_frozen_diagnostic_state() {
            struct ImmediateError { calls: usize }
            impl Write for ImmediateError {
                fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                    self.calls += 1;
                    Err(io::Error::new(io::ErrorKind::BrokenPipe, "R110D7_SYNTHETIC_IMMEDIATE_WRITE"))
                }
                fn flush(&mut self) -> io::Result<()> { Ok(()) }
            }
            struct ShortThenError { calls: usize, accepted: usize }
            impl Write for ShortThenError {
                fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.calls += 1;
                    if self.calls == 1 {
                        let accepted = bytes.len().min(7);
                        self.accepted += accepted;
                        Ok(accepted)
                    } else {
                        Err(io::Error::new(io::ErrorKind::BrokenPipe, "R110D7_SYNTHETIC_SHORT_WRITE"))
                    }
                }
                fn flush(&mut self) -> io::Result<()> { Ok(()) }
            }

            let diagnostic = Original20Diagnostic::new();
            diagnostic.enter(Original20Site::PersistedAppSnapshot);
            diagnostic.freeze(Original19Terminal::ReturnedError);
            let expected = b"\nC3B_ORIGINAL20_STAGE_V1 site=persisted_app_snapshot terminal=returned_error diagnostic=available\n";

            let mut immediate = ImmediateError { calls: 0 };
            let error = diagnostic.emit_to(&mut immediate).expect_err("immediate error propagates");
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(immediate.calls, 1);

            let mut partial = ShortThenError { calls: 0, accepted: 0 };
            let error = diagnostic.emit_to(&mut partial).expect_err("short write follow-up error propagates");
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(partial.calls, 2);
            assert_eq!(partial.accepted, 7);

            let (available, frame) = render(&diagnostic);
            assert!(available);
            assert_eq!(frame, expected);
            assert_physical_frame(&frame);
        }

        #[test]
        fn optional_field_observer_preserves_result_and_evaluation_order() {
            struct MoveOnly(&'static str);
            fn site(diagnostic: &Original20Diagnostic) -> &'static str {
                diagnostic.state.lock().expect("available synthetic state").site.literal()
            }
            fn propagate(
                diagnostic: &Original20Diagnostic,
                evaluations: &Cell<usize>,
                continuation: &Cell<usize>,
            ) -> Result<(), &'static str> {
                original20_at!(diagnostic, PersistedQuote, {
                    assert_eq!(site(diagnostic), "persisted_quote");
                    evaluations.set(evaluations.get() + 1);
                    Err::<(), &'static str>("R110D7_SYNTHETIC_EARLY")
                })?;
                continuation.set(continuation.get() + 1);
                Ok(())
            }

            let diagnostic = Original20Diagnostic::new();
            let evaluations = Cell::new(0);
            let some: Option<MoveOnly> = original20_at!(diagnostic, InitialApply, {
                assert_eq!(site(&diagnostic), "initial_apply");
                evaluations.set(evaluations.get() + 1);
                Some(MoveOnly("some"))
            });
            assert_eq!(some.map(|value| value.0), Some("some"));
            let none: Option<MoveOnly> = original20_at!(diagnostic, RemovedSnapshot, {
                assert_eq!(site(&diagnostic), "removed_snapshot");
                evaluations.set(evaluations.get() + 1);
                None
            });
            assert!(none.is_none());

            let value = Box::new(MoveOnly("ok"));
            let value_pointer = &*value as *const MoveOnly;
            let ok: Result<Box<MoveOnly>, Box<MoveOnly>> = original20_at!(diagnostic, PublishedChecks, {
                assert_eq!(site(&diagnostic), "published_checks");
                evaluations.set(evaluations.get() + 1);
                Ok(value)
            });
            let returned = match ok {
                Ok(value) => value,
                Err(_) => panic!("original Ok is preserved"),
            };
            assert_eq!(value_pointer, &*returned as *const MoveOnly);
            assert_eq!(returned.0, "ok");

            let error = Box::new(MoveOnly("error"));
            let error_pointer = &*error as *const MoveOnly;
            let failed: Result<(), Box<MoveOnly>> = original20_at!(diagnostic, FirstTransportChecks, {
                assert_eq!(site(&diagnostic), "first_transport_checks");
                evaluations.set(evaluations.get() + 1);
                Err(error)
            });
            let returned = failed.expect_err("original Err is preserved");
            assert_eq!(error_pointer, &*returned as *const MoveOnly);
            assert_eq!(returned.0, "error");

            let continuation = Cell::new(0);
            assert_eq!(propagate(&diagnostic, &evaluations, &continuation), Err("R110D7_SYNTHETIC_EARLY"));
            assert_eq!(continuation.get(), 0);
            assert_eq!(evaluations.get(), 5);
            assert_eq!(site(&diagnostic), "persisted_quote");
            diagnostic.freeze(Original19Terminal::ReturnedError);
            let (available, frame) = render(&diagnostic);
            assert!(available);
            assert_eq!(frame, b"\nC3B_ORIGINAL20_STAGE_V1 site=persisted_quote terminal=returned_error diagnostic=available\n");
        }
    }
}
