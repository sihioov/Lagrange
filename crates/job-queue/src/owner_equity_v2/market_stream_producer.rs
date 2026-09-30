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
#[cfg(test)]
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

    #[cfg(test)]
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
