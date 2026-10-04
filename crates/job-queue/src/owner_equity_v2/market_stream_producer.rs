//! Owning facade for one C2 market-stream socket and its C1 durable state.
//!
//! The facade is intentionally the only public orchestration boundary. It
//! retains transport capabilities and committed subscription proofs, and
//! exposes only committed rows or closed metadata outcomes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[cfg(test)]
use std::sync::{Arc, Mutex};

pub use kis_client::market_stream::StreamStatusCode as MarketStreamTransportStatus;
use kis_client::market_stream::{
    MarketStreamError, MarketStreamEvent, MarketStreamSession, MarketSubscriptionOperation,
};
#[cfg(all(test, feature = "market-stream-db-tests"))]
use kis_client::market_stream::MarketStreamCommandStateSnapshot;
#[cfg(test)]
use kis_client::market_stream_approval::ApprovalError;
#[cfg(test)]
use kis_client::market_stream_state::{BudgetError, StateError};
#[cfg(test)]
use kis_client::market_stream_wire::WireError;
use market_data::market_stream::StreamQuote;
use thiserror::Error;
#[cfg(test)]
use tokio::sync::Notify;
use uuid::Uuid;

use super::market_stream::{
    CommitResult, DesiredSet, DesiredStreamItem, MarketStreamStorageError,
    OwnerMarketStreamRepository, STREAM_MAX_ACTIVE_IDENTITIES, STREAM_MAX_ACTIVE_LEASES,
    STREAM_MAX_IDENTITIES_PER_LEASE, StreamEpochProof, StreamIdentity, StreamProducerLease,
    StreamPublicationContext, StreamPublicationItem, StreamPublicationObservation,
    StreamSessionProof, SubscriptionProof,
};

const MAX_REFERENCE_COUNT: u32 =
    (STREAM_MAX_ACTIVE_LEASES as u32) * (STREAM_MAX_IDENTITIES_PER_LEASE as u32);

// Keep this validation aligned with kis-client's private H0STCNT0 allowlist.
// The facade checks it before desired-state persistence so an unsupported
// symbol cannot create a durable row before C2 rejects the transport command.
pub(super) const APPROVED_STREAM_SYMBOLS: [&str; 30] = [
    "005930", "000660", "373220", "207940", "005380", "000270", "105560", "055550", "068270",
    "035420", "035720", "005490", "051910", "006400", "012330", "028260", "012450", "329180",
    "034020", "015760", "017670", "030200", "066570", "009150", "096770", "036570", "090430",
    "011200", "003490", "000810",
];

/// Safe, closed producer failures. Transport diagnostics are deliberately
/// collapsed so provider prose and protocol payloads cannot escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MarketStreamProducerError {
    #[error(transparent)]
    Storage(#[from] MarketStreamStorageError),
    #[error("market stream producer input is invalid")]
    InvalidInput,
    #[error("market stream producer is not ready")]
    NotReady,
    #[error("market stream producer is terminal")]
    Terminal,
    #[error("market stream transport failed")]
    TransportFailure,
    #[error("market stream transport and durable subscription state diverged")]
    SubscriptionOutOfSync,
    #[error("market stream session is closed or belongs to another epoch")]
    SessionUnavailable,
}

/// Metadata-only events emitted by a successful read operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStreamControlOutcome {
    ApplicationHeartbeat,
    ControlPong,
    TransportStatus(MarketStreamTransportStatus),
    NoCurrentAdmission,
}

/// Result of reading one transport event. Quotes appear only after the
/// repository returns a committed result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketStreamProducerOutcome {
    Published(CommitResult),
    Control(MarketStreamControlOutcome),
}

/// Bounded committed counts; completion is explicit in reconciliation progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketStreamApplyOutcome {
    pub active_symbols: u8,
    pub transport_commands: u8,
}

/// Per-invocation progress. Deferred is not a fully applied desired set,
/// a current lease/rights proof, or permission to replay a prepared command.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStreamReconcileOutcome {
    Complete(MarketStreamApplyOutcome),
    Deferred {
        committed: MarketStreamApplyOutcome,
        not_before_ms: i64,
    },
}

impl MarketStreamReconcileOutcome {
    pub fn into_complete(self) -> Result<MarketStreamApplyOutcome, MarketStreamProducerError> {
        match self {
            Self::Complete(outcome) => Ok(outcome),
            Self::Deferred { .. } => Err(MarketStreamProducerError::NotReady),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProducerAwaitPoint {
    DemandRead,
    DesiredPersistence,
    PendingPersistence,
    AckPersistence,
    Publication,
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct ProducerTestGate {
    point: ProducerAwaitPoint,
    reached: Arc<Notify>,
    resume: Arc<Notify>,
}

#[cfg(test)]
impl ProducerTestGate {
    pub(super) fn new(point: ProducerAwaitPoint) -> Self {
        Self {
            point,
            reached: Arc::new(Notify::new()),
            resume: Arc::new(Notify::new()),
        }
    }

    pub(super) async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    pub(super) fn resume(&self) {
        self.resume.notify_one();
    }
}

/// Closed, test-only metadata for the private C2 error that is deliberately
/// collapsed at the public producer boundary. No provider prose or payload is
/// retained, and every producer owns an isolated observation slot.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProducerTransportErrorObservation {
    Approval(ApprovalError),
    EndpointNotAllowed,
    Io,
    Handshake,
    FrameInvalid,
    Protocol,
    Closed,
    CommandPending,
    CommandCapabilityMismatch,
    CommandTimingInvalid,
    CommandTimingOverflow,
    AckTimeout,
    ControlWriteTimeout,
    AckMismatch,
    DuplicateAck,
    CommandRejected,
    DataBeforeAck,
    SymbolNotAllowed,
    ResubscribeRequiresNewEpoch,
    SubscriptionLimit,
    CommandBudget(BudgetError),
    ReconnectBudget(BudgetError),
    ReconnectExhausted,
    Wire(WireError),
    State(StateError),
    Cancelled,
    SessionProofRequired,
    SessionProofInvalid,
}

#[cfg(test)]
impl From<&MarketStreamError> for ProducerTransportErrorObservation {
    fn from(error: &MarketStreamError) -> Self {
        match error {
            MarketStreamError::Approval(error) => Self::Approval(error.clone()),
            MarketStreamError::EndpointNotAllowed => Self::EndpointNotAllowed,
            MarketStreamError::Io => Self::Io,
            MarketStreamError::Handshake => Self::Handshake,
            MarketStreamError::FrameInvalid => Self::FrameInvalid,
            MarketStreamError::Protocol => Self::Protocol,
            MarketStreamError::Closed => Self::Closed,
            MarketStreamError::CommandPending => Self::CommandPending,
            MarketStreamError::CommandCapabilityMismatch => Self::CommandCapabilityMismatch,
            MarketStreamError::CommandTimingInvalid => Self::CommandTimingInvalid,
            MarketStreamError::CommandTimingOverflow => Self::CommandTimingOverflow,
            MarketStreamError::AckTimeout => Self::AckTimeout,
            MarketStreamError::ControlWriteTimeout => Self::ControlWriteTimeout,
            MarketStreamError::AckMismatch => Self::AckMismatch,
            MarketStreamError::DuplicateAck => Self::DuplicateAck,
            MarketStreamError::CommandRejected => Self::CommandRejected,
            MarketStreamError::DataBeforeAck => Self::DataBeforeAck,
            MarketStreamError::SymbolNotAllowed => Self::SymbolNotAllowed,
            MarketStreamError::ResubscribeRequiresNewEpoch => Self::ResubscribeRequiresNewEpoch,
            MarketStreamError::SubscriptionLimit => Self::SubscriptionLimit,
            MarketStreamError::CommandBudget(error) => Self::CommandBudget(*error),
            MarketStreamError::ReconnectBudget(error) => Self::ReconnectBudget(*error),
            MarketStreamError::ReconnectExhausted => Self::ReconnectExhausted,
            MarketStreamError::Wire(error) => Self::Wire(error.clone()),
            MarketStreamError::State(error) => Self::State(error.clone()),
            MarketStreamError::Cancelled => Self::Cancelled,
            MarketStreamError::SessionProofRequired => Self::SessionProofRequired,
            MarketStreamError::SessionProofInvalid => Self::SessionProofInvalid,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct ProducerTransportErrorProbe {
    last: Arc<Mutex<Option<ProducerTransportErrorObservation>>>,
}

#[cfg(test)]
impl ProducerTransportErrorProbe {
    fn collapse(&self, error: MarketStreamError) -> MarketStreamProducerError {
        *self.last.lock().expect("producer transport probe lock") =
            Some(ProducerTransportErrorObservation::from(&error));
        transport_error(error)
    }

    pub(super) fn take(&self) -> Option<ProducerTransportErrorObservation> {
        self.last
            .lock()
            .expect("producer transport probe lock")
            .take()
    }
}

/// One producer owns the repository fence, epoch lineage, current authentic
/// subscription proofs, and one actual C2 session.
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// let _ = OwnerMarketStreamProducer {};
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// fn require_clone<T: Clone>() {}
/// require_clone::<OwnerMarketStreamProducer>();
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// let _: OwnerMarketStreamProducer = Default::default();
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<OwnerMarketStreamProducer>();
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// fn require_deserialize<T: serde::de::DeserializeOwned>() {}
/// require_deserialize::<OwnerMarketStreamProducer>();
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// fn leak_session(producer: &OwnerMarketStreamProducer) {
///     let _ = producer.session();
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// fn recover(producer: OwnerMarketStreamProducer) {
///     let _ = producer.into_inner();
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamProducer;
/// fn reconnect(producer: &mut OwnerMarketStreamProducer) {
///     let _ = producer.reconnect();
/// }
/// ```
pub struct OwnerMarketStreamProducer {
    repository: OwnerMarketStreamRepository,
    lease: StreamProducerLease,
    epoch: StreamEpochProof,
    subscriptions: BTreeMap<String, SubscriptionProof>,
    session: Option<MarketStreamSession>,
    phase: ProducerPhase,
    #[cfg(test)]
    panic_after_take: bool,
    #[cfg(test)]
    test_gate: Option<ProducerTestGate>,
    #[cfg(test)]
    test_transport_error: ProducerTransportErrorProbe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProducerPhase {
    Ready,
    InFlight,
    Terminal,
}

impl fmt::Debug for OwnerMarketStreamProducer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        ProducerDebugView {
            phase: self.phase,
            subscription_count: self.subscriptions.len(),
        }
        .fmt(formatter)
    }
}

struct ProducerDebugView {
    phase: ProducerPhase,
    subscription_count: usize,
}

impl fmt::Debug for ProducerDebugView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerMarketStreamProducer")
            .field("phase", &self.phase)
            .field("subscription_count", &self.subscription_count)
            .finish()
    }
}

#[cfg(test)]
pub(super) fn debug_synthetic_for_test(subscription_count: usize) -> String {
    format!(
        "{:?}",
        ProducerDebugView {
            phase: ProducerPhase::Ready,
            subscription_count,
        }
    )
}

impl OwnerMarketStreamProducer {
    /// Commit a new database epoch from the actual C2 session epoch. The
    /// session is consumed, cannot carry preexisting subscriptions, and is
    /// dropped on every failed, cancelled, or unknown start outcome.
    pub async fn start(
        repository: OwnerMarketStreamRepository,
        lease: StreamProducerLease,
        session_proof: StreamSessionProof,
        session: MarketStreamSession,
    ) -> Result<Self, MarketStreamProducerError> {
        if session.is_closed() {
            return Err(MarketStreamProducerError::SessionUnavailable);
        }
        if session.subscribed_symbols().next().is_some() {
            return Err(MarketStreamProducerError::SubscriptionOutOfSync);
        }
        let epoch_id = session.epoch().uuid();
        if epoch_id.is_nil() {
            return Err(MarketStreamProducerError::SessionUnavailable);
        }
        let epoch = repository
            .start_stream_epoch(&lease, session_proof, epoch_id)
            .await?;
        if epoch.epoch != epoch_id
            || epoch.credential_slot_id != lease.credential_slot_id
            || epoch.grant_revision != lease.grant_revision
            || epoch.fencing_token != lease.fencing_token
        {
            return Err(MarketStreamProducerError::SessionUnavailable);
        }
        Ok(Self {
            repository,
            lease,
            epoch,
            subscriptions: BTreeMap::new(),
            session: Some(session),
            phase: ProducerPhase::Ready,
            #[cfg(test)]
            panic_after_take: false,
            #[cfg(test)]
            test_gate: None,
            #[cfg(test)]
            test_transport_error: ProducerTransportErrorProbe::default(),
        })
    }

    /// Strict completion API. A spacing deferral returns NotReady, while
    /// preserving the idle session and known committed ACKs. It never reports
    /// partial work as complete; callers needing progress use reconcile_desired.
    pub async fn apply_desired(
        &mut self,
        desired: &DesiredSet,
    ) -> Result<MarketStreamApplyOutcome, MarketStreamProducerError> {
        self.reconcile_desired(desired).await?.into_complete()
    }

    /// Re-read exact current demand on every call. Before each needed command,
    /// check eligibility before changing that symbol's durable desired state.
    /// Deferred returns immediately with no prepared command and no timer.
    /// The owner may process control/data, stop, or later supply fresh demand;
    /// no continuation token or captured command is replayed.
    pub async fn reconcile_desired(
        &mut self,
        desired: &DesiredSet,
    ) -> Result<MarketStreamReconcileOutcome, MarketStreamProducerError> {
        self.ensure_ready()?;
        let supplied = normalize_demand(
            desired,
            self.lease.credential_slot_id,
            self.lease.owner_user_id,
        )?;
        let mut local_session = Some(self.take_session()?);
        #[cfg(test)]
        self.pause_at(ProducerAwaitPoint::DemandRead).await;
        let actual = match self
            .repository
            .read_stream_demand(self.lease.credential_slot_id)
            .await
        {
            Ok(actual) => actual,
            Err(error) => {
                return self
                    .fail_with_session(local_session.take().expect("owned session"), error.into());
            }
        };
        let current = match normalize_demand(
            &actual,
            self.lease.credential_slot_id,
            self.lease.owner_user_id,
        ) {
            Ok(current) => current,
            Err(error) => {
                return self.fail_with_session(local_session.take().expect("owned session"), error);
            }
        };
        if supplied != current {
            self.restore_ready(local_session.take().expect("owned session"))?;
            return Err(MarketStreamProducerError::InvalidInput);
        }
        let desired_by_symbol = demand_by_symbol(&actual.items)?;

        let removed_symbols = self
            .subscriptions
            .keys()
            .filter(|symbol| !desired_by_symbol.contains_key(*symbol))
            .cloned()
            .collect::<Vec<_>>();
        let mut commands = 0_u8;

        // Free old socket slots before adding new ones, keeping the C2 limit
        // even when one complete demand set is replaced by another.
        for symbol in removed_symbols {
            if local_session.is_none() {
                local_session = Some(self.take_session()?);
            }
            let mut session = local_session.take().expect("owned session");
            match session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Unsubscribe, &symbol,
            ) {
                Ok(Some(not_before_ms)) => {
                    return self.defer_reconciliation(session, commands, not_before_ms);
                }
                Ok(None) => {}
                Err(error) => {
                    let error = self.collapse_transport_error(error);
                    return self.fail_with_session(session, error);
                }
            }

            #[cfg(test)]
            self.pause_at(ProducerAwaitPoint::DesiredPersistence).await;
            if let Err(error) = self
                .repository
                .set_subscription_desired(&self.lease, self.epoch.epoch, &symbol, 0)
                .await
            {
                return self.fail_with_session(session, error.into());
            }
            if !session
                .subscribed_symbols()
                .any(|current| current == symbol)
            {
                return self
                    .fail_with_session(session, MarketStreamProducerError::SubscriptionOutOfSync);
            }
            if let Err(error) = self
                .run_command(session, &symbol, MarketSubscriptionOperation::Unsubscribe)
                .await
            {
                return Err(error);
            }
            commands = commands.saturating_add(1);
        }

        for (symbol, reference_count) in &desired_by_symbol {
            if local_session.is_none() {
                local_session = Some(self.take_session()?);
            }
            let mut session = local_session.take().expect("owned session");
            if !self.subscriptions.contains_key(symbol) {
                match session.subscription_command_not_before_ms(
                    MarketSubscriptionOperation::Subscribe, symbol,
                ) {
                    Ok(Some(not_before_ms)) => {
                        return self.defer_reconciliation(session, commands, not_before_ms);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let error = self.collapse_transport_error(error);
                        return self.fail_with_session(session, error);
                    }
                }
            }

            #[cfg(test)]
            self.pause_at(ProducerAwaitPoint::DesiredPersistence).await;
            if let Err(error) = self
                .repository
                .set_subscription_desired(&self.lease, self.epoch.epoch, symbol, *reference_count)
                .await
            {
                return self.fail_with_session(session, error.into());
            }
            if self.subscriptions.contains_key(symbol) {
                if !session
                    .subscribed_symbols()
                    .any(|current| current == symbol)
                {
                    return self.fail_with_session(
                        session,
                        MarketStreamProducerError::SubscriptionOutOfSync,
                    );
                }
                local_session = Some(session);
                continue;
            }
            if session
                .subscribed_symbols()
                .any(|current| current == symbol)
            {
                return self
                    .fail_with_session(session, MarketStreamProducerError::SubscriptionOutOfSync);
            }
            if let Err(error) = self
                .run_command(session, symbol, MarketSubscriptionOperation::Subscribe)
                .await
            {
                return Err(error);
            }
            commands = commands.saturating_add(1);
        }

        if let Some(session) = local_session {
            self.restore_ready(session)?;
        }
        Ok(MarketStreamReconcileOutcome::Complete(MarketStreamApplyOutcome {
            active_symbols: u8::try_from(desired_by_symbol.len())
                .map_err(|_| MarketStreamProducerError::InvalidInput)?,
            transport_commands: commands,
        }))
    }

    /// Read one C2 event and publish a receipt only for the supplied current
    /// admissions matching that receipt's symbol and an authentic owned ACK
    /// proof. Other symbols never enter the publication context.
    pub async fn read_and_publish(
        &mut self,
        admissions: &[StreamIdentity],
    ) -> Result<MarketStreamProducerOutcome, MarketStreamProducerError> {
        self.ensure_ready()?;
        validate_admissions(admissions, self.lease.owner_user_id)?;
        let mut session = self.take_session()?;
        if session.epoch().uuid() != self.epoch.epoch {
            return self.fail_with_session(session, MarketStreamProducerError::SessionUnavailable);
        }
        let event = match session.next_event().await {
            Ok(event) => event,
            Err(error) => {
                let error = self.collapse_transport_error(error);
                return self.fail_with_session(session, error);
            }
        };
        match event {
            MarketStreamEvent::Receipt(receipt) => {
                if receipt.epoch() != self.epoch.epoch
                    || receipt.session_date()
                        != self
                            .epoch
                            .session
                            .session_date
                            .format("%Y%m%d")
                            .to_string()
                            .parse::<u32>()
                            .unwrap_or_default()
                {
                    return self
                        .fail_with_session(session, MarketStreamProducerError::SessionUnavailable);
                }
                let symbol = receipt.observation().symbol.as_str();
                let matching = admissions
                    .iter()
                    .filter(|admission| admission.symbol() == symbol)
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    self.restore_ready(session)?;
                    return Ok(MarketStreamProducerOutcome::Control(
                        MarketStreamControlOutcome::NoCurrentAdmission,
                    ));
                }
                let Some(subscription) = self.subscriptions.get(symbol).cloned() else {
                    return self.fail_with_session(
                        session,
                        MarketStreamProducerError::SubscriptionOutOfSync,
                    );
                };
                let items = matching
                    .into_iter()
                    .map(|admission| StreamPublicationItem {
                        admission: admission.clone(),
                        subscription: subscription.clone(),
                    })
                    .collect::<Vec<_>>();
                let context = match StreamPublicationContext::new(
                    self.lease.clone(),
                    self.epoch.clone(),
                    items,
                ) {
                    Ok(context) => context,
                    Err(error) => return self.fail_with_session(session, error.into()),
                };
                // Keep the one supported conversion path explicit. The
                // repository repeats it under its own validation boundary.
                if StreamQuote::from_receipt(&receipt).is_err() {
                    return self.fail_with_session(
                        session,
                        MarketStreamStorageError::ReceiptInvalid.into(),
                    );
                }
                let observation = StreamPublicationObservation::from_receipt(receipt);
                #[cfg(test)]
                self.pause_at(ProducerAwaitPoint::Publication).await;
                match self
                    .repository
                    .publish_stream_latest(&context, &[observation])
                    .await
                {
                    Ok(committed) => {
                        self.restore_ready(session)?;
                        Ok(MarketStreamProducerOutcome::Published(committed))
                    }
                    Err(error) => self.fail_with_session(session, error.into()),
                }
            }
            MarketStreamEvent::ApplicationHeartbeat => {
                self.restore_ready(session)?;
                Ok(MarketStreamProducerOutcome::Control(
                    MarketStreamControlOutcome::ApplicationHeartbeat,
                ))
            }
            MarketStreamEvent::ControlPong => {
                self.restore_ready(session)?;
                Ok(MarketStreamProducerOutcome::Control(
                    MarketStreamControlOutcome::ControlPong,
                ))
            }
            MarketStreamEvent::Status { code, epoch } => {
                if epoch.uuid() != self.epoch.epoch
                    || code == MarketStreamTransportStatus::Closed
                    || code == MarketStreamTransportStatus::Gap
                    || session.is_closed()
                {
                    self.phase = ProducerPhase::Terminal;
                    drop(session);
                    return Ok(MarketStreamProducerOutcome::Control(
                        MarketStreamControlOutcome::TransportStatus(code),
                    ));
                }
                self.restore_ready(session)?;
                Ok(MarketStreamProducerOutcome::Control(
                    MarketStreamControlOutcome::TransportStatus(code),
                ))
            }
        }
    }

    fn defer_reconciliation(
        &mut self,
        session: MarketStreamSession,
        commands: u8,
        not_before_ms: i64,
    ) -> Result<MarketStreamReconcileOutcome, MarketStreamProducerError> {
        let active_symbols = match u8::try_from(self.subscriptions.len()) {
            Ok(count) => count,
            Err(_) => {
                return self.fail_with_session(session, MarketStreamProducerError::InvalidInput);
            }
        };
        self.restore_ready(session)?;
        Ok(MarketStreamReconcileOutcome::Deferred {
            committed: MarketStreamApplyOutcome {
                active_symbols,
                transport_commands: commands,
            },
            not_before_ms,
        })
    }

    async fn run_command(
        &mut self,
        mut session: MarketStreamSession,
        symbol: &str,
        operation: MarketSubscriptionOperation,
    ) -> Result<(), MarketStreamProducerError> {
        if self.phase != ProducerPhase::InFlight || self.session.is_some() {
            return self.fail_with_session(session, MarketStreamProducerError::Terminal);
        }
        let prepared = match operation {
            MarketSubscriptionOperation::Subscribe => session.prepare_subscribe(symbol),
            MarketSubscriptionOperation::Unsubscribe => session.prepare_unsubscribe(symbol),
        };
        let prepared = match prepared {
            Ok(Some(prepared)) => prepared,
            Ok(None) => {
                return self
                    .fail_with_session(session, MarketStreamProducerError::SubscriptionOutOfSync);
            }
            Err(error) => {
                let error = self.collapse_transport_error(error);
                return self.fail_with_session(session, error);
            }
        };
        if prepared.credential_slot_id() != self.lease.credential_slot_id
            || prepared.epoch().uuid() != self.epoch.epoch
            || prepared.symbol() != symbol
            || prepared.operation() != operation
        {
            return self
                .fail_with_session(session, MarketStreamProducerError::SubscriptionOutOfSync);
        }
        #[cfg(test)]
        self.pause_at(ProducerAwaitPoint::PendingPersistence).await;
        let pending = match self
            .repository
            .commit_prepared_subscription(&self.lease, &prepared)
            .await
        {
            Ok(pending) => pending,
            Err(error) => return self.fail_with_session(session, error.into()),
        };
        let ack = match session.send_prepared(prepared).await {
            Ok(ack) => ack,
            Err(error) => {
                let error = self.collapse_transport_error(error);
                return self.fail_with_session(session, error);
            }
        };
        #[cfg(test)]
        self.pause_at(ProducerAwaitPoint::AckPersistence).await;
        let committed_proof = match self
            .repository
            .commit_subscription_ack(&self.lease, pending, ack)
            .await
        {
            Ok(proof) => proof,
            Err(error) => return self.fail_with_session(session, error.into()),
        };

        // No await is permitted between known ACK commit and restoring the
        // owned session/Ready state.
        match operation {
            MarketSubscriptionOperation::Subscribe => {
                let Some(proof) = committed_proof else {
                    return self.fail_with_session(
                        session,
                        MarketStreamProducerError::SubscriptionOutOfSync,
                    );
                };
                if proof.credential_slot_id != self.lease.credential_slot_id
                    || proof.grant_revision != self.lease.grant_revision
                    || proof.epoch != self.epoch.epoch
                    || proof.symbol != symbol
                    || proof.state != "ACKED"
                    || proof.acked_at.is_none()
                    || proof.desired_reference_count == 0
                {
                    return self.fail_with_session(
                        session,
                        MarketStreamProducerError::SubscriptionOutOfSync,
                    );
                }
                self.subscriptions.insert(symbol.to_owned(), proof);
            }
            MarketSubscriptionOperation::Unsubscribe => {
                if committed_proof.is_some() {
                    return self.fail_with_session(
                        session,
                        MarketStreamProducerError::SubscriptionOutOfSync,
                    );
                }
                self.subscriptions.remove(symbol);
            }
        }
        self.restore_ready(session)
    }

    fn ensure_ready(&mut self) -> Result<(), MarketStreamProducerError> {
        match self.phase {
            ProducerPhase::Ready if self.session.as_ref().is_some_and(|s| !s.is_closed()) => Ok(()),
            ProducerPhase::Ready => {
                self.phase = ProducerPhase::Terminal;
                self.session.take();
                Err(MarketStreamProducerError::SessionUnavailable)
            }
            ProducerPhase::InFlight => {
                self.phase = ProducerPhase::Terminal;
                self.session.take();
                Err(MarketStreamProducerError::Terminal)
            }
            ProducerPhase::Terminal => Err(MarketStreamProducerError::Terminal),
        }
    }

    fn take_session(&mut self) -> Result<MarketStreamSession, MarketStreamProducerError> {
        self.ensure_ready()?;
        let Some(session) = self.session.take() else {
            self.phase = ProducerPhase::Terminal;
            return Err(MarketStreamProducerError::Terminal);
        };
        self.phase = ProducerPhase::InFlight;
        #[cfg(test)]
        if self.panic_after_take {
            self.panic_after_take = false;
            panic!("private C3B panic-after-session-take control");
        }
        Ok(session)
    }

    fn restore_ready(
        &mut self,
        session: MarketStreamSession,
    ) -> Result<(), MarketStreamProducerError> {
        if self.phase != ProducerPhase::InFlight
            || self.session.is_some()
            || session.is_closed()
            || session.epoch().uuid() != self.epoch.epoch
        {
            return self.fail_with_session(session, MarketStreamProducerError::SessionUnavailable);
        }
        self.session = Some(session);
        self.phase = ProducerPhase::Ready;
        Ok(())
    }

    fn fail_with_session<T>(
        &mut self,
        session: MarketStreamSession,
        error: MarketStreamProducerError,
    ) -> Result<T, MarketStreamProducerError> {
        self.phase = ProducerPhase::Terminal;
        self.session = None;
        drop(session);
        Err(error)
    }

    #[cfg(test)]
    pub(super) async fn record_awaiting_first_trade_for_test(
        &mut self,
        identity: &StreamIdentity,
    ) -> Result<CommitResult, MarketStreamProducerError> {
        use super::market_stream::{
            StreamAvailability, StreamConnectionState, StreamFreshness, StreamMarketState,
            StreamStatus, StreamStatusCode,
        };

        self.ensure_ready()?;
        let Some(subscription) = self.subscriptions.get(identity.symbol()) else {
            return Err(MarketStreamProducerError::SubscriptionOutOfSync);
        };
        let context = StreamPublicationContext::single(
            self.lease.clone(),
            self.epoch.clone(),
            identity.clone(),
            subscription.clone(),
        )
        .map_err(MarketStreamProducerError::from)?;
        let status = StreamStatus::new(
            StreamStatusCode::AwaitingFirstTrade,
            StreamConnectionState::Connected,
            StreamMarketState::Open,
            StreamFreshness::Unavailable,
            StreamAvailability::AwaitingFirstTrade,
            false,
            self.epoch.gap_generation,
        )
        .map_err(MarketStreamProducerError::from)?;
        self.repository
            .record_stream_status(&context, status)
            .await
            .map_err(MarketStreamProducerError::from)
    }

    #[cfg(test)]
    pub(super) fn panic_after_next_session_take(&mut self) {
        self.panic_after_take = true;
    }

    #[cfg(test)]
    pub(super) fn install_test_gate(&mut self, gate: ProducerTestGate) {
        self.test_gate = Some(gate);
    }

    #[cfg(test)]
    pub(super) fn transport_error_probe(&self) -> ProducerTransportErrorProbe {
        self.test_transport_error.clone()
    }

    #[cfg(all(test, feature = "market-stream-db-tests"))]
    pub(super) fn command_state_snapshot(
        &self,
    ) -> Result<MarketStreamCommandStateSnapshot, MarketStreamProducerError> {
        self.session
            .as_ref()
            .ok_or(MarketStreamProducerError::NotReady)?
            .test_command_state_snapshot()
            .map_err(|error| self.collapse_transport_error(error))
    }

    #[cfg(test)]
    pub(super) fn make_lease_stale_for_test(&mut self) -> Result<(), MarketStreamProducerError> {
        self.lease.fencing_token = self
            .lease
            .fencing_token
            .checked_add(1)
            .ok_or(MarketStreamProducerError::InvalidInput)?;
        Ok(())
    }

    fn collapse_transport_error(&self, error: MarketStreamError) -> MarketStreamProducerError {
        #[cfg(test)]
        {
            self.test_transport_error.collapse(error)
        }
        #[cfg(not(test))]
        {
            transport_error(error)
        }
    }

    #[cfg(test)]
    async fn pause_at(&self, point: ProducerAwaitPoint) {
        let Some(gate) = self
            .test_gate
            .as_ref()
            .filter(|gate| gate.point == point)
            .cloned()
        else {
            return;
        };
        gate.reached.notify_one();
        gate.resume.notified().await;
    }
}

pub(super) fn normalize_demand(
    demand: &DesiredSet,
    expected_slot: Uuid,
    expected_owner: Uuid,
) -> Result<BTreeMap<StreamIdentity, u32>, MarketStreamProducerError> {
    if demand.credential_slot_id != expected_slot
        || demand.owner_user_id != expected_owner
        || demand.items.len() > STREAM_MAX_ACTIVE_IDENTITIES
    {
        return Err(MarketStreamProducerError::InvalidInput);
    }
    let mut normalized = BTreeMap::new();
    for DesiredStreamItem {
        identity,
        reference_count,
    } in &demand.items
    {
        if identity.owner_user_id != expected_owner
            || *reference_count == 0
            || *reference_count > MAX_REFERENCE_COUNT
            || !approved_symbol(identity.symbol())
        {
            return Err(MarketStreamProducerError::InvalidInput);
        }
        StreamIdentity::new(
            identity.owner_user_id,
            identity.membership_id,
            identity.generation_id,
            identity.instrument_id.clone(),
            identity.generation,
        )
        .map_err(|_| MarketStreamProducerError::InvalidInput)?;
        if normalized
            .insert(identity.clone(), *reference_count)
            .is_some()
        {
            return Err(MarketStreamProducerError::InvalidInput);
        }
    }
    Ok(normalized)
}

fn demand_by_symbol(
    items: &[DesiredStreamItem],
) -> Result<BTreeMap<String, u32>, MarketStreamProducerError> {
    let mut by_symbol = BTreeMap::new();
    for item in items {
        let symbol = item.identity.symbol();
        if !approved_symbol(symbol) {
            return Err(MarketStreamProducerError::InvalidInput);
        }
        let count = by_symbol.entry(symbol.to_owned()).or_insert(0_u32);
        *count = count
            .checked_add(item.reference_count)
            .filter(|sum| *sum <= MAX_REFERENCE_COUNT)
            .ok_or(MarketStreamProducerError::InvalidInput)?;
    }
    Ok(by_symbol)
}

pub(super) fn validate_admissions(
    admissions: &[StreamIdentity],
    expected_owner: Uuid,
) -> Result<(), MarketStreamProducerError> {
    if admissions.is_empty() || admissions.len() > STREAM_MAX_IDENTITIES_PER_LEASE {
        return Err(MarketStreamProducerError::InvalidInput);
    }
    let mut membership_ids = BTreeSet::new();
    for admission in admissions {
        if admission.owner_user_id != expected_owner
            || !approved_symbol(admission.symbol())
            || !membership_ids.insert(admission.membership_id)
        {
            return Err(MarketStreamProducerError::InvalidInput);
        }
        StreamIdentity::new(
            admission.owner_user_id,
            admission.membership_id,
            admission.generation_id,
            admission.instrument_id.clone(),
            admission.generation,
        )
        .map_err(|_| MarketStreamProducerError::InvalidInput)?;
    }
    Ok(())
}

fn approved_symbol(symbol: &str) -> bool {
    symbol.len() == 6
        && symbol.bytes().all(|byte| byte.is_ascii_digit())
        && APPROVED_STREAM_SYMBOLS.contains(&symbol)
}

fn transport_error(_error: MarketStreamError) -> MarketStreamProducerError {
    MarketStreamProducerError::TransportFailure
}

#[cfg(test)]
mod transport_error_observation_unit_tests {
    use super::*;

    #[test]
    fn stale_epoch_error_is_preserved_while_public_error_stays_closed() {
        let probe = ProducerTransportErrorProbe::default();

        assert_eq!(
            probe.collapse(MarketStreamError::Wire(WireError::StaleEpochObservation)),
            MarketStreamProducerError::TransportFailure
        );
        assert_eq!(
            probe.take(),
            Some(ProducerTransportErrorObservation::Wire(
                WireError::StaleEpochObservation
            ))
        );
        assert_eq!(probe.take(), None);
    }

    #[test]
    fn transport_error_observations_are_per_producer_and_consumed_once() {
        let first = ProducerTransportErrorProbe::default();
        let second = ProducerTransportErrorProbe::default();

        assert_eq!(
            first.collapse(MarketStreamError::AckMismatch),
            MarketStreamProducerError::TransportFailure
        );
        assert_eq!(second.take(), None);
        assert_eq!(
            second.collapse(MarketStreamError::Protocol),
            MarketStreamProducerError::TransportFailure
        );
        assert_eq!(
            first.take(),
            Some(ProducerTransportErrorObservation::AckMismatch)
        );
        assert_eq!(
            second.take(),
            Some(ProducerTransportErrorObservation::Protocol)
        );
        assert_eq!(first.take(), None);
        assert_eq!(second.take(), None);
    }
}

mod runtime_buffer {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::time::{Duration, Instant};

    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use kis_client::market_stream_wire::MarketReceipt;
    use market_data::market_stream::StreamQuote;
    use uuid::Uuid;

    use super::super::market_stream::{
        STREAM_MAX_IDENTITIES_PER_LEASE, STREAM_PUBLICATION_MAX_AGE, StreamEpochProof,
        StreamIdentity, StreamProducerLease, StreamPublicationContext, StreamPublicationItem,
        StreamPublicationObservation, SubscriptionProof,
    };

    const MAX_RECEIPT_SLOTS: usize = STREAM_MAX_IDENTITIES_PER_LEASE;
    const INITIAL_COALESCE: Duration = Duration::from_millis(250);
    const MAX_TRANSACTION_STARTS: usize = 4;
    const TRANSACTION_WINDOW: Duration = Duration::from_secs(1);
    const MAX_AGE_CHRONO: ChronoDuration = ChronoDuration::seconds(3);

    /// Closed buffer failures. These labels contain no provider, SQL, identity,
    /// receipt, or credential data.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum BufferError {
        InvalidContext,
        BindingRejected,
        DuplicateSymbol,
        TooManyIdentities,
        ReceiptRejected,
        InvalidCapture,
        ClockInvalid,
        PipelineLag,
        Terminal,
        WriterBusy,
        BatchNotReady,
        StaleBatch,
        BatchNotStarted,
        BatchAlreadyStarted,
        CounterExhausted,
    }

    impl BufferError {
        pub(super) const fn code(self) -> &'static str {
            match self {
                Self::InvalidContext => "MARKET_STREAM_BUFFER_CONTEXT_INVALID",
                Self::BindingRejected => "MARKET_STREAM_BUFFER_BINDING_REJECTED",
                Self::DuplicateSymbol => "MARKET_STREAM_BUFFER_SYMBOL_CONFLICT",
                Self::TooManyIdentities => "MARKET_STREAM_BUFFER_CAPACITY_EXCEEDED",
                Self::ReceiptRejected => "MARKET_STREAM_BUFFER_RECEIPT_REJECTED",
                Self::InvalidCapture => "MARKET_STREAM_BUFFER_CAPTURE_INVALID",
                Self::ClockInvalid => "MARKET_STREAM_BUFFER_CLOCK_INVALID",
                Self::PipelineLag => "MARKET_STREAM_BUFFER_PIPELINE_LAG",
                Self::Terminal => "MARKET_STREAM_BUFFER_TERMINAL",
                Self::WriterBusy => "MARKET_STREAM_BUFFER_WRITER_BUSY",
                Self::BatchNotReady => "MARKET_STREAM_BUFFER_BATCH_NOT_READY",
                Self::StaleBatch => "MARKET_STREAM_BUFFER_BATCH_STALE",
                Self::BatchNotStarted => "MARKET_STREAM_BUFFER_BATCH_NOT_STARTED",
                Self::BatchAlreadyStarted => "MARKET_STREAM_BUFFER_BATCH_ALREADY_STARTED",
                Self::CounterExhausted => "MARKET_STREAM_BUFFER_COUNTER_EXHAUSTED",
            }
        }
    }

    /// Metadata-only outcomes; a lag drop never carries a quote or receipt.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum OfferDisposition {
        Buffered,
        Replaced,
        Stale,
        PipelineLag,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(super) struct BufferDiagnostics {
        pub(super) pending_slots: u8,
        pub(super) high_water_slots: u8,
        pub(super) outstanding_slots: u8,
        pub(super) replaced: u64,
        pub(super) stale: u64,
        pub(super) rejected: u64,
        pub(super) age_lag_drops: u64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum InvalidationCause {
        RightsLost,
        LeaseLost,
        EpochEnded,
        SessionChanged,
        TerminalTransport,
        ContextChanged,
    }

    pub(super) enum BatchAvailability {
        Ready(PublicationBatch),
        Empty,
        Waiting,
        PipelineLag,
    }

    struct ClockReading {
        wall: DateTime<Utc>,
        monotonic: Instant,
    }

    impl ClockReading {
        fn now() -> Self {
            Self {
                wall: Utc::now(),
                monotonic: Instant::now(),
            }
        }
    }

    #[derive(Clone, PartialEq, Eq)]
    struct Capture {
        wall: DateTime<Utc>,
        monotonic: Instant,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CaptureFreshness {
        Fresh,
        Aged,
        Invalid,
    }

    fn capture_freshness(capture: &Capture, now: &ClockReading) -> CaptureFreshness {
        let wall_age = now.wall.signed_duration_since(capture.wall);
        if wall_age < ChronoDuration::zero() {
            return CaptureFreshness::Invalid;
        }
        if wall_age > MAX_AGE_CHRONO {
            return CaptureFreshness::Aged;
        }
        let Some(monotonic_age) = now.monotonic.checked_duration_since(capture.monotonic) else {
            return CaptureFreshness::Invalid;
        };
        if monotonic_age > STREAM_PUBLICATION_MAX_AGE {
            CaptureFreshness::Aged
        } else {
            CaptureFreshness::Fresh
        }
    }

    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct BindingKey {
        identity: StreamIdentity,
        epoch: Uuid,
        subscription_revision: Uuid,
    }

    fn checked_binding_key(
        item: &StreamPublicationItem,
        owner_user_id: Uuid,
        credential_slot_id: Uuid,
        grant_revision: Uuid,
        epoch: Uuid,
    ) -> Result<BindingKey, BufferError> {
        let admission = &item.admission;
        let subscription = &item.subscription;
        if owner_user_id.is_nil()
            || credential_slot_id.is_nil()
            || grant_revision.is_nil()
            || epoch.is_nil()
            || admission.owner_user_id != owner_user_id
            || subscription.credential_slot_id != credential_slot_id
            || subscription.grant_revision != grant_revision
            || subscription.epoch != epoch
            || subscription.state != "ACKED"
            || subscription.acked_at.is_none()
            || subscription.desired_reference_count == 0
            || subscription.subscription_revision.is_nil()
            || subscription.symbol != admission.symbol()
        {
            return Err(BufferError::BindingRejected);
        }
        Ok(BindingKey {
            identity: admission.clone(),
            epoch,
            subscription_revision: subscription.subscription_revision,
        })
    }

    #[derive(Clone, PartialEq, Eq)]
    struct LeaseAuthority {
        owner_user_id: Uuid,
        credential_slot_id: Uuid,
        grant_id: Uuid,
        grant_revision: Uuid,
        holder_id: Uuid,
        fencing_token: u64,
    }

    #[derive(Clone, PartialEq, Eq)]
    struct AuthorityScope {
        lease: LeaseAuthority,
        epoch: StreamEpochProof,
    }

    struct CurrentBinding {
        item: StreamPublicationItem,
        key: BindingKey,
        symbol: String,
    }

    struct StoredReceipt {
        observation: StreamPublicationObservation,
    }

    /// Concrete receipt adapter. It accepts only a transport-minted receipt;
    /// tests exercise its generic, private kernel with non-clone marker values.
    pub(super) struct ReceiptBuffer {
        kernel: LatestKernel<BindingKey, StoredReceipt>,
        scope: Option<AuthorityScope>,
        producer: Option<StreamProducerLease>,
        epoch: Option<StreamEpochProof>,
        bindings: BTreeMap<BindingKey, CurrentBinding>,
        terminal: bool,
    }

    impl ReceiptBuffer {
        pub(super) fn new() -> Self {
            Self {
                kernel: LatestKernel::new(),
                scope: None,
                producer: None,
                epoch: None,
                bindings: BTreeMap::new(),
                terminal: false,
            }
        }

        /// Update the current validated ACK set and accept one actual receipt.
        /// No caller-created quote or timestamp can enter this adapter.
        pub(super) fn record_receipt(
            &mut self,
            context: &StreamPublicationContext,
            receipt: MarketReceipt,
        ) -> Result<OfferDisposition, BufferError> {
            #[cfg(any(test, feature = "market-stream-db-tests"))]
            let measurement_slot = context.producer.credential_slot_id;
            let result = self.record_receipt_inner(context, receipt);
            #[cfg(any(test, feature = "market-stream-db-tests"))]
            {
                let diagnostics = self.kernel.diagnostics();
                let outcome = match &result {
                    Ok(OfferDisposition::Buffered) => {
                        super::super::market_stream_measurements::BufferOfferOutcome::Buffered
                    }
                    Ok(OfferDisposition::Replaced) => {
                        super::super::market_stream_measurements::BufferOfferOutcome::Replaced
                    }
                    Ok(OfferDisposition::Stale) => {
                        super::super::market_stream_measurements::BufferOfferOutcome::Stale
                    }
                    Ok(OfferDisposition::PipelineLag) => {
                        super::super::market_stream_measurements::BufferOfferOutcome::OfferAgeLag
                    }
                    Err(_) => {
                        super::super::market_stream_measurements::BufferOfferOutcome::Rejected
                    }
                };
                super::super::market_stream_measurements::record_buffer_offer(
                    measurement_slot,
                    outcome,
                    diagnostics.pending_slots,
                    diagnostics.high_water_slots,
                    diagnostics.outstanding_slots,
                );
            }
            result
        }

        fn record_receipt_inner(
            &mut self,
            context: &StreamPublicationContext,
            receipt: MarketReceipt,
        ) -> Result<OfferDisposition, BufferError> {
            let now = ClockReading::now();
            self.refresh_context(context, &now.wall)?;
            let symbol = receipt.observation().symbol.as_str();
            let mut matches = self
                .bindings
                .values()
                .filter(|binding| binding.symbol == symbol);
            let Some(binding) = matches.next() else {
                return self.reject(BufferError::ReceiptRejected);
            };
            if matches.next().is_some() {
                self.invalidate(InvalidationCause::ContextChanged);
                return Err(BufferError::DuplicateSymbol);
            }

            let expected_date = self
                .scope
                .as_ref()
                .map(|scope| scope.epoch.session.session_date)
                .ok_or(BufferError::InvalidContext)?;
            let Some(expected_wire_date) = expected_date
                .format("%Y%m%d")
                .to_string()
                .parse::<u32>()
                .ok()
            else {
                return self.reject(BufferError::InvalidContext);
            };
            if receipt.epoch() != binding.key.epoch
                || receipt.session_date() != expected_wire_date
                || receipt.observation().business_date != expected_wire_date
            {
                return self.reject(BufferError::ReceiptRejected);
            }
            if StreamQuote::from_receipt(&receipt).is_err() {
                return self.reject(BufferError::ReceiptRejected);
            }
            let Some(wall) = DateTime::<Utc>::from_timestamp_millis(receipt.received_at_ms())
            else {
                return self.reject(BufferError::InvalidCapture);
            };
            let capture = Capture {
                wall,
                monotonic: receipt.received_monotonic(),
            };
            let ordinal = receipt.receive_ordinal();
            let key = binding.key.clone();
            self.kernel.offer(
                key,
                ordinal,
                capture,
                &now,
                StoredReceipt {
                    observation: StreamPublicationObservation::from_receipt(receipt),
                },
            )
        }

        /// Remove all demand data while preserving only the global current
        /// epoch scope. Reintroduced bindings receive new kernel incarnations.
        pub(super) fn clear_zero_demand(&mut self) -> Result<(), BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            self.kernel.set_current(Vec::new())?;
            self.bindings.clear();
            Ok(())
        }

        /// Invalidate one identity after unsubscribe, pending ACK, or a
        /// committed revision replacement. An outstanding batch stays busy
        /// and becomes stale; its contents are never replayed.
        pub(super) fn invalidate_identity(
            &mut self,
            identity: &StreamIdentity,
        ) -> Result<(), BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            let removed = self
                .bindings
                .keys()
                .filter(|key| &key.identity == identity)
                .cloned()
                .collect::<Vec<_>>();
            for key in &removed {
                self.bindings.remove(key);
            }
            let remaining = self.bindings.keys().cloned().collect();
            self.kernel.set_current(remaining)
        }

        /// Rights, lease, epoch, day, and terminal transport loss close this
        /// buffer. A new epoch requires constructing a fresh buffer.
        pub(super) fn invalidate(&mut self, _cause: InvalidationCause) {
            self.terminal = true;
            self.bindings.clear();
            self.scope = None;
            self.producer = None;
            self.epoch = None;
            self.kernel.invalidate();
        }

        /// Move at most thirty fresh slots into the single outstanding batch.
        /// The writer must call `mark_transaction_start` immediately before
        /// the repository publish call, then `complete_success` only on known
        /// success. A dropped or failed batch intentionally leaves this buffer
        /// busy and cannot be retried.
        pub(super) fn handoff_batch(&mut self) -> Result<BatchAvailability, BufferError> {
            #[cfg(any(test, feature = "market-stream-db-tests"))]
            let measurement = self.producer.as_ref().map(|lease| {
                (
                    lease.credential_slot_id,
                    self.kernel.diagnostics().age_lag_drops,
                )
            });
            let result = self.handoff_batch_inner();
            #[cfg(any(test, feature = "market-stream-db-tests"))]
            {
                if let Some((slot, age_before)) = measurement {
                    let diagnostics = self.kernel.diagnostics();
                    super::super::market_stream_measurements::record_buffer_handoff(
                        slot,
                        age_before,
                        diagnostics.age_lag_drops,
                        diagnostics.pending_slots,
                        diagnostics.high_water_slots,
                        diagnostics.outstanding_slots,
                    );
                }
            }
            result
        }

        fn handoff_batch_inner(&mut self) -> Result<BatchAvailability, BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            let now = ClockReading::now();
            let result = self.kernel.handoff(&now)?;
            let batch = match result {
                KernelHandoff::Empty => return Ok(BatchAvailability::Empty),
                KernelHandoff::Waiting => return Ok(BatchAvailability::Waiting),
                KernelHandoff::PipelineLag => return Ok(BatchAvailability::PipelineLag),
                KernelHandoff::Ready(batch) => batch,
            };

            let producer = self.producer.clone().ok_or(BufferError::InvalidContext)?;
            let epoch = self.epoch.clone().ok_or(BufferError::InvalidContext)?;
            let scope = self.scope.clone().ok_or(BufferError::InvalidContext)?;
            let mut items = Vec::with_capacity(batch.guard.slots.len());
            for slot in &batch.guard.slots {
                let Some(binding) = self.bindings.get(&slot.key) else {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::StaleBatch);
                };
                items.push(binding.item.clone());
            }
            let context = match StreamPublicationContext::new(producer, epoch, items) {
                Ok(context) => context,
                Err(_) => {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::InvalidContext);
                }
            };
            for stored in &batch.values {
                if StreamQuote::from_receipt(stored.observation.receipt()).is_err() {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::ReceiptRejected);
                }
            }
            let (guard, stored) = batch.into_parts();
            let observations = stored
                .into_iter()
                .map(|stored| stored.observation)
                .collect();
            Ok(BatchAvailability::Ready(PublicationBatch {
                guard,
                context,
                observations,
                scope,
            }))
        }

        pub(super) fn batch_is_current(&self, batch: &PublicationBatch) -> bool {
            !self.terminal
                && self.scope.as_ref() == Some(&batch.scope)
                && self.kernel.guard_is_current(&batch.guard)
        }

        /// Call synchronously at the writer's actual transaction-start
        /// boundary, with no intervening await before repository publication.
        pub(super) fn mark_transaction_start(
            &mut self,
            batch: &PublicationBatch,
        ) -> Result<(), BufferError> {
            if !self.batch_is_current(batch) {
                return Err(BufferError::StaleBatch);
            }
            self.kernel
                .mark_transaction_start(&batch.guard, &ClockReading::now())
        }

        /// A scheduling acknowledgement only; D2 may call it solely after a
        /// known successful publication. It is not commit evidence itself.
        pub(super) fn complete_success(
            &mut self,
            batch: PublicationBatch,
        ) -> Result<(), BufferError> {
            if !self.batch_is_current(&batch) {
                return Err(BufferError::StaleBatch);
            }
            self.kernel.complete_success(batch.guard)
        }

        pub(super) fn diagnostics(&self) -> BufferDiagnostics {
            self.kernel.diagnostics()
        }

        /// Refresh only the existing validated runtime binding context.
        pub(super) fn refresh_runtime_context(
            &mut self,
            context: &StreamPublicationContext,
        ) -> Result<(), BufferError> {
            self.refresh_context(context, &Utc::now())
        }

        fn refresh_context(
            &mut self,
            context: &StreamPublicationContext,
            now: &DateTime<Utc>,
        ) -> Result<(), BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            let validated = match StreamPublicationContext::new(
                context.producer.clone(),
                context.epoch.clone(),
                context.items.clone(),
            ) {
                Ok(context) => context,
                Err(_) => {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::InvalidContext);
                }
            };
            if validated.producer.lease_expires_at <= *now {
                self.invalidate(InvalidationCause::LeaseLost);
                return Err(BufferError::BindingRejected);
            }
            let lease = &validated.producer;
            if lease.owner_user_id.is_nil()
                || lease.credential_slot_id.is_nil()
                || lease.grant_id.is_nil()
                || lease.grant_revision.is_nil()
                || lease.holder_id.is_nil()
                || lease.fencing_token == 0
                || validated.epoch.credential_slot_id != lease.credential_slot_id
                || validated.epoch.grant_revision != lease.grant_revision
                || validated.epoch.fencing_token != lease.fencing_token
                || validated.epoch.epoch.is_nil()
            {
                self.invalidate(InvalidationCause::ContextChanged);
                return Err(BufferError::InvalidContext);
            }
            let next_scope = AuthorityScope {
                lease: LeaseAuthority {
                    owner_user_id: lease.owner_user_id,
                    credential_slot_id: lease.credential_slot_id,
                    grant_id: lease.grant_id,
                    grant_revision: lease.grant_revision,
                    holder_id: lease.holder_id,
                    fencing_token: lease.fencing_token,
                },
                epoch: validated.epoch.clone(),
            };
            if let Some(current) = &self.scope {
                if current != &next_scope {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::BindingRejected);
                }
            }

            let mut next = BTreeMap::new();
            let mut symbols = BTreeSet::new();
            for item in &validated.items {
                let key = match checked_binding_key(
                    item,
                    lease.owner_user_id,
                    lease.credential_slot_id,
                    lease.grant_revision,
                    validated.epoch.epoch,
                ) {
                    Ok(key) => key,
                    Err(error) => {
                        self.kernel.note_rejected();
                        self.invalidate(InvalidationCause::ContextChanged);
                        return Err(error);
                    }
                };
                let symbol = item.admission.symbol().to_owned();
                if !symbols.insert(symbol.clone()) {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::DuplicateSymbol);
                }
                if let Some(old) = self.bindings.get(&key) {
                    if old.item.subscription.acked_at != item.subscription.acked_at {
                        self.kernel.rotate_binding(&key)?;
                    }
                }
                if next
                    .insert(
                        key.clone(),
                        CurrentBinding {
                            item: item.clone(),
                            key,
                            symbol,
                        },
                    )
                    .is_some()
                {
                    self.invalidate(InvalidationCause::ContextChanged);
                    return Err(BufferError::BindingRejected);
                }
            }
            if next.len() > MAX_RECEIPT_SLOTS {
                self.invalidate(InvalidationCause::ContextChanged);
                return Err(BufferError::TooManyIdentities);
            }
            let current_keys = next.keys().cloned().collect();
            if let Err(error) = self.kernel.set_current(current_keys) {
                self.invalidate(InvalidationCause::ContextChanged);
                return Err(error);
            }
            self.bindings = next;
            self.scope = Some(next_scope);
            self.producer = Some(validated.producer);
            self.epoch = Some(validated.epoch);
            Ok(())
        }

        fn reject<T>(&mut self, error: BufferError) -> Result<T, BufferError> {
            self.kernel.note_rejected();
            Err(error)
        }
    }

    pub(super) struct PublicationBatch {
        guard: BatchGuard<BindingKey>,
        context: StreamPublicationContext,
        observations: Vec<StreamPublicationObservation>,
        scope: AuthorityScope,
    }

    impl PublicationBatch {
        pub(super) fn context(&self) -> &StreamPublicationContext {
            &self.context
        }

        pub(super) fn observations(&self) -> &[StreamPublicationObservation] {
            &self.observations
        }

        pub(super) fn len(&self) -> usize {
            self.observations.len()
        }
    }

    struct KernelEntry<P> {
        incarnation: u64,
        ordinal: u64,
        capture: Capture,
        queued_at: Instant,
        value: P,
    }

    struct GuardSlot<K> {
        key: K,
        incarnation: u64,
        ordinal: u64,
        capture: Capture,
    }

    impl<K: PartialEq> PartialEq for GuardSlot<K> {
        fn eq(&self, other: &Self) -> bool {
            self.key == other.key
                && self.incarnation == other.incarnation
                && self.ordinal == other.ordinal
                && self.capture == other.capture
        }
    }

    impl<K: Eq> Eq for GuardSlot<K> {}

    struct BatchGuard<K> {
        id: u64,
        slots: Vec<GuardSlot<K>>,
    }

    struct Outstanding<K> {
        id: u64,
        slots: Vec<GuardSlot<K>>,
        started: bool,
    }

    struct KernelBatch<K, P> {
        guard: BatchGuard<K>,
        values: Vec<P>,
    }

    impl<K, P> KernelBatch<K, P> {
        fn into_parts(self) -> (BatchGuard<K>, Vec<P>) {
            (self.guard, self.values)
        }
    }

    enum KernelHandoff<K, P> {
        Ready(KernelBatch<K, P>),
        Empty,
        Waiting,
        PipelineLag,
    }

    #[derive(Default)]
    struct Counters {
        replaced: u64,
        stale: u64,
        rejected: u64,
        age_lag_drops: u64,
    }

    struct LatestKernel<K: Ord, P> {
        current: BTreeMap<K, u64>,
        pending: BTreeMap<K, KernelEntry<P>>,
        high_water: BTreeMap<K, u64>,
        next_incarnation: u64,
        next_batch_id: u64,
        outstanding: Option<Outstanding<K>>,
        cadence: PublicationCadence,
        pending_since: Option<Instant>,
        counters: Counters,
        terminal: bool,
    }

    impl<K: Ord + Clone + PartialEq, P> LatestKernel<K, P> {
        fn new() -> Self {
            Self {
                current: BTreeMap::new(),
                pending: BTreeMap::new(),
                high_water: BTreeMap::new(),
                next_incarnation: 1,
                next_batch_id: 1,
                outstanding: None,
                cadence: PublicationCadence::new(),
                pending_since: None,
                counters: Counters::default(),
                terminal: false,
            }
        }

        fn set_current(&mut self, keys: Vec<K>) -> Result<(), BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            if keys.len() > MAX_RECEIPT_SLOTS {
                self.note_rejected();
                return Err(BufferError::TooManyIdentities);
            }
            let next = keys.iter().cloned().collect::<BTreeSet<_>>();
            if next.len() != keys.len() {
                self.note_rejected();
                return Err(BufferError::BindingRejected);
            }
            let removed = self
                .current
                .keys()
                .filter(|key| !next.contains(*key))
                .cloned()
                .collect::<Vec<_>>();
            for key in removed {
                self.current.remove(&key);
                self.pending.remove(&key);
                self.high_water.remove(&key);
            }
            for key in next {
                if !self.current.contains_key(&key) {
                    let Some(next_incarnation) = self.next_incarnation.checked_add(1) else {
                        self.invalidate();
                        return Err(BufferError::CounterExhausted);
                    };
                    let incarnation = self.next_incarnation;
                    self.next_incarnation = next_incarnation;
                    self.current.insert(key, incarnation);
                }
            }
            self.recompute_pending_since();
            Ok(())
        }

        fn rotate_binding(&mut self, key: &K) -> Result<(), BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            self.current.remove(key);
            self.pending.remove(key);
            self.high_water.remove(key);
            self.recompute_pending_since();
            Ok(())
        }

        fn offer(
            &mut self,
            key: K,
            ordinal: u64,
            capture: Capture,
            now: &ClockReading,
            value: P,
        ) -> Result<OfferDisposition, BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            let Some(&incarnation) = self.current.get(&key) else {
                self.note_rejected();
                return Err(BufferError::BindingRejected);
            };
            if ordinal == 0 {
                self.note_rejected();
                return Err(BufferError::ReceiptRejected);
            }
            if self
                .high_water
                .get(&key)
                .is_some_and(|previous| ordinal <= *previous)
            {
                self.counters.stale = self.counters.stale.saturating_add(1);
                return Ok(OfferDisposition::Stale);
            }
            self.high_water.insert(key.clone(), ordinal);
            match capture_freshness(&capture, now) {
                CaptureFreshness::Fresh => {}
                CaptureFreshness::Aged => {
                    self.counters.age_lag_drops = self.counters.age_lag_drops.saturating_add(1);
                    return Ok(OfferDisposition::PipelineLag);
                }
                CaptureFreshness::Invalid => {
                    self.note_rejected();
                    return Err(BufferError::InvalidCapture);
                }
            }
            let replaced = self.pending.contains_key(&key);
            let queued_at = self
                .pending
                .get(&key)
                .map_or(now.monotonic, |entry| entry.queued_at);
            let entry = KernelEntry {
                incarnation,
                ordinal,
                capture,
                queued_at,
                value,
            };
            self.pending.insert(key, entry);
            if replaced {
                self.counters.replaced = self.counters.replaced.saturating_add(1);
            }
            self.recompute_pending_since();
            Ok(if replaced {
                OfferDisposition::Replaced
            } else {
                OfferDisposition::Buffered
            })
        }

        fn handoff(&mut self, now: &ClockReading) -> Result<KernelHandoff<K, P>, BufferError> {
            if self.terminal {
                return Err(BufferError::Terminal);
            }
            if self.outstanding.is_some() {
                return Err(BufferError::WriterBusy);
            }
            if self.pending.is_empty() {
                return Ok(KernelHandoff::Empty);
            }

            let mut aged = Vec::new();
            for (key, entry) in &self.pending {
                match capture_freshness(&entry.capture, now) {
                    CaptureFreshness::Fresh => {}
                    CaptureFreshness::Aged => aged.push(key.clone()),
                    CaptureFreshness::Invalid => {
                        self.note_rejected();
                        self.invalidate();
                        return Err(BufferError::ClockInvalid);
                    }
                }
            }
            for key in &aged {
                self.pending.remove(key);
            }
            if !aged.is_empty() {
                self.counters.age_lag_drops = self
                    .counters
                    .age_lag_drops
                    .saturating_add(aged.len() as u64);
            }
            self.recompute_pending_since();
            if self.pending.is_empty() {
                return Ok(if aged.is_empty() {
                    KernelHandoff::Empty
                } else {
                    KernelHandoff::PipelineLag
                });
            }
            let Some(pending_since) = self.pending_since else {
                self.invalidate();
                return Err(BufferError::ClockInvalid);
            };
            if !self.cadence.handoff_ready(pending_since, now.monotonic)? {
                return Ok(KernelHandoff::Waiting);
            }

            let id = self.next_batch_id;
            let Some(next_batch_id) = self.next_batch_id.checked_add(1) else {
                self.invalidate();
                return Err(BufferError::CounterExhausted);
            };
            self.next_batch_id = next_batch_id;
            let mut slots = Vec::with_capacity(self.pending.len());
            let mut values = Vec::with_capacity(self.pending.len());
            for (key, entry) in std::mem::take(&mut self.pending) {
                slots.push(GuardSlot {
                    key: key.clone(),
                    incarnation: entry.incarnation,
                    ordinal: entry.ordinal,
                    capture: entry.capture,
                });
                values.push(entry.value);
            }
            self.outstanding = Some(Outstanding {
                id,
                slots: slots
                    .iter()
                    .map(|slot| GuardSlot {
                        key: slot.key.clone(),
                        incarnation: slot.incarnation,
                        ordinal: slot.ordinal,
                        capture: slot.capture.clone(),
                    })
                    .collect(),
                started: false,
            });
            self.pending_since = None;
            Ok(KernelHandoff::Ready(KernelBatch {
                guard: BatchGuard { id, slots },
                values,
            }))
        }

        fn guard_is_current(&self, guard: &BatchGuard<K>) -> bool {
            if self.terminal {
                return false;
            }
            let Some(outstanding) = &self.outstanding else {
                return false;
            };
            outstanding.id == guard.id
                && outstanding.slots == guard.slots
                && guard
                    .slots
                    .iter()
                    .all(|slot| self.current.get(&slot.key) == Some(&slot.incarnation))
        }

        fn mark_transaction_start(
            &mut self,
            guard: &BatchGuard<K>,
            now: &ClockReading,
        ) -> Result<(), BufferError> {
            if !self.guard_is_current(guard) {
                return Err(BufferError::StaleBatch);
            }
            if self
                .outstanding
                .as_ref()
                .is_some_and(|outstanding| outstanding.started)
            {
                return Err(BufferError::BatchAlreadyStarted);
            }
            let mut aged = 0usize;
            for slot in &guard.slots {
                match capture_freshness(&slot.capture, now) {
                    CaptureFreshness::Fresh => {}
                    CaptureFreshness::Aged => aged += 1,
                    CaptureFreshness::Invalid => {
                        self.note_rejected();
                        self.invalidate();
                        return Err(BufferError::ClockInvalid);
                    }
                }
            }
            if aged > 0 {
                self.counters.age_lag_drops =
                    self.counters.age_lag_drops.saturating_add(aged as u64);
                self.invalidate();
                return Err(BufferError::PipelineLag);
            }
            let start_allowed = match self.cadence.start_allowed(now.monotonic) {
                Ok(allowed) => allowed,
                Err(error) => {
                    self.note_rejected();
                    self.invalidate();
                    return Err(error);
                }
            };
            if !start_allowed {
                return Err(BufferError::BatchNotReady);
            }
            self.cadence.record_start(now.monotonic);
            if let Some(outstanding) = &mut self.outstanding {
                outstanding.started = true;
            }
            Ok(())
        }

        fn complete_success(&mut self, guard: BatchGuard<K>) -> Result<(), BufferError> {
            if !self.guard_is_current(&guard) {
                return Err(BufferError::StaleBatch);
            }
            if !self
                .outstanding
                .as_ref()
                .is_some_and(|outstanding| outstanding.started)
            {
                return Err(BufferError::BatchNotStarted);
            }
            self.outstanding = None;
            Ok(())
        }

        fn invalidate(&mut self) {
            self.terminal = true;
            self.current.clear();
            self.pending.clear();
            self.high_water.clear();
            self.pending_since = None;
        }

        fn note_rejected(&mut self) {
            self.counters.rejected = self.counters.rejected.saturating_add(1);
        }

        fn diagnostics(&self) -> BufferDiagnostics {
            BufferDiagnostics {
                pending_slots: self.pending.len().min(u8::MAX as usize) as u8,
                high_water_slots: self.high_water.len().min(u8::MAX as usize) as u8,
                outstanding_slots: self.outstanding.as_ref().map_or(0, |outstanding| {
                    outstanding.slots.len().min(u8::MAX as usize) as u8
                }),
                replaced: self.counters.replaced,
                stale: self.counters.stale,
                rejected: self.counters.rejected,
                age_lag_drops: self.counters.age_lag_drops,
            }
        }

        fn recompute_pending_since(&mut self) {
            self.pending_since = self.pending.values().map(|entry| entry.queued_at).min();
        }
    }

    struct PublicationCadence {
        starts: VecDeque<Instant>,
        last_start: Option<Instant>,
    }

    impl PublicationCadence {
        fn new() -> Self {
            Self {
                starts: VecDeque::with_capacity(MAX_TRANSACTION_STARTS),
                last_start: None,
            }
        }

        fn handoff_ready(
            &mut self,
            pending_since: Instant,
            now: Instant,
        ) -> Result<bool, BufferError> {
            let Some(waited) = now.checked_duration_since(pending_since) else {
                return Err(BufferError::ClockInvalid);
            };
            if waited < INITIAL_COALESCE {
                return Ok(false);
            }
            self.start_allowed(now)
        }

        fn start_allowed(&mut self, now: Instant) -> Result<bool, BufferError> {
            if let Some(last) = self.last_start {
                let Some(spaced) = now.checked_duration_since(last) else {
                    return Err(BufferError::ClockInvalid);
                };
                if spaced < INITIAL_COALESCE {
                    return Ok(false);
                }
            }
            while let Some(first) = self.starts.front().copied() {
                let Some(age) = now.checked_duration_since(first) else {
                    return Err(BufferError::ClockInvalid);
                };
                if age < TRANSACTION_WINDOW {
                    break;
                }
                self.starts.pop_front();
            }
            Ok(self.starts.len() < MAX_TRANSACTION_STARTS)
        }

        fn record_start(&mut self, now: Instant) {
            self.starts.push_back(now);
            self.last_start = Some(now);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::cell::Cell;
        use std::rc::Rc;

        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
        struct TestKey {
            identity: u8,
            epoch: u8,
            revision: u8,
        }

        struct Marker {
            id: u32,
            drops: Rc<Cell<usize>>,
        }

        impl Marker {
            fn new(id: u32, drops: &Rc<Cell<usize>>) -> Self {
                Self {
                    id,
                    drops: Rc::clone(drops),
                }
            }
        }

        impl Drop for Marker {
            fn drop(&mut self) {
                self.drops.set(self.drops.get().saturating_add(1));
            }
        }

        fn wall(ms: i64) -> DateTime<Utc> {
            DateTime::<Utc>::from_timestamp_millis(ms).unwrap()
        }

        fn key(identity: u8, epoch: u8, revision: u8) -> TestKey {
            TestKey {
                identity,
                epoch,
                revision,
            }
        }

        fn reading(base: Instant, wall_ms: i64, offset_ms: u64) -> ClockReading {
            ClockReading {
                wall: wall(wall_ms),
                monotonic: base + Duration::from_millis(offset_ms),
            }
        }

        fn active_kernel(keys: Vec<TestKey>) -> LatestKernel<TestKey, Marker> {
            let mut kernel = LatestKernel::new();
            kernel.set_current(keys).unwrap();
            kernel
        }

        #[test]
        fn latest_ordinal_and_capture_are_preserved() {
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let drops = Rc::new(Cell::new(0));
            let slot = key(1, 7, 11);
            let mut kernel = active_kernel(vec![slot.clone()]);
            assert_eq!(
                kernel.offer(
                    slot.clone(),
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(1, &drops),
                ),
                Ok(OfferDisposition::Buffered)
            );
            let latest_capture = Capture {
                wall: wall(wall0 + 100),
                monotonic: base + Duration::from_millis(100),
            };
            assert_eq!(
                kernel.offer(
                    slot.clone(),
                    2,
                    latest_capture.clone(),
                    &reading(base, wall0 + 100, 100),
                    Marker::new(2, &drops),
                ),
                Ok(OfferDisposition::Replaced)
            );
            assert_eq!(drops.get(), 1);
            let entry = kernel.pending.get(&slot).unwrap();
            assert_eq!(entry.ordinal, 2);
            assert!(entry.capture == latest_capture);
            assert_eq!(entry.value.id, 2);
            let KernelHandoff::Ready(drained) =
                kernel.handoff(&reading(base, wall0 + 350, 350)).unwrap()
            else {
                panic!("expected the buffered observation to drain");
            };
            assert_eq!(drained.guard.slots.len(), 1);
            assert_eq!(drained.guard.slots[0].ordinal, 2);
            assert!(drained.guard.slots[0].capture == latest_capture);
            assert_eq!(
                kernel.offer(
                    slot.clone(),
                    2,
                    Capture {
                        wall: wall(wall0 + 360),
                        monotonic: base + Duration::from_millis(360),
                    },
                    &reading(base, wall0 + 360, 360),
                    Marker::new(3, &drops),
                ),
                Ok(OfferDisposition::Stale)
            );
            assert!(kernel.pending.is_empty());
            assert_eq!(kernel.high_water.get(&slot), Some(&2));
            assert_eq!(kernel.counters.replaced, 1);
            assert_eq!(kernel.counters.stale, 1);
            drop(drained);
        }

        #[test]
        fn current_identity_epoch_and_ack_revision_are_required() {
            let owner = Uuid::from_u128(1);
            let slot = Uuid::from_u128(2);
            let grant_revision = Uuid::from_u128(3);
            let epoch = Uuid::from_u128(4);
            let subscription_revision = Uuid::from_u128(5);
            let identity = StreamIdentity::new(
                owner,
                Uuid::from_u128(6),
                Uuid::from_u128(7),
                "005930.KRX".to_owned(),
                1,
            )
            .unwrap();
            let acked_at = wall(1_798_000_000_000);
            let subscription = SubscriptionProof {
                credential_slot_id: slot,
                symbol: "005930".to_owned(),
                grant_revision,
                epoch,
                state: "ACKED".to_owned(),
                subscription_revision,
                acked_at: Some(acked_at),
                desired_reference_count: 1,
            };
            let valid = checked_binding_key(
                &&StreamPublicationItem {
                    admission: identity.clone(),
                    subscription: subscription.clone(),
                },
                owner,
                slot,
                grant_revision,
                epoch,
            )
            .unwrap();
            assert_eq!(valid.identity, identity);
            assert_eq!(valid.epoch, epoch);
            assert_eq!(valid.subscription_revision, subscription_revision);
            let mut positive_count = subscription.clone();
            positive_count.desired_reference_count = 7;
            let same_binding = checked_binding_key(
                &StreamPublicationItem {
                    admission: identity.clone(),
                    subscription: positive_count,
                },
                owner,
                slot,
                grant_revision,
                epoch,
            )
            .unwrap();
            assert!(same_binding == valid);

            // A positive reference-count update leaves the key (identity,
            // epoch, committed revision) intact, so its ordinal high-water
            // survives. A changed revision or identity creates a new binding.
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let drops = Rc::new(Cell::new(0));
            let mut kernel = active_kernel(vec![key(1, 4, 5)]);
            let kernel_key = key(1, 4, 5);
            kernel
                .offer(
                    kernel_key.clone(),
                    9,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(9, &drops),
                )
                .unwrap();
            kernel.set_current(vec![kernel_key.clone()]).unwrap();
            assert_eq!(kernel.high_water.get(&kernel_key), Some(&9));
            assert_eq!(kernel.pending.len(), 1);
            let revised = key(1, 4, 6);
            kernel.set_current(vec![revised.clone()]).unwrap();
            assert!(kernel.high_water.is_empty());
            assert!(kernel.pending.is_empty());
            assert!(
                kernel
                    .offer(
                        kernel_key,
                        10,
                        Capture {
                            wall: wall(wall0),
                            monotonic: base,
                        },
                        &reading(base, wall0, 0),
                        Marker::new(10, &drops),
                    )
                    .is_err()
            );
            kernel
                .offer(
                    revised.clone(),
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(11, &drops),
                )
                .unwrap();
            assert_eq!(kernel.pending.len(), 1);
            let changed_identity = key(2, 4, 6);
            kernel.set_current(vec![changed_identity]).unwrap();
            assert!(kernel.pending.is_empty());
            assert!(kernel.high_water.is_empty());

            let mut pending = subscription.clone();
            pending.state = "PENDING".to_owned();
            assert!(matches!(
                checked_binding_key(
                    &StreamPublicationItem {
                        admission: identity.clone(),
                        subscription: pending,
                    },
                    owner,
                    slot,
                    grant_revision,
                    epoch,
                ),
                Err(BufferError::BindingRejected)
            ));
            let mut missing_ack = subscription.clone();
            missing_ack.acked_at = None;
            assert!(matches!(
                checked_binding_key(
                    &StreamPublicationItem {
                        admission: identity.clone(),
                        subscription: missing_ack,
                    },
                    owner,
                    slot,
                    grant_revision,
                    epoch,
                ),
                Err(BufferError::BindingRejected)
            ));
            let mut wrong_revision = subscription.clone();
            wrong_revision.subscription_revision = Uuid::from_u128(8);
            assert_ne!(
                checked_binding_key(
                    &StreamPublicationItem {
                        admission: identity.clone(),
                        subscription: wrong_revision,
                    },
                    owner,
                    slot,
                    grant_revision,
                    epoch,
                )
                .unwrap()
                .subscription_revision,
                valid.subscription_revision
            );
            let mut wrong_epoch = subscription.clone();
            wrong_epoch.epoch = Uuid::from_u128(9);
            assert!(matches!(
                checked_binding_key(
                    &StreamPublicationItem {
                        admission: identity.clone(),
                        subscription: wrong_epoch,
                    },
                    owner,
                    slot,
                    grant_revision,
                    epoch,
                ),
                Err(BufferError::BindingRejected)
            ));
            assert!(matches!(
                checked_binding_key(
                    &StreamPublicationItem {
                        admission: identity.clone(),
                        subscription: subscription.clone(),
                    },
                    Uuid::from_u128(10),
                    slot,
                    grant_revision,
                    epoch,
                ),
                Err(BufferError::BindingRejected)
            ));
            let changed_identity = StreamIdentity::new(
                Uuid::from_u128(10),
                identity.membership_id,
                identity.generation_id,
                identity.instrument_id.clone(),
                identity.generation,
            )
            .unwrap();
            assert!(matches!(
                checked_binding_key(
                    &StreamPublicationItem {
                        admission: changed_identity,
                        subscription,
                    },
                    owner,
                    slot,
                    grant_revision,
                    epoch,
                ),
                Err(BufferError::BindingRejected)
            ));
        }

        #[test]
        fn buffer_bounds_hold_under_hot_symbol_load() {
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let keys = (0..MAX_RECEIPT_SLOTS)
                .map(|id| key(id as u8, 7, 11))
                .collect::<Vec<_>>();
            let mut kernel = active_kernel(keys.clone());
            let drops = Rc::new(Cell::new(0));
            for (index, slot) in keys.iter().cloned().enumerate() {
                assert_eq!(
                    kernel.offer(
                        slot,
                        1,
                        Capture {
                            wall: wall(wall0),
                            monotonic: base,
                        },
                        &reading(base, wall0, 0),
                        Marker::new(index as u32, &drops),
                    ),
                    Ok(OfferDisposition::Buffered)
                );
            }
            assert_eq!(kernel.pending.len(), 30);
            for ordinal in 2..=101 {
                assert_eq!(
                    kernel.offer(
                        keys[0].clone(),
                        ordinal,
                        Capture {
                            wall: wall(wall0),
                            monotonic: base,
                        },
                        &reading(base, wall0, 0),
                        Marker::new(ordinal as u32, &drops),
                    ),
                    Ok(OfferDisposition::Replaced)
                );
            }
            assert_eq!(kernel.pending.len(), 30);
            assert_eq!(kernel.counters.replaced, 100);
            let mut over_capacity = keys.clone();
            over_capacity.push(key(31, 7, 11));
            assert_eq!(
                kernel.set_current(over_capacity),
                Err(BufferError::TooManyIdentities)
            );
            assert_eq!(kernel.current.len(), 30);
            assert_eq!(kernel.pending.len(), 30);
            assert!(
                kernel
                    .offer(
                        key(31, 7, 11),
                        1,
                        Capture {
                            wall: wall(wall0),
                            monotonic: base,
                        },
                        &reading(base, wall0, 0),
                        Marker::new(31, &drops),
                    )
                    .is_err()
            );
            assert_eq!(kernel.pending.len(), 30);
            assert_eq!(kernel.high_water.len(), 30);

            // A continuously updated symbol must still publish at the first
            // 250ms boundary. Replacements change the capture, not the start
            // of the pending coalescing window.
            let hot = key(1, 7, 11);
            let mut continuous = active_kernel(vec![hot.clone()]);
            for (index, offset) in [0_u64, 100, 200, 249].into_iter().enumerate() {
                continuous
                    .offer(
                        hot.clone(),
                        index as u64 + 1,
                        Capture {
                            wall: wall(wall0 + offset as i64),
                            monotonic: base + Duration::from_millis(offset),
                        },
                        &reading(base, wall0 + offset as i64, offset),
                        Marker::new(index as u32 + 1, &drops),
                    )
                    .unwrap();
            }
            assert!(matches!(
                continuous
                    .handoff(&reading(base, wall0 + 249, 249))
                    .unwrap(),
                KernelHandoff::Waiting
            ));
            let KernelHandoff::Ready(first_hot_batch) = continuous
                .handoff(&reading(base, wall0 + 250, 250))
                .unwrap()
            else {
                panic!("continuous updates postponed the first 250ms publication");
            };
            assert_eq!(first_hot_batch.guard.slots[0].ordinal, 4);
            assert_eq!(
                first_hot_batch.guard.slots[0].capture.wall,
                wall(wall0 + 249)
            );
            continuous
                .mark_transaction_start(&first_hot_batch.guard, &reading(base, wall0 + 250, 250))
                .unwrap();

            // Continue receiving while the first batch is in flight. The
            // next batch keeps the newest value and its original capture,
            // and becomes eligible at 500ms without waiting for silence.
            for (offset, ordinal) in [(250_u64, 5_u64), (350, 6), (499, 7)] {
                continuous
                    .offer(
                        hot.clone(),
                        ordinal,
                        Capture {
                            wall: wall(wall0 + offset as i64),
                            monotonic: base + Duration::from_millis(offset),
                        },
                        &reading(base, wall0 + offset as i64, offset),
                        Marker::new(ordinal as u32, &drops),
                    )
                    .unwrap();
            }
            assert_eq!(
                continuous.handoff(&reading(base, wall0 + 500, 500)).err(),
                Some(BufferError::WriterBusy)
            );
            assert_eq!(continuous.diagnostics().pending_slots, 1);
            continuous.complete_success(first_hot_batch.guard).unwrap();
            let KernelHandoff::Ready(second_hot_batch) = continuous
                .handoff(&reading(base, wall0 + 500, 500))
                .unwrap()
            else {
                panic!("continuous updates postponed the second 250ms publication");
            };
            assert_eq!(second_hot_batch.guard.slots.len(), 1);
            assert_eq!(second_hot_batch.guard.slots[0].ordinal, 7);
            assert_eq!(
                second_hot_batch.guard.slots[0].capture.wall,
                wall(wall0 + 499)
            );
            continuous
                .mark_transaction_start(&second_hot_batch.guard, &reading(base, wall0 + 500, 500))
                .unwrap();
            continuous.complete_success(second_hot_batch.guard).unwrap();
        }

        #[test]
        fn writer_busy_coalesces_without_new_batch() {
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let drops = Rc::new(Cell::new(0));
            let first = key(1, 7, 11);
            let second = key(2, 7, 12);
            let mut kernel = active_kernel(vec![first.clone(), second.clone()]);
            kernel
                .offer(
                    first,
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(1, &drops),
                )
                .unwrap();
            let KernelHandoff::Ready(first_batch) =
                kernel.handoff(&reading(base, wall0 + 250, 250)).unwrap()
            else {
                panic!("expected the first coalesced batch");
            };
            kernel
                .mark_transaction_start(&first_batch.guard, &reading(base, wall0 + 250, 250))
                .unwrap();
            kernel
                .offer(
                    second,
                    1,
                    Capture {
                        wall: wall(wall0 + 300),
                        monotonic: base + Duration::from_millis(300),
                    },
                    &reading(base, wall0 + 300, 300),
                    Marker::new(2, &drops),
                )
                .unwrap();
            assert_eq!(
                kernel.handoff(&reading(base, wall0 + 500, 500)).err(),
                Some(BufferError::WriterBusy)
            );
            assert_eq!(kernel.pending.len(), 1);
            kernel.complete_success(first_batch.guard).unwrap();
            assert!(matches!(
                kernel.handoff(&reading(base, wall0 + 549, 549)).unwrap(),
                KernelHandoff::Waiting
            ));
            assert!(matches!(
                kernel.handoff(&reading(base, wall0 + 550, 550)).unwrap(),
                KernelHandoff::Ready(_)
            ));
            assert_eq!(kernel.outstanding.as_ref().unwrap().slots.len(), 1);
        }

        #[test]
        fn publication_spacing_has_no_catch_up() {
            let base = Instant::now();
            let mut cadence = PublicationCadence::new();
            for offset in [0, 250, 500, 750] {
                let at = base + Duration::from_millis(offset);
                assert!(cadence.start_allowed(at).unwrap());
                cadence.record_start(at);
            }
            assert!(
                !cadence
                    .start_allowed(base + Duration::from_millis(999))
                    .unwrap()
            );
            assert!(
                cadence
                    .start_allowed(base + Duration::from_millis(1_000))
                    .unwrap()
            );
            cadence.record_start(base + Duration::from_millis(1_000));

            let delayed = base + Duration::from_secs(60);
            assert!(cadence.start_allowed(delayed).unwrap());
            cadence.record_start(delayed);
            assert!(
                !cadence
                    .start_allowed(delayed + Duration::from_millis(249))
                    .unwrap()
            );
            assert!(
                cadence
                    .start_allowed(delayed + Duration::from_millis(250))
                    .unwrap()
            );
        }

        #[test]
        fn invalidation_purges_pending_and_blocks_stale_batch() {
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let drops = Rc::new(Cell::new(0));
            let slot = key(1, 7, 11);
            let mut kernel = active_kernel(vec![slot.clone()]);
            kernel
                .offer(
                    slot,
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(1, &drops),
                )
                .unwrap();
            let KernelHandoff::Ready(batch) =
                kernel.handoff(&reading(base, wall0 + 250, 250)).unwrap()
            else {
                panic!("expected a batch before invalidation");
            };
            kernel
                .mark_transaction_start(&batch.guard, &reading(base, wall0 + 250, 250))
                .unwrap();
            let old_id = batch.guard.id;
            kernel.invalidate();
            assert_eq!(kernel.pending.len(), 0);
            assert!(kernel.terminal);
            assert!(!kernel.guard_is_current(&batch.guard));
            assert_eq!(
                kernel.complete_success(batch.guard),
                Err(BufferError::StaleBatch)
            );
            assert_eq!(kernel.outstanding.as_ref().unwrap().id, old_id);
            assert_eq!(
                kernel.handoff(&reading(base, wall0 + 500, 500)).err(),
                Some(BufferError::Terminal)
            );
        }

        #[test]
        fn age_and_future_capture_fail_closed() {
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let drops = Rc::new(Cell::new(0));
            let boundary = key(1, 7, 11);
            let mut exact = active_kernel(vec![boundary.clone()]);
            assert_eq!(
                exact.offer(
                    boundary,
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0 + 3_000, 3_000),
                    Marker::new(1, &drops),
                ),
                Ok(OfferDisposition::Buffered)
            );
            assert_eq!(exact.counters.age_lag_drops, 0);

            let old = key(2, 7, 11);
            let mut aged = active_kernel(vec![old.clone()]);
            assert_eq!(
                aged.offer(
                    old,
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0 + 3_001, 3_001),
                    Marker::new(2, &drops),
                ),
                Ok(OfferDisposition::PipelineLag)
            );
            assert_eq!(aged.counters.age_lag_drops, 1);
            assert_eq!(aged.pending.len(), 0);

            let future = key(3, 7, 11);
            let mut future_kernel = active_kernel(vec![future.clone()]);
            assert_eq!(
                future_kernel.offer(
                    future,
                    1,
                    Capture {
                        wall: wall(wall0 + 1),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(3, &drops),
                ),
                Err(BufferError::InvalidCapture)
            );
            assert_eq!(future_kernel.counters.rejected, 1);
            assert_eq!(future_kernel.pending.len(), 0);

            let monotonic_future = key(4, 7, 11);
            let mut monotonic_kernel = active_kernel(vec![monotonic_future.clone()]);
            assert_eq!(
                monotonic_kernel.offer(
                    monotonic_future,
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base + Duration::from_millis(1),
                    },
                    &reading(base, wall0, 0),
                    Marker::new(4, &drops),
                ),
                Err(BufferError::InvalidCapture)
            );
        }

        #[test]
        fn failed_or_dropped_batch_cannot_be_requeued() {
            let base = Instant::now();
            let wall0 = 1_798_000_000_000;
            let drops = Rc::new(Cell::new(0));
            let slot = key(1, 7, 11);
            let mut kernel = active_kernel(vec![slot.clone()]);
            kernel
                .offer(
                    slot,
                    1,
                    Capture {
                        wall: wall(wall0),
                        monotonic: base,
                    },
                    &reading(base, wall0, 0),
                    Marker::new(1, &drops),
                )
                .unwrap();
            let KernelHandoff::Ready(batch) =
                kernel.handoff(&reading(base, wall0 + 250, 250)).unwrap()
            else {
                panic!("expected one in-flight batch");
            };
            kernel
                .mark_transaction_start(&batch.guard, &reading(base, wall0 + 250, 250))
                .unwrap();
            drop(batch); // Models writer failure/cancellation: no success ACK.
            assert_eq!(drops.get(), 1);
            assert_eq!(kernel.pending.len(), 0);
            assert_eq!(
                kernel.handoff(&reading(base, wall0 + 500, 500)).err(),
                Some(BufferError::WriterBusy)
            );
            assert_eq!(kernel.outstanding.as_ref().unwrap().started, true);
        }
    }
}
mod runtime_owner {
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use chrono::{DateTime, FixedOffset, Utc};
    use kis_client::market_stream::{
        MarketStreamEvent, MarketStreamSession, MarketSubscriptionOperation,
    };
    use kis_client::market_stream_wire::MarketReceipt;
    use tokio::task::JoinSet;
    use uuid::Uuid;

    use super::super::market_stream::{
        DesiredSet, MarketStreamStorageError, RuntimeMarketStreamRepository,
        RuntimeMarketStreamStorageError, RuntimeStatusCommit, RuntimeTransition, StreamEpochProof,
        StreamIdentity, StreamProducerLease, StreamPublicationContext, StreamPublicationItem,
        StreamStatusCode, SubscriptionProof,
    };
    use super::super::market_stream_producer::runtime_buffer::{
        BatchAvailability, BufferError, InvalidationCause, OfferDisposition, PublicationBatch,
        ReceiptBuffer,
    };
    use super::super::market_stream_runtime::ResolvedMarketStreamDay;
    use super::{
        MarketStreamProducerError, MarketStreamTransportStatus, RuntimeOwnedExit, normalize_demand,
    };

    const OBSERVATION_TICK: Duration = Duration::from_millis(50);
    const DEMAND_FRESHNESS: Duration = Duration::from_secs(2);
    const CLEAN_DRAIN_BOUND: Duration = Duration::from_secs(5);
    const ROUTINE_STATUS_SPACING: Duration = Duration::from_secs(1);
    const MIN_LEASE_MARGIN: chrono::Duration = chrono::Duration::seconds(5);
    const KST_OFFSET_SECONDS: i32 = 9 * 60 * 60;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StopReason {
        Shutdown,
        NoDemand,
        DayClosed,
        CleanClosed,
        FreshEpochRequired,
    }

    impl StopReason {
        fn priority(self) -> u8 {
            match self {
                Self::Shutdown => 1,
                Self::NoDemand => 2,
                Self::DayClosed => 3,
                Self::CleanClosed => 4,
                Self::FreshEpochRequired => 5,
            }
        }

        fn exit(self) -> RuntimeOwnedExit {
            match self {
                Self::Shutdown => RuntimeOwnedExit::Shutdown,
                Self::NoDemand => RuntimeOwnedExit::NoDemand,
                Self::DayClosed => RuntimeOwnedExit::DayClosed,
                Self::CleanClosed => RuntimeOwnedExit::CleanClosed,
                Self::FreshEpochRequired => RuntimeOwnedExit::FreshEpochRequired,
            }
        }

        fn retirement_code(self, gap: Option<RuntimeTransition>) -> StreamStatusCode {
            match self {
                Self::Shutdown => StreamStatusCode::ProducerUnavailable,
                Self::NoDemand => StreamStatusCode::NoActiveDemand,
                Self::DayClosed => StreamStatusCode::SessionClosed,
                Self::CleanClosed => StreamStatusCode::ConnectionLost,
                Self::FreshEpochRequired if gap == Some(RuntimeTransition::PipelineLag) => {
                    StreamStatusCode::PipelineLag
                }
                Self::FreshEpochRequired => StreamStatusCode::ResyncRequired,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CommandKind {
        Subscribe,
        Unsubscribe,
        CountChange,
    }

    impl From<CommandKind> for MarketSubscriptionOperation {
        fn from(value: CommandKind) -> Self {
            match value {
                CommandKind::Subscribe => Self::Subscribe,
                CommandKind::Unsubscribe | CommandKind::CountChange => Self::Unsubscribe,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DemandBinding {
        identity: StreamIdentity,
        reference_count: u32,
    }

    #[derive(Clone)]
    struct AckBinding {
        identity: StreamIdentity,
        proof: SubscriptionProof,
    }

    #[derive(Clone)]
    struct PendingBinding {
        identity: StreamIdentity,
        kind: CommandKind,
    }

    #[derive(Clone)]
    struct CommandPlan {
        symbol: String,
        identity: StreamIdentity,
        kind: CommandKind,
        reference_count: u32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DemandApplyError {
        Invalid,
        FreshEpoch,
    }

    #[derive(Default)]
    struct BindingBook {
        desired: BTreeMap<String, DemandBinding>,
        acked: BTreeMap<String, AckBinding>,
        pending: BTreeMap<String, PendingBinding>,
        unsubscribed: BTreeSet<String>,
        persisted_counts: BTreeMap<String, u32>,
        deferred_until_ms: BTreeMap<String, i64>,
    }

    impl BindingBook {
        fn apply_demand(
            &mut self,
            demand: &DesiredSet,
            lease: &StreamProducerLease,
        ) -> Result<Vec<StreamIdentity>, DemandApplyError> {
            let next = normalize_runtime_demand(demand, lease)?;
            if next.keys().any(|symbol| self.unsubscribed.contains(symbol)) {
                return Err(DemandApplyError::FreshEpoch);
            }
            let mut invalidated = BTreeSet::new();
            for (symbol, binding) in &self.acked {
                if next
                    .get(symbol)
                    .is_none_or(|desired| desired.identity != binding.identity)
                {
                    invalidated.insert(binding.identity.clone());
                }
            }
            for (symbol, pending) in &self.pending {
                if next
                    .get(symbol)
                    .is_none_or(|desired| desired.identity != pending.identity)
                {
                    invalidated.insert(pending.identity.clone());
                }
            }
            self.deferred_until_ms.retain(|symbol, _| {
                self.desired.get(symbol).is_some_and(|old| {
                    next.get(symbol)
                        .is_some_and(|current| current.identity == old.identity)
                })
            });
            self.desired = next;
            Ok(invalidated.into_iter().collect())
        }

        fn mark_pending(&mut self, command: &CommandPlan) {
            self.pending.insert(
                command.symbol.clone(),
                PendingBinding {
                    identity: command.identity.clone(),
                    kind: command.kind,
                },
            );
        }

        fn finish_subscribe(
            &mut self,
            symbol: &str,
            proof: SubscriptionProof,
            committed_count: u32,
        ) {
            let identity = self
                .pending
                .remove(symbol)
                .map(|pending| pending.identity)
                .or_else(|| {
                    self.desired
                        .get(symbol)
                        .map(|binding| binding.identity.clone())
                });
            if let Some(identity) = identity {
                self.acked
                    .insert(symbol.to_owned(), AckBinding { identity, proof });
            }
            self.persisted_counts
                .insert(symbol.to_owned(), committed_count);
            self.deferred_until_ms.remove(symbol);
        }

        fn finish_unsubscribe(&mut self, symbol: &str) {
            self.pending.remove(symbol);
            self.acked.remove(symbol);
            self.persisted_counts.remove(symbol);
            self.deferred_until_ms.remove(symbol);
            self.unsubscribed.insert(symbol.to_owned());
        }

        fn persist_count(&mut self, symbol: &str, reference_count: u32) {
            self.persisted_counts
                .insert(symbol.to_owned(), reference_count);
        }

        fn remember_deferred(&mut self, symbol: &str, not_before_ms: i64) {
            self.deferred_until_ms
                .insert(symbol.to_owned(), not_before_ms);
        }

        fn clear_deferred(&mut self, symbol: &str) {
            self.deferred_until_ms.remove(symbol);
        }

        fn next_command(&self, now_ms: i64) -> Result<Option<CommandPlan>, DemandApplyError> {
            if !self.pending.is_empty() {
                return Ok(None);
            }
            for (symbol, acked) in &self.acked {
                if self
                    .desired
                    .get(symbol)
                    .is_none_or(|desired| desired.identity != acked.identity)
                {
                    if self
                        .deferred_until_ms
                        .get(symbol)
                        .is_some_and(|not_before| now_ms < *not_before)
                    {
                        continue;
                    }
                    return Ok(Some(CommandPlan {
                        symbol: symbol.clone(),
                        identity: acked.identity.clone(),
                        kind: CommandKind::Unsubscribe,
                        reference_count: 0,
                    }));
                }
            }
            for (symbol, desired) in &self.desired {
                if let Some(acked) = self.acked.get(symbol) {
                    if acked.identity == desired.identity
                        && self.persisted_counts.get(symbol) != Some(&desired.reference_count)
                    {
                        return Ok(Some(CommandPlan {
                            symbol: symbol.clone(),
                            identity: desired.identity.clone(),
                            kind: CommandKind::CountChange,
                            reference_count: desired.reference_count,
                        }));
                    }
                    continue;
                }
                if self.unsubscribed.contains(symbol) {
                    return Err(DemandApplyError::FreshEpoch);
                }
                if self
                    .deferred_until_ms
                    .get(symbol)
                    .is_some_and(|not_before| now_ms < *not_before)
                {
                    continue;
                }
                return Ok(Some(CommandPlan {
                    symbol: symbol.clone(),
                    identity: desired.identity.clone(),
                    kind: CommandKind::Subscribe,
                    reference_count: desired.reference_count,
                }));
            }
            Ok(None)
        }

        fn publishable_items(&self) -> Vec<StreamPublicationItem> {
            self.desired
                .iter()
                .filter_map(|(symbol, desired)| {
                    if self.pending.contains_key(symbol) {
                        return None;
                    }
                    let acked = self.acked.get(symbol)?;
                    if acked.identity != desired.identity
                        || acked.proof.state != "ACKED"
                        || acked.proof.acked_at.is_none()
                        || acked.proof.desired_reference_count == 0
                        || acked.proof.symbol != symbol.as_str()
                    {
                        return None;
                    }
                    Some(StreamPublicationItem {
                        admission: desired.identity.clone(),
                        subscription: acked.proof.clone(),
                    })
                })
                .collect()
        }

        fn is_publishable(&self, symbol: &str) -> bool {
            let Some(desired) = self.desired.get(symbol) else {
                return false;
            };
            if self.pending.contains_key(symbol) {
                return false;
            }
            self.acked
                .get(symbol)
                .is_some_and(|acked| acked.identity == desired.identity)
        }

        fn record_pending_for_tests(
            &mut self,
            symbol: &str,
            identity: StreamIdentity,
            kind: CommandKind,
        ) {
            self.pending
                .insert(symbol.to_owned(), PendingBinding { identity, kind });
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct StatusWork {
        transition: RuntimeTransition,
        urgent: bool,
    }

    #[derive(Default)]
    struct StatusMailbox {
        routine: Option<RuntimeTransition>,
        urgent: Option<RuntimeTransition>,
        last_start: Option<Instant>,
        last_transition: Option<RuntimeTransition>,
        known_quote: bool,
    }

    impl StatusMailbox {
        fn routine(&mut self, transition: RuntimeTransition) {
            if self.last_transition == Some(transition)
                || (self.routine == Some(transition) && !self.known_quote)
            {
                return;
            }
            if self.known_quote && transition == RuntimeTransition::AwaitingFirstTrade {
                return;
            }
            self.routine = Some(transition);
        }

        fn urgent(&mut self, transition: RuntimeTransition) {
            match self.urgent {
                Some(RuntimeTransition::AccessRevoked) => {}
                _ if transition == RuntimeTransition::AccessRevoked => {
                    self.urgent = Some(transition);
                }
                Some(_) => {}
                None => self.urgent = Some(transition),
            }
        }

        fn known_quote(&mut self) {
            self.known_quote = true;
            if self.routine == Some(RuntimeTransition::AwaitingFirstTrade) {
                self.routine = None;
            }
        }

        fn take_ready(&mut self, now: Instant) -> Option<StatusWork> {
            if let Some(transition) = self.urgent.take() {
                self.last_start = Some(now);
                self.last_transition = Some(transition);
                return Some(StatusWork {
                    transition,
                    urgent: true,
                });
            }
            let transition = self.routine?;
            if self
                .last_start
                .is_some_and(|last| now.saturating_duration_since(last) < ROUTINE_STATUS_SPACING)
            {
                return None;
            }
            self.routine = None;
            self.last_start = Some(now);
            self.last_transition = Some(transition);
            Some(StatusWork {
                transition,
                urgent: false,
            })
        }

        fn has_work(&self) -> bool {
            self.urgent.is_some() || self.routine.is_some()
        }
    }

    #[derive(Default)]
    struct RuntimeLifecycle {
        stopping: bool,
        reason: Option<StopReason>,
        failure: Option<MarketStreamProducerError>,
        gap_transition: Option<RuntimeTransition>,
        socket_closed: bool,
    }

    impl RuntimeLifecycle {
        fn request_stop(&mut self, reason: StopReason) {
            self.stopping = true;
            if self
                .reason
                .is_none_or(|current| reason.priority() > current.priority())
            {
                self.reason = Some(reason);
            }
        }

        fn request_gap(&mut self, transition: RuntimeTransition) {
            self.gap_transition.get_or_insert(transition);
            self.request_stop(StopReason::FreshEpochRequired);
        }

        #[cfg_attr(feature = "market-stream-db-tests", track_caller)]
        fn fail(&mut self, error: MarketStreamProducerError) {
            #[cfg(feature = "market-stream-db-tests")]
            if self.failure.is_none() {
                eprintln!(
                    "KIS_TEST_DIAG owner_failure error={error:?} line={}",
                    std::panic::Location::caller().line()
                );
            }
            self.failure = Some(error);
            self.stopping = true;
        }

        fn record_close(&mut self, result: Result<(), MarketStreamProducerError>) {
            match result {
                Ok(()) => {
                    self.socket_closed = true;
                    if self.reason.is_none() {
                        self.request_stop(StopReason::CleanClosed);
                    } else {
                        self.stopping = true;
                    }
                }
                Err(error) => self.fail(error),
            }
        }

        fn clean_exit(&self) -> Option<RuntimeOwnedExit> {
            if self.failure.is_some() || !self.stopping || !self.socket_closed {
                return None;
            }
            self.reason.map(StopReason::exit)
        }
    }

    fn command_plan_matches_current(
        bindings: &BindingBook,
        lifecycle: &RuntimeLifecycle,
        selected: &CommandPlan,
        demand_is_fresh: bool,
        now_ms: i64,
    ) -> Result<bool, DemandApplyError> {
        if lifecycle.stopping || lifecycle.failure.is_some() || !demand_is_fresh {
            return Ok(false);
        }
        let Some(current) = bindings.next_command(now_ms)? else {
            return Ok(false);
        };
        Ok(current.symbol == selected.symbol
            && current.identity == selected.identity
            && current.kind == selected.kind
            && current.reference_count == selected.reference_count)
    }

    struct RuntimeState {
        lease: StreamProducerLease,
        epoch: StreamEpochProof,
        day: ResolvedMarketStreamDay,
        buffer: ReceiptBuffer,
        bindings: BindingBook,
        lifecycle: RuntimeLifecycle,
        mailbox: StatusMailbox,
        demand_observed_at: Instant,
    }

    impl RuntimeState {
        fn new(
            lease: StreamProducerLease,
            epoch: StreamEpochProof,
            day: ResolvedMarketStreamDay,
        ) -> Self {
            let mut mailbox = StatusMailbox::default();
            mailbox.routine(RuntimeTransition::AwaitingFirstTrade);
            Self {
                lease,
                epoch,
                day,
                buffer: ReceiptBuffer::new(),
                bindings: BindingBook::default(),
                lifecycle: RuntimeLifecycle::default(),
                mailbox,
                demand_observed_at: Instant::now(),
            }
        }

        fn apply_demand(&mut self, demand: &DesiredSet) -> Result<(), MarketStreamProducerError> {
            match self.bindings.apply_demand(demand, &self.lease) {
                Ok(invalidated) => {
                    for identity in invalidated {
                        self.buffer
                            .invalidate_identity(&identity)
                            .map_err(|_| MarketStreamProducerError::Terminal)?;
                    }
                }
                Err(DemandApplyError::FreshEpoch) => {
                    self.request_gap(RuntimeTransition::ResyncRequired);
                    return Ok(());
                }
                Err(DemandApplyError::Invalid) => {
                    return Err(MarketStreamProducerError::InvalidInput);
                }
            }
            self.demand_observed_at = Instant::now();
            if self.bindings.desired.is_empty() && !self.lifecycle.stopping {
                self.buffer
                    .clear_zero_demand()
                    .map_err(|_| MarketStreamProducerError::Terminal)?;
                self.mailbox.routine(RuntimeTransition::NoActiveDemand);
                self.lifecycle.request_stop(StopReason::NoDemand);
            }
            Ok(())
        }

        fn update_lease(
            &mut self,
            lease: StreamProducerLease,
            now: DateTime<Utc>,
        ) -> Result<(), MarketStreamProducerError> {
            if !lease_refresh_is_valid(
                &self.lease,
                &lease,
                self.epoch.gap_generation,
                self.day.session_date(),
                self.day.open_at(),
                self.day.close_at(),
                now,
            ) {
                self.fail(MarketStreamProducerError::Terminal);
                return Err(MarketStreamProducerError::Terminal);
            }
            self.lease = lease;
            Ok(())
        }

        fn check_local_eligibility(&mut self, now: DateTime<Utc>) {
            if self.lifecycle.stopping || self.lifecycle.failure.is_some() {
                return;
            }
            if !date_and_window_are_open(
                self.day.session_date(),
                self.day.open_at(),
                self.day.close_at(),
                now,
            ) {
                self.stop(StopReason::DayClosed);
                return;
            }
            if self.lease.lease_expires_at <= now + MIN_LEASE_MARGIN {
                self.fail(MarketStreamProducerError::Terminal);
            }
        }

        fn request_gap(&mut self, transition: RuntimeTransition) {
            request_gap_parts(
                &mut self.lifecycle,
                &mut self.buffer,
                &mut self.mailbox,
                transition,
            );
        }

        fn queue_transition(&mut self, transition: RuntimeTransition, urgent: bool) {
            if transition.values().opens_gap {
                self.request_gap(transition);
                return;
            }
            if urgent {
                self.mailbox.urgent(transition);
            } else {
                self.mailbox.routine(transition);
            }
        }

        fn stop(&mut self, reason: StopReason) {
            self.lifecycle.request_stop(reason);
            if reason == StopReason::NoDemand {
                let _ = self.buffer.clear_zero_demand();
                self.mailbox.routine(RuntimeTransition::NoActiveDemand);
            } else {
                self.buffer.invalidate(InvalidationCause::EpochEnded);
                self.mailbox.routine = None;
            }
        }

        #[cfg_attr(feature = "market-stream-db-tests", track_caller)]
        fn fail(&mut self, error: MarketStreamProducerError) {
            self.lifecycle.fail(error);
            self.buffer.invalidate(InvalidationCause::TerminalTransport);
            self.mailbox.routine = None;
        }

        fn refresh_buffer_context(
            &mut self,
        ) -> Result<Option<StreamPublicationContext>, MarketStreamProducerError> {
            let items = self.bindings.publishable_items();
            if items.is_empty() {
                self.buffer
                    .clear_zero_demand()
                    .map_err(|_| MarketStreamProducerError::Terminal)?;
                return Ok(None);
            }
            let context =
                StreamPublicationContext::new(self.lease.clone(), self.epoch.clone(), items)
                    .map_err(MarketStreamProducerError::from)?;
            self.buffer
                .refresh_runtime_context(&context)
                .map_err(|_| MarketStreamProducerError::Terminal)?;
            Ok(Some(context))
        }

        fn mark_known_quote(&mut self) {
            self.mailbox.known_quote();
        }

        fn fail_if_demand_stale(&mut self, now: Instant) {
            if now.saturating_duration_since(self.demand_observed_at) > DEMAND_FRESHNESS {
                self.fail(MarketStreamProducerError::Terminal);
            }
        }

        fn command_plan_is_admissible(
            &mut self,
            selected: &CommandPlan,
            now_wall: DateTime<Utc>,
            now_mono: Instant,
        ) -> Result<bool, MarketStreamProducerError> {
            self.fail_if_demand_stale(now_mono);
            self.check_local_eligibility(now_wall);
            let demand_is_fresh =
                now_mono.saturating_duration_since(self.demand_observed_at) <= DEMAND_FRESHNESS;
            match command_plan_matches_current(
                &self.bindings,
                &self.lifecycle,
                selected,
                demand_is_fresh,
                now_wall.timestamp_millis(),
            ) {
                Ok(is_current) => Ok(is_current),
                Err(DemandApplyError::FreshEpoch) => {
                    self.request_gap(RuntimeTransition::ResyncRequired);
                    Ok(false)
                }
                Err(DemandApplyError::Invalid) => {
                    self.fail(MarketStreamProducerError::InvalidInput);
                    Err(MarketStreamProducerError::InvalidInput)
                }
            }
        }

        fn defer_command_if_current(
            &mut self,
            selected: &CommandPlan,
            not_before_ms: i64,
            now_wall: DateTime<Utc>,
            now_mono: Instant,
        ) -> Result<DeferredCommandAdmission, MarketStreamProducerError> {
            if !self.command_plan_is_admissible(selected, now_wall, now_mono)? {
                return Ok(DeferredCommandAdmission::Stale);
            }
            let now_ms = now_wall.timestamp_millis();
            if now_ms >= not_before_ms {
                return Ok(DeferredCommandAdmission::Ready);
            }
            self.bindings
                .remember_deferred(&selected.symbol, not_before_ms);
            Ok(DeferredCommandAdmission::Deferred)
        }

        fn admit_command(
            &mut self,
            selected: &CommandPlan,
            is_wire_command: bool,
            now_wall: DateTime<Utc>,
            now_mono: Instant,
        ) -> Result<Option<StreamProducerLease>, MarketStreamProducerError> {
            if !self.command_plan_is_admissible(selected, now_wall, now_mono)? {
                return Ok(None);
            }
            if is_wire_command {
                self.bindings.clear_deferred(&selected.symbol);
                self.bindings.mark_pending(selected);
                self.buffer
                    .invalidate_identity(&selected.identity)
                    .map_err(|_| MarketStreamProducerError::Terminal)?;
            }
            Ok(Some(self.lease.clone()))
        }
    }

    fn fixed_kst() -> FixedOffset {
        FixedOffset::east_opt(KST_OFFSET_SECONDS).expect("fixed KST offset")
    }

    fn date_and_window_are_open(
        session_date: chrono::NaiveDate,
        open_at: DateTime<Utc>,
        close_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> bool {
        open_at <= now
            && now < close_at
            && now.with_timezone(&fixed_kst()).date_naive() == session_date
    }

    fn lease_refresh_is_valid(
        previous: &StreamProducerLease,
        next: &StreamProducerLease,
        epoch_gap_generation: u64,
        session_date: chrono::NaiveDate,
        open_at: DateTime<Utc>,
        close_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> bool {
        previous.credential_slot_id == next.credential_slot_id
            && previous.grant_id == next.grant_id
            && previous.grant_revision == next.grant_revision
            && previous.owner_user_id == next.owner_user_id
            && previous.holder_id == next.holder_id
            && previous.fencing_token == next.fencing_token
            && previous.fencing_token > 0
            && next.gap_generation == epoch_gap_generation
            && next.gap_generation == previous.gap_generation
            && next.heartbeat_at >= previous.heartbeat_at
            && next.lease_expires_at >= previous.lease_expires_at
            && next.heartbeat_at <= now
            && next.lease_expires_at > next.heartbeat_at
            && next.lease_expires_at > now + MIN_LEASE_MARGIN
            && date_and_window_are_open(session_date, open_at, close_at, now)
            && !next.credential_slot_id.is_nil()
            && !next.grant_id.is_nil()
            && !next.grant_revision.is_nil()
            && !next.owner_user_id.is_nil()
            && !next.holder_id.is_nil()
    }

    fn normalize_runtime_demand(
        demand: &DesiredSet,
        lease: &StreamProducerLease,
    ) -> Result<BTreeMap<String, DemandBinding>, DemandApplyError> {
        let normalized = normalize_demand(demand, lease.credential_slot_id, lease.owner_user_id)
            .map_err(|_| DemandApplyError::Invalid)?;
        let mut by_symbol = BTreeMap::new();
        for (identity, reference_count) in normalized {
            if by_symbol
                .insert(
                    identity.symbol().to_owned(),
                    DemandBinding {
                        identity,
                        reference_count,
                    },
                )
                .is_some()
            {
                return Err(DemandApplyError::Invalid);
            }
        }
        Ok(by_symbol)
    }

    impl RuntimeState {
        fn apply_demand_checked(
            &mut self,
            demand: &DesiredSet,
        ) -> Result<(), MarketStreamProducerError> {
            self.apply_demand(demand)
        }
    }

    struct SharedOwner {
        state: Mutex<RuntimeState>,
        changed: tokio::sync::Notify,
    }

    struct RunCancellationGuard {
        shared: Arc<SharedOwner>,
        completed: bool,
    }

    impl RunCancellationGuard {
        fn new(shared: Arc<SharedOwner>) -> Self {
            Self {
                shared,
                completed: false,
            }
        }
    }

    impl Drop for RunCancellationGuard {
        fn drop(&mut self) {
            if !self.completed {
                self.shared.fail(MarketStreamProducerError::Terminal);
            }
        }
    }

    impl SharedOwner {
        fn new(state: RuntimeState) -> Self {
            Self {
                state: Mutex::new(state),
                changed: tokio::sync::Notify::new(),
            }
        }

        fn lock(&self) -> Result<MutexGuard<'_, RuntimeState>, MarketStreamProducerError> {
            self.state
                .lock()
                .map_err(|_| MarketStreamProducerError::Terminal)
        }

        fn wake(&self) {
            self.changed.notify_waiters();
        }

        #[cfg_attr(feature = "market-stream-db-tests", track_caller)]
        fn fail(&self, error: MarketStreamProducerError) {
            if let Ok(mut state) = self.state.lock() {
                state.fail(error);
            }
            self.wake();
        }

        fn request_stop(&self, reason: StopReason) {
            if let Ok(mut state) = self.state.lock() {
                state.stop(reason);
            }
            self.wake();
        }
    }

    /// JoinSet owns every child; dropping the runtime future aborts every
    /// outstanding child instead of detaching the socket or writer.
    struct OwnedTaskGroup {
        tasks: JoinSet<()>,
    }

    impl OwnedTaskGroup {
        fn new() -> Self {
            Self {
                tasks: JoinSet::new(),
            }
        }

        fn spawn<F>(&mut self, future: F)
        where
            F: Future<Output = ()> + Send + 'static,
        {
            self.tasks.spawn(future);
        }

        async fn join_next(&mut self) -> Option<Result<(), tokio::task::JoinError>> {
            self.tasks.join_next().await
        }

        fn is_empty(&self) -> bool {
            self.tasks.is_empty()
        }

        async fn abort_and_join(&mut self) {
            self.tasks.abort_all();
            while let Some(result) = self.tasks.join_next().await {
                match result {
                    Ok(()) => {}
                    Err(error) if error.is_cancelled() => {}
                    Err(error) if error.is_panic() => {}
                    Err(_) => {}
                }
            }
        }

        async fn join_all_until(
            &mut self,
            deadline: tokio::time::Instant,
        ) -> Result<(), MarketStreamProducerError> {
            while !self.tasks.is_empty() {
                match tokio::time::timeout_at(deadline, self.tasks.join_next()).await {
                    Ok(Some(Ok(()))) => {}
                    Ok(Some(Err(_))) => {
                        self.abort_and_join().await;
                        return Err(MarketStreamProducerError::Terminal);
                    }
                    Ok(None) => return Err(MarketStreamProducerError::Terminal),
                    Err(_) => {
                        self.abort_and_join().await;
                        return Err(MarketStreamProducerError::Terminal);
                    }
                }
            }
            Ok(())
        }
    }

    enum SocketTurn {
        Close,
        Continue(Option<CommandPlan>),
    }

    async fn socket_task(
        mut session: MarketStreamSession,
        shared: Arc<SharedOwner>,
        repository: RuntimeMarketStreamRepository,
        day: ResolvedMarketStreamDay,
        epoch: Uuid,
    ) {
        loop {
            let turn = {
                let mut state = match shared.lock() {
                    Ok(state) => state,
                    Err(error) => {
                        shared.fail(error);
                        drop(session);
                        return;
                    }
                };
                state.check_local_eligibility(Utc::now());
                if state.lifecycle.failure.is_some() {
                    drop(state);
                    drop(session);
                    return;
                }
                if state.lifecycle.stopping {
                    SocketTurn::Close
                } else {
                    let command = match state.bindings.next_command(Utc::now().timestamp_millis()) {
                        Ok(command) => command,
                        Err(DemandApplyError::FreshEpoch) => {
                            state.request_gap(RuntimeTransition::ResyncRequired);
                            None
                        }
                        Err(DemandApplyError::Invalid) => {
                            state.fail(MarketStreamProducerError::InvalidInput);
                            None
                        }
                    };
                    SocketTurn::Continue(command)
                }
            };
            if matches!(turn, SocketTurn::Close) {
                let close_result = session
                    .close()
                    .await
                    .map_err(|_| MarketStreamProducerError::TransportFailure);
                if let Ok(mut state) = shared.lock() {
                    state.lifecycle.record_close(close_result);
                }
                shared.wake();
                return;
            }
            let action = match turn {
                SocketTurn::Continue(action) => action,
                SocketTurn::Close => None,
            };
            shared.wake();
            if let Some(command) = action {
                match apply_socket_command(&mut session, &shared, &repository, epoch, command).await
                {
                    Ok(CommandDisposition::Committed) => continue,
                    Ok(CommandDisposition::Deferred) => {}
                    Err(error) => {
                        shared.fail(error);
                        drop(session);
                        return;
                    }
                }
            }

            let should_close = {
                let mut state = match shared.lock() {
                    Ok(state) => state,
                    Err(error) => {
                        shared.fail(error);
                        drop(session);
                        return;
                    }
                };
                state.check_local_eligibility(Utc::now());
                if state.lifecycle.failure.is_some() {
                    drop(state);
                    drop(session);
                    return;
                }
                if state.lifecycle.stopping {
                    true
                } else if let Err(error) = state.refresh_buffer_context() {
                    state.fail(error);
                    drop(state);
                    shared.wake();
                    drop(session);
                    return;
                } else {
                    false
                }
            };
            if should_close {
                let close_result = session
                    .close()
                    .await
                    .map_err(|_| MarketStreamProducerError::TransportFailure);
                if let Ok(mut state) = shared.lock() {
                    state.lifecycle.record_close(close_result);
                }
                shared.wake();
                return;
            }

            let event = match session
                .next_event_until(std::time::Instant::now() + OBSERVATION_TICK)
                .await
            {
                Ok(event) => event,
                Err(_) => {
                    shared.fail(MarketStreamProducerError::TransportFailure);
                    drop(session);
                    return;
                }
            };
            {
                let mut state = match shared.lock() {
                    Ok(state) => state,
                    Err(error) => {
                        shared.fail(error);
                        drop(session);
                        return;
                    }
                };
                state.check_local_eligibility(Utc::now());
                state.fail_if_demand_stale(Instant::now());
                if state.lifecycle.stopping || state.lifecycle.failure.is_some() {
                    drop(state);
                    continue;
                }
            }
            match event {
                None => {}
                Some(MarketStreamEvent::ApplicationHeartbeat | MarketStreamEvent::ControlPong) => {}
                Some(MarketStreamEvent::Receipt(receipt)) => {
                    let symbol = receipt.observation().symbol.as_str().to_owned();
                    let result = match shared.lock() {
                        Ok(mut state) => match state.refresh_buffer_context() {
                            Ok(Some(context)) if state.bindings.is_publishable(&symbol) => {
                                state.buffer.record_receipt(&context, receipt)
                            }
                            Ok(_) => continue,
                            Err(_) => {
                                state.fail(MarketStreamProducerError::Terminal);
                                shared.wake();
                                drop(session);
                                return;
                            }
                        },
                        Err(error) => {
                            shared.fail(error);
                            drop(session);
                            return;
                        }
                    };
                    match result {
                        Ok(
                            OfferDisposition::Buffered
                            | OfferDisposition::Replaced
                            | OfferDisposition::Stale,
                        ) => {}
                        Ok(OfferDisposition::PipelineLag) | Err(BufferError::PipelineLag) => {
                            if let Ok(mut state) = shared.lock() {
                                state.request_gap(RuntimeTransition::PipelineLag);
                            } else {
                                shared.fail(MarketStreamProducerError::Terminal);
                            }
                        }
                        Err(_) => shared.fail(MarketStreamProducerError::Terminal),
                    }
                    shared.wake();
                }
                Some(MarketStreamEvent::Status {
                    code,
                    epoch: event_epoch,
                }) => {
                    if event_epoch.uuid() != epoch {
                        shared.fail(MarketStreamProducerError::SessionUnavailable);
                        drop(session);
                        return;
                    }
                    match code {
                        MarketStreamTransportStatus::Connected
                        | MarketStreamTransportStatus::DataBeforeAck
                        | MarketStreamTransportStatus::DuplicateObservation
                        | MarketStreamTransportStatus::StaleObservation => {}
                        MarketStreamTransportStatus::Gap => {
                            if let Ok(mut state) = shared.lock() {
                                state.request_gap(RuntimeTransition::PipelineLag);
                            } else {
                                shared.fail(MarketStreamProducerError::Terminal);
                                drop(session);
                                return;
                            }
                            shared.wake();
                        }
                        MarketStreamTransportStatus::Closed => {
                            shared.request_stop(StopReason::CleanClosed);
                        }
                    }
                }
            }
            // `day` is the same immutable lineage used by C1 publication; the
            // local check only stops the socket and never replaces DB checks.
            if Utc::now() >= day.close_at()
                || Utc::now().with_timezone(&fixed_kst()).date_naive() != day.session_date()
            {
                shared.request_stop(StopReason::DayClosed);
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CommandDisposition {
        Committed,
        Deferred,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DeferredCommandAdmission {
        Stale,
        Deferred,
        Ready,
    }

    async fn apply_socket_command(
        session: &mut MarketStreamSession,
        shared: &SharedOwner,
        repository: &RuntimeMarketStreamRepository,
        epoch: Uuid,
        command: CommandPlan,
    ) -> Result<CommandDisposition, MarketStreamProducerError> {
        if command.kind == CommandKind::CountChange {
            let lease = {
                let mut state = shared.lock()?;
                state.admit_command(&command, false, Utc::now(), Instant::now())?
            };
            let Some(lease) = lease else {
                shared.wake();
                return Ok(CommandDisposition::Deferred);
            };
            repository
                .set_subscription_desired(&lease, epoch, &command.symbol, command.reference_count)
                .await
                .map_err(map_runtime_storage)?;
            {
                let mut state = shared.lock()?;
                if state
                    .bindings
                    .acked
                    .get(&command.symbol)
                    .is_some_and(|acked| acked.identity == command.identity)
                {
                    state
                        .bindings
                        .persist_count(&command.symbol, command.reference_count);
                }
            }
            shared.wake();
            return Ok(CommandDisposition::Committed);
        }
        let current = {
            let mut state = shared.lock()?;
            state.command_plan_is_admissible(&command, Utc::now(), Instant::now())?
        };
        if !current {
            shared.wake();
            return Ok(CommandDisposition::Deferred);
        }
        let operation: MarketSubscriptionOperation = command.kind.into();
        match session.subscription_command_not_before_ms(operation, &command.symbol) {
            Ok(Some(not_before_ms)) if Utc::now().timestamp_millis() < not_before_ms => {
                let admission = {
                    let mut state = shared.lock()?;
                    state.defer_command_if_current(
                        &command,
                        not_before_ms,
                        Utc::now(),
                        Instant::now(),
                    )?
                };
                if admission != DeferredCommandAdmission::Ready {
                    shared.wake();
                    return Ok(CommandDisposition::Deferred);
                }
            }
            Ok(_) => {}
            Err(_) => return Err(MarketStreamProducerError::TransportFailure),
        }
        let lease = {
            let mut state = shared.lock()?;
            state.admit_command(&command, true, Utc::now(), Instant::now())?
        };
        let Some(lease) = lease else {
            shared.wake();
            return Ok(CommandDisposition::Deferred);
        };
        shared.wake();
        repository
            .set_subscription_desired(&lease, epoch, &command.symbol, command.reference_count)
            .await
            .map_err(map_runtime_storage)?;
        {
            let mut state = shared.lock()?;
            state
                .bindings
                .persist_count(&command.symbol, command.reference_count);
        }
        let prepared = match command.kind {
            CommandKind::Subscribe => session.prepare_subscribe(&command.symbol),
            CommandKind::Unsubscribe => session.prepare_unsubscribe(&command.symbol),
            CommandKind::CountChange => return Err(MarketStreamProducerError::InvalidInput),
        }
        .map_err(|error| {
            #[cfg(feature = "market-stream-db-tests")]
            eprintln!("KIS_TEST_DIAG prepare_command error={error}");
            let _ = error;
            MarketStreamProducerError::TransportFailure
        })?
        .ok_or(MarketStreamProducerError::SubscriptionOutOfSync)?;
        if prepared.credential_slot_id() != lease.credential_slot_id
            || prepared.epoch().uuid() != epoch
            || prepared.symbol() != command.symbol
            || prepared.operation() != operation
        {
            return Err(MarketStreamProducerError::SubscriptionOutOfSync);
        }
        let pending = repository
            .commit_prepared_subscription(&lease, &prepared)
            .await
            .inspect_err(|_error| {
                #[cfg(feature = "market-stream-db-tests")]
                eprintln!("KIS_TEST_DIAG persist_pending error={_error:?}");
            })
            .map_err(map_runtime_storage)?;
        let ack = session.send_prepared(prepared).await.map_err(|error| {
            #[cfg(feature = "market-stream-db-tests")]
            eprintln!("KIS_TEST_DIAG send_prepared error={error}");
            let _ = error;
            MarketStreamProducerError::TransportFailure
        })?;
        let committed = repository
            .commit_subscription_ack(&lease, pending, ack)
            .await
            .inspect_err(|_error| {
                #[cfg(feature = "market-stream-db-tests")]
                eprintln!("KIS_TEST_DIAG persist_ack error={_error:?}");
            })
            .map_err(map_runtime_storage)?;
        match command.kind {
            CommandKind::Subscribe => {
                let proof = committed.ok_or(MarketStreamProducerError::SubscriptionOutOfSync)?;
                if proof.credential_slot_id != lease.credential_slot_id
                    || proof.grant_revision != lease.grant_revision
                    || proof.epoch != epoch
                    || proof.symbol != command.symbol
                    || proof.state != "ACKED"
                    || proof.acked_at.is_none()
                    || proof.desired_reference_count == 0
                    || proof.subscription_revision.is_nil()
                {
                    return Err(MarketStreamProducerError::SubscriptionOutOfSync);
                }
                let mut state = shared.lock()?;
                state
                    .bindings
                    .finish_subscribe(&command.symbol, proof, command.reference_count);
            }
            CommandKind::Unsubscribe => {
                if committed.is_some() {
                    return Err(MarketStreamProducerError::SubscriptionOutOfSync);
                }
                let mut state = shared.lock()?;
                state.bindings.finish_unsubscribe(&command.symbol);
                if state
                    .bindings
                    .desired
                    .get(&command.symbol)
                    .is_some_and(|binding| binding.reference_count > 0)
                {
                    state.request_gap(RuntimeTransition::ResyncRequired);
                }
            }
            CommandKind::CountChange => return Err(MarketStreamProducerError::InvalidInput),
        }
        shared.wake();
        Ok(CommandDisposition::Committed)
    }

    fn map_runtime_storage(error: RuntimeMarketStreamStorageError) -> MarketStreamProducerError {
        match error {
            RuntimeMarketStreamStorageError::Storage(error) => error.into(),
            RuntimeMarketStreamStorageError::DeadlineExceeded
            | RuntimeMarketStreamStorageError::Terminal => MarketStreamProducerError::Terminal,
        }
    }

    async fn publication_denial_stop_reason<F, Fut>(
        error: RuntimeMarketStreamStorageError,
        lease: &StreamProducerLease,
        read_demand: F,
    ) -> Result<StopReason, MarketStreamProducerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<DesiredSet, RuntimeMarketStreamStorageError>>,
    {
        // Release or logout can invalidate a batch after its handoff. A known
        // denial alone is ambiguous: only a fresh, rights-checked empty demand
        // observation may select cleanup. No denied batch is retried or committed.
        if !matches!(
            error,
            RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::RightsInvalid | MarketStreamStorageError::SessionInvalid
            )
        ) {
            return Err(map_runtime_storage(error));
        }
        let demand = read_demand().await.map_err(map_runtime_storage)?;
        if normalize_runtime_demand(&demand, lease).is_ok_and(|bindings| bindings.is_empty()) {
            Ok(StopReason::NoDemand)
        } else {
            Err(map_runtime_storage(error))
        }
    }

    async fn publication_membership_stop_reason<F, Fut>(
        lease: &StreamProducerLease,
        denied_identities: &[StreamIdentity],
        read_demand: F,
    ) -> Result<StopReason, MarketStreamProducerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<DesiredSet, RuntimeMarketStreamStorageError>>,
    {
        let denial = MarketStreamProducerError::Storage(MarketStreamStorageError::MembershipNotReady);
        if denied_identities.is_empty()
            || denied_identities
                .iter()
                .any(|identity| identity.owner_user_id != lease.owner_user_id)
        {
            return Err(denial);
        }
        // The repository has already rejected this batch before commit. Only
        // fresh rights-checked demand can confirm an identity transition. Drop
        // the old batch and drain its epoch; never replay its receipts or proofs.
        let demand = read_demand().await.map_err(map_runtime_storage)?;
        let current = normalize_runtime_demand(&demand, lease).map_err(|_| denial)?;
        let changed = denied_identities.iter().any(|identity| {
            current
                .get(identity.symbol())
                .is_none_or(|binding| &binding.identity != identity)
        });
        if !changed {
            return Err(denial);
        }
        Ok(if current.is_empty() {
            StopReason::NoDemand
        } else {
            StopReason::FreshEpochRequired
        })
    }

    async fn writer_task(
        shared: Arc<SharedOwner>,
        repository: RuntimeMarketStreamRepository,
        day: ResolvedMarketStreamDay,
        epoch: Uuid,
    ) {
        loop {
            let action = {
                let mut state = match shared.lock() {
                    Ok(state) => state,
                    Err(error) => {
                        shared.fail(error);
                        return;
                    }
                };
                if state.lifecycle.failure.is_some() {
                    return;
                }
                let now = Instant::now();
                if let Some(work) = state.mailbox.take_ready(now) {
                    Some(WriterAction::Status {
                        lease: state.lease.clone(),
                        work,
                        expected_gap: state.epoch.gap_generation,
                    })
                } else if state.lifecycle.stopping {
                    if state.mailbox.has_work() {
                        None
                    } else {
                        return;
                    }
                } else {
                    match state.refresh_buffer_context() {
                        Ok(Some(_)) => match state.buffer.handoff_batch() {
                            Ok(BatchAvailability::Ready(batch)) => {
                                if !state.buffer.batch_is_current(&batch) {
                                    state.request_gap(RuntimeTransition::ResyncRequired);
                                    None
                                } else if state.buffer.mark_transaction_start(&batch).is_err() {
                                    state.request_gap(RuntimeTransition::PipelineLag);
                                    None
                                } else {
                                    Some(WriterAction::Publish { batch })
                                }
                            }
                            Ok(BatchAvailability::PipelineLag) => {
                                state.request_gap(RuntimeTransition::PipelineLag);
                                None
                            }
                            Ok(BatchAvailability::Empty | BatchAvailability::Waiting) => None,
                            Err(_) => {
                                state.fail(MarketStreamProducerError::Terminal);
                                None
                            }
                        },
                        Ok(None) => None,
                        Err(error) => {
                            state.fail(error);
                            None
                        }
                    }
                }
            };
            let Some(action) = action else {
                tokio::select! {
                    _ = shared.changed.notified() => {},
                    _ = tokio::time::sleep(OBSERVATION_TICK) => {},
                }
                continue;
            };
            match action {
                WriterAction::Status {
                    lease,
                    work,
                    expected_gap,
                } => {
                    let result = repository
                        .record_runtime_status(&lease, Some(epoch), work.transition)
                        .await;
                    match result {
                        Ok(committed) => {
                            if !status_commit_matches(
                                committed,
                                epoch,
                                expected_gap,
                                work.transition.values().opens_gap,
                            ) {
                                shared.fail(MarketStreamProducerError::Terminal);
                                return;
                            }
                        }
                        Err(error) => {
                            shared.fail(map_runtime_storage(error));
                            return;
                        }
                    }
                }
                WriterAction::Publish { batch } => {
                    let result = repository
                        .publish_stream_latest_resolved(batch.context(), &day, batch.observations())
                        .await;
                    match result {
                        Ok(_) => {
                            let mut state = match shared.lock() {
                                Ok(state) => state,
                                Err(error) => {
                                    shared.fail(error);
                                    return;
                                }
                            };
                            if state.buffer.batch_is_current(&batch) {
                                if state.buffer.complete_success(batch).is_err() {
                                    state.fail(MarketStreamProducerError::Terminal);
                                    shared.wake();
                                    return;
                                }
                                state.mark_known_quote();
                            } else if !state.lifecycle.stopping {
                                state.request_gap(RuntimeTransition::ResyncRequired);
                            }
                            drop(state);
                            shared.wake();
                        }
                        Err(error) => {
                            let read_demand = || {
                                repository.read_stream_demand(
                                    batch.context().producer.credential_slot_id,
                                )
                            };
                            let reason = if error
                                == RuntimeMarketStreamStorageError::Storage(
                                    MarketStreamStorageError::MembershipNotReady,
                                )
                            {
                                let identities = batch
                                    .context()
                                    .items
                                    .iter()
                                    .map(|item| item.admission.clone())
                                    .collect::<Vec<_>>();
                                publication_membership_stop_reason(
                                    &batch.context().producer,
                                    &identities,
                                    read_demand,
                                )
                                .await
                            } else {
                                publication_denial_stop_reason(
                                    error,
                                    &batch.context().producer,
                                    read_demand,
                                )
                                .await
                            };
                            match reason {
                                Ok(reason) => shared.request_stop(reason),
                                Err(error) => {
                                    shared.fail(error);
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    enum WriterAction {
        Status {
            lease: StreamProducerLease,
            work: StatusWork,
            expected_gap: u64,
        },
        Publish {
            batch: PublicationBatch,
        },
    }

    fn gap_generation_matches(expected: u64, observed: u64, opens_gap: bool) -> bool {
        if opens_gap {
            observed == expected || observed == expected.saturating_add(1)
        } else {
            observed == expected
        }
    }

    fn status_commit_matches(
        committed: RuntimeStatusCommit,
        epoch: Uuid,
        expected_gap: u64,
        opens_gap: bool,
    ) -> bool {
        committed.current_epoch() == Some(epoch)
            && gap_generation_matches(expected_gap, committed.gap_generation(), opens_gap)
    }

    fn request_gap_parts(
        lifecycle: &mut RuntimeLifecycle,
        buffer: &mut ReceiptBuffer,
        mailbox: &mut StatusMailbox,
        transition: RuntimeTransition,
    ) {
        lifecycle.request_gap(transition);
        buffer.invalidate(InvalidationCause::TerminalTransport);
        mailbox.routine = None;
        mailbox.urgent(transition);
    }

    fn retirement_transition(
        reason: StopReason,
        gap: Option<RuntimeTransition>,
    ) -> StreamStatusCode {
        reason.retirement_code(gap)
    }

    fn validate_start_inputs(
        lease: &StreamProducerLease,
        day: &ResolvedMarketStreamDay,
        session: &MarketStreamSession,
        now: DateTime<Utc>,
    ) -> Result<Uuid, MarketStreamProducerError> {
        if session.is_closed() || session.subscribed_symbols().next().is_some() {
            return Err(MarketStreamProducerError::SessionUnavailable);
        }
        let epoch = session.epoch().uuid();
        if epoch.is_nil()
            || lease.credential_slot_id.is_nil()
            || lease.grant_id.is_nil()
            || lease.grant_revision.is_nil()
            || lease.owner_user_id.is_nil()
            || lease.holder_id.is_nil()
            || lease.fencing_token == 0
            || lease.lease_expires_at <= lease.heartbeat_at
            || lease.heartbeat_at > now
            || lease.lease_expires_at <= now + MIN_LEASE_MARGIN
            || !date_and_window_are_open(day.session_date(), day.open_at(), day.close_at(), now)
        {
            return Err(MarketStreamProducerError::SessionUnavailable);
        }
        Ok(epoch)
    }

    pub(super) async fn start_resolved(
        repository: RuntimeMarketStreamRepository,
        lease: StreamProducerLease,
        day: ResolvedMarketStreamDay,
        session: MarketStreamSession,
    ) -> Result<super::RuntimeOwnedProducer, MarketStreamProducerError> {
        let epoch_id = validate_start_inputs(&lease, &day, &session, Utc::now())?;
        let epoch = repository
            .start_stream_epoch_resolved(&lease, &day, epoch_id)
            .await
            .map_err(map_runtime_storage)?;
        if epoch.epoch != epoch_id
            || epoch.credential_slot_id != lease.credential_slot_id
            || epoch.grant_revision != lease.grant_revision
            || epoch.fencing_token != lease.fencing_token
            || epoch.gap_generation != lease.gap_generation
            || epoch.session != day.session()
            || epoch.session.session_date != day.session_date()
            || epoch_id.is_nil()
        {
            return Err(MarketStreamProducerError::SessionUnavailable);
        }
        Ok(super::RuntimeOwnedProducer {
            repository,
            lease,
            epoch,
            day,
            session: Some(session),
        })
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialRunMode {
        Active,
        CleanupOnly,
    }

    impl InitialRunMode {
        fn observes_control(self) -> bool {
            self == Self::Active
        }
    }

    fn initial_run_mode(
        initial_shutdown: bool,
        initialize_active: impl FnOnce() -> Result<(), MarketStreamProducerError>,
    ) -> Result<InitialRunMode, MarketStreamProducerError> {
        if initial_shutdown {
            // A committed owner can arrive after its activation window closed.
            // Cleanup must retain that owner without refreshing or admitting inputs.
            return Ok(InitialRunMode::CleanupOnly);
        }
        initialize_active()?;
        Ok(InitialRunMode::Active)
    }

    pub(super) async fn run_owned(
        owner: super::RuntimeOwnedProducer,
        mut demand: tokio::sync::watch::Receiver<DesiredSet>,
        mut lease: tokio::sync::watch::Receiver<StreamProducerLease>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<RuntimeOwnedExit, MarketStreamProducerError> {
        let state = RuntimeState::new(owner.lease.clone(), owner.epoch.clone(), owner.day.clone());
        let shared = Arc::new(SharedOwner::new(state));
        let mut cancellation_guard = RunCancellationGuard::new(shared.clone());
        let mode = {
            let initial_shutdown = *shutdown.borrow_and_update();
            let mut state = shared.lock()?;
            let mode = initial_run_mode(initial_shutdown, || {
                let initial_demand = demand.borrow_and_update();
                state.apply_demand_checked(&initial_demand)?;
                state.demand_observed_at = Instant::now();
                let initial_lease = lease.borrow_and_update().clone();
                state.update_lease(initial_lease, Utc::now())
            })?;
            if mode == InitialRunMode::CleanupOnly || *shutdown.borrow_and_update() {
                state.stop(StopReason::Shutdown);
            }
            mode
        };
        let session = owner
            .session
            .ok_or(MarketStreamProducerError::SessionUnavailable)?;
        let epoch_id = owner.epoch.epoch;
        let day = owner.day.clone();
        let mut group = OwnedTaskGroup::new();
        group.spawn(socket_task(
            session,
            shared.clone(),
            owner.repository.clone(),
            day.clone(),
            epoch_id,
        ));
        group.spawn(writer_task(
            shared.clone(),
            owner.repository.clone(),
            day,
            epoch_id,
        ));

        // Cleanup-only startup owns no admitted work. Its original committed
        // lease remains the retirement input; watch contents are not fresh evidence.
        let mut demand_open = mode.observes_control();
        let mut lease_open = mode.observes_control();
        let mut shutdown_open = mode.observes_control();
        let mut joined = 0_u8;
        let mut drain_deadline = None;
        while joined < 2 {
            let now_wall = Utc::now();
            let now_mono = Instant::now();
            let observation = match shared.lock() {
                Ok(mut state) => {
                    if mode.observes_control() {
                        state.fail_if_demand_stale(now_mono);
                    }
                    state.check_local_eligibility(now_wall);
                    if state.lifecycle.stopping && drain_deadline.is_none() {
                        drain_deadline = Some(tokio::time::Instant::now() + CLEAN_DRAIN_BOUND);
                    }
                    Ok(state.lifecycle.failure.is_some())
                }
                Err(error) => Err(error),
            };
            let failure_pending = match observation {
                Ok(failure_pending) => failure_pending,
                Err(error) => {
                    group.abort_and_join().await;
                    return Err(error);
                }
            };
            if failure_pending {
                shared.wake();
            }
            if let Some(deadline) = drain_deadline
                && tokio::time::Instant::now() >= deadline
            {
                shared.fail(MarketStreamProducerError::Terminal);
                group.abort_and_join().await;
                break;
            }
            let mut child_failed = false;
            tokio::select! {
                changed = demand.changed(), if demand_open => {
                    match changed {
                        Ok(()) => {
                            let applied = {
                                let latest = demand.borrow_and_update();
                                shared.lock().and_then(|mut state| {
                                    state.apply_demand_checked(&latest)
                                })
                            };
                            if let Err(error) = applied { shared.fail(error); }
                            else if let Ok(mut state) = shared.lock() { state.demand_observed_at = Instant::now(); }
                            shared.wake();
                        }
                        Err(_) => { demand_open = false; shared.fail(MarketStreamProducerError::Terminal); }
                    }
                }
                changed = lease.changed(), if lease_open => {
                    match changed {
                        Ok(()) => {
                            let latest = lease.borrow_and_update().clone();
                            let updated = shared.lock().and_then(|mut state| state.update_lease(latest, Utc::now()));
                            if let Err(error) = updated { shared.fail(error); }
                            shared.wake();
                        }
                        Err(_) => { lease_open = false; shared.fail(MarketStreamProducerError::Terminal); }
                    }
                }
                changed = shutdown.changed(), if shutdown_open => {
                    match changed {
                        Ok(()) if *shutdown.borrow_and_update() => shared.request_stop(StopReason::Shutdown),
                        Ok(()) => {},
                        Err(_) => { shutdown_open = false; shared.fail(MarketStreamProducerError::Terminal); }
                    }
                }
                result = group.join_next() => {
                    match result {
                        Some(Ok(())) => joined = joined.saturating_add(1),
                        Some(Err(_)) => {
                            joined = joined.saturating_add(1);
                            shared.fail(MarketStreamProducerError::Terminal);
                            child_failed = true;
                        }
                        None => joined = 2,
                    }
                }
                _ = tokio::time::sleep(OBSERVATION_TICK) => {}
            }
            if child_failed {
                group.abort_and_join().await;
                break;
            }
        }
        if !group.is_empty() {
            group.abort_and_join().await;
            return Err(MarketStreamProducerError::Terminal);
        }
        let (failure, reason, gap_transition, closed, final_lease) = {
            let state = shared.lock()?;
            (
                state.lifecycle.failure,
                state.lifecycle.reason,
                state.lifecycle.gap_transition,
                state.lifecycle.socket_closed,
                state.lease.clone(),
            )
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if !closed {
            return Err(MarketStreamProducerError::Terminal);
        }
        let reason = reason.ok_or(MarketStreamProducerError::Terminal)?;
        let retired = owner
            .repository
            .retire_stream_producer(
                &final_lease,
                Some(epoch_id),
                retirement_transition(reason, gap_transition),
            )
            .await
            .map_err(map_runtime_storage)?;
        if retired.current_epoch().is_some()
            || (gap_transition.is_some() && retired.gap_generation() < final_lease.gap_generation)
        {
            return Err(MarketStreamProducerError::Terminal);
        }
        cancellation_guard.completed = true;
        Ok(reason.exit())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use chrono::{DateTime, Utc};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};
        use tokio::sync::oneshot;

        use super::super::super::market_stream::{DesiredStreamItem, MarketStreamStorageError};

        fn test_now() -> DateTime<Utc> {
            DateTime::parse_from_rfc3339("2026-10-02T01:00:00Z")
                .expect("fixed timestamp")
                .with_timezone(&Utc)
        }

        fn test_date() -> chrono::NaiveDate {
            chrono::NaiveDate::from_ymd_opt(2026, 10, 2).expect("fixed date")
        }

        fn test_window() -> (DateTime<Utc>, DateTime<Utc>) {
            (
                DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
                    .expect("fixed open")
                    .with_timezone(&Utc),
                DateTime::parse_from_rfc3339("2026-10-02T08:00:00Z")
                    .expect("fixed close")
                    .with_timezone(&Utc),
            )
        }

        fn test_lease(now: DateTime<Utc>) -> StreamProducerLease {
            StreamProducerLease {
                credential_slot_id: Uuid::new_v4(),
                grant_id: Uuid::new_v4(),
                grant_revision: Uuid::new_v4(),
                owner_user_id: Uuid::new_v4(),
                holder_id: Uuid::new_v4(),
                fencing_token: 7,
                gap_generation: 3,
                lease_expires_at: now + chrono::Duration::seconds(45),
                heartbeat_at: now - chrono::Duration::seconds(2),
            }
        }

        fn test_identity(owner: Uuid, symbol: &str, membership: Uuid) -> StreamIdentity {
            StreamIdentity::new(
                owner,
                membership,
                Uuid::new_v4(),
                format!("{symbol}.KRX"),
                1,
            )
            .expect("synthetic identity")
        }

        fn test_demand(
            lease: &StreamProducerLease,
            items: Vec<(StreamIdentity, u32)>,
        ) -> DesiredSet {
            DesiredSet {
                credential_slot_id: lease.credential_slot_id,
                owner_user_id: lease.owner_user_id,
                items: items
                    .into_iter()
                    .map(|(identity, reference_count)| DesiredStreamItem {
                        identity,
                        reference_count,
                    })
                    .collect(),
            }
        }

        fn test_proof(
            lease: &StreamProducerLease,
            identity: &StreamIdentity,
            epoch: Uuid,
            count: u32,
        ) -> SubscriptionProof {
            SubscriptionProof {
                credential_slot_id: lease.credential_slot_id,
                symbol: identity.symbol().to_owned(),
                grant_revision: lease.grant_revision,
                epoch,
                state: "ACKED".to_owned(),
                subscription_revision: Uuid::new_v4(),
                acked_at: Some(test_now()),
                desired_reference_count: count,
            }
        }

        #[tokio::test]
        async fn publication_generation_change_waits_for_fresh_demand_then_drains() {
            let lease = test_lease(test_now());
            let old = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let mut replacement = old.clone();
            replacement.generation_id = Uuid::new_v4();
            replacement.generation += 1;
            let denied = vec![old];
            for items in [vec![(replacement, 1)], Vec::new()] {
                let expected = if items.is_empty() {
                    StopReason::NoDemand
                } else {
                    StopReason::FreshEpochRequired
                };
                let reads = AtomicUsize::new(0);
                let (release, observed) = oneshot::channel();
                let mut resolution = Box::pin(publication_membership_stop_reason(
                    &lease,
                    &denied,
                    || async {
                        reads.fetch_add(1, Ordering::SeqCst);
                        observed.await.expect("fresh demand released")
                    },
                ));
                let mut context = std::task::Context::from_waker(std::task::Waker::noop());
                assert!(resolution.as_mut().poll(&mut context).is_pending());
                assert_eq!(reads.load(Ordering::SeqCst), 1);
                release.send(Ok(test_demand(&lease, items))).unwrap();
                let reason = resolution.await.expect("confirmed identity transition");
                assert_eq!(reason, expected);
                assert_eq!(reads.load(Ordering::SeqCst), 1);
                let mut lifecycle = RuntimeLifecycle::default();
                lifecycle.request_stop(reason);
                assert_eq!(lifecycle.clean_exit(), None);
                lifecycle.record_close(Ok(()));
                assert_eq!(lifecycle.clean_exit(), Some(reason.exit()));
            }
        }

        #[tokio::test]
        async fn publication_generation_denial_rejects_unchanged_or_invalid_demand() {
            let lease = test_lease(test_now());
            let old = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let live = test_demand(&lease, vec![(old.clone(), 1)]);
            let mut wrong_owner = test_demand(&lease, Vec::new());
            wrong_owner.owner_user_id = Uuid::new_v4();
            let mut wrong_slot = test_demand(&lease, Vec::new());
            wrong_slot.credential_slot_id = Uuid::new_v4();
            for demand in [live, wrong_owner, wrong_slot] {
                let reads = AtomicUsize::new(0);
                let result = publication_membership_stop_reason(
                    &lease,
                    std::slice::from_ref(&old),
                    || async {
                        reads.fetch_add(1, Ordering::SeqCst);
                        Ok(demand)
                    },
                )
                .await;
                assert_eq!(reads.load(Ordering::SeqCst), 1);
                assert_eq!(
                    result,
                    Err(MarketStreamProducerError::Storage(
                        MarketStreamStorageError::MembershipNotReady
                    ))
                );
            }
        }

        #[tokio::test]
        async fn publication_generation_denial_preserves_failed_demand_read() {
            let lease = test_lease(test_now());
            let old = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            for error in [
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::RightsInvalid),
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::CommitUnknown),
                RuntimeMarketStreamStorageError::Storage(
                    MarketStreamStorageError::DatabaseUnavailable,
                ),
                RuntimeMarketStreamStorageError::DeadlineExceeded,
                RuntimeMarketStreamStorageError::Terminal,
            ] {
                let reads = AtomicUsize::new(0);
                let result = publication_membership_stop_reason(
                    &lease,
                    std::slice::from_ref(&old),
                    || async {
                        reads.fetch_add(1, Ordering::SeqCst);
                        Err(error)
                    },
                )
                .await;
                assert_eq!(reads.load(Ordering::SeqCst), 1);
                assert_eq!(result, Err(map_runtime_storage(error)));
            }
        }

        #[tokio::test]
        async fn publication_session_revocation_waits_for_authoritative_empty_demand() {
            let lease = test_lease(test_now());
            let reads = AtomicUsize::new(0);
            let (release, observed) = oneshot::channel();
            let mut resolution = Box::pin(publication_denial_stop_reason(
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::SessionInvalid),
                &lease,
                || async {
                    reads.fetch_add(1, Ordering::SeqCst);
                    observed.await.expect("fresh observation released")
                },
            ));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(
                resolution.as_mut().poll(&mut context).is_pending(),
                "session revocation must await authoritative demand before choosing cleanup"
            );
            assert_eq!(reads.load(Ordering::SeqCst), 1);
            release
                .send(Ok(test_demand(&lease, Vec::new())))
                .expect("fresh observation receiver");
            let reason = resolution.await.expect("confirmed logout has no demand");
            assert_eq!(reason, StopReason::NoDemand);
            assert_eq!(reads.load(Ordering::SeqCst), 1);
            let mut lifecycle = RuntimeLifecycle::default();
            lifecycle.request_stop(reason);
            assert_eq!(lifecycle.clean_exit(), None);
            assert!(lifecycle.failure.is_none());
            lifecycle.record_close(Ok(()));
            assert_eq!(lifecycle.clean_exit(), Some(RuntimeOwnedExit::NoDemand));
            assert_eq!(
                retirement_transition(reason, None),
                StreamStatusCode::NoActiveDemand
            );
        }

        #[tokio::test]
        async fn publication_session_revocation_rejects_live_or_mismatched_demand() {
            let lease = test_lease(test_now());
            let identity = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let live = test_demand(&lease, vec![(identity, 1)]);
            let mut wrong_slot = test_demand(&lease, Vec::new());
            wrong_slot.credential_slot_id = Uuid::new_v4();
            let mut wrong_owner = test_demand(&lease, Vec::new());
            wrong_owner.owner_user_id = Uuid::new_v4();
            for demand in [live, wrong_slot, wrong_owner] {
                let reads = AtomicUsize::new(0);
                let result = publication_denial_stop_reason(
                    RuntimeMarketStreamStorageError::Storage(
                        MarketStreamStorageError::SessionInvalid,
                    ),
                    &lease,
                    || async {
                        reads.fetch_add(1, Ordering::SeqCst);
                        Ok(demand)
                    },
                )
                .await;
                assert_eq!(reads.load(Ordering::SeqCst), 1);
                assert_eq!(
                    result,
                    Err(MarketStreamProducerError::Storage(
                        MarketStreamStorageError::SessionInvalid
                    ))
                );
            }
        }

        #[tokio::test]
        async fn publication_session_revocation_preserves_failed_demand_observation() {
            let lease = test_lease(test_now());
            for error in [
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::RightsInvalid),
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::SessionInvalid),
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::CommitUnknown),
                RuntimeMarketStreamStorageError::Storage(
                    MarketStreamStorageError::DatabaseUnavailable,
                ),
                RuntimeMarketStreamStorageError::DeadlineExceeded,
                RuntimeMarketStreamStorageError::Terminal,
            ] {
                let reads = AtomicUsize::new(0);
                let result = publication_denial_stop_reason(
                    RuntimeMarketStreamStorageError::Storage(
                        MarketStreamStorageError::SessionInvalid,
                    ),
                    &lease,
                    || async {
                        reads.fetch_add(1, Ordering::SeqCst);
                        Err(error)
                    },
                )
                .await;
                assert_eq!(reads.load(Ordering::SeqCst), 1);
                assert_eq!(result, Err(map_runtime_storage(error)));
            }
        }

        #[tokio::test]
        async fn publication_denial_waits_for_authoritative_empty_demand() {
            let lease = test_lease(test_now());
            let empty = test_demand(&lease, Vec::new());
            let reads = AtomicUsize::new(0);
            let (release, observed) = oneshot::channel();
            let mut resolution = Box::pin(publication_denial_stop_reason(
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::RightsInvalid),
                &lease,
                || async {
                    reads.fetch_add(1, Ordering::SeqCst);
                    observed.await.expect("authoritative observation released")
                },
            ));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(resolution.as_mut().poll(&mut context).is_pending());
            assert_eq!(reads.load(Ordering::SeqCst), 1);
            release
                .send(Ok(empty))
                .expect("pending observation receiver");
            let reason = resolution
                .await
                .expect("confirmed zero demand selects cleanup");
            assert_eq!(reason, StopReason::NoDemand);
            assert_eq!(reads.load(Ordering::SeqCst), 1);
            let mut lifecycle = RuntimeLifecycle::default();
            lifecycle.request_stop(reason);
            assert!(lifecycle.stopping);
            assert_eq!(lifecycle.clean_exit(), None);
            assert!(lifecycle.failure.is_none());
            lifecycle.record_close(Ok(()));
            assert_eq!(lifecycle.clean_exit(), Some(RuntimeOwnedExit::NoDemand));
            assert_eq!(
                retirement_transition(reason, None),
                StreamStatusCode::NoActiveDemand
            );
        }

        #[tokio::test]
        async fn publication_denial_rejects_live_or_mismatched_demand() {
            let lease = test_lease(test_now());
            let identity = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let live = test_demand(&lease, vec![(identity, 1)]);
            let mut wrong_slot = test_demand(&lease, Vec::new());
            wrong_slot.credential_slot_id = Uuid::new_v4();
            let mut wrong_owner = test_demand(&lease, Vec::new());
            wrong_owner.owner_user_id = Uuid::new_v4();
            for demand in [live, wrong_slot, wrong_owner] {
                let result = publication_denial_stop_reason(
                    RuntimeMarketStreamStorageError::Storage(
                        MarketStreamStorageError::RightsInvalid,
                    ),
                    &lease,
                    || async { Ok(demand) },
                )
                .await;
                assert_eq!(
                    result,
                    Err(MarketStreamProducerError::Storage(
                        MarketStreamStorageError::RightsInvalid
                    ))
                );
            }
        }

        #[tokio::test]
        async fn publication_denial_preserves_failed_demand_observation() {
            let lease = test_lease(test_now());
            for error in [
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::RightsInvalid),
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::CommitUnknown),
                RuntimeMarketStreamStorageError::Storage(
                    MarketStreamStorageError::DatabaseUnavailable,
                ),
                RuntimeMarketStreamStorageError::DeadlineExceeded,
                RuntimeMarketStreamStorageError::Terminal,
            ] {
                let result = publication_denial_stop_reason(
                    RuntimeMarketStreamStorageError::Storage(
                        MarketStreamStorageError::RightsInvalid,
                    ),
                    &lease,
                    || async { Err(error) },
                )
                .await;
                assert_eq!(result, Err(map_runtime_storage(error)));
            }
        }

        #[tokio::test]
        async fn publication_uncertainty_never_starts_demand_reconciliation() {
            let lease = test_lease(test_now());
            let reads = AtomicUsize::new(0);
            for error in [
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::CommitUnknown),
                RuntimeMarketStreamStorageError::Storage(MarketStreamStorageError::ProducerLost),
                RuntimeMarketStreamStorageError::Storage(
                    MarketStreamStorageError::MembershipNotReady,
                ),
                RuntimeMarketStreamStorageError::DeadlineExceeded,
                RuntimeMarketStreamStorageError::Terminal,
            ] {
                let result = publication_denial_stop_reason(error, &lease, || async {
                    reads.fetch_add(1, Ordering::SeqCst);
                    Ok(test_demand(&lease, Vec::new()))
                })
                .await;
                assert_eq!(result, Err(map_runtime_storage(error)));
            }
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }

        #[test]
        fn initial_shutdown_skips_stale_activation_without_freshening_inputs() {
            let (open, close) = test_window();
            let lease = test_lease(close - chrono::Duration::seconds(1));
            let identity = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let mut stale_demand = test_demand(&lease, vec![(identity, 1)]);
            stale_demand.credential_slot_id = Uuid::new_v4();
            let original_observation = Instant::now() - Duration::from_secs(10);

            for now in [close, close + chrono::Duration::days(1)] {
                assert!(!lease_refresh_is_valid(
                    &lease,
                    &lease,
                    lease.gap_generation,
                    test_date(),
                    open,
                    close,
                    now,
                ));
                let mut observed_at = original_observation;
                let mut activation_calls = 0;
                let mode = initial_run_mode(true, || {
                    activation_calls += 1;
                    observed_at = Instant::now();
                    normalize_runtime_demand(&stale_demand, &lease)
                        .map_err(|_| MarketStreamProducerError::InvalidInput)?;
                    if !lease_refresh_is_valid(
                        &lease,
                        &lease,
                        lease.gap_generation,
                        test_date(),
                        open,
                        close,
                        now,
                    ) {
                        return Err(MarketStreamProducerError::Terminal);
                    }
                    Ok(())
                })
                .expect("latched shutdown reaches cleanup despite stale activation inputs");
                assert_eq!(mode, InitialRunMode::CleanupOnly);
                assert!(!mode.observes_control());
                assert_eq!(activation_calls, 0);
                assert_eq!(observed_at, original_observation);
            }
        }

        #[test]
        fn active_startup_preserves_input_validation_and_error_types() {
            let now = test_now();
            let (open, close) = test_window();
            let lease = test_lease(now);
            let identity = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let demand = test_demand(&lease, vec![(identity, 1)]);
            let activation_calls = std::cell::Cell::new(0);
            let activate = |desired: &DesiredSet, next: &StreamProducerLease, at: DateTime<Utc>| {
                initial_run_mode(false, || {
                    activation_calls.set(activation_calls.get() + 1);
                    normalize_runtime_demand(desired, &lease)
                        .map_err(|_| MarketStreamProducerError::InvalidInput)?;
                    if !lease_refresh_is_valid(
                        &lease,
                        next,
                        lease.gap_generation,
                        test_date(),
                        open,
                        close,
                        at,
                    ) {
                        return Err(MarketStreamProducerError::Terminal);
                    }
                    Ok(())
                })
            };
            let mode = activate(&demand, &lease, now).expect("eligible active startup");
            assert_eq!(mode, InitialRunMode::Active);
            assert!(mode.observes_control());

            let mut wrong_owner = lease.clone();
            wrong_owner.holder_id = Uuid::new_v4();
            assert_eq!(
                activate(&demand, &wrong_owner, now),
                Err(MarketStreamProducerError::Terminal)
            );
            assert_eq!(
                activate(&demand, &lease, close),
                Err(MarketStreamProducerError::Terminal)
            );
            let mut invalid_demand = demand.clone();
            invalid_demand.credential_slot_id = Uuid::new_v4();
            assert_eq!(
                activate(&invalid_demand, &lease, now),
                Err(MarketStreamProducerError::InvalidInput)
            );
            assert_eq!(activation_calls.get(), 4);
        }

        #[test]
        fn lease_refresh_keeps_authority_and_margin() {
            let now = test_now();
            let (open, close) = test_window();
            let previous = test_lease(now);
            let mut next = previous.clone();
            next.heartbeat_at = now - chrono::Duration::seconds(1);
            next.lease_expires_at = now + chrono::Duration::seconds(50);
            assert!(lease_refresh_is_valid(
                &previous,
                &next,
                previous.gap_generation,
                test_date(),
                open,
                close,
                now
            ));

            let mut changed = next.clone();
            changed.holder_id = Uuid::new_v4();
            assert!(!lease_refresh_is_valid(
                &previous,
                &changed,
                previous.gap_generation,
                test_date(),
                open,
                close,
                now
            ));
            let mut short = next.clone();
            short.lease_expires_at = now + MIN_LEASE_MARGIN;
            assert!(!lease_refresh_is_valid(
                &previous,
                &short,
                previous.gap_generation,
                test_date(),
                open,
                close,
                now
            ));
            let mut regressed = next.clone();
            regressed.heartbeat_at = previous.heartbeat_at - chrono::Duration::seconds(1);
            assert!(!lease_refresh_is_valid(
                &previous,
                &regressed,
                previous.gap_generation,
                test_date(),
                open,
                close,
                now
            ));
            assert!(!lease_refresh_is_valid(
                &previous,
                &next,
                previous.gap_generation,
                test_date(),
                open,
                close,
                close,
            ));
            assert!(!lease_refresh_is_valid(
                &previous,
                &next,
                previous.gap_generation,
                test_date(),
                open,
                close,
                DateTime::parse_from_rfc3339("2026-10-01T14:59:00Z")
                    .expect("prior KST date")
                    .with_timezone(&Utc),
            ));
            assert_eq!(next.fencing_token, previous.fencing_token);
            assert_eq!(next.grant_revision, previous.grant_revision);
        }

        #[test]
        fn demand_changes_suppress_removed_bindings() {
            let now = test_now();
            let lease = test_lease(now);
            let epoch = Uuid::new_v4();
            let identity = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let mut book = BindingBook::default();
            assert!(
                book.apply_demand(&test_demand(&lease, vec![(identity.clone(), 1)]), &lease)
                    .unwrap()
                    .is_empty()
            );
            book.acked.insert(
                identity.symbol().to_owned(),
                AckBinding {
                    identity: identity.clone(),
                    proof: test_proof(&lease, &identity, epoch, 1),
                },
            );
            book.persist_count(identity.symbol(), 1);
            assert!(book.is_publishable(identity.symbol()));

            let changed_count = test_demand(&lease, vec![(identity.clone(), 2)]);
            assert!(
                book.apply_demand(&changed_count, &lease)
                    .unwrap()
                    .is_empty()
            );
            assert!(book.is_publishable(identity.symbol()));
            assert!(matches!(
                book.next_command(Utc::now().timestamp_millis()).unwrap(),
                Some(CommandPlan {
                    kind: CommandKind::CountChange,
                    reference_count: 2,
                    ..
                })
            ));

            let replacement = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let invalidated = book
                .apply_demand(&test_demand(&lease, vec![(replacement, 1)]), &lease)
                .unwrap();
            assert_eq!(invalidated, vec![identity.clone()]);
            assert!(!book.is_publishable(identity.symbol()));

            let replacement = test_identity(lease.owner_user_id, "000660", Uuid::new_v4());
            book.record_pending_for_tests("000660", replacement.clone(), CommandKind::Subscribe);
            assert!(!book.is_publishable("000660"));
            let changed = test_demand(
                &lease,
                vec![(
                    test_identity(lease.owner_user_id, "000660", Uuid::new_v4()),
                    1,
                )],
            );
            let removed = book.apply_demand(&changed, &lease).unwrap();
            assert!(removed.contains(&replacement));

            let too_many = (0..=30)
                .map(|index| {
                    (
                        test_identity(lease.owner_user_id, "005930", Uuid::from_u128(index + 1)),
                        1,
                    )
                })
                .collect();
            assert_eq!(
                normalize_runtime_demand(&test_demand(&lease, too_many), &lease),
                Err(DemandApplyError::Invalid)
            );
            let duplicate_symbol = test_demand(
                &lease,
                vec![
                    (
                        test_identity(lease.owner_user_id, "005930", Uuid::new_v4()),
                        1,
                    ),
                    (
                        test_identity(lease.owner_user_id, "005930", Uuid::new_v4()),
                        1,
                    ),
                ],
            );
            assert_eq!(
                normalize_runtime_demand(&duplicate_symbol, &lease),
                Err(DemandApplyError::Invalid)
            );
        }

        #[test]
        fn reintroduced_unsubscribed_symbol_requires_fresh_epoch() {
            let lease = test_lease(test_now());
            let identity = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let mut book = BindingBook::default();
            let demand = test_demand(&lease, vec![(identity.clone(), 1)]);
            book.apply_demand(&demand, &lease).unwrap();
            book.record_pending_for_tests("005930", identity, CommandKind::Unsubscribe);
            book.finish_unsubscribe("005930");
            assert_eq!(
                book.apply_demand(&demand, &lease),
                Err(DemandApplyError::FreshEpoch)
            );

            let mut deferred = BindingBook::default();
            deferred.remember_deferred("005930", 1234);
            assert_eq!(deferred.deferred_until_ms.get("005930"), Some(&1234));
            assert!(!deferred.pending.contains_key("005930"));
            assert!(!deferred.persisted_counts.contains_key("005930"));
        }

        #[test]
        fn status_mailbox_preserves_terminal_and_bounds_rate() {
            let base = Instant::now();
            let mut mailbox = StatusMailbox::default();
            mailbox.routine(RuntimeTransition::AwaitingFirstTrade);
            assert_eq!(
                mailbox.take_ready(base).map(|work| work.transition),
                Some(RuntimeTransition::AwaitingFirstTrade)
            );
            mailbox.routine(RuntimeTransition::ConnectionLost);
            mailbox.routine(RuntimeTransition::BudgetExhausted);
            assert_eq!(mailbox.take_ready(base + Duration::from_millis(999)), None);
            assert_eq!(
                mailbox
                    .take_ready(base + ROUTINE_STATUS_SPACING)
                    .map(|work| work.transition),
                Some(RuntimeTransition::BudgetExhausted)
            );
            mailbox.routine(RuntimeTransition::PipelineLag);
            mailbox.urgent(RuntimeTransition::AccessRevoked);
            mailbox.urgent(RuntimeTransition::SubscriptionRejected);
            assert_eq!(
                mailbox
                    .take_ready(base + ROUTINE_STATUS_SPACING)
                    .map(|work| work.transition),
                Some(RuntimeTransition::AccessRevoked)
            );
            assert_eq!(mailbox.routine, Some(RuntimeTransition::PipelineLag));
            let mut after_quote = StatusMailbox::default();
            after_quote.routine(RuntimeTransition::AwaitingFirstTrade);
            after_quote.known_quote();
            assert!(!after_quote.has_work());
        }

        #[test]
        fn close_or_gap_requires_fresh_epoch_without_proof_rewrite() {
            let mut lifecycle = RuntimeLifecycle::default();
            lifecycle.request_stop(StopReason::Shutdown);
            assert_eq!(lifecycle.clean_exit(), None);
            lifecycle.record_close(Ok(()));
            assert_eq!(lifecycle.clean_exit(), Some(RuntimeOwnedExit::Shutdown));

            let mut gap = RuntimeLifecycle::default();
            let mut buffer = ReceiptBuffer::new();
            let mut mailbox = StatusMailbox::default();
            let immutable_epoch_gap_generation = 19_u64;
            request_gap_parts(
                &mut gap,
                &mut buffer,
                &mut mailbox,
                RuntimeTransition::PipelineLag,
            );
            assert_eq!(gap.clean_exit(), None);
            assert!(matches!(buffer.handoff_batch(), Err(BufferError::Terminal)));
            assert_eq!(mailbox.urgent, Some(RuntimeTransition::PipelineLag));
            assert!(gap_generation_matches(19, 20, true));
            assert!(!gap_generation_matches(19, 21, true));
            assert!(!gap_generation_matches(19, 20, false));
            assert_eq!(
                retirement_transition(
                    StopReason::FreshEpochRequired,
                    Some(RuntimeTransition::PipelineLag),
                ),
                StreamStatusCode::PipelineLag
            );
            assert_eq!(
                retirement_transition(
                    StopReason::FreshEpochRequired,
                    Some(RuntimeTransition::ResyncRequired),
                ),
                StreamStatusCode::ResyncRequired
            );
            assert_eq!(immutable_epoch_gap_generation, 19);
            gap.record_close(Ok(()));
            assert_eq!(gap.clean_exit(), Some(RuntimeOwnedExit::FreshEpochRequired));
            gap.fail(MarketStreamProducerError::Terminal);
            assert_eq!(gap.clean_exit(), None);
        }

        struct DropNotice(Option<oneshot::Sender<()>>);

        impl Drop for DropNotice {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        #[tokio::test]
        async fn owned_task_group_drop_aborts_children() {
            let (first_tx, first_rx) = oneshot::channel();
            let (second_tx, second_rx) = oneshot::channel();
            let mut group = OwnedTaskGroup::new();
            group.spawn(async move {
                let _drop = DropNotice(Some(first_tx));
                std::future::pending::<()>().await;
            });
            group.spawn(async move {
                let _drop = DropNotice(Some(second_tx));
                std::future::pending::<()>().await;
            });
            tokio::task::yield_now().await;
            drop(group);
            tokio::time::timeout(Duration::from_secs(1), first_rx)
                .await
                .expect("first child aborted")
                .expect("first drop signal");
            tokio::time::timeout(Duration::from_secs(1), second_rx)
                .await
                .expect("second child aborted")
                .expect("second drop signal");
        }

        #[tokio::test]
        async fn graceful_task_group_joins_all_and_marks_forced_abort_terminal() {
            let (first_tx, first_rx) = oneshot::channel::<()>();
            let (second_tx, second_rx) = oneshot::channel::<()>();
            let mut graceful = OwnedTaskGroup::new();
            graceful.spawn(async move {
                let _ = first_rx.await;
            });
            graceful.spawn(async move {
                let _ = second_rx.await;
            });
            let first = first_tx.send(());
            let second = second_tx.send(());
            assert!(first.is_ok() && second.is_ok());
            graceful
                .join_all_until(tokio::time::Instant::now() + Duration::from_secs(1))
                .await
                .expect("both cooperative children joined");
            assert!(graceful.is_empty());

            let dropped = Arc::new(AtomicUsize::new(0));
            let dropped_child = dropped.clone();
            let mut forced = OwnedTaskGroup::new();
            forced.spawn(async move {
                let _drop = CountDrop(dropped_child);
                std::future::pending::<()>().await;
            });
            forced.spawn(async {});
            tokio::task::yield_now().await;
            assert_eq!(
                forced
                    .join_all_until(tokio::time::Instant::now() + Duration::from_millis(10))
                    .await,
                Err(MarketStreamProducerError::Terminal)
            );
            assert!(forced.is_empty());
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
        }

        struct CountDrop(Arc<AtomicUsize>);

        impl Drop for CountDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        #[tokio::test]
        async fn child_panic_or_db_failure_cannot_return_clean_exit() {
            let mut lifecycle = RuntimeLifecycle::default();
            lifecycle.request_stop(StopReason::Shutdown);
            let dropped = Arc::new(AtomicUsize::new(0));
            let dropped_child = dropped.clone();
            let mut group = OwnedTaskGroup::new();
            group.spawn(async {
                panic!("synthetic owned child panic");
            });
            group.spawn(async move {
                let _drop = CountDrop(dropped_child);
                std::future::pending::<()>().await;
            });
            tokio::task::yield_now().await;
            assert_eq!(
                group
                    .join_all_until(tokio::time::Instant::now() + Duration::from_secs(1))
                    .await,
                Err(MarketStreamProducerError::Terminal)
            );
            lifecycle.record_close(Ok(()));
            lifecycle.fail(MarketStreamProducerError::Terminal);
            assert_eq!(lifecycle.clean_exit(), None);
            assert!(group.is_empty());
            assert_eq!(dropped.load(Ordering::SeqCst), 1);

            let mut db_failure = RuntimeLifecycle::default();
            db_failure.request_stop(StopReason::FreshEpochRequired);
            db_failure.record_close(Ok(()));
            db_failure.fail(MarketStreamProducerError::Storage(
                MarketStreamStorageError::CommitUnknown,
            ));
            assert_eq!(db_failure.clean_exit(), None);
        }

        fn admission_metadata_snapshot(
            book: &BindingBook,
        ) -> (
            BTreeMap<String, (StreamIdentity, CommandKind)>,
            BTreeMap<String, u32>,
            BTreeMap<String, (StreamIdentity, Uuid)>,
        ) {
            (
                book.pending
                    .iter()
                    .map(|(symbol, pending)| {
                        (symbol.clone(), (pending.identity.clone(), pending.kind))
                    })
                    .collect(),
                book.persisted_counts.clone(),
                book.acked
                    .iter()
                    .map(|(symbol, acked)| {
                        (
                            symbol.clone(),
                            (acked.identity.clone(), acked.proof.subscription_revision),
                        )
                    })
                    .collect(),
            )
        }

        #[test]
        fn command_admission_rechecks_current_demand_and_stop() {
            let now = test_now();
            let now_ms = now.timestamp_millis();
            let lease = test_lease(now);
            let epoch = Uuid::new_v4();
            let lifecycle = RuntimeLifecycle::default();
            let stale_and_fresh = |book: &BindingBook, selected: &CommandPlan| {
                command_plan_matches_current(book, &lifecycle, selected, true, now_ms)
            };

            let first = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let other = test_identity(lease.owner_user_id, "000660", Uuid::new_v4());
            let mut subscribe = BindingBook::default();
            subscribe
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("canonical subscribe demand");
            let subscribe_plan = subscribe
                .next_command(now_ms)
                .expect("current subscribe plan")
                .expect("one subscribe");
            assert_eq!(subscribe_plan.kind, CommandKind::Subscribe);
            assert!(stale_and_fresh(&subscribe, &subscribe_plan).expect("admitted subscribe"));

            let mut removed_with_other_demand = subscribe;
            removed_with_other_demand
                .apply_demand(&test_demand(&lease, vec![(other.clone(), 1)]), &lease)
                .expect("other positive demand remains");
            let before = admission_metadata_snapshot(&removed_with_other_demand);
            assert!(
                !stale_and_fresh(&removed_with_other_demand, &subscribe_plan)
                    .expect("removed selected symbol is stale")
            );
            assert_eq!(
                admission_metadata_snapshot(&removed_with_other_demand),
                before
            );

            let replacement = test_identity(lease.owner_user_id, "005930", Uuid::new_v4());
            let mut replaced = BindingBook::default();
            replaced
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("original membership");
            let old_membership_plan = replaced
                .next_command(now_ms)
                .expect("old membership plan")
                .expect("old membership subscribe");
            assert_eq!(old_membership_plan.symbol, replacement.symbol());
            assert_ne!(
                old_membership_plan.identity.membership_id,
                replacement.membership_id
            );
            assert_ne!(
                old_membership_plan.identity.generation_id,
                replacement.generation_id
            );
            assert_ne!(old_membership_plan.identity, replacement);
            replaced
                .apply_demand(&test_demand(&lease, vec![(replacement, 1)]), &lease)
                .expect("replacement membership");
            let before = admission_metadata_snapshot(&replaced);
            assert!(
                !stale_and_fresh(&replaced, &old_membership_plan)
                    .expect("replaced membership is stale")
            );
            assert_eq!(admission_metadata_snapshot(&replaced), before);

            let mut count_change = BindingBook::default();
            count_change
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("initial count demand");
            let proof = test_proof(&lease, &first, epoch, 1);
            let revision = proof.subscription_revision;
            count_change.acked.insert(
                first.symbol().to_owned(),
                AckBinding {
                    identity: first.clone(),
                    proof,
                },
            );
            count_change.persist_count(first.symbol(), 1);
            count_change
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 2)]), &lease)
                .expect("positive count change");
            let count_plan = count_change
                .next_command(now_ms)
                .expect("current count plan")
                .expect("one count change");
            assert_eq!(count_plan.kind, CommandKind::CountChange);
            let before = admission_metadata_snapshot(&count_change);
            assert!(stale_and_fresh(&count_change, &count_plan).expect("admitted count change"));
            assert_eq!(admission_metadata_snapshot(&count_change), before);
            let acked = count_change
                .acked
                .get(first.symbol())
                .expect("unchanged ACK binding");
            assert_eq!(acked.identity, first);
            assert_eq!(acked.proof.subscription_revision, revision);
            assert_eq!(count_change.persisted_counts.get(first.symbol()), Some(&1));

            count_change
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 3)]), &lease)
                .expect("newer positive count");
            let before = admission_metadata_snapshot(&count_change);
            assert!(
                !stale_and_fresh(&count_change, &count_plan)
                    .expect("changed count invalidates selected plan")
            );
            assert_eq!(admission_metadata_snapshot(&count_change), before);

            let mut unsubscribe = BindingBook::default();
            unsubscribe
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("subscribed identity demand");
            unsubscribe.acked.insert(
                first.symbol().to_owned(),
                AckBinding {
                    identity: first.clone(),
                    proof: test_proof(&lease, &first, epoch, 1),
                },
            );
            unsubscribe.persist_count(first.symbol(), 1);
            unsubscribe
                .apply_demand(&test_demand(&lease, Vec::new()), &lease)
                .expect("zero demand removal");
            let unsubscribe_plan = unsubscribe
                .next_command(now_ms)
                .expect("current unsubscribe plan")
                .expect("one unsubscribe");
            assert_eq!(unsubscribe_plan.kind, CommandKind::Unsubscribe);
            assert!(
                stale_and_fresh(&unsubscribe, &unsubscribe_plan).expect("admitted unsubscribe")
            );

            let mut pending_changed = BindingBook::default();
            pending_changed
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("pending test demand");
            let pending_plan = pending_changed
                .next_command(now_ms)
                .expect("pre-pending plan")
                .expect("pre-pending subscribe");
            pending_changed.record_pending_for_tests(
                first.symbol(),
                first.clone(),
                CommandKind::Subscribe,
            );
            let before = admission_metadata_snapshot(&pending_changed);
            assert!(
                !stale_and_fresh(&pending_changed, &pending_plan)
                    .expect("pending command blocks admission")
            );
            assert_eq!(admission_metadata_snapshot(&pending_changed), before);

            let mut tombstoned = BindingBook::default();
            tombstoned
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("tombstone test demand");
            let tombstone_plan = tombstoned
                .next_command(now_ms)
                .expect("pre-tombstone plan")
                .expect("pre-tombstone subscribe");
            tombstoned.unsubscribed.insert(first.symbol().to_owned());
            let before = admission_metadata_snapshot(&tombstoned);
            assert_eq!(
                stale_and_fresh(&tombstoned, &tombstone_plan),
                Err(DemandApplyError::FreshEpoch)
            );
            assert_eq!(admission_metadata_snapshot(&tombstoned), before);

            let mut deferred = BindingBook::default();
            deferred
                .apply_demand(&test_demand(&lease, vec![(first.clone(), 1)]), &lease)
                .expect("deferred test demand");
            let deferred_plan = deferred
                .next_command(now_ms)
                .expect("pre-deferred plan")
                .expect("pre-deferred subscribe");
            deferred.remember_deferred(first.symbol(), now_ms + 1_000);
            let before = admission_metadata_snapshot(&deferred);
            assert!(
                !stale_and_fresh(&deferred, &deferred_plan)
                    .expect("deferred command is not yet admissible")
            );
            assert_eq!(admission_metadata_snapshot(&deferred), before);
            assert_eq!(
                deferred.deferred_until_ms.get(first.symbol()),
                Some(&(now_ms + 1_000))
            );

            let current_count_plan = count_change
                .next_command(now_ms)
                .expect("fresh current count plan")
                .expect("current positive count change");
            assert_eq!(current_count_plan.kind, CommandKind::CountChange);
            assert_eq!(current_count_plan.reference_count, 3);
            assert!(
                stale_and_fresh(&count_change, &current_count_plan)
                    .expect("current count plan admitted before lifecycle checks")
            );
            let mut stopping = RuntimeLifecycle::default();
            stopping.request_stop(StopReason::Shutdown);
            let before = admission_metadata_snapshot(&count_change);
            assert!(
                !command_plan_matches_current(
                    &count_change,
                    &stopping,
                    &current_count_plan,
                    true,
                    now_ms,
                )
                .expect("stopped count change rejected")
            );
            let mut failed = RuntimeLifecycle::default();
            failed.fail(MarketStreamProducerError::Terminal);
            assert!(
                !command_plan_matches_current(
                    &count_change,
                    &failed,
                    &current_count_plan,
                    true,
                    now_ms,
                )
                .expect("failed count change rejected")
            );
            assert!(
                !command_plan_matches_current(
                    &count_change,
                    &lifecycle,
                    &current_count_plan,
                    false,
                    now_ms,
                )
                .expect("stale demand observation rejected")
            );
            assert_eq!(admission_metadata_snapshot(&count_change), before);
        }

        type TestDropGate = Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>;

        struct DropGateRelease(TestDropGate);

        impl DropGateRelease {
            fn release(&self) {
                if let Ok(mut released) = self.0.0.lock() {
                    *released = true;
                    self.0.1.notify_all();
                }
            }
        }

        impl Drop for DropGateRelease {
            fn drop(&mut self) {
                self.release();
            }
        }

        struct HeldDropNotice {
            gate: TestDropGate,
            started: Option<oneshot::Sender<()>>,
            completed: Option<oneshot::Sender<()>>,
            drops: Arc<AtomicUsize>,
        }

        impl Drop for HeldDropNotice {
            fn drop(&mut self) {
                if let Some(started) = self.started.take() {
                    let _ = started.send(());
                }
                if let Ok(released) = self.gate.0.lock() {
                    let _ = self.gate.1.wait_timeout_while(
                        released,
                        Duration::from_secs(3),
                        |released| !*released,
                    );
                }
                self.drops.fetch_add(1, Ordering::SeqCst);
                if let Some(completed) = self.completed.take() {
                    let _ = completed.send(());
                }
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn abort_join_waits_for_all_owned_children() {
            let started_at = Instant::now();
            let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
            let release = DropGateRelease(gate.clone());
            let drops = Arc::new(AtomicUsize::new(0));
            let (first_started_tx, first_started_rx) = oneshot::channel();
            let (second_started_tx, second_started_rx) = oneshot::channel();
            let (first_drop_started_tx, mut first_drop_started_rx) = oneshot::channel();
            let (first_drop_completed_tx, mut first_drop_completed_rx) = oneshot::channel();
            let (second_drop_completed_tx, mut second_drop_completed_rx) = oneshot::channel();
            let first_drops = drops.clone();
            let held_gate = gate.clone();
            let mut group = OwnedTaskGroup::new();
            group.spawn(async move {
                let _drop = HeldDropNotice {
                    gate: held_gate,
                    started: Some(first_drop_started_tx),
                    completed: Some(first_drop_completed_tx),
                    drops: first_drops,
                };
                let _ = first_started_tx.send(());
                std::future::pending::<()>().await;
            });
            group.spawn(async move {
                let _drop = DropNotice(Some(second_drop_completed_tx));
                let _ = second_started_tx.send(());
                std::future::pending::<()>().await;
            });
            let (first_started, second_started) = tokio::join!(
                tokio::time::timeout(Duration::from_millis(500), first_started_rx),
                tokio::time::timeout(Duration::from_millis(500), second_started_rx),
            );
            let both_started = first_started.is_ok_and(|result| result.is_ok())
                && second_started.is_ok_and(|result| result.is_ok());

            let mut abort_and_join = Box::pin(group.abort_and_join());
            let mut completed_early = false;
            let drop_started = tokio::select! {
                result = &mut first_drop_started_rx => result.is_ok(),
                _ = &mut abort_and_join => {
                    completed_early = true;
                    false
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => false,
            };
            let completed_while_drop_held = if drop_started && !completed_early {
                tokio::time::timeout(Duration::from_millis(1_100), abort_and_join.as_mut())
                    .await
                    .is_ok()
            } else {
                completed_early
            };
            release.release();
            let mut joined = completed_early || completed_while_drop_held;
            if !joined {
                abort_and_join.as_mut().await;
                joined = true;
            }
            drop(abort_and_join);
            // Reclaim every child even if the implementation under test returned early.
            group.tasks.abort_all();
            while group.join_next().await.is_some() {}
            let first_drop_completed = first_drop_completed_rx.try_recv().is_ok();
            let second_drop_completed = second_drop_completed_rx.try_recv().is_ok();
            let group_empty = group.is_empty();
            let elapsed = started_at.elapsed();
            drop(release);

            assert!(both_started, "both children started before cancellation");
            assert!(drop_started, "first child entered its held destructor");
            assert!(
                !completed_while_drop_held,
                "join waits for destructor completion"
            );
            assert!(joined, "abort_and_join completed after release");
            assert!(first_drop_completed, "first child destructor completed");
            assert!(second_drop_completed, "second child destructor completed");
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            assert!(group_empty, "all child join results were collected");
            assert!(
                elapsed <= Duration::from_secs(5),
                "synthetic test stays bounded"
            );
        }
    }
}

pub(super) struct RuntimeOwnedProducer {
    repository: super::market_stream::RuntimeMarketStreamRepository,
    lease: StreamProducerLease,
    epoch: StreamEpochProof,
    day: super::market_stream_runtime::ResolvedMarketStreamDay,
    session: Option<MarketStreamSession>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeOwnedExit {
    Shutdown,
    NoDemand,
    DayClosed,
    CleanClosed,
    FreshEpochRequired,
}

impl OwnerMarketStreamProducer {
    pub(super) async fn start_resolved(
        repository: super::market_stream::RuntimeMarketStreamRepository,
        lease: StreamProducerLease,
        day: super::market_stream_runtime::ResolvedMarketStreamDay,
        session: MarketStreamSession,
    ) -> Result<RuntimeOwnedProducer, MarketStreamProducerError> {
        runtime_owner::start_resolved(repository, lease, day, session).await
    }
}

impl RuntimeOwnedProducer {
    pub(super) async fn run_owned(
        self,
        demand: tokio::sync::watch::Receiver<DesiredSet>,
        lease: tokio::sync::watch::Receiver<StreamProducerLease>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<RuntimeOwnedExit, MarketStreamProducerError> {
        runtime_owner::run_owned(self, demand, lease, shutdown).await
    }
}
