//! Typed PostgreSQL boundary for the owner-only KIS market stream.
//!
//! This repository is deliberately separate from `intraday.rs`.  The REST
//! intraday path keeps its attempt reservation and `IntradayQuote` contract;
//! this module accepts only WS-2 `MarketReceipt` values and stores one latest
//! value per admitted membership.  Provider I/O, reconnect orchestration,
//! command batching, and runner configuration belong to WS-3B.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use kis_client::market_stream::{
    MarketSubscriptionAck, MarketSubscriptionOperation, PreparedMarketSubscriptionCommand,
};
use kis_client::market_stream_wire::MarketReceipt;
use market_data::market_stream::{
    STREAM_CURRENCY, STREAM_SOURCE, STREAM_VENUE, STREAM_WIRE_VERSION, StreamBasePriceReason,
    StreamQuote, StreamQuoteDirection,
};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::intraday::{IntradaySessionProof, validate_session_lineage};
#[cfg(not(test))]
use super::market_stream_runtime::load_snapshot_window_contract;
use super::market_stream_runtime::{
    OwnerMarketStreamRuntimeConfig, ResolvedMarketStreamDay, SnapshotWindowEvidence,
    canonical_day_proof_sha256, snapshot_window_for,
};

pub const STREAM_SCHEMA_VERSION: u32 = 2;
pub const STREAM_LEASE_SECONDS: i64 = 30;
pub const STREAM_RENEW_AFTER_MS: i64 = 15_000;
pub const STREAM_MAX_ACTIVE_LEASES: i64 = 20;
pub const STREAM_MAX_IDENTITIES_PER_LEASE: usize = 30;
pub const STREAM_MAX_ACTIVE_IDENTITIES: usize = 30;
pub const STREAM_PRODUCER_LEASE_SECONDS: i64 = 20;
pub const STREAM_PRODUCER_RENEW_AFTER_SECONDS: i64 = 5;
pub const STREAM_PUBLICATION_MAX_AGE: Duration = Duration::from_secs(3);
pub const STREAM_CACHE_RETENTION_HOURS: i64 = 24;

const KST_OFFSET_SECONDS: i32 = 9 * 60 * 60;
const NOTIFY_CHANNEL: &str = "owner_market_stream_changed";
const NOTIFY_PAYLOAD: &str = "v1";

#[cfg(test)]
struct ResolvedCommitTestGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(test)]
static RESOLVED_COMMIT_TEST_GATES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<Uuid, ResolvedCommitTestGate>>,
> = std::sync::OnceLock::new();

/// Typed failures for the stream storage boundary.  Provider prose, SQL
/// text, raw frames, credentials, and session tokens never cross this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MarketStreamStorageError {
    #[error("MARKET_STREAM_INPUT_INVALID")]
    InvalidInput,
    #[error("MARKET_STREAM_LEASE_NOT_FOUND")]
    LeaseNotFound,
    #[error("MARKET_STREAM_LEASE_RELEASED")]
    LeaseReleased,
    #[error("MARKET_STREAM_LEASE_EXPIRED")]
    LeaseExpired,
    #[error("MARKET_STREAM_LEASE_SESSION_MISMATCH")]
    LeaseSessionMismatch,
    #[error("MARKET_STREAM_IDEMPOTENCY_MISMATCH")]
    IdempotencyMismatch,
    #[error("MARKET_STREAM_SEQUENCE_CONFLICT")]
    SequenceConflict,
    #[error("MARKET_STREAM_LEASE_CAPACITY")]
    LeaseCapacity,
    #[error("MARKET_STREAM_IDENTITY_CAPACITY")]
    IdentityCapacity,
    #[error("MARKET_STREAM_MEMBERSHIP_NOT_READY")]
    MembershipNotReady,
    #[error("MARKET_STREAM_RIGHTS_INVALID")]
    RightsInvalid,
    #[error("MARKET_STREAM_SESSION_INVALID")]
    SessionInvalid,
    #[error("MARKET_STREAM_PRODUCER_HELD")]
    ProducerHeld,
    #[error("MARKET_STREAM_PRODUCER_LOST")]
    ProducerLost,
    #[error("MARKET_STREAM_FENCE_EXHAUSTED")]
    FenceExhausted,
    #[error("MARKET_STREAM_EPOCH_INVALID")]
    EpochInvalid,
    #[error("MARKET_STREAM_SUBSCRIPTION_INVALID")]
    SubscriptionInvalid,
    #[error("MARKET_STREAM_SUBSCRIPTION_NOT_ACKED")]
    SubscriptionNotAcked,
    #[error("MARKET_STREAM_RECEIPT_INVALID")]
    ReceiptInvalid,
    #[error("MARKET_STREAM_RECEIPT_STALE")]
    ReceiptStale,
    #[error("MARKET_STREAM_PIPELINE_LAG")]
    PipelineLag,
    #[error("MARKET_STREAM_CACHE_NOT_FOUND")]
    CacheNotFound,
    #[error("MARKET_STREAM_VERSION_EXHAUSTED")]
    VersionExhausted,
    #[error("MARKET_STREAM_STATUS_INVALID")]
    StatusInvalid,
    #[error("MARKET_STREAM_DATABASE_UNAVAILABLE")]
    DatabaseUnavailable,
    #[error("MARKET_STREAM_DATABASE_INTEGRITY")]
    DatabaseIntegrity,
    #[error("MARKET_STREAM_PERMISSION_DENIED")]
    PermissionDenied,
    #[error("MARKET_STREAM_COMMIT_OUTCOME_UNKNOWN")]
    CommitUnknown,
}

#[cfg(test)]
pub(super) fn register_resolved_commit_test_gate(
    key: Uuid,
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), MarketStreamStorageError> {
    let gates = RESOLVED_COMMIT_TEST_GATES.get_or_init(Default::default);
    let mut gates = gates
        .lock()
        .map_err(|_| MarketStreamStorageError::InvalidInput)?;
    if gates.contains_key(&key) {
        return Err(MarketStreamStorageError::InvalidInput);
    }
    gates.insert(key, ResolvedCommitTestGate { entered, release });
    Ok(())
}

#[cfg(test)]
pub(super) fn clear_resolved_commit_test_gate(key: Uuid) {
    if let Some(gates) = RESOLVED_COMMIT_TEST_GATES.get()
        && let Ok(mut gates) = gates.lock()
    {
        gates.remove(&key);
    }
}

#[cfg(test)]
async fn wait_at_resolved_commit_test_gate(key: Uuid) -> Result<(), MarketStreamStorageError> {
    let gate = {
        let Some(gates) = RESOLVED_COMMIT_TEST_GATES.get() else {
            return Ok(());
        };
        gates
            .lock()
            .map_err(|_| MarketStreamStorageError::InvalidInput)?
            .remove(&key)
    };
    if let Some(gate) = gate {
        let _ = gate.entered.send(());
        gate.release
            .await
            .map_err(|_| MarketStreamStorageError::InvalidInput)?;
    }
    Ok(())
}

#[cfg(test)]
#[tokio::test]
async fn unregistered_resolved_commit_gate_is_noop() {
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        wait_at_resolved_commit_test_gate(Uuid::new_v4()),
    )
    .await
    .expect("an unregistered test gate must not wait");
    assert_eq!(result, Ok(()));
}

impl MarketStreamStorageError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "MARKET_STREAM_INPUT_INVALID",
            Self::LeaseNotFound => "MARKET_STREAM_LEASE_NOT_FOUND",
            Self::LeaseReleased => "MARKET_STREAM_LEASE_RELEASED",
            Self::LeaseExpired => "MARKET_STREAM_LEASE_EXPIRED",
            Self::LeaseSessionMismatch => "MARKET_STREAM_LEASE_SESSION_MISMATCH",
            Self::IdempotencyMismatch => "MARKET_STREAM_IDEMPOTENCY_MISMATCH",
            Self::SequenceConflict => "MARKET_STREAM_SEQUENCE_CONFLICT",
            Self::LeaseCapacity => "MARKET_STREAM_LEASE_CAPACITY",
            Self::IdentityCapacity => "MARKET_STREAM_IDENTITY_CAPACITY",
            Self::MembershipNotReady => "MARKET_STREAM_MEMBERSHIP_NOT_READY",
            Self::RightsInvalid => "MARKET_STREAM_RIGHTS_INVALID",
            Self::SessionInvalid => "MARKET_STREAM_SESSION_INVALID",
            Self::ProducerHeld => "MARKET_STREAM_PRODUCER_HELD",
            Self::ProducerLost => "MARKET_STREAM_PRODUCER_LOST",
            Self::FenceExhausted => "MARKET_STREAM_FENCE_EXHAUSTED",
            Self::EpochInvalid => "MARKET_STREAM_EPOCH_INVALID",
            Self::SubscriptionInvalid => "MARKET_STREAM_SUBSCRIPTION_INVALID",
            Self::SubscriptionNotAcked => "MARKET_STREAM_SUBSCRIPTION_NOT_ACKED",
            Self::ReceiptInvalid => "MARKET_STREAM_RECEIPT_INVALID",
            Self::ReceiptStale => "MARKET_STREAM_RECEIPT_STALE",
            Self::PipelineLag => "MARKET_STREAM_PIPELINE_LAG",
            Self::CacheNotFound => "MARKET_STREAM_CACHE_NOT_FOUND",
            Self::VersionExhausted => "MARKET_STREAM_VERSION_EXHAUSTED",
            Self::StatusInvalid => "MARKET_STREAM_STATUS_INVALID",
            Self::DatabaseUnavailable => "MARKET_STREAM_DATABASE_UNAVAILABLE",
            Self::DatabaseIntegrity => "MARKET_STREAM_DATABASE_INTEGRITY",
            Self::PermissionDenied => "MARKET_STREAM_PERMISSION_DENIED",
            Self::CommitUnknown => "MARKET_STREAM_COMMIT_OUTCOME_UNKNOWN",
        }
    }
}

/// Exact 0053 owner/membership/generation lineage used by stream rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamIdentity {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
}

impl StreamIdentity {
    pub fn new(
        owner_user_id: Uuid,
        membership_id: Uuid,
        generation_id: Uuid,
        instrument_id: String,
        generation: u64,
    ) -> Result<Self, MarketStreamStorageError> {
        let identity = Self {
            owner_user_id,
            membership_id,
            generation_id,
            instrument_id,
            generation,
        };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> Result<(), MarketStreamStorageError> {
        if self.owner_user_id.is_nil()
            || self.membership_id.is_nil()
            || self.generation_id.is_nil()
            || !canonical_instrument(&self.instrument_id)
            || self.generation == 0
            || i64::try_from(self.generation).is_err()
        {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        Ok(())
    }

    pub fn symbol(&self) -> &str {
        self.instrument_id.get(..6).unwrap_or_default()
    }

    fn generation_i64(&self) -> i64 {
        i64::try_from(self.generation).expect("validated stream generation fits bigint")
    }
}

/// The body-level identity supplied by the lease route.  The generation id is
/// resolved from the current 0053 admission inside the lease transaction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamLeaseIdentity {
    pub membership_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
}

impl StreamLeaseIdentity {
    pub fn new(
        membership_id: Uuid,
        instrument_id: String,
        generation: u64,
    ) -> Result<Self, MarketStreamStorageError> {
        let identity = Self {
            membership_id,
            instrument_id,
            generation,
        };
        if identity.membership_id.is_nil()
            || !canonical_instrument(&identity.instrument_id)
            || identity.generation == 0
            || i64::try_from(identity.generation).is_err()
        {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        Ok(identity)
    }

    fn generation_i64(&self) -> i64 {
        i64::try_from(self.generation).expect("validated stream generation fits bigint")
    }
}

/// Canonical replacement request for one browser-tab lease.  The raw
/// idempotency key is transient and never appears in a database row.
pub struct StreamLeaseRequest {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
    identities: Vec<StreamLeaseIdentity>,
    idempotency_key: String,
}

impl StreamLeaseRequest {
    pub fn new(
        consumer_id: Uuid,
        renewal_sequence: u64,
        mut identities: Vec<StreamLeaseIdentity>,
        idempotency_key: String,
    ) -> Result<Self, MarketStreamStorageError> {
        identities.sort();
        let request = Self {
            schema_version: STREAM_SCHEMA_VERSION,
            consumer_id,
            renewal_sequence,
            identities,
            idempotency_key,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn identities(&self) -> &[StreamLeaseIdentity] {
        &self.identities
    }

    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    fn validate(&self) -> Result<(), MarketStreamStorageError> {
        if self.schema_version != STREAM_SCHEMA_VERSION
            || self.consumer_id.is_nil()
            || i64::try_from(self.renewal_sequence).is_err()
            || self.identities.is_empty()
            || self.identities.len() > STREAM_MAX_IDENTITIES_PER_LEASE
            || !valid_idempotency_key(&self.idempotency_key)
        {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        for pair in self.identities.windows(2) {
            if pair[0].membership_id == pair[1].membership_id {
                return Err(MarketStreamStorageError::InvalidInput);
            }
        }
        let mut instruments = BTreeSet::new();
        for identity in &self.identities {
            if !instruments.insert(identity.instrument_id.as_str()) {
                return Err(MarketStreamStorageError::InvalidInput);
            }
        }
        Ok(())
    }

    fn sequence_i64(&self) -> i64 {
        i64::try_from(self.renewal_sequence).expect("validated stream renewal sequence fits bigint")
    }

    fn idempotency_digest(&self) -> String {
        prefixed_sha256(self.idempotency_key.as_bytes())
    }

    fn request_digest(&self) -> String {
        let mut canonical = format!(
            "schema_version={};consumer_id={};renewal_sequence={};",
            self.schema_version, self.consumer_id, self.renewal_sequence
        );
        for identity in &self.identities {
            canonical.push_str(&format!(
                "membership_id={};instrument_id={};generation={};",
                identity.membership_id, identity.instrument_id, identity.generation
            ));
        }
        prefixed_sha256(canonical.as_bytes())
    }
}

impl fmt::Debug for StreamLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamLeaseRequest")
            .field("schema_version", &self.schema_version)
            .field("consumer_id", &self.consumer_id)
            .field("renewal_sequence", &self.renewal_sequence)
            .field("identities", &self.identities)
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamLease {
    pub lease_id: Uuid,
    pub owner_user_id: Uuid,
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
    pub lease_expires_at: DateTime<Utc>,
    pub renew_after_ms: i64,
    pub identities: Vec<StreamIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseOutcome {
    pub lease_id: Uuid,
    pub released: bool,
}

fn release_binding_is_tombstone_replay(
    row_session_hash: &str,
    expected_session_hash: &str,
    row_consumer_id: Uuid,
    expected_consumer_id: Option<Uuid>,
    released: bool,
) -> Result<bool, MarketStreamStorageError> {
    if row_session_hash != expected_session_hash {
        return Err(MarketStreamStorageError::LeaseSessionMismatch);
    }
    if expected_consumer_id.is_some_and(|consumer_id| consumer_id.is_nil()) {
        return Err(MarketStreamStorageError::InvalidInput);
    }
    if expected_consumer_id.is_some_and(|consumer_id| consumer_id != row_consumer_id) {
        return Err(MarketStreamStorageError::LeaseNotFound);
    }
    Ok(released)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredStreamItem {
    pub identity: StreamIdentity,
    pub reference_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredSet {
    pub credential_slot_id: Uuid,
    pub owner_user_id: Uuid,
    pub items: Vec<DesiredStreamItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamProducerLease {
    pub credential_slot_id: Uuid,
    pub grant_id: Uuid,
    pub grant_revision: Uuid,
    pub owner_user_id: Uuid,
    pub holder_id: Uuid,
    pub fencing_token: u64,
    pub gap_generation: u64,
    pub lease_expires_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSessionProof {
    pub session_date: NaiveDate,
    pub session_proof_id: Uuid,
    pub session_proof_sha256: String,
    pub calendar_source_batch_id: Uuid,
    pub calendar_content_sha256: String,
    pub window_contract_sha256: String,
}

impl StreamSessionProof {
    pub fn new(
        session_date: NaiveDate,
        session_proof_id: Uuid,
        session_proof_sha256: String,
        calendar_source_batch_id: Uuid,
        calendar_content_sha256: String,
        window_contract_sha256: String,
    ) -> Result<Self, MarketStreamStorageError> {
        let proof = Self {
            session_date,
            session_proof_id,
            session_proof_sha256,
            calendar_source_batch_id,
            calendar_content_sha256,
            window_contract_sha256,
        };
        proof.validate()?;
        Ok(proof)
    }

    fn validate(&self) -> Result<(), MarketStreamStorageError> {
        if self.session_proof_id.is_nil()
            || self.calendar_source_batch_id.is_nil()
            || !canonical_prefixed_sha256(&self.session_proof_sha256)
            || !canonical_unprefixed_sha256(&self.calendar_content_sha256)
            || !canonical_prefixed_sha256(&self.window_contract_sha256)
        {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StreamEpochProof {
    pub credential_slot_id: Uuid,
    pub grant_revision: Uuid,
    pub epoch: Uuid,
    pub session: StreamSessionProof,
    pub fencing_token: u64,
    pub gap_generation: u64,
    resolved_day_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionOperation {
    Subscribe,
    Unsubscribe,
}

impl SubscriptionOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Subscribe => "SUBSCRIBE",
            Self::Unsubscribe => "UNSUBSCRIBE",
        }
    }
}

/// A storage reservation tied to the exact C2 command that was prepared.
/// This value is minted only after the pending row commit is known successful.
pub(super) struct PendingSubscriptionCommit {
    credential_slot_id: Uuid,
    grant_revision: Uuid,
    epoch: Uuid,
    symbol: String,
    operation: SubscriptionOperation,
    ordinal: u64,
    reserved_at_ms: i64,
    reserved_at_monotonic: Instant,
    deadline_at_ms: i64,
    deadline_monotonic: Instant,
    subscription_revision: Uuid,
}

fn exact_wall_millis(ms: i64) -> Result<DateTime<Utc>, MarketStreamStorageError> {
    DateTime::<Utc>::from_timestamp_millis(ms).ok_or(MarketStreamStorageError::SubscriptionInvalid)
}

fn validate_ack_capture_window(
    reserved_at_ms: i64,
    reserved_at_monotonic: Instant,
    deadline_at_ms: i64,
    deadline_monotonic: Instant,
    captured_at_ms: i64,
    captured_at_monotonic: Instant,
) -> Result<(), MarketStreamStorageError> {
    if reserved_at_ms >= deadline_at_ms
        || captured_at_ms < reserved_at_ms
        || captured_at_ms >= deadline_at_ms
        || reserved_at_monotonic >= deadline_monotonic
        || captured_at_monotonic < reserved_at_monotonic
        || captured_at_monotonic >= deadline_monotonic
    {
        return Err(MarketStreamStorageError::SubscriptionInvalid);
    }
    Ok(())
}

fn storage_operation(operation: MarketSubscriptionOperation) -> SubscriptionOperation {
    match operation {
        MarketSubscriptionOperation::Subscribe => SubscriptionOperation::Subscribe,
        MarketSubscriptionOperation::Unsubscribe => SubscriptionOperation::Unsubscribe,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SubscriptionProof {
    pub credential_slot_id: Uuid,
    pub symbol: String,
    pub grant_revision: Uuid,
    pub epoch: Uuid,
    pub state: String,
    pub subscription_revision: Uuid,
    pub acked_at: Option<DateTime<Utc>>,
    pub desired_reference_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StreamPublicationItem {
    pub admission: StreamIdentity,
    pub subscription: SubscriptionProof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StreamPublicationContext {
    pub producer: StreamProducerLease,
    pub epoch: StreamEpochProof,
    pub session: StreamSessionProof,
    pub items: Vec<StreamPublicationItem>,
}

impl StreamPublicationContext {
    pub(super) fn new(
        producer: StreamProducerLease,
        epoch: StreamEpochProof,
        mut items: Vec<StreamPublicationItem>,
    ) -> Result<Self, MarketStreamStorageError> {
        items.sort_by(|left, right| {
            left.admission
                .membership_id
                .cmp(&right.admission.membership_id)
        });
        if items.is_empty() || items.len() > STREAM_MAX_IDENTITIES_PER_LEASE {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let context = Self {
            session: epoch.session.clone(),
            producer,
            epoch,
            items,
        };
        context.validate()?;
        Ok(context)
    }

    pub(super) fn single(
        producer: StreamProducerLease,
        epoch: StreamEpochProof,
        admission: StreamIdentity,
        subscription: SubscriptionProof,
    ) -> Result<Self, MarketStreamStorageError> {
        Self::new(
            producer,
            epoch,
            vec![StreamPublicationItem {
                admission,
                subscription,
            }],
        )
    }

    fn validate(&self) -> Result<(), MarketStreamStorageError> {
        if self.producer.credential_slot_id != self.epoch.credential_slot_id
            || self.producer.grant_revision != self.epoch.grant_revision
            || self.producer.fencing_token != self.epoch.fencing_token
            || self.epoch.epoch.is_nil()
            || self.epoch.grant_revision.is_nil()
            || i64::try_from(self.epoch.gap_generation).is_err()
            || self.session != self.epoch.session
        {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let mut memberships = BTreeSet::new();
        for item in &self.items {
            item.admission.validate()?;
            if item.admission.owner_user_id != self.producer.owner_user_id
                || item.subscription.credential_slot_id != self.producer.credential_slot_id
                || item.subscription.grant_revision != self.producer.grant_revision
                || item.subscription.epoch != self.epoch.epoch
                || item.subscription.state != "ACKED"
                || item.subscription.acked_at.is_none()
                || item.subscription.desired_reference_count == 0
                || item.subscription.symbol != item.admission.symbol()
                || !memberships.insert(item.admission.membership_id)
            {
                return Err(MarketStreamStorageError::InvalidInput);
            }
        }
        Ok(())
    }
}

/// A receipt is kept opaque except for the transport-owned accessors.  The
/// constructor does not accept an observation or timestamp separately.
#[derive(Clone)]
pub(super) struct StreamPublicationObservation {
    receipt: MarketReceipt,
}

impl StreamPublicationObservation {
    pub(super) fn from_receipt(receipt: MarketReceipt) -> Self {
        Self { receipt }
    }

    pub(super) fn receipt(&self) -> &MarketReceipt {
        &self.receipt
    }
}

impl fmt::Debug for StreamPublicationObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamPublicationObservation")
            .field("epoch", &self.receipt.epoch())
            .field("receive_ordinal", &self.receipt.receive_ordinal())
            .field("symbol", &self.receipt.observation().symbol)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Backoff,
    Stopped,
}

impl StreamConnectionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "DISCONNECTED",
            Self::Connecting => "CONNECTING",
            Self::Connected => "CONNECTED",
            Self::Backoff => "BACKOFF",
            Self::Stopped => "STOPPED",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "DISCONNECTED" => Self::Disconnected,
            "CONNECTING" => Self::Connecting,
            "CONNECTED" => Self::Connected,
            "BACKOFF" => Self::Backoff,
            "STOPPED" => Self::Stopped,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMarketState {
    Open,
    Closed,
    Unknown,
}

impl StreamMarketState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Closed => "CLOSED",
            Self::Unknown => "UNKNOWN",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "OPEN" => Self::Open,
            "CLOSED" => Self::Closed,
            "UNKNOWN" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFreshness {
    Recent,
    Stale,
    Unavailable,
}

impl StreamFreshness {
    fn as_str(self) -> &'static str {
        match self {
            Self::Recent => "RECENT",
            Self::Stale => "STALE",
            Self::Unavailable => "UNAVAILABLE",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "RECENT" => Self::Recent,
            "STALE" => Self::Stale,
            "UNAVAILABLE" => Self::Unavailable,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamAvailability {
    Live,
    LastKnown,
    AwaitingFirstTrade,
    Unavailable,
}

impl StreamAvailability {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "LIVE",
            Self::LastKnown => "LAST_KNOWN",
            Self::AwaitingFirstTrade => "AWAITING_FIRST_TRADE",
            Self::Unavailable => "UNAVAILABLE",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "LIVE" => Self::Live,
            "LAST_KNOWN" => Self::LastKnown,
            "AWAITING_FIRST_TRADE" => Self::AwaitingFirstTrade,
            "UNAVAILABLE" => Self::Unavailable,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatusCode {
    FeatureDisabled,
    NoActiveDemand,
    CalendarUnavailable,
    SessionWindowUnavailable,
    SessionClosed,
    AwaitingFirstTrade,
    QuoteStale,
    ConnectionLost,
    ReconnectGap,
    SubscriptionPending,
    SubscriptionRejected,
    SubscriptionAmbiguous,
    ApprovalUnavailable,
    BudgetExhausted,
    ProducerUnavailable,
    PipelineLag,
    WireSchemaMismatch,
    ProviderResponseInvalid,
    QuoteValueInvalid,
    MarketClassUnsupported,
    LocalIngressLimit,
    ResyncRequired,
    AccessRevoked,
}

impl StreamStatusCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::FeatureDisabled => "FEATURE_DISABLED",
            Self::NoActiveDemand => "NO_ACTIVE_DEMAND",
            Self::CalendarUnavailable => "CALENDAR_UNAVAILABLE",
            Self::SessionWindowUnavailable => "SESSION_WINDOW_UNAVAILABLE",
            Self::SessionClosed => "SESSION_CLOSED",
            Self::AwaitingFirstTrade => "AWAITING_FIRST_TRADE",
            Self::QuoteStale => "QUOTE_STALE",
            Self::ConnectionLost => "CONNECTION_LOST",
            Self::ReconnectGap => "RECONNECT_GAP",
            Self::SubscriptionPending => "SUBSCRIPTION_PENDING",
            Self::SubscriptionRejected => "SUBSCRIPTION_REJECTED",
            Self::SubscriptionAmbiguous => "SUBSCRIPTION_AMBIGUOUS",
            Self::ApprovalUnavailable => "APPROVAL_UNAVAILABLE",
            Self::BudgetExhausted => "BUDGET_EXHAUSTED",
            Self::ProducerUnavailable => "PRODUCER_UNAVAILABLE",
            Self::PipelineLag => "PIPELINE_LAG",
            Self::WireSchemaMismatch => "WIRE_SCHEMA_MISMATCH",
            Self::ProviderResponseInvalid => "PROVIDER_RESPONSE_INVALID",
            Self::QuoteValueInvalid => "QUOTE_VALUE_INVALID",
            Self::MarketClassUnsupported => "MARKET_CLASS_UNSUPPORTED",
            Self::LocalIngressLimit => "LOCAL_INGRESS_LIMIT",
            Self::ResyncRequired => "RESYNC_REQUIRED",
            Self::AccessRevoked => "ACCESS_REVOKED",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "FEATURE_DISABLED" => Self::FeatureDisabled,
            "NO_ACTIVE_DEMAND" => Self::NoActiveDemand,
            "CALENDAR_UNAVAILABLE" => Self::CalendarUnavailable,
            "SESSION_WINDOW_UNAVAILABLE" => Self::SessionWindowUnavailable,
            "SESSION_CLOSED" => Self::SessionClosed,
            "AWAITING_FIRST_TRADE" => Self::AwaitingFirstTrade,
            "QUOTE_STALE" => Self::QuoteStale,
            "CONNECTION_LOST" => Self::ConnectionLost,
            "RECONNECT_GAP" => Self::ReconnectGap,
            "SUBSCRIPTION_PENDING" => Self::SubscriptionPending,
            "SUBSCRIPTION_REJECTED" => Self::SubscriptionRejected,
            "SUBSCRIPTION_AMBIGUOUS" => Self::SubscriptionAmbiguous,
            "APPROVAL_UNAVAILABLE" => Self::ApprovalUnavailable,
            "BUDGET_EXHAUSTED" => Self::BudgetExhausted,
            "PRODUCER_UNAVAILABLE" => Self::ProducerUnavailable,
            "PIPELINE_LAG" => Self::PipelineLag,
            "WIRE_SCHEMA_MISMATCH" => Self::WireSchemaMismatch,
            "PROVIDER_RESPONSE_INVALID" => Self::ProviderResponseInvalid,
            "QUOTE_VALUE_INVALID" => Self::QuoteValueInvalid,
            "MARKET_CLASS_UNSUPPORTED" => Self::MarketClassUnsupported,
            "LOCAL_INGRESS_LIMIT" => Self::LocalIngressLimit,
            "RESYNC_REQUIRED" => Self::ResyncRequired,
            "ACCESS_REVOKED" => Self::AccessRevoked,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamStatus {
    pub code: StreamStatusCode,
    pub connection: StreamConnectionState,
    pub market_state: StreamMarketState,
    pub freshness: StreamFreshness,
    pub availability: StreamAvailability,
    pub gap_open: bool,
    pub gap_generation: u64,
}

impl StreamStatus {
    pub fn new(
        code: StreamStatusCode,
        connection: StreamConnectionState,
        market_state: StreamMarketState,
        freshness: StreamFreshness,
        availability: StreamAvailability,
        gap_open: bool,
        gap_generation: u64,
    ) -> Result<Self, MarketStreamStorageError> {
        if i64::try_from(gap_generation).is_err() {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        Ok(Self {
            code,
            connection,
            market_state,
            freshness,
            availability,
            gap_open,
            gap_generation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamCacheRow {
    pub identity: StreamIdentity,
    pub row_generation: Uuid,
    pub quote: Option<StreamQuote>,
    pub quote_version: u64,
    pub state_version: u64,
    pub status: Option<StreamStatus>,
    pub epoch: Option<Uuid>,
    pub receive_ordinal: Option<u64>,
    pub received_at: Option<DateTime<Utc>>,
    pub committed_at: Option<DateTime<Utc>>,
    pub gap_open: bool,
    pub gap_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    pub changed: bool,
    pub rows: Vec<StreamCacheRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSnapshot {
    pub lease_id: Uuid,
    pub lease_expires_at: DateTime<Utc>,
    pub rows: Vec<StreamCacheRow>,
    pub delivery_rows: Vec<StreamDeliveryRow>,
}

/// Closed subscription metadata safe for an Owner delivery snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSubscriptionDeliveryState {
    Desired,
    PendingSubscribe,
    Acked,
    PendingUnsubscribe,
    Absent,
    Rejected,
    Ambiguous,
}

impl StreamSubscriptionDeliveryState {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "DESIRED" => Self::Desired,
            "PENDING_SUBSCRIBE" => Self::PendingSubscribe,
            "ACKED" => Self::Acked,
            "PENDING_UNSUBSCRIBE" => Self::PendingUnsubscribe,
            "ABSENT" => Self::Absent,
            "REJECTED" => Self::Rejected,
            "AMBIGUOUS" => Self::Ambiguous,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSubscriptionDelivery {
    pub state: StreamSubscriptionDeliveryState,
    pub epoch: Option<Uuid>,
    pub revision: Uuid,
    pub updated_at: DateTime<Utc>,
}

/// Safe producer fields returned by the SECURITY DEFINER delivery helper.
/// Slot, holder, fence, entitlement and session hash are intentionally absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamProducerDelivery {
    pub connection: StreamConnectionState,
    pub reason: Option<StreamStatusCode>,
    pub status_at: Option<DateTime<Utc>>,
    pub heartbeat_at: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
    pub current_epoch: Option<Uuid>,
    pub session_date: Option<NaiveDate>,
    pub session_proof_id: Option<Uuid>,
    pub session_proof_sha256: Option<String>,
    pub calendar_source_batch_id: Option<Uuid>,
    pub calendar_content_sha256: Option<String>,
    pub window_contract_sha256: Option<String>,
    pub gap_since: Option<DateTime<Utc>>,
    pub session_has_gap: bool,
    pub gap_generation: u64,
    pub state_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamDeliveryRow {
    pub identity: StreamIdentity,
    pub row_generation: Uuid,
    pub state_version: u64,
    pub cache: Option<StreamCacheRow>,
    pub subscription: Option<StreamSubscriptionDelivery>,
    pub producer: Option<StreamProducerDelivery>,
    /// Provider-free evidence that the same authorized read transaction
    /// validated this row's current KST calendar lineage and window.
    pub read_evidence: Option<StreamDeliveryEvidence>,
    /// True only when committed quote, ACK, producer lease, calendar lineage,
    /// current KST day, and half-open pinned window all agree.
    pub live: bool,
}

/// Provider-free metadata from one authorized snapshot read. This is not a
/// publication capability or durable proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamDeliveryEvidence {
    pub observed_at: DateTime<Utc>,
    pub session_date: NaiveDate,
    pub calendar_content_sha256: String,
    pub window_contract_sha256: String,
    pub open_at: DateTime<Utc>,
    pub close_at: DateTime<Utc>,
    pub quote_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeGrant {
    credential_slot_id: Uuid,
    grant_id: Uuid,
    grant_revision: Uuid,
    owner_user_id: Uuid,
    credential_generation: u64,
    network_contract_sha256: String,
    identity_list_sha256: String,
}

impl RuntimeGrant {
    pub(super) const fn owner_user_id(&self) -> Uuid {
        self.owner_user_id
    }

    pub(super) const fn credential_slot_id(&self) -> Uuid {
        self.credential_slot_id
    }

    pub(super) const fn grant_id(&self) -> Uuid {
        self.grant_id
    }

    pub(super) const fn grant_revision(&self) -> Uuid {
        self.grant_revision
    }

    pub(super) const fn credential_generation(&self) -> u64 {
        self.credential_generation
    }

    pub(super) fn network_contract_sha256(&self) -> &str {
        &self.network_contract_sha256
    }

    pub(super) fn identity_list_sha256(&self) -> &str {
        &self.identity_list_sha256
    }
}

/// A finite set of lifecycle transitions. There is intentionally no
/// arbitrary `StreamStatus` or LIVE constructor on this pre-ACK path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeTransition {
    Connecting,
    AwaitingFirstTrade,
    NoActiveDemand,
    CalendarUnavailable,
    WindowUnavailable,
    SessionClosed,
    ConnectionLost,
    ReconnectGap,
    SubscriptionPending,
    SubscriptionRejected,
    SubscriptionAmbiguous,
    ApprovalUnavailable,
    BudgetExhausted,
    PipelineLag,
    WireSchemaMismatch,
    ProviderResponseInvalid,
    QuoteValueInvalid,
    MarketClassUnsupported,
    LocalIngressLimit,
    ResyncRequired,
    AccessRevoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeTransitionValues {
    pub(super) code: StreamStatusCode,
    pub(super) connection: StreamConnectionState,
    pub(super) market_state: StreamMarketState,
    pub(super) freshness: StreamFreshness,
    pub(super) availability: StreamAvailability,
    pub(super) opens_gap: bool,
}

impl RuntimeTransition {
    pub(super) fn values(self) -> RuntimeTransitionValues {
        use RuntimeTransition as T;
        use StreamAvailability as A;
        use StreamConnectionState as C;
        use StreamFreshness as F;
        use StreamMarketState as M;
        use StreamStatusCode as S;
        let (code, connection, market_state, freshness, availability, opens_gap) = match self {
            T::Connecting | T::SubscriptionPending => (
                S::SubscriptionPending,
                C::Connecting,
                M::Unknown,
                F::Unavailable,
                A::AwaitingFirstTrade,
                false,
            ),
            T::AwaitingFirstTrade => (
                S::AwaitingFirstTrade,
                C::Connected,
                M::Open,
                F::Recent,
                A::AwaitingFirstTrade,
                false,
            ),
            T::NoActiveDemand => (
                S::NoActiveDemand,
                C::Disconnected,
                M::Unknown,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
            T::CalendarUnavailable => (
                S::CalendarUnavailable,
                C::Disconnected,
                M::Unknown,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
            T::WindowUnavailable => (
                S::SessionWindowUnavailable,
                C::Disconnected,
                M::Unknown,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
            T::SessionClosed => (
                S::SessionClosed,
                C::Disconnected,
                M::Closed,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
            T::ConnectionLost => (
                S::ConnectionLost,
                C::Backoff,
                M::Unknown,
                F::Stale,
                A::LastKnown,
                true,
            ),
            T::ReconnectGap => (
                S::ReconnectGap,
                C::Backoff,
                M::Unknown,
                F::Stale,
                A::LastKnown,
                true,
            ),
            T::SubscriptionRejected => (
                S::SubscriptionRejected,
                C::Connected,
                M::Open,
                F::Stale,
                A::LastKnown,
                false,
            ),
            T::SubscriptionAmbiguous => (
                S::SubscriptionAmbiguous,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::LastKnown,
                true,
            ),
            T::ApprovalUnavailable => (
                S::ApprovalUnavailable,
                C::Disconnected,
                M::Unknown,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
            T::BudgetExhausted => (
                S::BudgetExhausted,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
            T::PipelineLag => (
                S::PipelineLag,
                C::Connected,
                M::Open,
                F::Stale,
                A::LastKnown,
                true,
            ),
            T::WireSchemaMismatch => (
                S::WireSchemaMismatch,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::LastKnown,
                true,
            ),
            T::ProviderResponseInvalid => (
                S::ProviderResponseInvalid,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::LastKnown,
                true,
            ),
            T::QuoteValueInvalid => (
                S::QuoteValueInvalid,
                C::Connected,
                M::Open,
                F::Stale,
                A::LastKnown,
                false,
            ),
            T::MarketClassUnsupported => (
                S::MarketClassUnsupported,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::LastKnown,
                true,
            ),
            T::LocalIngressLimit => (
                S::LocalIngressLimit,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::LastKnown,
                true,
            ),
            T::ResyncRequired => (
                S::ResyncRequired,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::LastKnown,
                true,
            ),
            T::AccessRevoked => (
                S::AccessRevoked,
                C::Stopped,
                M::Unknown,
                F::Unavailable,
                A::Unavailable,
                false,
            ),
        };
        RuntimeTransitionValues {
            code,
            connection,
            market_state,
            freshness,
            availability,
            opens_gap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeStatusCommit {
    changed: bool,
    current_epoch: Option<Uuid>,
    gap_generation: u64,
    state_version: u64,
}

impl RuntimeStatusCommit {
    pub(super) const fn changed(self) -> bool {
        self.changed
    }

    pub(super) const fn current_epoch(self) -> Option<Uuid> {
        self.current_epoch
    }

    pub(super) const fn gap_generation(self) -> u64 {
        self.gap_generation
    }

    pub(super) const fn state_version(self) -> u64 {
        self.state_version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerTransactionPolicy {
    Legacy,
    Runtime,
}

impl WorkerTransactionPolicy {
    const fn sql_limits(self) -> (&'static str, &'static str) {
        match self {
            Self::Legacy => (
                "SET LOCAL lock_timeout = '5s'",
                "SET LOCAL statement_timeout = '30s'",
            ),
            Self::Runtime => (
                "SET LOCAL lock_timeout = '1s'",
                "SET LOCAL statement_timeout = '1s'",
            ),
        }
    }
}

/// Public repository surface intentionally excludes raw producer transitions.
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::SubscriptionCommandOrAck;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::SubscriptionProof;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamEpochProof;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamPublicationItem;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamPublicationContext;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamPublicationObservation;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::PendingSubscriptionCommit;
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_issue() {
///     let _ = OwnerMarketStreamRepository::record_subscription;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_set_desired() {
///     let _ = OwnerMarketStreamRepository::set_subscription_desired;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_commit_prepared() {
///     let _ = OwnerMarketStreamRepository::commit_prepared_subscription;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_commit_ack() {
///     let _ = OwnerMarketStreamRepository::commit_subscription_ack;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_start_epoch() {
///     let _ = OwnerMarketStreamRepository::start_stream_epoch;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_publish() {
///     let _ = OwnerMarketStreamRepository::publish_stream_latest;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn cannot_set_status() {
///     let _ = OwnerMarketStreamRepository::record_stream_status;
/// }
/// ```
///
/// ```no_run
/// use job_queue::owner_equity_v2::OwnerMarketStreamRepository;
/// fn main() {
///     let public_demand = OwnerMarketStreamRepository::read_stream_demand;
///     assert!(!std::any::type_name_of_val(&public_demand).is_empty());
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamPublicationContext;
/// fn cannot_construct_context() {
///     let _ = StreamPublicationContext::single;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamPublicationContext;
/// fn cannot_construct_context() {
///     let _ = StreamPublicationContext::new;
/// }
/// ```
///
/// ```compile_fail
/// use job_queue::owner_equity_v2::StreamPublicationObservation;
/// fn cannot_construct_observation() {
///     let _ = StreamPublicationObservation::from_receipt;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct OwnerMarketStreamRepository {
    pool: PgPool,
    worker_transaction_policy: WorkerTransactionPolicy,
}

#[cfg(test)]
mod c3a_timing_tests {
    use super::*;

    #[test]
    fn strict_capture_window_accepts_reservation_and_just_before_deadline() {
        let reserved = Instant::now();
        let deadline = reserved + Duration::from_millis(10);
        assert_eq!(
            validate_ack_capture_window(100, reserved, 110, deadline, 100, reserved),
            Ok(())
        );
        assert_eq!(
            validate_ack_capture_window(
                100,
                reserved,
                110,
                deadline,
                109,
                deadline - Duration::from_nanos(1),
            ),
            Ok(())
        );
    }

    #[test]
    fn strict_capture_window_rejects_deadline_equality_on_either_clock() {
        let reserved = Instant::now();
        let deadline = reserved + Duration::from_millis(10);
        assert_eq!(
            validate_ack_capture_window(
                100,
                reserved,
                110,
                deadline,
                110,
                deadline - Duration::from_nanos(1)
            ),
            Err(MarketStreamStorageError::SubscriptionInvalid)
        );
        assert_eq!(
            validate_ack_capture_window(100, reserved, 110, deadline, 109, deadline),
            Err(MarketStreamStorageError::SubscriptionInvalid)
        );
    }

    #[test]
    fn strict_capture_window_rejects_capture_before_either_reservation_clock() {
        let reserved = Instant::now();
        let deadline = reserved + Duration::from_millis(10);
        assert_eq!(
            validate_ack_capture_window(100, reserved, 110, deadline, 99, reserved),
            Err(MarketStreamStorageError::SubscriptionInvalid)
        );
        assert_eq!(
            validate_ack_capture_window(
                100,
                reserved,
                110,
                deadline,
                100,
                reserved - Duration::from_nanos(1),
            ),
            Err(MarketStreamStorageError::SubscriptionInvalid)
        );
    }

    #[test]
    fn exact_wall_millisecond_conversion_is_checked() {
        assert_eq!(exact_wall_millis(1_000).unwrap().timestamp_millis(), 1_000);
        assert_eq!(
            exact_wall_millis(i64::MAX),
            Err(MarketStreamStorageError::SubscriptionInvalid)
        );
    }
}

impl OwnerMarketStreamRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            worker_transaction_policy: WorkerTransactionPolicy::Legacy,
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(super) fn runtime_repository(&self) -> RuntimeMarketStreamRepository {
        RuntimeMarketStreamRepository {
            inner: Self {
                pool: self.pool.clone(),
                worker_transaction_policy: WorkerTransactionPolicy::Runtime,
            },
            terminal: RuntimeTerminalLatch::default(),
        }
    }

    /// Load only the exact grant pins authorized for worker SELECT by 0056.
    /// Rights are then locked and rechecked through the existing C1 helper.
    pub(super) async fn load_runtime_grant(
        &self,
        config: &OwnerMarketStreamRuntimeConfig,
    ) -> Result<RuntimeGrant, MarketStreamStorageError> {
        let mut tx = self.begin_worker_transaction().await?;
        let row: Option<RuntimeGrantDbRow> = sqlx::query_as(
            "SELECT id, credential_slot_id, owner_user_id, grant_revision,
                    credential_generation, network_contract_sha256,
                    identity_list_sha256
               FROM public.owner_market_stream_grants
              WHERE id = $1 AND credential_slot_id = $2 AND state = 'ACTIVE'",
        )
        .bind(config.grant())
        .bind(config.slot())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let row = row.ok_or(MarketStreamStorageError::RightsInvalid)?;
        let generation = row
            .credential_generation
            .parse::<u64>()
            .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?;
        if row.credential_slot_id != config.slot()
            || row.id != config.grant()
            || generation != config.credential_generation()
            || generation.to_string() != row.credential_generation
            || row.network_contract_sha256 != config.contract_sha256()
            || !super::market_stream_runtime::fixed_stream_contract_is_canonical(
                &row.network_contract_sha256,
                &row.identity_list_sha256,
                market_data::STREAM_WIRE_VERSION,
            )
        {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let session_date = kst_database_date(&mut tx).await?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(row.credential_slot_id)
                .bind(row.id)
                .bind(row.grant_revision)
                .bind(row.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let grant = RuntimeGrant {
            credential_slot_id: row.credential_slot_id,
            grant_id: row.id,
            grant_revision: row.grant_revision,
            owner_user_id: row.owner_user_id,
            credential_generation: generation,
            network_contract_sha256: row.network_contract_sha256,
            identity_list_sha256: row.identity_list_sha256,
        };
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(grant)
    }

    /// Commit one finite runtime status transition behind the exact producer
    /// fence and epoch. `None` is an exact no-epoch expectation.
    pub(super) async fn record_runtime_status(
        &self,
        lease: &StreamProducerLease,
        expected_epoch: Option<Uuid>,
        transition: RuntimeTransition,
    ) -> Result<RuntimeStatusCommit, MarketStreamStorageError> {
        validate_producer_lease(lease)?;
        if expected_epoch.is_some_and(|epoch| epoch.is_nil()) {
            return Err(MarketStreamStorageError::EpochInvalid);
        }
        let values = transition.values();
        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let producer = load_runtime_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !producer.matches_lease(lease) || producer.current_epoch != expected_epoch {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let session_date = kst_database_date(&mut tx).await?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(producer.credential_slot_id)
                .bind(producer.grant_id)
                .bind(producer.grant_revision)
                .bind(producer.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        lock_owner_capacity(&mut tx, producer.owner_user_id).await?;
        let demand = lock_runtime_demand_inputs(&mut tx, &producer, false).await?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        if producer.lease_expires_at <= fresh_now {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        ensure_owner_identity_capacity(&mut tx, producer.owner_user_id, fresh_now).await?;
        let gap_opens = values.opens_gap
            && expected_epoch.is_some()
            && producer.session_date == Some(session_date);
        let same_status = producer.status_code.as_deref() == Some(values.code.as_str())
            && producer.connection_state == values.connection.as_str()
            && (if gap_opens {
                producer.gap_since.is_some()
            } else {
                true
            });
        if same_status {
            let committed = producer.status_commit(false)?;
            tx.commit()
                .await
                .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
            return Ok(committed);
        }
        if producer.state_version == i64::MAX
            || (gap_opens && producer.gap_since.is_none() && producer.gap_generation == i64::MAX)
        {
            return Err(MarketStreamStorageError::VersionExhausted);
        }
        let next_state_version = producer.state_version + 1;
        let next_gap_generation =
            producer.gap_generation + i64::from(gap_opens && producer.gap_since.is_none());
        let updated: RuntimeProducerDbRow = sqlx::query_as(
            "UPDATE public.owner_market_stream_producers
                SET status_code = $5, status_at = $6, connection_state = $7,
                    gap_since = CASE WHEN $8 THEN COALESCE(gap_since, $6)
                                     ELSE gap_since END,
                    session_has_gap = CASE WHEN $8 AND session_date IS NOT NULL
                                           THEN TRUE ELSE session_has_gap END,
                    gap_generation = $9, state_version = $10, updated_at = $6
              WHERE credential_slot_id = $1
                AND holder_id = $2 AND fencing_token = $3
                AND current_epoch IS NOT DISTINCT FROM $4
              RETURNING credential_slot_id, grant_id, grant_revision,
                        owner_user_id, holder_id, fencing_token, current_epoch,
                        session_date, session_proof_id, session_proof_sha256,
                        calendar_source_batch_id, calendar_content_sha256,
                        window_contract_sha256, lease_expires_at, heartbeat_at,
                        connection_state, status_code, status_at, gap_since,
                        session_has_gap, gap_generation, state_version, updated_at",
        )
        .bind(lease.credential_slot_id)
        .bind(lease.holder_id)
        .bind(
            i64::try_from(lease.fencing_token)
                .map_err(|_| MarketStreamStorageError::InvalidInput)?,
        )
        .bind(expected_epoch)
        .bind(values.code.as_str())
        .bind(fresh_now)
        .bind(values.connection.as_str())
        .bind(gap_opens)
        .bind(next_gap_generation)
        .bind(next_state_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?
        .ok_or(MarketStreamStorageError::ProducerLost)?;
        for identity in &demand.identities {
            update_runtime_cache_status(&mut tx, identity, &producer, &updated, values, fresh_now)
                .await?;
        }
        notify_changed(&mut tx).await?;
        let committed = updated.status_commit(true)?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(committed)
    }

    /// Retire only the exact current epoch after the caller has closed its
    /// socket. Protected lineage and accumulated gap metadata are retained.
    pub(super) async fn retire_stream_producer(
        &self,
        lease: &StreamProducerLease,
        expected_epoch: Option<Uuid>,
        reason: StreamStatusCode,
    ) -> Result<RuntimeStatusCommit, MarketStreamStorageError> {
        validate_producer_lease(lease)?;
        if expected_epoch.is_some_and(|epoch| epoch.is_nil()) {
            return Err(MarketStreamStorageError::EpochInvalid);
        }
        let transition = runtime_retirement_values(reason);
        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let producer = load_runtime_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !producer.matches_lease(lease) || producer.current_epoch != expected_epoch {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let session_date = kst_database_date(&mut tx).await?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(producer.credential_slot_id)
                .bind(producer.grant_id)
                .bind(producer.grant_revision)
                .bind(producer.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        lock_owner_capacity(&mut tx, producer.owner_user_id).await?;
        let demand = lock_runtime_demand_inputs(&mut tx, &producer, true).await?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        if producer.lease_expires_at <= fresh_now {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        ensure_owner_identity_capacity(&mut tx, producer.owner_user_id, fresh_now).await?;
        let gap_opens = expected_epoch.is_some()
            && producer.session_date == Some(session_date)
            && transition.opens_gap;
        let same_status = producer.current_epoch.is_none()
            && producer.connection_state == StreamConnectionState::Stopped.as_str()
            && producer.status_code.as_deref() == Some(reason.as_str())
            && producer.status_at.is_some();
        if same_status {
            let committed = producer.status_commit(false)?;
            tx.commit()
                .await
                .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
            return Ok(committed);
        }
        if producer.state_version == i64::MAX
            || (gap_opens && producer.gap_since.is_none() && producer.gap_generation == i64::MAX)
        {
            return Err(MarketStreamStorageError::VersionExhausted);
        }
        let next_state_version = producer.state_version + 1;
        let next_gap_generation =
            producer.gap_generation + i64::from(gap_opens && producer.gap_since.is_none());
        let updated: RuntimeProducerDbRow = sqlx::query_as(
            "UPDATE public.owner_market_stream_producers
                SET current_epoch = NULL, connection_state = 'STOPPED',
                    status_code = $5, status_at = $6,
                    gap_since = CASE WHEN $7 THEN COALESCE(gap_since, $6)
                                     ELSE gap_since END,
                    session_has_gap = CASE WHEN $7 AND session_date IS NOT NULL
                                           THEN TRUE ELSE session_has_gap END,
                    gap_generation = $8, state_version = $9, updated_at = $6
              WHERE credential_slot_id = $1
                AND holder_id = $2 AND fencing_token = $3
                AND current_epoch IS NOT DISTINCT FROM $4
              RETURNING credential_slot_id, grant_id, grant_revision,
                        owner_user_id, holder_id, fencing_token, current_epoch,
                        session_date, session_proof_id, session_proof_sha256,
                        calendar_source_batch_id, calendar_content_sha256,
                        window_contract_sha256, lease_expires_at, heartbeat_at,
                        connection_state, status_code, status_at, gap_since,
                        session_has_gap, gap_generation, state_version, updated_at",
        )
        .bind(lease.credential_slot_id)
        .bind(lease.holder_id)
        .bind(
            i64::try_from(lease.fencing_token)
                .map_err(|_| MarketStreamStorageError::InvalidInput)?,
        )
        .bind(expected_epoch)
        .bind(reason.as_str())
        .bind(fresh_now)
        .bind(gap_opens)
        .bind(next_gap_generation)
        .bind(next_state_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?
        .ok_or(MarketStreamStorageError::ProducerLost)?;
        sqlx::query(
            "UPDATE public.owner_market_stream_subscriptions
                SET epoch = NULL, state = 'ABSENT',
                    desired_reference_count = CASE WHEN $3 THEN 0
                                                   ELSE desired_reference_count END,
                    pending_operation = NULL, pending_ordinal = NULL,
                    pending_reserved_at = NULL, pending_deadline = NULL,
                    acked_at = NULL,
                    subscription_revision = pg_catalog.gen_random_uuid(),
                    updated_at = $2
              WHERE credential_slot_id = $1",
        )
        .bind(lease.credential_slot_id)
        .bind(fresh_now)
        .bind(demand.identities.is_empty())
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        for identity in &demand.identities {
            update_runtime_cache_status(
                &mut tx,
                identity,
                &producer,
                &updated,
                RuntimeTransitionValues {
                    code: reason,
                    connection: StreamConnectionState::Stopped,
                    market_state: StreamMarketState::Unknown,
                    freshness: StreamFreshness::Unavailable,
                    availability: StreamAvailability::LastKnown,
                    opens_gap: gap_opens,
                },
                fresh_now,
            )
            .await?;
        }
        notify_changed(&mut tx).await?;
        let committed = updated.status_commit(true)?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(committed)
    }

    /// Create, replace, or renew one complete browser demand set.  Capacity
    /// and identity locks are taken before the lease/item rows, and a
    /// released or expired consumer id can never be resurrected.
    pub async fn replace_stream_lease(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        request: &StreamLeaseRequest,
    ) -> Result<StreamLease, MarketStreamStorageError> {
        if owner_user_id.is_nil() || !canonical_session_hash(session_hash) {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        request.validate()?;
        let mut tx = self
            .begin_actor_transaction(owner_user_id, session_hash)
            .await?;

        lock_owner_capacity(&mut tx, owner_user_id).await?;
        let admissions =
            lock_requested_admissions(&mut tx, owner_user_id, session_hash, request).await?;

        let existing: Option<LeaseDbRow> = sqlx::query_as(
            "SELECT id, owner_user_id, consumer_id, session_hash, kind, renewal_sequence, state,
                    lease_expires_at, released_at, idempotency_key_sha256, request_sha256
               FROM public.owner_market_stream_leases
              WHERE owner_user_id = $1 AND consumer_id = $2
              FOR UPDATE",
        )
        .bind(owner_user_id)
        .bind(request.consumer_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let body_hash = request.request_digest();
        let idem_hash = request.idempotency_digest();
        if let Some(row) = existing.as_ref() {
            if row.session_hash.as_str() != session_hash {
                return Err(MarketStreamStorageError::LeaseSessionMismatch);
            }
            if row.state == "RELEASED" {
                return Err(MarketStreamStorageError::LeaseReleased);
            }
            let before_item_lock = fresh_database_time(&mut tx).await?;
            if row.lease_expires_at <= before_item_lock {
                return Err(MarketStreamStorageError::LeaseExpired);
            }
            lock_existing_stream_lease_items(&mut tx, owner_user_id, session_hash, row.id).await?;
            let fresh_now = fresh_database_time(&mut tx).await?;
            if row.lease_expires_at <= fresh_now {
                return Err(MarketStreamStorageError::LeaseExpired);
            }
            if row.request_sha256 == body_hash && row.renewal_sequence == request.sequence_i64() {
                let lease = load_stream_lease(&mut tx, row.id, owner_user_id).await?;
                tx.commit()
                    .await
                    .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
                return Ok(lease);
            }
            if row.renewal_sequence == request.sequence_i64() && row.request_sha256 != body_hash {
                return Err(MarketStreamStorageError::IdempotencyMismatch);
            }
            let next = row
                .renewal_sequence
                .checked_add(1)
                .ok_or(MarketStreamStorageError::SequenceConflict)?;
            if request.sequence_i64() != next {
                return Err(MarketStreamStorageError::SequenceConflict);
            }
            let updated: LeaseDbRow = sqlx::query_as(
                "UPDATE public.owner_market_stream_leases
                    SET renewal_sequence = $3,
                        lease_expires_at = $4 + INTERVAL '30 seconds',
                        idempotency_key_sha256 = $5,
                        request_sha256 = $6,
                        updated_at = $4
                  WHERE id = $1 AND owner_user_id = $2 AND state = 'ACTIVE'
                  RETURNING id, owner_user_id, consumer_id, session_hash, kind,
                            renewal_sequence, state, lease_expires_at,
                            released_at, idempotency_key_sha256, request_sha256",
            )
            .bind(row.id)
            .bind(owner_user_id)
            .bind(request.sequence_i64())
            .bind(fresh_now)
            .bind(&idem_hash)
            .bind(&body_hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database_error)?;
            replace_lease_items(&mut tx, &updated, &admissions).await?;
            ensure_owner_identity_capacity(&mut tx, owner_user_id, fresh_now).await?;
            notify_changed(&mut tx).await?;
            let lease = load_stream_lease(&mut tx, updated.id, owner_user_id).await?;
            tx.commit()
                .await
                .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
            return Ok(lease);
        }

        if request.renewal_sequence != 0 {
            return Err(MarketStreamStorageError::LeaseNotFound);
        }
        let fresh_now = fresh_database_time(&mut tx).await?;
        let active_count: i64 = sqlx::query_scalar(
            "SELECT count(*)
               FROM public.owner_market_stream_leases
              WHERE owner_user_id = $1
                AND state = 'ACTIVE'
                AND lease_expires_at > $2",
        )
        .bind(owner_user_id)
        .bind(fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        if active_count >= STREAM_MAX_ACTIVE_LEASES {
            return Err(MarketStreamStorageError::LeaseCapacity);
        }

        let inserted: LeaseDbRow = sqlx::query_as(
            "INSERT INTO public.owner_market_stream_leases
                (id, owner_user_id, consumer_id, session_hash, kind,
                 renewal_sequence, state, lease_expires_at,
                 idempotency_key_sha256, request_sha256)
             VALUES ($1, $2, $3, $4, 'BROWSER', 0, 'ACTIVE',
                     $5 + INTERVAL '30 seconds', $6, $7)
             RETURNING id, owner_user_id, consumer_id, session_hash, kind, renewal_sequence,
                       state, lease_expires_at, released_at,
                       idempotency_key_sha256, request_sha256",
        )
        .bind(Uuid::new_v4())
        .bind(owner_user_id)
        .bind(request.consumer_id)
        .bind(session_hash)
        .bind(fresh_now)
        .bind(&idem_hash)
        .bind(&body_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        insert_lease_items(&mut tx, &inserted, &admissions).await?;
        ensure_owner_identity_capacity(&mut tx, owner_user_id, fresh_now).await?;
        notify_changed(&mut tx).await?;
        let lease = load_stream_lease(&mut tx, inserted.id, owner_user_id).await?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(lease)
    }

    /// Release is a tombstone transition.  It does not delete items and it
    /// remains idempotent for the same session; later calls cannot revive the
    /// consumer id.
    pub async fn release_stream_lease(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
        sequence: u64,
    ) -> Result<ReleaseOutcome, MarketStreamStorageError> {
        self.release_stream_lease_inner(owner_user_id, session_hash, lease_id, None, sequence)
            .await
    }

    /// Release one lease only when its immutable consumer binding also matches.
    pub async fn release_stream_lease_for_consumer(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
        consumer_id: Uuid,
        sequence: u64,
    ) -> Result<ReleaseOutcome, MarketStreamStorageError> {
        self.release_stream_lease_inner(
            owner_user_id,
            session_hash,
            lease_id,
            Some(consumer_id),
            sequence,
        )
        .await
    }

    async fn release_stream_lease_inner(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
        consumer_id: Option<Uuid>,
        sequence: u64,
    ) -> Result<ReleaseOutcome, MarketStreamStorageError> {
        if owner_user_id.is_nil()
            || lease_id.is_nil()
            || consumer_id.is_some_and(|consumer_id| consumer_id.is_nil())
            || !canonical_session_hash(session_hash)
            || i64::try_from(sequence).is_err()
        {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let mut tx = self
            .begin_actor_transaction(owner_user_id, session_hash)
            .await?;
        lock_owner_capacity(&mut tx, owner_user_id).await?;
        let row: Option<LeaseDbRow> = sqlx::query_as(
            "SELECT id, owner_user_id, consumer_id, session_hash, kind, renewal_sequence, state,
                    lease_expires_at, released_at, idempotency_key_sha256, request_sha256
               FROM public.owner_market_stream_leases
              WHERE id = $1 AND owner_user_id = $2
              FOR UPDATE",
        )
        .bind(lease_id)
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let row = row.ok_or(MarketStreamStorageError::LeaseNotFound)?;
        if release_binding_is_tombstone_replay(
            row.session_hash.as_str(),
            session_hash,
            row.consumer_id,
            consumer_id,
            row.state == "RELEASED",
        )? {
            tx.commit()
                .await
                .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
            return Ok(ReleaseOutcome {
                lease_id,
                released: true,
            });
        }
        if row.renewal_sequence != i64::try_from(sequence).unwrap_or(i64::MAX) {
            return Err(MarketStreamStorageError::SequenceConflict);
        }
        let fresh_now = fresh_database_time(&mut tx).await?;
        sqlx::query(
            "UPDATE public.owner_market_stream_leases
                SET state = 'RELEASED', released_at = $3, updated_at = $3
              WHERE id = $1 AND owner_user_id = $2 AND state = 'ACTIVE'",
        )
        .bind(lease_id)
        .bind(owner_user_id)
        .bind(fresh_now)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        notify_changed(&mut tx).await?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(ReleaseOutcome {
            lease_id,
            released: true,
        })
    }

    /// Merge active, unexpired, session-valid leases for one credential slot
    /// into unique desired market identities.  The result is bounded rather
    /// than silently truncated.
    pub async fn read_stream_demand(
        &self,
        credential_slot_id: Uuid,
    ) -> Result<DesiredSet, MarketStreamStorageError> {
        if credential_slot_id.is_nil() {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let mut tx = self.begin_worker_transaction().await?;
        let grant: Option<GrantDbRow> = sqlx::query_as(
            "SELECT id, credential_slot_id, owner_user_id, grant_revision
               FROM public.owner_market_stream_grants
              WHERE credential_slot_id = $1 AND state = 'ACTIVE'
              ORDER BY id
              LIMIT 1",
        )
        .bind(credential_slot_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let grant = grant.ok_or(MarketStreamStorageError::RightsInvalid)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        let session_date = kst_database_date(&mut tx).await?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.owner_market_stream_rights_valid($1, $2, $3)")
                .bind(grant.id)
                .bind(grant.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let pruned = prune_stream_cache(&mut tx, credential_slot_id).await?;
        if pruned {
            notify_changed(&mut tx).await?;
        }
        let rows: Vec<DesiredDbRow> = sqlx::query_as(
            "SELECT item.owner_user_id, item.membership_id, item.generation_id,
                    item.instrument_id, item.generation,
                    count(*)::bigint AS reference_count
               FROM public.owner_market_stream_leases AS lease
               JOIN public.owner_market_stream_lease_items AS item
                 ON item.lease_id = lease.id
                AND item.owner_user_id = lease.owner_user_id
               JOIN public.owner_equity_memberships AS membership
                 ON membership.id = item.membership_id
                AND membership.owner_user_id = item.owner_user_id
                AND membership.instrument_id = item.instrument_id
                AND membership.state = 'READY'
               JOIN public.owner_equity_generation_admissions AS admission
                 ON admission.generation_id = item.generation_id
                AND admission.owner_user_id = item.owner_user_id
                AND admission.membership_id = item.membership_id
                AND admission.instrument_id = item.instrument_id
                AND admission.generation = item.generation
              WHERE lease.owner_user_id = $1
                AND lease.state = 'ACTIVE'
                AND lease.lease_expires_at > $2
                AND public.owner_market_stream_session_valid(
                    lease.session_hash, lease.owner_user_id
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM public.owner_equity_generation_admissions AS newer
                     WHERE newer.owner_user_id = admission.owner_user_id
                       AND newer.membership_id = admission.membership_id
                       AND newer.instrument_id = admission.instrument_id
                       AND newer.generation > admission.generation
                )
              GROUP BY item.owner_user_id, item.membership_id, item.generation_id,
                       item.instrument_id, item.generation
              ORDER BY item.instrument_id, item.membership_id",
        )
        .bind(grant.owner_user_id)
        .bind(fresh_now)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_database_error)?;
        if rows.len() > STREAM_MAX_ACTIVE_IDENTITIES {
            return Err(MarketStreamStorageError::IdentityCapacity);
        }
        let items = rows
            .into_iter()
            .map(DesiredDbRow::into_item)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(DesiredSet {
            credential_slot_id,
            owner_user_id: grant.owner_user_id,
            items,
        })
    }

    /// Acquire the single producer slot for a credential grant.  The
    /// producer row is the durable fence anchor; an expired DB lease cannot
    /// steal a process that still owns the WS-2 lifetime lock, which is a
    /// separate boundary owned by the transport.
    pub async fn claim_stream_producer(
        &self,
        credential_slot_id: Uuid,
        holder_id: Uuid,
        grant_revision: Uuid,
    ) -> Result<StreamProducerLease, MarketStreamStorageError> {
        if credential_slot_id.is_nil() || holder_id.is_nil() || grant_revision.is_nil() {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, credential_slot_id).await?;
        let grant: GrantDbRow = sqlx::query_as(
            "SELECT id, credential_slot_id, owner_user_id, grant_revision
               FROM public.owner_market_stream_grants
              WHERE credential_slot_id = $1
                AND grant_revision = $2
                AND state = 'ACTIVE'",
        )
        .bind(credential_slot_id)
        .bind(grant_revision)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?
        .ok_or(MarketStreamStorageError::RightsInvalid)?;
        let session_date = kst_database_date(&mut tx).await?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(grant.credential_slot_id)
                .bind(grant.id)
                .bind(grant.grant_revision)
                .bind(grant.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let existing: Option<ProducerDbRow> = sqlx::query_as(
            "SELECT credential_slot_id, grant_id, grant_revision, owner_user_id,
                    holder_id, fencing_token, current_epoch, session_date,
                    session_proof_id, session_proof_sha256, calendar_source,
                    calendar_source_version, calendar_source_batch_id,
                    calendar_content_sha256, window_contract_sha256,
                    lease_expires_at, heartbeat_at, connection_state,
                    gap_generation, state_version, updated_at
               FROM public.owner_market_stream_producers
              WHERE credential_slot_id = $1
              FOR UPDATE",
        )
        .bind(credential_slot_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        let producer = match existing {
            Some(row) => {
                if row.grant_id != grant.id
                    || row.grant_revision != grant.grant_revision
                    || row.owner_user_id != grant.owner_user_id
                {
                    return Err(MarketStreamStorageError::ProducerLost);
                }
                if row.lease_expires_at > fresh_now && row.holder_id != holder_id {
                    return Err(MarketStreamStorageError::ProducerHeld);
                }
                if row.lease_expires_at > fresh_now && row.holder_id == holder_id {
                    // Re-claiming an unexpired lease by the same holder is an
                    // idempotent heartbeat.  It must not fence the caller's
                    // own socket or manufacture a reconnect gap.
                    sqlx::query_as(
                        "UPDATE public.owner_market_stream_producers
                            SET lease_expires_at = $2 + INTERVAL '20 seconds',
                                heartbeat_at = $2, updated_at = $2
                          WHERE credential_slot_id = $1
                            AND holder_id = $3 AND fencing_token = $4
                          RETURNING credential_slot_id, grant_id, grant_revision,
                                    owner_user_id, holder_id, fencing_token,
                                    current_epoch, session_date, session_proof_id,
                                    session_proof_sha256, calendar_source,
                                    calendar_source_version, calendar_source_batch_id,
                                    calendar_content_sha256, window_contract_sha256,
                                    lease_expires_at, heartbeat_at, connection_state,
                                    gap_generation, state_version, updated_at",
                    )
                    .bind(credential_slot_id)
                    .bind(fresh_now)
                    .bind(holder_id)
                    .bind(row.fencing_token)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_database_error)?
                } else {
                    let fence = row
                        .fencing_token
                        .checked_add(1)
                        .ok_or(MarketStreamStorageError::FenceExhausted)?;
                    let updated: ProducerDbRow = sqlx::query_as(
                        "UPDATE public.owner_market_stream_producers
                        SET holder_id = $2, fencing_token = $3,
                            lease_expires_at = $4 + INTERVAL '20 seconds',
                            heartbeat_at = $4, connection_state = 'DISCONNECTED',
                            current_epoch = NULL,
                            status_code = CASE
                                WHEN $5 AND current_epoch IS NOT NULL
                                THEN 'CONNECTION_LOST' ELSE status_code END,
                            status_at = CASE
                                WHEN $5 AND current_epoch IS NOT NULL
                                THEN $4 ELSE status_at END,
                            gap_since = CASE
                                WHEN $5 AND current_epoch IS NOT NULL
                                THEN COALESCE(gap_since, $4) ELSE gap_since END,
                            session_has_gap = CASE
                                WHEN $5 AND current_epoch IS NOT NULL AND session_date IS NOT NULL
                                THEN TRUE ELSE session_has_gap END,
                            gap_generation = CASE
                                WHEN $5 AND current_epoch IS NOT NULL AND gap_since IS NULL
                                THEN gap_generation + 1 ELSE gap_generation END,
                            state_version = state_version + 1, updated_at = $4
                      WHERE credential_slot_id = $1
                      RETURNING credential_slot_id, grant_id, grant_revision,
                                owner_user_id, holder_id, fencing_token,
                                current_epoch, session_date, session_proof_id,
                                session_proof_sha256, calendar_source,
                                calendar_source_version, calendar_source_batch_id,
                                calendar_content_sha256, window_contract_sha256,
                                lease_expires_at, heartbeat_at, connection_state,
                                gap_generation, state_version, updated_at",
                    )
                    .bind(credential_slot_id)
                    .bind(holder_id)
                    .bind(fence)
                    .bind(fresh_now)
                    .bind(row.lease_expires_at <= fresh_now)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_database_error)?;
                    updated
                }
            }
            None => sqlx::query_as(
                "INSERT INTO public.owner_market_stream_producers
                    (credential_slot_id, grant_id, grant_revision, owner_user_id,
                     holder_id, fencing_token, lease_expires_at, heartbeat_at,
                     connection_state, gap_generation, state_version, updated_at)
                 VALUES ($1, $2, $3, $4, $5, 1,
                         $6 + INTERVAL '20 seconds', $6,
                         'DISCONNECTED', 0, 1, $6)
                 RETURNING credential_slot_id, grant_id, grant_revision,
                           owner_user_id, holder_id, fencing_token,
                           current_epoch, session_date, session_proof_id,
                           session_proof_sha256, calendar_source,
                           calendar_source_version, calendar_source_batch_id,
                           calendar_content_sha256, window_contract_sha256,
                           lease_expires_at, heartbeat_at, connection_state,
                           gap_generation, state_version, updated_at",
            )
            .bind(credential_slot_id)
            .bind(grant.id)
            .bind(grant.grant_revision)
            .bind(grant.owner_user_id)
            .bind(holder_id)
            .bind(fresh_now)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database_error)?,
        };
        notify_changed(&mut tx).await?;
        let lease = producer.into_lease()?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(lease)
    }

    /// Renew only the exact holder/fence.  A stale process cannot extend a
    /// taken-over producer lease.
    pub async fn renew_stream_producer(
        &self,
        lease: &StreamProducerLease,
    ) -> Result<StreamProducerLease, MarketStreamStorageError> {
        validate_producer_lease(lease)?;
        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let row: ProducerDbRow =
            load_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !row.matches_lease(lease) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let fresh_now = fresh_database_time(&mut tx).await?;
        if row.lease_expires_at <= fresh_now {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let refreshed: ProducerDbRow = sqlx::query_as(
            "UPDATE public.owner_market_stream_producers
                SET lease_expires_at = $2 + INTERVAL '20 seconds',
                    heartbeat_at = $2, updated_at = $2
              WHERE credential_slot_id = $1
                AND holder_id = $3 AND fencing_token = $4
              RETURNING credential_slot_id, grant_id, grant_revision,
                        owner_user_id, holder_id, fencing_token,
                        current_epoch, session_date, session_proof_id,
                        session_proof_sha256, calendar_source,
                        calendar_source_version, calendar_source_batch_id,
                        calendar_content_sha256, window_contract_sha256,
                        lease_expires_at, heartbeat_at, connection_state,
                        gap_generation, state_version, updated_at",
        )
        .bind(lease.credential_slot_id)
        .bind(fresh_now)
        .bind(lease.holder_id)
        .bind(
            i64::try_from(lease.fencing_token)
                .map_err(|_| MarketStreamStorageError::InvalidInput)?,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        refreshed.into_lease()
    }

    /// Commit a new socket epoch and session lineage before any subscription
    /// command is allowed.  Reconnect invalidates all prior ACKs while the
    /// latest-value cache remains untouched until a new receipt is published.
    pub(super) async fn start_stream_epoch(
        &self,
        lease: &StreamProducerLease,
        session: StreamSessionProof,
        epoch: Uuid,
    ) -> Result<StreamEpochProof, MarketStreamStorageError> {
        self.start_stream_epoch_inner(lease, session, epoch, None)
            .await
    }

    /// Runtime-only epoch entry. Unlike the compatibility facade above, this
    /// accepts only a private resolved day and rechecks both calendar lineage
    /// and the pinned half-open window after the producer lock and fresh DB clock.
    pub(super) async fn start_stream_epoch_resolved(
        &self,
        lease: &StreamProducerLease,
        day: &ResolvedMarketStreamDay,
        epoch: Uuid,
    ) -> Result<StreamEpochProof, MarketStreamStorageError> {
        let session = day.session();
        self.start_stream_epoch_inner(lease, session, epoch, Some(day))
            .await
    }

    async fn start_stream_epoch_inner(
        &self,
        lease: &StreamProducerLease,
        session: StreamSessionProof,
        epoch: Uuid,
        resolved_day: Option<&ResolvedMarketStreamDay>,
    ) -> Result<StreamEpochProof, MarketStreamStorageError> {
        validate_producer_lease(lease)?;
        session.validate()?;
        if epoch.is_nil() {
            return Err(MarketStreamStorageError::EpochInvalid);
        }
        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let row = load_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !row.matches_lease(lease) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(lease.credential_slot_id)
                .bind(lease.grant_id)
                .bind(lease.grant_revision)
                .bind(lease.owner_user_id)
                .bind(session.session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let fresh_now = fresh_database_time(&mut tx).await?;
        let kst_offset = FixedOffset::east_opt(KST_OFFSET_SECONDS)
            .ok_or(MarketStreamStorageError::SessionInvalid)?;
        if fresh_now.with_timezone(&kst_offset).date_naive() != session.session_date {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        if let Some(day) = resolved_day {
            if day.session() != session {
                return Err(MarketStreamStorageError::SessionInvalid);
            }
            validate_resolved_day(&mut tx, day, fresh_now).await?;
        }
        if row.lease_expires_at <= fresh_now + chrono::Duration::seconds(5) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let next_state = row
            .state_version
            .checked_add(1)
            .ok_or(MarketStreamStorageError::VersionExhausted)?;
        sqlx::query(
            "UPDATE public.owner_market_stream_producers
                SET current_epoch = $2, session_date = $3,
                    session_proof_id = $4, session_proof_sha256 = $5,
                    calendar_source = 'kis',
                    calendar_source_version = 'kis-chk-holiday-v1:schema-1',
                    calendar_source_batch_id = $6,
                    calendar_content_sha256 = $7,
                    window_contract_sha256 = $8,
                    connection_state = 'CONNECTED',
                    gap_since = CASE WHEN session_date IS DISTINCT FROM $3
                                     THEN NULL ELSE gap_since END,
                    session_has_gap = CASE WHEN session_date IS DISTINCT FROM $3
                                           THEN FALSE ELSE session_has_gap END,
                    status_code = 'SUBSCRIPTION_PENDING', status_at = $11,
                    gap_generation = $9,
                    state_version = $10, updated_at = $11
              WHERE credential_slot_id = $1
                AND holder_id = $12 AND fencing_token = $13",
        )
        .bind(lease.credential_slot_id)
        .bind(epoch)
        .bind(session.session_date)
        .bind(session.session_proof_id)
        .bind(&session.session_proof_sha256)
        .bind(session.calendar_source_batch_id)
        .bind(&session.calendar_content_sha256)
        .bind(&session.window_contract_sha256)
        .bind(row.gap_generation)
        .bind(next_state)
        .bind(fresh_now)
        .bind(lease.holder_id)
        .bind(
            i64::try_from(lease.fencing_token)
                .map_err(|_| MarketStreamStorageError::InvalidInput)?,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        sqlx::query(
            "UPDATE public.owner_market_stream_subscriptions
                SET epoch = $2,
                    grant_revision = $3,
                    state = CASE WHEN desired_reference_count > 0
                                 THEN 'DESIRED' ELSE 'ABSENT' END,
                    pending_operation = NULL, pending_ordinal = NULL,
                    pending_reserved_at = NULL, pending_deadline = NULL,
                    acked_at = NULL,
                    subscription_revision = pg_catalog.gen_random_uuid(),
                    updated_at = $4
              WHERE credential_slot_id = $1",
        )
        .bind(lease.credential_slot_id)
        .bind(epoch)
        .bind(lease.grant_revision)
        .bind(fresh_now)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        notify_changed(&mut tx).await?;
        let proof = StreamEpochProof {
            credential_slot_id: lease.credential_slot_id,
            grant_revision: lease.grant_revision,
            epoch,
            session: session.clone(),
            fencing_token: lease.fencing_token,
            gap_generation: u64::try_from(row.gap_generation)
                .map_err(|_| MarketStreamStorageError::VersionExhausted)?,
            resolved_day_id: resolved_day.map(ResolvedMarketStreamDay::resolution_id),
        };
        if let Some(day) = resolved_day {
            #[cfg(test)]
            wait_at_resolved_commit_test_gate(epoch).await?;
            revalidate_resolved_commit_guard(&mut tx, lease, day, epoch).await?;
        }
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(proof)
    }

    /// Persist an exact desired count. Repeating the current value is a true
    /// no-op and never clears or overwrites an in-flight transport command.
    pub(super) async fn set_subscription_desired(
        &self,
        lease: &StreamProducerLease,
        epoch: Uuid,
        symbol: &str,
        reference_count: u32,
    ) -> Result<(), MarketStreamStorageError> {
        validate_producer_lease(lease)?;
        if epoch.is_nil() || !is_six_ascii_digits(symbol) {
            return Err(MarketStreamStorageError::SubscriptionInvalid);
        }
        if reference_count as usize
            > STREAM_MAX_ACTIVE_LEASES as usize * STREAM_MAX_IDENTITIES_PER_LEASE
        {
            return Err(MarketStreamStorageError::IdentityCapacity);
        }
        let reference_count = i32::try_from(reference_count)
            .map_err(|_| MarketStreamStorageError::IdentityCapacity)?;
        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let producer = load_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !producer.matches_lease(lease) || producer.current_epoch != Some(epoch) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let session_date = producer
            .session_date
            .ok_or(MarketStreamStorageError::SessionInvalid)?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(lease.credential_slot_id)
                .bind(lease.grant_id)
                .bind(lease.grant_revision)
                .bind(lease.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let current: Option<SubscriptionDbRow> = sqlx::query_as(
            "SELECT credential_slot_id, symbol, provider, environment, venue,
                    tr_id, grant_revision, epoch, state, pending_operation,
                    pending_ordinal, pending_reserved_at, pending_deadline,
                    acked_at, desired_reference_count, subscription_revision,
                    updated_at
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2
              FOR UPDATE",
        )
        .bind(lease.credential_slot_id)
        .bind(symbol)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        if producer.lease_expires_at
            <= fresh_now + chrono::Duration::seconds(STREAM_PRODUCER_RENEW_AFTER_SECONDS)
        {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        if kst_database_date(&mut tx).await? != session_date {
            return Err(MarketStreamStorageError::SessionInvalid);
        }

        let state = if reference_count == 0 {
            "ABSENT"
        } else {
            "DESIRED"
        };
        match current {
            Some(row) => {
                if row.provider != "kis"
                    || row.environment != "live"
                    || row.venue != STREAM_VENUE
                    || row.tr_id != "H0STCNT0"
                    || row.epoch != Some(epoch)
                    || row.grant_revision != lease.grant_revision
                    || row.symbol != symbol
                {
                    return Err(MarketStreamStorageError::SubscriptionInvalid);
                }
                if (row.pending_operation.is_some()
                    && (row.pending_ordinal.is_none()
                        || row.pending_reserved_at.is_none()
                        || row.pending_deadline.is_none()))
                    || (row.pending_operation.is_none()
                        && (row.pending_ordinal.is_some()
                            || row.pending_reserved_at.is_some()
                            || row.pending_deadline.is_some()))
                {
                    return Err(MarketStreamStorageError::DatabaseIntegrity);
                }
                if row.desired_reference_count == reference_count {
                    tx.commit()
                        .await
                        .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
                    return Ok(());
                }
                if row.pending_operation.is_some()
                    || matches!(
                        row.state.as_str(),
                        "PENDING_SUBSCRIBE" | "PENDING_UNSUBSCRIBE"
                    )
                {
                    return Err(MarketStreamStorageError::SubscriptionInvalid);
                }
                if row.state == "ACKED"
                    && row.desired_reference_count > 0
                    && reference_count > 0
                    && row.acked_at.is_some()
                    && row.pending_operation.is_none()
                    && row.pending_ordinal.is_none()
                    && row.pending_reserved_at.is_none()
                    && row.pending_deadline.is_none()
                {
                    sqlx::query(
                        "UPDATE public.owner_market_stream_subscriptions
                            SET desired_reference_count = $3, updated_at = $4
                          WHERE credential_slot_id = $1 AND symbol = $2
                            AND state = 'ACKED'",
                    )
                    .bind(lease.credential_slot_id)
                    .bind(symbol)
                    .bind(reference_count)
                    .bind(fresh_now)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_database_error)?;
                    notify_changed(&mut tx).await?;
                    tx.commit()
                        .await
                        .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
                    return Ok(());
                }
                sqlx::query(
                    "UPDATE public.owner_market_stream_subscriptions
                        SET state = $3, desired_reference_count = $4, acked_at = NULL,
                            subscription_revision = pg_catalog.gen_random_uuid(),
                            updated_at = $5
                      WHERE credential_slot_id = $1 AND symbol = $2",
                )
                .bind(lease.credential_slot_id)
                .bind(symbol)
                .bind(state)
                .bind(reference_count)
                .bind(fresh_now)
                .execute(&mut *tx)
                .await
                .map_err(map_database_error)?;
            }
            None if reference_count == 0 => {
                tx.commit()
                    .await
                    .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
                return Ok(());
            }
            None => {
                sqlx::query(
                    "INSERT INTO public.owner_market_stream_subscriptions
                        (credential_slot_id, symbol, provider, environment, venue,
                         tr_id, grant_revision, epoch, state, desired_reference_count,
                         updated_at)
                     VALUES ($1, $2, 'kis', 'live', 'KRX', 'H0STCNT0',
                             $3, $4, $5, $6, $7)",
                )
                .bind(lease.credential_slot_id)
                .bind(symbol)
                .bind(lease.grant_revision)
                .bind(epoch)
                .bind(state)
                .bind(reference_count)
                .bind(fresh_now)
                .execute(&mut *tx)
                .await
                .map_err(map_database_error)?;
            }
        }
        notify_changed(&mut tx).await?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)
    }

    /// Commit a pending identity derived from an authentic, still-owned C2
    /// command. No pending token exists unless this transaction commits.
    pub(super) async fn commit_prepared_subscription(
        &self,
        lease: &StreamProducerLease,
        prepared: &PreparedMarketSubscriptionCommand,
    ) -> Result<PendingSubscriptionCommit, MarketStreamStorageError> {
        validate_producer_lease(lease)?;
        let slot = prepared.credential_slot_id();
        let epoch = prepared.epoch().uuid();
        let symbol = prepared.symbol();
        let operation = storage_operation(prepared.operation());
        let ordinal = prepared.ordinal();
        let reserved_at_ms = prepared.reserved_at_ms();
        let reserved_at_monotonic = prepared.reserved_at_monotonic();
        let deadline_at_ms = prepared.deadline_at_ms();
        let deadline_monotonic = prepared.deadline_monotonic();
        if slot != lease.credential_slot_id
            || epoch.is_nil()
            || ordinal == 0
            || !is_six_ascii_digits(symbol)
        {
            return Err(MarketStreamStorageError::SubscriptionInvalid);
        }
        validate_ack_capture_window(
            reserved_at_ms,
            reserved_at_monotonic,
            deadline_at_ms,
            deadline_monotonic,
            reserved_at_ms,
            reserved_at_monotonic,
        )?;
        let reserved_at = exact_wall_millis(reserved_at_ms)?;
        let deadline = exact_wall_millis(deadline_at_ms)?;

        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let producer = load_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !producer.matches_lease(lease) || producer.current_epoch != Some(epoch) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let session_date = producer
            .session_date
            .ok_or(MarketStreamStorageError::SessionInvalid)?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(lease.credential_slot_id)
                .bind(lease.grant_id)
                .bind(lease.grant_revision)
                .bind(lease.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let current: Option<SubscriptionDbRow> = sqlx::query_as(
            "SELECT credential_slot_id, symbol, provider, environment, venue,
                    tr_id, grant_revision, epoch, state, pending_operation,
                    pending_ordinal, pending_reserved_at, pending_deadline,
                    acked_at, desired_reference_count, subscription_revision,
                    updated_at
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2
              FOR UPDATE",
        )
        .bind(slot)
        .bind(symbol)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        if producer.lease_expires_at
            <= fresh_now + chrono::Duration::seconds(STREAM_PRODUCER_RENEW_AFTER_SECONDS)
        {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        if kst_database_date(&mut tx).await? != session_date {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        let row = current.ok_or(MarketStreamStorageError::SubscriptionInvalid)?;
        if row.provider != "kis"
            || row.environment != "live"
            || row.venue != STREAM_VENUE
            || row.tr_id != "H0STCNT0"
            || row.credential_slot_id != slot
            || row.symbol != symbol
            || row.grant_revision != lease.grant_revision
            || row.epoch != Some(epoch)
            || row.pending_operation.is_some()
            || row.pending_ordinal.is_some()
            || row.pending_reserved_at.is_some()
            || row.pending_deadline.is_some()
        {
            return Err(MarketStreamStorageError::SubscriptionInvalid);
        }
        match operation {
            SubscriptionOperation::Subscribe
                if row.desired_reference_count > 0 && row.state == "DESIRED" => {}
            SubscriptionOperation::Unsubscribe
                if row.desired_reference_count == 0 && row.state == "ABSENT" => {}
            _ => return Err(MarketStreamStorageError::SubscriptionInvalid),
        }
        let pending_state = match operation {
            SubscriptionOperation::Subscribe => "PENDING_SUBSCRIBE",
            SubscriptionOperation::Unsubscribe => "PENDING_UNSUBSCRIBE",
        };
        let ordinal_i64 =
            i64::try_from(ordinal).map_err(|_| MarketStreamStorageError::SubscriptionInvalid)?;
        let updated: SubscriptionDbRow = sqlx::query_as(
            "UPDATE public.owner_market_stream_subscriptions
                SET state = $3, pending_operation = $4, pending_ordinal = $5,
                    pending_reserved_at = $6, pending_deadline = $7, acked_at = NULL,
                    subscription_revision = pg_catalog.gen_random_uuid(), updated_at = $8
              WHERE credential_slot_id = $1 AND symbol = $2
              RETURNING credential_slot_id, symbol, provider, environment, venue,
                        tr_id, grant_revision, epoch, state, pending_operation,
                        pending_ordinal, pending_reserved_at, pending_deadline,
                        acked_at, desired_reference_count, subscription_revision,
                        updated_at",
        )
        .bind(slot)
        .bind(symbol)
        .bind(pending_state)
        .bind(operation.as_str())
        .bind(ordinal_i64)
        .bind(reserved_at)
        .bind(deadline)
        .bind(fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        notify_changed(&mut tx).await?;
        let subscription_revision = updated.subscription_revision;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(PendingSubscriptionCommit {
            credential_slot_id: slot,
            grant_revision: lease.grant_revision,
            epoch,
            symbol: symbol.to_owned(),
            operation,
            ordinal,
            reserved_at_ms,
            reserved_at_monotonic,
            deadline_at_ms,
            deadline_monotonic,
            subscription_revision,
        })
    }

    /// Commit only the authentic ACK returned by the C2 session that consumed
    /// the command. The pending token is single-use and is not cloneable.
    pub(super) async fn commit_subscription_ack(
        &self,
        lease: &StreamProducerLease,
        pending: PendingSubscriptionCommit,
        ack: MarketSubscriptionAck,
    ) -> Result<Option<SubscriptionProof>, MarketStreamStorageError> {
        if lease.credential_slot_id != pending.credential_slot_id {
            return Err(MarketStreamStorageError::SubscriptionInvalid);
        }
        validate_producer_lease(lease)?;
        let ack_epoch = ack.epoch().uuid();
        if ack.credential_slot_id() != pending.credential_slot_id
            || ack_epoch != pending.epoch
            || ack.symbol() != pending.symbol
            || storage_operation(ack.operation()) != pending.operation
            || ack.ordinal() != pending.ordinal
            || ack.reserved_at_ms() != pending.reserved_at_ms
            || ack.reserved_at_monotonic() != pending.reserved_at_monotonic
            || ack.deadline_at_ms() != pending.deadline_at_ms
            || ack.deadline_monotonic() != pending.deadline_monotonic
        {
            return Err(MarketStreamStorageError::SubscriptionInvalid);
        }
        validate_ack_capture_window(
            pending.reserved_at_ms,
            pending.reserved_at_monotonic,
            pending.deadline_at_ms,
            pending.deadline_monotonic,
            ack.ack_received_at_ms(),
            ack.ack_received_monotonic(),
        )?;
        let reserved_at = exact_wall_millis(pending.reserved_at_ms)?;
        let deadline = exact_wall_millis(pending.deadline_at_ms)?;
        let acked_at = exact_wall_millis(ack.ack_received_at_ms())?;

        let mut tx = self.begin_worker_transaction().await?;
        lock_producer_slot(&mut tx, lease.credential_slot_id).await?;
        let producer = load_producer_for_update(&mut tx, lease.credential_slot_id).await?;
        if !producer.matches_lease(lease) || producer.current_epoch != Some(pending.epoch) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let session_date = producer
            .session_date
            .ok_or(MarketStreamStorageError::SessionInvalid)?;
        let rights: bool =
            sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
                .bind(lease.credential_slot_id)
                .bind(lease.grant_id)
                .bind(lease.grant_revision)
                .bind(lease.owner_user_id)
                .bind(session_date)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !rights {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
        let current: Option<SubscriptionDbRow> = sqlx::query_as(
            "SELECT credential_slot_id, symbol, provider, environment, venue,
                    tr_id, grant_revision, epoch, state, pending_operation,
                    pending_ordinal, pending_reserved_at, pending_deadline,
                    acked_at, desired_reference_count, subscription_revision,
                    updated_at
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2
              FOR UPDATE",
        )
        .bind(pending.credential_slot_id)
        .bind(&pending.symbol)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        if producer.lease_expires_at
            <= fresh_now + chrono::Duration::seconds(STREAM_PRODUCER_RENEW_AFTER_SECONDS)
        {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        if kst_database_date(&mut tx).await? != session_date {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        let row = current.ok_or(MarketStreamStorageError::SubscriptionInvalid)?;
        let ordinal_i64 = i64::try_from(pending.ordinal)
            .map_err(|_| MarketStreamStorageError::SubscriptionInvalid)?;
        if row.credential_slot_id != pending.credential_slot_id
            || row.symbol != pending.symbol
            || row.provider != "kis"
            || row.environment != "live"
            || row.venue != STREAM_VENUE
            || row.tr_id != "H0STCNT0"
            || row.grant_revision != pending.grant_revision
            || row.grant_revision != lease.grant_revision
            || row.epoch != Some(pending.epoch)
            || row.state
                != match pending.operation {
                    SubscriptionOperation::Subscribe => "PENDING_SUBSCRIBE",
                    SubscriptionOperation::Unsubscribe => "PENDING_UNSUBSCRIBE",
                }
            || row.pending_operation.as_deref() != Some(pending.operation.as_str())
            || row.pending_ordinal != Some(ordinal_i64)
            || row.pending_reserved_at != Some(reserved_at)
            || row.pending_deadline != Some(deadline)
            || row.subscription_revision != pending.subscription_revision
            || acked_at > fresh_now
        {
            return Err(MarketStreamStorageError::SubscriptionInvalid);
        }
        match pending.operation {
            SubscriptionOperation::Subscribe if row.desired_reference_count > 0 => {}
            SubscriptionOperation::Unsubscribe if row.desired_reference_count == 0 => {}
            _ => return Err(MarketStreamStorageError::SubscriptionInvalid),
        }
        let (state, acked_at_value) = match pending.operation {
            SubscriptionOperation::Subscribe => ("ACKED", Some(acked_at)),
            SubscriptionOperation::Unsubscribe => ("ABSENT", None),
        };
        let updated: SubscriptionDbRow = sqlx::query_as(
            "UPDATE public.owner_market_stream_subscriptions
                SET state = $3, pending_operation = NULL, pending_ordinal = NULL,
                    pending_reserved_at = NULL, pending_deadline = NULL,
                    acked_at = $4,
                    subscription_revision = pg_catalog.gen_random_uuid(), updated_at = $5
              WHERE credential_slot_id = $1 AND symbol = $2
              RETURNING credential_slot_id, symbol, provider, environment, venue,
                        tr_id, grant_revision, epoch, state, pending_operation,
                        pending_ordinal, pending_reserved_at, pending_deadline,
                        acked_at, desired_reference_count, subscription_revision,
                        updated_at",
        )
        .bind(pending.credential_slot_id)
        .bind(&pending.symbol)
        .bind(state)
        .bind(acked_at_value)
        .bind(fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        notify_changed(&mut tx).await?;
        let proof = if pending.operation == SubscriptionOperation::Subscribe {
            Some(updated.into_proof()?)
        } else {
            None
        };
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(proof)
    }

    /// Publish one bounded latest-value batch.  Every receipt is converted
    /// through `StreamQuote::from_receipt`; no REST reservation or caller
    /// timestamp can enter this transaction.
    pub(super) async fn publish_stream_latest(
        &self,
        context: &StreamPublicationContext,
        observations: &[StreamPublicationObservation],
    ) -> Result<CommitResult, MarketStreamStorageError> {
        self.publish_stream_latest_inner(context, observations, None)
            .await
    }

    /// Runtime-only publication entry. The private day is bound to the
    /// context and revalidated in the same locked transaction as cache writes.
    pub(super) async fn publish_stream_latest_resolved(
        &self,
        context: &StreamPublicationContext,
        day: &ResolvedMarketStreamDay,
        observations: &[StreamPublicationObservation],
    ) -> Result<CommitResult, MarketStreamStorageError> {
        if context.session != day.session()
            || context.epoch.resolved_day_id != Some(day.resolution_id())
        {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        self.publish_stream_latest_inner(context, observations, Some(day))
            .await
    }

    async fn publish_stream_latest_inner(
        &self,
        context: &StreamPublicationContext,
        observations: &[StreamPublicationObservation],
        resolved_day: Option<&ResolvedMarketStreamDay>,
    ) -> Result<CommitResult, MarketStreamStorageError> {
        context.validate()?;
        if observations.is_empty() || observations.len() > STREAM_MAX_IDENTITIES_PER_LEASE {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let mut prepared = Vec::with_capacity(observations.len());
        let mut symbols = BTreeSet::new();
        for observation in observations {
            let quote = StreamQuote::from_receipt(observation.receipt())
                .map_err(|_| MarketStreamStorageError::ReceiptInvalid)?;
            if !symbols.insert(quote.symbol.clone()) {
                return Err(MarketStreamStorageError::ReceiptInvalid);
            }
            let mut matched = false;
            for item in &context.items {
                if item.admission.symbol() == quote.symbol {
                    prepared.push((item, observation.receipt(), quote.clone()));
                    matched = true;
                }
            }
            if !matched {
                return Err(MarketStreamStorageError::ReceiptInvalid);
            }
        }

        let mut tx = self.begin_worker_transaction().await?;
        #[cfg(feature = "market-stream-db-tests")]
        let mut qa_publication = super::market_stream_measurements::begin_publication(
            context.producer.credential_slot_id,
            prepared.len(),
        );
        let locked = lock_publication_inputs(&mut tx, context, true, resolved_day).await?;
        let fresh_now = locked.fresh_now;
        validate_locked_publication_context(&locked, context, fresh_now).await?;

        let mut rows = Vec::with_capacity(prepared.len());
        let mut changed = false;
        for (item, receipt, quote) in prepared {
            let received_at = DateTime::<Utc>::from_timestamp_millis(receipt.received_at_ms())
                .ok_or(MarketStreamStorageError::ReceiptInvalid)?;
            let wall_age = fresh_now.signed_duration_since(received_at);
            if wall_age < chrono::Duration::zero()
                || wall_age > chrono::Duration::seconds(STREAM_PUBLICATION_MAX_AGE.as_secs() as i64)
            {
                return Err(MarketStreamStorageError::PipelineLag);
            }
            let monotonic_age = std::time::Instant::now()
                .checked_duration_since(receipt.received_monotonic())
                .ok_or(MarketStreamStorageError::ReceiptInvalid)?;
            if monotonic_age > STREAM_PUBLICATION_MAX_AGE {
                return Err(MarketStreamStorageError::PipelineLag);
            }
            if receipt.epoch() != context.epoch.epoch
                || receipt.session_date() != yyyymmdd(context.session.session_date)
                || quote.business_date != context.session.session_date
                || receipt.receive_ordinal() == 0
            {
                return Err(MarketStreamStorageError::ReceiptInvalid);
            }
            let prior = locked
                .caches
                .get(&item.admission.membership_id)
                .and_then(Option::as_ref);
            if let Some(prior) = prior {
                if prior.epoch == Some(receipt.epoch())
                    && prior.receive_ordinal
                        == Some(
                            i64::try_from(receipt.receive_ordinal())
                                .map_err(|_| MarketStreamStorageError::ReceiptInvalid)?,
                        )
                {
                    rows.push(prior.clone().into_public()?);
                    continue;
                }
                if prior.epoch == Some(receipt.epoch())
                    && prior.receive_ordinal.is_some_and(|ordinal| {
                        ordinal > i64::try_from(receipt.receive_ordinal()).unwrap_or(i64::MAX)
                    })
                {
                    return Err(MarketStreamStorageError::ReceiptStale);
                }
                if prior.state_version == i64::MAX
                    || (prior.generation_id == item.admission.generation_id
                        && prior.quote_version == i64::MAX)
                {
                    return Err(MarketStreamStorageError::VersionExhausted);
                }
            }
            let row =
                upsert_stream_quote(&mut tx, item, context, &quote, receipt, fresh_now, prior)
                    .await?;
            #[cfg(feature = "market-stream-db-tests")]
            if let Some(trace) = qa_publication.as_mut() {
                trace.row_changed();
            }
            rows.push(row.into_public()?);
            changed = true;
        }
        if changed {
            sqlx::query(
                "UPDATE public.owner_market_stream_producers
                    SET status_code = NULL, status_at = NULL, gap_since = NULL,
                        state_version = state_version + 1,
                        updated_at = $4
                  WHERE credential_slot_id = $1
                    AND holder_id = $2 AND fencing_token = $3
                    AND current_epoch = $5
                    AND (status_code IS NOT NULL OR gap_since IS NOT NULL)",
            )
            .bind(context.producer.credential_slot_id)
            .bind(context.producer.holder_id)
            .bind(
                i64::try_from(context.producer.fencing_token)
                    .map_err(|_| MarketStreamStorageError::InvalidInput)?,
            )
            .bind(fresh_now)
            .bind(context.epoch.epoch)
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
            notify_changed(&mut tx).await?;
        }
        let precommit_now = if let Some(day) = resolved_day {
            #[cfg(test)]
            wait_at_resolved_commit_test_gate(context.epoch.epoch).await?;
            revalidate_resolved_commit_guard(&mut tx, &context.producer, day, context.epoch.epoch)
                .await?
        } else {
            fresh_database_time(&mut tx).await?
        };
        for observation in observations {
            let receipt = observation.receipt();
            let received_at = DateTime::<Utc>::from_timestamp_millis(receipt.received_at_ms())
                .ok_or(MarketStreamStorageError::ReceiptInvalid)?;
            let wall_age = precommit_now.signed_duration_since(received_at);
            let monotonic_age = std::time::Instant::now()
                .checked_duration_since(receipt.received_monotonic())
                .ok_or(MarketStreamStorageError::ReceiptInvalid)?;
            if wall_age < chrono::Duration::zero()
                || wall_age > chrono::Duration::seconds(STREAM_PUBLICATION_MAX_AGE.as_secs() as i64)
                || monotonic_age > STREAM_PUBLICATION_MAX_AGE
            {
                return Err(MarketStreamStorageError::PipelineLag);
            }
        }
        #[cfg(feature = "market-stream-db-tests")]
        if let Some(trace) = qa_publication.as_mut() {
            trace.commit_started();
        }
        match tx.commit().await {
            Ok(()) => {
                #[cfg(feature = "market-stream-db-tests")]
                if let Some(trace) = qa_publication.take() {
                    trace.finish(
                        super::market_stream_measurements::StreamPublicationProbeOutcome::Committed,
                    );
                }
                Ok(CommitResult { changed, rows })
            }
            Err(_) => {
                // A commit error is indeterminate.  Do one read of committed
                // epoch/ordinal/version state before reporting uncertainty;
                // never fan out the in-memory value or blindly retry.
                match self.reread_publication_outcome(context, observations).await {
                    Ok(Some(committed)) => {
                        #[cfg(feature = "market-stream-db-tests")]
                        if let Some(trace) = qa_publication.take() {
                            trace.finish(
                                super::market_stream_measurements::StreamPublicationProbeOutcome::ConfirmedByReread,
                            );
                        }
                        Ok(CommitResult {
                            changed: false,
                            rows: committed,
                        })
                    }
                    Ok(None) | Err(_) => {
                        #[cfg(feature = "market-stream-db-tests")]
                        if let Some(trace) = qa_publication.take() {
                            trace.finish(
                                super::market_stream_measurements::StreamPublicationProbeOutcome::CommitUncertain,
                            );
                        }
                        Err(MarketStreamStorageError::CommitUnknown)
                    }
                }
            }
        }
    }

    /// Persist typed connection/status state without replacing a last-known
    /// quote.  Equal status writes are no-ops and do not advance state_version.
    pub(super) async fn record_stream_status(
        &self,
        context: &StreamPublicationContext,
        status: StreamStatus,
    ) -> Result<CommitResult, MarketStreamStorageError> {
        context.validate()?;
        if status.gap_generation != context.epoch.gap_generation {
            return Err(MarketStreamStorageError::StatusInvalid);
        }
        let mut tx = self.begin_worker_transaction().await?;
        let locked = lock_publication_inputs(&mut tx, context, false, None).await?;
        let fresh_now = locked.fresh_now;
        validate_locked_publication_context(&locked, context, fresh_now).await?;
        let mut rows = Vec::with_capacity(context.items.len());
        let mut changed = false;
        for item in &context.items {
            let prior = locked
                .caches
                .get(&item.admission.membership_id)
                .and_then(Option::as_ref);
            if prior.is_some_and(|row| {
                row.status_code.as_deref() == Some(status.code.as_str())
                    && row.connection_state == status.connection.as_str()
                    && row.market_state == status.market_state.as_str()
                    && row.freshness == status.freshness.as_str()
                    && row.availability == status.availability.as_str()
                    && row.gap_generation
                        == i64::try_from(status.gap_generation).unwrap_or(i64::MAX)
                    && row.gap_since.is_some() == status.gap_open
                    && row.generation_id == item.admission.generation_id
                    && row.session_date == context.session.session_date
            }) {
                rows.push(prior.unwrap().clone().into_public()?);
                continue;
            }
            if prior.is_some_and(|row| {
                row.generation_id == item.admission.generation_id
                    && row.session_date == context.session.session_date
                    && row.state_version == i64::MAX
            }) {
                return Err(MarketStreamStorageError::VersionExhausted);
            }
            let row =
                upsert_stream_status(&mut tx, item, context, status, fresh_now, prior).await?;
            rows.push(row.into_public()?);
            changed = true;
        }
        if changed {
            notify_changed(&mut tx).await?;
        }
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(CommitResult { changed, rows })
    }

    /// Read a consistent, RLS-filtered latest snapshot for one lease.  This
    /// method never renews the lease, calls KIS, or trusts a delivery cursor.
    pub async fn read_stream_snapshot(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
    ) -> Result<StreamSnapshot, MarketStreamStorageError> {
        #[cfg(not(test))]
        let window_contract = load_snapshot_window_contract();
        // Unit/DB tests use only explicit synthetic evidence seams and never
        // read the pinned operational window path.
        #[cfg(test)]
        let window_contract = None;
        self.read_stream_snapshot_inner(owner_user_id, session_hash, lease_id, |date| {
            snapshot_window_for(window_contract.as_ref(), date)
        })
        .await
    }

    /// Read a snapshot against the caller's already validated immutable window.
    /// This path never loads configuration from the filesystem or environment.
    pub async fn read_stream_snapshot_with_window(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
        window_contract: Option<&collectors::intraday_quotes::IntradaySessionWindowContract>,
    ) -> Result<StreamSnapshot, MarketStreamStorageError> {
        self.read_stream_snapshot_inner(owner_user_id, session_hash, lease_id, |date| {
            snapshot_window_for(window_contract, date)
        })
        .await
    }

    #[cfg(test)]
    pub(super) async fn read_stream_snapshot_with_window_fixture(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
        window: Option<SnapshotWindowEvidence>,
    ) -> Result<StreamSnapshot, MarketStreamStorageError> {
        self.read_stream_snapshot_inner(owner_user_id, session_hash, lease_id, move |date| {
            window
                .as_ref()
                .filter(|evidence| evidence.date() == date)
                .cloned()
        })
        .await
    }

    async fn read_stream_snapshot_inner<F>(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
        lease_id: Uuid,
        window_for: F,
    ) -> Result<StreamSnapshot, MarketStreamStorageError>
    where
        F: Fn(NaiveDate) -> Option<SnapshotWindowEvidence>,
    {
        if owner_user_id.is_nil() || lease_id.is_nil() || !canonical_session_hash(session_hash) {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        // A concurrent app lease renewal can invalidate the REPEATABLE READ
        // snapshot before FOR SHARE acquires its row lock (SQLSTATE40001).
        // Nothing has been published or mutated here. Roll back and rebuild
        // actor/session/MVCC context once; never retry another SQL failure or
        // commit ambiguity. The caller's one absolute read deadline still
        // covers both attempts and rollback; no new timer is introduced.
        let mut retried_lease_conflict = false;
        let (mut tx, lease) = loop {
            let mut tx = self
                .begin_actor_snapshot_transaction(owner_user_id, session_hash)
                .await?;
            let lease: Result<Option<LeaseDbRow>, sqlx::Error> = sqlx::query_as(
                "SELECT id, owner_user_id, consumer_id, session_hash, kind, renewal_sequence, state,
                    lease_expires_at, released_at, idempotency_key_sha256, request_sha256
               FROM public.owner_market_stream_leases
              WHERE id = $1 AND owner_user_id = $2 AND session_hash = $3
              FOR SHARE",
            )
            .bind(lease_id)
            .bind(owner_user_id)
            .bind(session_hash)
            .fetch_optional(&mut *tx)
            .await;
            match lease {
                Ok(Some(lease)) => break (tx, lease),
                Ok(None) => return Err(MarketStreamStorageError::LeaseNotFound),
                Err(error) => {
                    let serialization_conflict = matches!(
                        &error,
                        sqlx::Error::Database(database)
                            if database.code().as_deref() == Some("40001")
                    );
                    if retried_lease_conflict || !serialization_conflict {
                        return Err(map_database_error(error));
                    }
                    tx.rollback().await.map_err(map_database_error)?;
                    retried_lease_conflict = true;
                }
            }
        };
        let fresh_now = fresh_database_time(&mut tx).await?;
        if lease.state != "ACTIVE" {
            return Err(MarketStreamStorageError::LeaseReleased);
        }
        if lease.lease_expires_at <= fresh_now {
            return Err(MarketStreamStorageError::LeaseExpired);
        }
        // The app may not SELECT private grants. Cache RLS revalidates the
        // current grant/session/membership, and the immutable composite FK
        // binds grant id, owner, slot and revision (migration 0055). Read only
        // those already authorized cache rows; do not widen app privileges.
        let deliveries: Vec<DeliveryStateDbRow> =
            sqlx::query_as("SELECT * FROM public.owner_market_stream_delivery_state($1, $2, $3)")
                .bind(owner_user_id)
                .bind(session_hash)
                .bind(lease_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if deliveries.is_empty() {
            return Err(MarketStreamStorageError::MembershipNotReady);
        }
        if deliveries.len() > STREAM_MAX_IDENTITIES_PER_LEASE {
            return Err(MarketStreamStorageError::IdentityCapacity);
        }
        let memberships = deliveries
            .iter()
            .map(|delivery| delivery.membership_id)
            .collect::<Vec<_>>();
        let cache_rows: Vec<CacheDbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CACHE_SELECT}
               FROM public.owner_market_stream_cache AS cache
              WHERE cache.owner_user_id = $1
                AND cache.membership_id = ANY($2::uuid[])
              ORDER BY cache.instrument_id, cache.membership_id"
        )))
        .bind(owner_user_id)
        .bind(&memberships)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let mut caches = BTreeMap::new();
        for row in cache_rows {
            if caches.insert(row.membership_id, row).is_some() {
                return Err(MarketStreamStorageError::DatabaseIntegrity);
            }
        }

        let kst_offset = FixedOffset::east_opt(KST_OFFSET_SECONDS)
            .ok_or(MarketStreamStorageError::SessionInvalid)?;
        let current_date = fresh_now.with_timezone(&kst_offset).date_naive();
        let mut rows = Vec::with_capacity(deliveries.len());
        let mut delivery_rows = Vec::with_capacity(deliveries.len());
        for delivery in deliveries {
            let identity = delivery.identity()?;
            let subscription = delivery.subscription()?;
            let producer = delivery.producer()?;
            let matched_cache = match caches.remove(&identity.membership_id) {
                Some(row) if row.matches_identity(&identity) => Some(row),
                Some(_) | None => None,
            };
            let window = producer
                .as_ref()
                .and_then(|producer| producer.session_date)
                .and_then(&window_for);
            let lineage_valid = match producer.as_ref() {
                Some(producer) if producer.session_date == Some(current_date) => {
                    match window.as_ref() {
                        Some(window) => {
                            validate_snapshot_lineage(&mut tx, producer, fresh_now, window).await?
                        }
                        None => false,
                    }
                }
                _ => false,
            };
            let read_evidence = stream_delivery_evidence(
                fresh_now,
                current_date,
                producer.as_ref(),
                window.as_ref(),
                lineage_valid,
                matched_cache.as_ref(),
                &identity,
            );
            let cache_live = matched_cache.as_ref().is_some_and(|row| {
                cache_row_is_live(
                    row,
                    fresh_now,
                    current_date,
                    producer.as_ref(),
                    subscription.as_ref(),
                )
            });
            let committed_cache = matched_cache.map(CacheDbRow::into_public).transpose()?;
            if let Some(cache) = committed_cache.as_ref() {
                rows.push(cache.clone());
            }
            let live = delivery_is_live(
                committed_cache.as_ref(),
                subscription.as_ref(),
                producer.as_ref(),
                fresh_now,
                current_date,
                window.as_ref(),
                lineage_valid,
                cache_live,
            );
            let row_generation = committed_cache
                .as_ref()
                .map(|cache| cache.row_generation)
                .unwrap_or(lease.id);
            let state_version = committed_cache
                .as_ref()
                .map(|cache| cache.state_version)
                .unwrap_or(0);
            delivery_rows.push(StreamDeliveryRow {
                identity,
                row_generation,
                state_version,
                cache: committed_cache,
                subscription,
                producer,
                read_evidence,
                live,
            });
        }
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(StreamSnapshot {
            lease_id,
            lease_expires_at: lease.lease_expires_at,
            rows,
            delivery_rows,
        })
    }

    async fn reread_publication_outcome(
        &self,
        context: &StreamPublicationContext,
        observations: &[StreamPublicationObservation],
    ) -> Result<Option<Vec<StreamCacheRow>>, MarketStreamStorageError> {
        if self.worker_transaction_policy == WorkerTransactionPolicy::Runtime {
            return self
                .reread_publication_outcome_runtime(context, observations)
                .await;
        }
        let mut rows = Vec::with_capacity(context.items.len());
        for item in &context.items {
            let row: Option<CacheDbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {CACHE_SELECT}
                   FROM public.owner_market_stream_cache AS cache
                  WHERE cache.owner_user_id = $1
                    AND cache.membership_id = $2"
            )))
            .bind(item.admission.owner_user_id)
            .bind(item.admission.membership_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_database_error)?;
            let Some(row) = row else {
                return Ok(None);
            };
            for observation in observations {
                if observation.receipt().observation().symbol == item.admission.symbol()
                    && (row.epoch != Some(observation.receipt().epoch())
                        || row.receive_ordinal
                            != Some(
                                i64::try_from(observation.receipt().receive_ordinal())
                                    .map_err(|_| MarketStreamStorageError::ReceiptInvalid)?,
                            ))
                {
                    return Ok(None);
                }
            }
            rows.push(row.into_public()?);
        }
        Ok(Some(rows))
    }

    async fn reread_publication_outcome_runtime(
        &self,
        context: &StreamPublicationContext,
        observations: &[StreamPublicationObservation],
    ) -> Result<Option<Vec<StreamCacheRow>>, MarketStreamStorageError> {
        let mut tx = self.begin_worker_transaction().await?;
        let mut rows = Vec::with_capacity(context.items.len());
        let mut matched = true;
        for item in &context.items {
            let row: Option<CacheDbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {CACHE_SELECT}
                   FROM public.owner_market_stream_cache AS cache
                  WHERE cache.owner_user_id = $1
                    AND cache.membership_id = $2"
            )))
            .bind(item.admission.owner_user_id)
            .bind(item.admission.membership_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_database_error)?;
            let Some(row) = row else {
                matched = false;
                break;
            };
            for observation in observations {
                if observation.receipt().observation().symbol == item.admission.symbol()
                    && (row.epoch != Some(observation.receipt().epoch())
                        || row.receive_ordinal
                            != Some(
                                i64::try_from(observation.receipt().receive_ordinal())
                                    .map_err(|_| MarketStreamStorageError::ReceiptInvalid)?,
                            ))
                {
                    matched = false;
                    break;
                }
            }
            if !matched {
                break;
            }
            rows.push(row.into_public()?);
        }
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(matched.then_some(rows))
    }

    async fn begin_actor_transaction(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
    ) -> Result<Transaction<'_, Postgres>, MarketStreamStorageError> {
        let mut tx = self.pool.begin().await.map_err(map_database_error)?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_user_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
            .bind(session_hash)
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        let valid: bool =
            sqlx::query_scalar("SELECT public.owner_market_stream_session_valid($1, $2)")
                .bind(session_hash)
                .bind(owner_user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !valid {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query("SET LOCAL statement_timeout = '30s'")
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        Ok(tx)
    }

    async fn begin_actor_snapshot_transaction(
        &self,
        owner_user_id: Uuid,
        session_hash: &str,
    ) -> Result<Transaction<'_, Postgres>, MarketStreamStorageError> {
        let mut tx = self.pool.begin().await.map_err(map_database_error)?;
        // The helper and cache rows must observe one actor-authorized MVCC
        // snapshot. This must be the first statement after BEGIN.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_user_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
            .bind(session_hash)
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        let valid: bool =
            sqlx::query_scalar("SELECT public.owner_market_stream_session_valid($1, $2)")
                .bind(session_hash)
                .bind(owner_user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_database_error)?;
        if !valid {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query("SET LOCAL statement_timeout = '30s'")
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        Ok(tx)
    }

    async fn begin_worker_transaction(
        &self,
    ) -> Result<Transaction<'_, Postgres>, MarketStreamStorageError> {
        let mut tx = self.pool.begin().await.map_err(map_database_error)?;
        let (lock_timeout, statement_timeout) = self.worker_transaction_policy.sql_limits();
        sqlx::query(lock_timeout)
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query(statement_timeout)
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
        Ok(tx)
    }
}

async fn validate_snapshot_lineage(
    tx: &mut Transaction<'_, Postgres>,
    producer: &StreamProducerDelivery,
    fresh_now: DateTime<Utc>,
    window: &SnapshotWindowEvidence,
) -> Result<bool, MarketStreamStorageError> {
    let Some(session_date) = producer.session_date else {
        return Ok(false);
    };
    let (Some(proof_id), Some(batch_id), Some(content_hash), Some(window_hash)) = (
        producer.session_proof_id,
        producer.calendar_source_batch_id,
        producer.calendar_content_sha256.as_deref(),
        producer.window_contract_sha256.as_deref(),
    ) else {
        return Ok(false);
    };
    if proof_id.is_nil() || window.date() != session_date || window.contract_sha256() != window_hash
    {
        return Ok(false);
    }
    let Ok(calendar) = IntradaySessionProof::new(
        session_date,
        batch_id,
        content_hash.to_owned(),
        window_hash.to_owned(),
    ) else {
        return Ok(false);
    };
    let canonical = canonical_day_proof_sha256(
        session_date,
        batch_id,
        content_hash,
        window_hash,
        window.open_at(),
        window.close_at(),
    );
    if producer.session_proof_sha256.as_deref() != Some(canonical.as_str()) {
        return Ok(false);
    }

    // A stale/missing proof returns non-live. A SQL error from the validator
    // is isolated so it cannot poison the actor snapshot transaction.
    sqlx::query("SAVEPOINT market_stream_snapshot_lineage")
        .execute(&mut **tx)
        .await
        .map_err(map_database_error)?;
    let valid = validate_session_lineage(tx, &calendar, fresh_now)
        .await
        .is_ok();
    if valid {
        sqlx::query("RELEASE SAVEPOINT market_stream_snapshot_lineage")
            .execute(&mut **tx)
            .await
            .map_err(map_database_error)?;
        Ok(true)
    } else {
        sqlx::query("ROLLBACK TO SAVEPOINT market_stream_snapshot_lineage")
            .execute(&mut **tx)
            .await
            .map_err(map_database_error)?;
        sqlx::query("RELEASE SAVEPOINT market_stream_snapshot_lineage")
            .execute(&mut **tx)
            .await
            .map_err(map_database_error)?;
        Ok(false)
    }
}

fn delivery_is_live(
    _cache: Option<&StreamCacheRow>,
    subscription: Option<&StreamSubscriptionDelivery>,
    producer: Option<&StreamProducerDelivery>,
    fresh_now: DateTime<Utc>,
    current_date: NaiveDate,
    window: Option<&SnapshotWindowEvidence>,
    lineage_valid: bool,
    cache_live: bool,
) -> bool {
    let (Some(subscription), Some(producer), Some(window)) = (subscription, producer, window)
    else {
        return false;
    };
    let heartbeat_age = fresh_now.signed_duration_since(producer.heartbeat_at);
    subscription.state == StreamSubscriptionDeliveryState::Acked
        && producer.current_epoch.is_some()
        && subscription.epoch == producer.current_epoch
        && producer.connection == StreamConnectionState::Connected
        && producer.reason.is_none()
        && producer.session_date == Some(current_date)
        && producer.lease_expires_at > fresh_now + chrono::Duration::seconds(5)
        && producer.heartbeat_at <= fresh_now
        && heartbeat_age <= chrono::Duration::seconds(10)
        && producer.gap_since.is_none()
        && window.date() == current_date
        && window.contains(fresh_now)
        && producer.window_contract_sha256.as_deref() == Some(window.contract_sha256())
        && lineage_valid
        && cache_live
}

fn cache_row_is_live(
    row: &CacheDbRow,
    fresh_now: DateTime<Utc>,
    current_date: NaiveDate,
    producer: Option<&StreamProducerDelivery>,
    subscription: Option<&StreamSubscriptionDelivery>,
) -> bool {
    let (Some(producer), Some(subscription)) = (producer, subscription) else {
        return false;
    };
    let quoted = row.price.is_some()
        && row.quote_version > 0
        && row.epoch.is_some()
        && row.fencing_token.is_some_and(|fence| fence > 0)
        && row.subscription_revision.is_some()
        && row.receive_ordinal.is_some_and(|ordinal| ordinal > 0)
        && row.received_at.is_some()
        && row.committed_at.is_some();
    // Publication keeps its independent three-second commit deadline. Display
    // freshness ages both the original receipt and the provider event at read time.
    let fresh_capture = [row.received_at, row.provider_trade_at]
        .into_iter()
        .all(|captured_at| {
            captured_at.is_some_and(|captured_at| {
                captured_at <= fresh_now
                    && fresh_now.signed_duration_since(captured_at) <= chrono::Duration::seconds(30)
            })
        });
    quoted
        && fresh_capture
        && row.session_date == current_date
        && row.epoch == producer.current_epoch
        && row.gap_since.is_none()
        && row.connection_state == StreamConnectionState::Connected.as_str()
        && row.market_state == StreamMarketState::Open.as_str()
        && row.freshness == StreamFreshness::Recent.as_str()
        && row.availability == StreamAvailability::Live.as_str()
        && row.status_code.is_none()
        && row.session_proof_id == producer.session_proof_id.unwrap_or(Uuid::nil())
        && producer.session_proof_sha256.as_deref() == Some(row.session_proof_sha256.as_str())
        && Some(row.calendar_source_batch_id) == producer.calendar_source_batch_id
        && producer.calendar_content_sha256.as_deref() == Some(row.calendar_content_sha256.as_str())
        && producer.window_contract_sha256.as_deref() == Some(row.window_contract_sha256.as_str())
        && row.gap_generation == i64::try_from(producer.gap_generation).unwrap_or(i64::MAX)
        && subscription.state == StreamSubscriptionDeliveryState::Acked
        && subscription.revision == row.subscription_revision.unwrap_or(Uuid::nil())
        && subscription.epoch == producer.current_epoch
}

fn stream_delivery_evidence(
    fresh_now: DateTime<Utc>,
    current_date: NaiveDate,
    producer: Option<&StreamProducerDelivery>,
    window: Option<&SnapshotWindowEvidence>,
    lineage_valid: bool,
    cache: Option<&CacheDbRow>,
    identity: &StreamIdentity,
) -> Option<StreamDeliveryEvidence> {
    let (Some(producer), Some(window)) = (producer, window) else {
        return None;
    };
    let (Some(calendar_content_sha256), Some(window_contract_sha256)) = (
        producer.calendar_content_sha256.as_ref(),
        producer.window_contract_sha256.as_ref(),
    ) else {
        return None;
    };
    if !lineage_valid
        || producer.session_date != Some(current_date)
        || window.date() != current_date
        || window_contract_sha256.as_str() != window.contract_sha256()
        || producer
            .session_proof_id
            .is_none_or(|proof_id| proof_id.is_nil())
        || producer
            .calendar_source_batch_id
            .is_none_or(|batch_id| batch_id.is_nil())
        || producer.session_proof_sha256.is_none()
    {
        return None;
    }

    let quote_eligible = cache.is_some_and(|cache| {
        snapshot_quote_is_eligible(cache, identity, producer, window, fresh_now, current_date)
    });
    Some(StreamDeliveryEvidence {
        observed_at: fresh_now,
        session_date: current_date,
        calendar_content_sha256: calendar_content_sha256.clone(),
        window_contract_sha256: window_contract_sha256.clone(),
        open_at: window.open_at(),
        close_at: window.close_at(),
        quote_eligible,
    })
}

fn snapshot_quote_is_eligible(
    row: &CacheDbRow,
    identity: &StreamIdentity,
    producer: &StreamProducerDelivery,
    window: &SnapshotWindowEvidence,
    fresh_now: DateTime<Utc>,
    current_date: NaiveDate,
) -> bool {
    let (Some(provider_trade_at), Some(received_at), Some(committed_at)) =
        (row.provider_trade_at, row.received_at, row.committed_at)
    else {
        return false;
    };
    let (Some(business_date), Some(trade_time)) = (row.business_date, row.trade_time) else {
        return false;
    };
    let Some(kst) = FixedOffset::east_opt(KST_OFFSET_SECONDS) else {
        return false;
    };

    let capture_is_current = [provider_trade_at, received_at, committed_at]
        .into_iter()
        .all(|captured_at| captured_at <= fresh_now);
    let capture_is_in_window = [provider_trade_at, received_at, committed_at]
        .into_iter()
        .all(|captured_at| window.contains(captured_at));
    let commit_after_receive = committed_at >= received_at
        && committed_at.signed_duration_since(received_at) <= chrono::Duration::seconds(3);
    let provider_close_to_receive =
        provider_trade_at.signed_duration_since(received_at) <= chrono::Duration::seconds(2);

    row.matches_identity(identity)
        && !row.grant_id.is_nil()
        && !row.credential_slot_id.is_nil()
        && !row.grant_revision.is_nil()
        && row.session_date == current_date
        && row.session_date == window.date()
        && business_date == current_date
        && row.calendar_source == "kis"
        && row.calendar_source_version == "kis-chk-holiday-v1:schema-1"
        && row.source == STREAM_SOURCE
        && row.wire_version == STREAM_WIRE_VERSION
        && row.venue == STREAM_VENUE
        && row.currency == STREAM_CURRENCY
        && row.session_proof_id != Uuid::nil()
        && producer.session_proof_sha256.as_deref() == Some(row.session_proof_sha256.as_str())
        && producer.calendar_source_batch_id == Some(row.calendar_source_batch_id)
        && producer.calendar_content_sha256.as_deref() == Some(row.calendar_content_sha256.as_str())
        && producer.window_contract_sha256.as_deref() == Some(row.window_contract_sha256.as_str())
        && window.contract_sha256() == row.window_contract_sha256
        && row.epoch.is_some_and(|epoch| !epoch.is_nil())
        && row.fencing_token.is_some_and(|fence| fence > 0)
        && row
            .subscription_revision
            .is_some_and(|revision| !revision.is_nil())
        && row.receive_ordinal.is_some_and(|ordinal| ordinal > 0)
        && row.quote_version > 0
        && row.base_price.is_none()
        && row.price.as_deref().is_some_and(|value| !value.is_empty())
        && row
            .change_amount
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        && row
            .change_percent
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        && row
            .direction
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        && row.trade_volume.is_some_and(|volume| volume >= 0)
        && row.cumulative_volume.is_some_and(|volume| volume >= 0)
        && row.halted.is_some()
        && capture_is_current
        && capture_is_in_window
        && commit_after_receive
        && provider_close_to_receive
        && provider_trade_at.with_timezone(&kst).date_naive() == business_date
        && provider_trade_at.with_timezone(&kst).time() == trade_time
}

const CACHE_SELECT: &str = "
    cache.owner_user_id, cache.membership_id, cache.row_generation,
    cache.generation_id, cache.instrument_id, cache.generation,
    cache.grant_id, cache.credential_slot_id, cache.grant_revision,
    cache.source, cache.wire_version, cache.venue, cache.currency,
    cache.session_date, cache.calendar_source, cache.calendar_source_version,
    cache.session_proof_id, cache.session_proof_sha256,
    cache.calendar_source_batch_id, cache.calendar_content_sha256,
    cache.window_contract_sha256, cache.epoch, cache.fencing_token,
    cache.subscription_revision, cache.receive_ordinal,
    cache.price::text AS price, cache.base_price::text AS base_price,
    cache.change_amount::text AS change_amount,
    cache.change_percent::text AS change_percent, cache.direction,
    cache.trade_volume, cache.cumulative_volume, cache.halted,
    cache.business_date, cache.trade_time, cache.provider_trade_at,
    cache.quote_version, cache.state_version, cache.status_code,
    cache.status_at, cache.connection_state, cache.market_state,
    cache.freshness, cache.availability, cache.gap_since,
    cache.gap_generation, cache.received_at, cache.committed_at,
    cache.created_at, cache.updated_at";

#[derive(Debug, FromRow)]
struct LeaseDbRow {
    id: Uuid,
    owner_user_id: Uuid,
    consumer_id: Uuid,
    session_hash: String,
    kind: String,
    renewal_sequence: i64,
    state: String,
    lease_expires_at: DateTime<Utc>,
    released_at: Option<DateTime<Utc>>,
    idempotency_key_sha256: String,
    request_sha256: String,
}

#[derive(Debug, FromRow)]
struct AdmissionDbRow {
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
}

#[derive(Debug, FromRow)]
struct DesiredDbRow {
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    reference_count: i64,
}

#[derive(Debug, FromRow)]
struct GrantDbRow {
    id: Uuid,
    credential_slot_id: Uuid,
    owner_user_id: Uuid,
    grant_revision: Uuid,
}

#[derive(Debug, FromRow)]
struct RuntimeGrantDbRow {
    id: Uuid,
    credential_slot_id: Uuid,
    owner_user_id: Uuid,
    grant_revision: Uuid,
    credential_generation: String,
    network_contract_sha256: String,
    identity_list_sha256: String,
}

#[derive(Debug, FromRow, Clone)]
struct RuntimeProducerDbRow {
    credential_slot_id: Uuid,
    grant_id: Uuid,
    grant_revision: Uuid,
    owner_user_id: Uuid,
    holder_id: Uuid,
    fencing_token: i64,
    current_epoch: Option<Uuid>,
    session_date: Option<NaiveDate>,
    session_proof_id: Option<Uuid>,
    session_proof_sha256: Option<String>,
    calendar_source_batch_id: Option<Uuid>,
    calendar_content_sha256: Option<String>,
    window_contract_sha256: Option<String>,
    lease_expires_at: DateTime<Utc>,
    heartbeat_at: DateTime<Utc>,
    connection_state: String,
    status_code: Option<String>,
    status_at: Option<DateTime<Utc>>,
    gap_since: Option<DateTime<Utc>>,
    session_has_gap: bool,
    gap_generation: i64,
    state_version: i64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct DeliveryStateDbRow {
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    subscription_state: Option<String>,
    subscription_epoch: Option<Uuid>,
    subscription_revision: Option<Uuid>,
    subscription_updated_at: Option<DateTime<Utc>>,
    producer_connection_state: Option<String>,
    producer_status_code: Option<String>,
    producer_status_at: Option<DateTime<Utc>>,
    producer_heartbeat_at: Option<DateTime<Utc>>,
    producer_lease_expires_at: Option<DateTime<Utc>>,
    producer_current_epoch: Option<Uuid>,
    producer_session_date: Option<NaiveDate>,
    producer_session_proof_id: Option<Uuid>,
    producer_session_proof_sha256: Option<String>,
    producer_calendar_source_batch_id: Option<Uuid>,
    producer_calendar_content_sha256: Option<String>,
    producer_window_contract_sha256: Option<String>,
    producer_gap_since: Option<DateTime<Utc>>,
    producer_session_has_gap: Option<bool>,
    producer_gap_generation: Option<i64>,
    producer_state_version: Option<i64>,
}

impl DeliveryStateDbRow {
    fn identity(&self) -> Result<StreamIdentity, MarketStreamStorageError> {
        StreamIdentity::new(
            self.owner_user_id,
            self.membership_id,
            self.generation_id,
            self.instrument_id.clone(),
            u64::try_from(self.generation)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        )
    }

    fn subscription(&self) -> Result<Option<StreamSubscriptionDelivery>, MarketStreamStorageError> {
        match (
            self.subscription_state.as_deref(),
            self.subscription_epoch,
            self.subscription_revision,
            self.subscription_updated_at,
        ) {
            (None, None, None, None) => Ok(None),
            (Some(state), epoch, Some(revision), Some(updated_at)) => {
                Ok(Some(StreamSubscriptionDelivery {
                    state: StreamSubscriptionDeliveryState::parse(state)
                        .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    epoch,
                    revision,
                    updated_at,
                }))
            }
            _ => Err(MarketStreamStorageError::DatabaseIntegrity),
        }
    }

    fn producer(&self) -> Result<Option<StreamProducerDelivery>, MarketStreamStorageError> {
        let Some(connection) = self.producer_connection_state.as_deref() else {
            if self.producer_status_code.is_some()
                || self.producer_status_at.is_some()
                || self.producer_heartbeat_at.is_some()
                || self.producer_lease_expires_at.is_some()
                || self.producer_current_epoch.is_some()
                || self.producer_session_date.is_some()
                || self.producer_session_proof_id.is_some()
                || self.producer_session_proof_sha256.is_some()
                || self.producer_calendar_source_batch_id.is_some()
                || self.producer_calendar_content_sha256.is_some()
                || self.producer_window_contract_sha256.is_some()
                || self.producer_gap_since.is_some()
                || self.producer_session_has_gap.is_some()
                || self.producer_gap_generation.is_some()
                || self.producer_state_version.is_some()
            {
                return Err(MarketStreamStorageError::DatabaseIntegrity);
            }
            return Ok(None);
        };
        if self.producer_status_code.is_some() != self.producer_status_at.is_some() {
            return Err(MarketStreamStorageError::DatabaseIntegrity);
        }
        let heartbeat_at = self
            .producer_heartbeat_at
            .ok_or(MarketStreamStorageError::DatabaseIntegrity)?;
        let lease_expires_at = self
            .producer_lease_expires_at
            .ok_or(MarketStreamStorageError::DatabaseIntegrity)?;
        let session_has_gap = self
            .producer_session_has_gap
            .ok_or(MarketStreamStorageError::DatabaseIntegrity)?;
        let gap_generation = u64::try_from(
            self.producer_gap_generation
                .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
        )
        .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?;
        let state_version = u64::try_from(
            self.producer_state_version
                .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
        )
        .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?;
        let reason = match self.producer_status_code.as_deref() {
            Some(value) => Some(
                StreamStatusCode::parse(value)
                    .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
            ),
            None => None,
        };
        Ok(Some(StreamProducerDelivery {
            connection: StreamConnectionState::parse(connection)
                .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
            reason,
            status_at: self.producer_status_at,
            heartbeat_at,
            lease_expires_at,
            current_epoch: self.producer_current_epoch,
            session_date: self.producer_session_date,
            session_proof_id: self.producer_session_proof_id,
            session_proof_sha256: self.producer_session_proof_sha256.clone(),
            calendar_source_batch_id: self.producer_calendar_source_batch_id,
            calendar_content_sha256: self.producer_calendar_content_sha256.clone(),
            window_contract_sha256: self.producer_window_contract_sha256.clone(),
            gap_since: self.producer_gap_since,
            session_has_gap,
            gap_generation,
            state_version,
        }))
    }
}

impl RuntimeProducerDbRow {
    fn matches_lease(&self, lease: &StreamProducerLease) -> bool {
        self.credential_slot_id == lease.credential_slot_id
            && self.grant_id == lease.grant_id
            && self.grant_revision == lease.grant_revision
            && self.owner_user_id == lease.owner_user_id
            && self.holder_id == lease.holder_id
            && self.fencing_token == i64::try_from(lease.fencing_token).unwrap_or(-1)
    }

    fn status_commit(
        &self,
        changed: bool,
    ) -> Result<RuntimeStatusCommit, MarketStreamStorageError> {
        Ok(RuntimeStatusCommit {
            changed,
            current_epoch: self.current_epoch,
            gap_generation: u64::try_from(self.gap_generation)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
            state_version: u64::try_from(self.state_version)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        })
    }
}

#[derive(Debug, FromRow)]
struct ProducerDbRow {
    credential_slot_id: Uuid,
    grant_id: Uuid,
    grant_revision: Uuid,
    owner_user_id: Uuid,
    holder_id: Uuid,
    fencing_token: i64,
    current_epoch: Option<Uuid>,
    session_date: Option<NaiveDate>,
    session_proof_id: Option<Uuid>,
    session_proof_sha256: Option<String>,
    calendar_source: Option<String>,
    calendar_source_version: Option<String>,
    calendar_source_batch_id: Option<Uuid>,
    calendar_content_sha256: Option<String>,
    window_contract_sha256: Option<String>,
    lease_expires_at: DateTime<Utc>,
    heartbeat_at: DateTime<Utc>,
    connection_state: String,
    gap_generation: i64,
    state_version: i64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct SubscriptionDbRow {
    credential_slot_id: Uuid,
    symbol: String,
    provider: String,
    environment: String,
    venue: String,
    tr_id: String,
    grant_revision: Uuid,
    epoch: Option<Uuid>,
    state: String,
    pending_operation: Option<String>,
    pending_ordinal: Option<i64>,
    pending_reserved_at: Option<DateTime<Utc>>,
    pending_deadline: Option<DateTime<Utc>>,
    acked_at: Option<DateTime<Utc>>,
    desired_reference_count: i32,
    subscription_revision: Uuid,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct LeaseItemLockDbRow {
    lease_id: Uuid,
    owner_user_id: Uuid,
    session_hash: String,
    lease_expires_at: DateTime<Utc>,
    state: String,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
}

#[derive(Debug, FromRow, Clone)]
struct CacheDbRow {
    owner_user_id: Uuid,
    membership_id: Uuid,
    row_generation: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    grant_id: Uuid,
    credential_slot_id: Uuid,
    grant_revision: Uuid,
    source: String,
    wire_version: String,
    venue: String,
    currency: String,
    session_date: NaiveDate,
    calendar_source: String,
    calendar_source_version: String,
    session_proof_id: Uuid,
    session_proof_sha256: String,
    calendar_source_batch_id: Uuid,
    calendar_content_sha256: String,
    window_contract_sha256: String,
    epoch: Option<Uuid>,
    fencing_token: Option<i64>,
    subscription_revision: Option<Uuid>,
    receive_ordinal: Option<i64>,
    price: Option<String>,
    base_price: Option<String>,
    change_amount: Option<String>,
    change_percent: Option<String>,
    direction: Option<String>,
    trade_volume: Option<i64>,
    cumulative_volume: Option<i64>,
    halted: Option<bool>,
    business_date: Option<NaiveDate>,
    trade_time: Option<NaiveTime>,
    provider_trade_at: Option<DateTime<Utc>>,
    quote_version: i64,
    state_version: i64,
    status_code: Option<String>,
    status_at: Option<DateTime<Utc>>,
    connection_state: String,
    market_state: String,
    freshness: String,
    availability: String,
    gap_since: Option<DateTime<Utc>>,
    gap_generation: i64,
    received_at: Option<DateTime<Utc>>,
    committed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl AdmissionDbRow {
    fn into_identity(self) -> Result<StreamIdentity, MarketStreamStorageError> {
        StreamIdentity::new(
            self.owner_user_id,
            self.membership_id,
            self.generation_id,
            self.instrument_id,
            u64::try_from(self.generation)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        )
    }
}

impl DesiredDbRow {
    fn into_item(self) -> Result<DesiredStreamItem, MarketStreamStorageError> {
        if self.reference_count <= 0 {
            return Err(MarketStreamStorageError::DatabaseIntegrity);
        }
        Ok(DesiredStreamItem {
            identity: StreamIdentity::new(
                self.owner_user_id,
                self.membership_id,
                self.generation_id,
                self.instrument_id,
                u64::try_from(self.generation)
                    .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
            )?,
            reference_count: u32::try_from(self.reference_count)
                .map_err(|_| MarketStreamStorageError::IdentityCapacity)?,
        })
    }
}

impl ProducerDbRow {
    fn into_lease(self) -> Result<StreamProducerLease, MarketStreamStorageError> {
        Ok(StreamProducerLease {
            credential_slot_id: self.credential_slot_id,
            grant_id: self.grant_id,
            grant_revision: self.grant_revision,
            owner_user_id: self.owner_user_id,
            holder_id: self.holder_id,
            fencing_token: u64::try_from(self.fencing_token)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
            gap_generation: u64::try_from(self.gap_generation)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
            lease_expires_at: self.lease_expires_at,
            heartbeat_at: self.heartbeat_at,
        })
    }

    fn matches_lease(&self, lease: &StreamProducerLease) -> bool {
        self.credential_slot_id == lease.credential_slot_id
            && self.grant_id == lease.grant_id
            && self.grant_revision == lease.grant_revision
            && self.owner_user_id == lease.owner_user_id
            && self.holder_id == lease.holder_id
            && self.fencing_token == i64::try_from(lease.fencing_token).unwrap_or(-1)
    }
}

impl SubscriptionDbRow {
    fn into_proof(self) -> Result<SubscriptionProof, MarketStreamStorageError> {
        if self.provider != "kis"
            || self.environment != "live"
            || self.venue != STREAM_VENUE
            || self.tr_id != "H0STCNT0"
            || !is_six_ascii_digits(&self.symbol)
        {
            return Err(MarketStreamStorageError::DatabaseIntegrity);
        }
        Ok(SubscriptionProof {
            credential_slot_id: self.credential_slot_id,
            symbol: self.symbol,
            grant_revision: self.grant_revision,
            epoch: self
                .epoch
                .ok_or(MarketStreamStorageError::SubscriptionInvalid)?,
            state: self.state,
            subscription_revision: self.subscription_revision,
            acked_at: self.acked_at,
            desired_reference_count: u32::try_from(self.desired_reference_count)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        })
    }
}

impl CacheDbRow {
    fn matches_identity(&self, identity: &StreamIdentity) -> bool {
        self.owner_user_id == identity.owner_user_id
            && self.membership_id == identity.membership_id
            && self.generation_id == identity.generation_id
            && self.instrument_id == identity.instrument_id
            && self.generation == i64::try_from(identity.generation).unwrap_or(-1)
    }

    fn into_public(self) -> Result<StreamCacheRow, MarketStreamStorageError> {
        if self.source != STREAM_SOURCE
            || self.wire_version != STREAM_WIRE_VERSION
            || self.venue != STREAM_VENUE
            || self.currency != STREAM_CURRENCY
        {
            return Err(MarketStreamStorageError::DatabaseIntegrity);
        }
        let identity = StreamIdentity::new(
            self.owner_user_id,
            self.membership_id,
            self.generation_id,
            self.instrument_id.clone(),
            u64::try_from(self.generation)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        )?;
        let quote = match (
            self.price,
            self.change_amount,
            self.change_percent,
            self.direction,
            self.trade_volume,
            self.cumulative_volume,
            self.halted,
            self.business_date,
            self.trade_time,
            self.provider_trade_at,
        ) {
            (None, None, None, None, None, None, None, None, None, None) => None,
            (
                Some(price),
                Some(change_amount),
                Some(change_percent),
                Some(direction),
                Some(trade_volume),
                Some(cumulative_volume),
                Some(halted),
                Some(business_date),
                Some(trade_time),
                Some(provider_trade_at),
            ) => {
                let direction = match direction.as_str() {
                    "UP" => StreamQuoteDirection::Up,
                    "DOWN" => StreamQuoteDirection::Down,
                    "FLAT" => StreamQuoteDirection::Flat,
                    "LIMIT_UP" => StreamQuoteDirection::LimitUp,
                    "LIMIT_DOWN" => StreamQuoteDirection::LimitDown,
                    _ => return Err(MarketStreamStorageError::DatabaseIntegrity),
                };
                Some(StreamQuote {
                    symbol: identity.symbol().to_owned(),
                    business_date,
                    trade_time,
                    provider_trade_at: provider_trade_at.with_timezone(
                        &FixedOffset::east_opt(KST_OFFSET_SECONDS)
                            .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    ),
                    price,
                    change_from_previous_day: change_amount,
                    change_percent_from_previous_day: change_percent,
                    direction,
                    trade_volume: u64::try_from(trade_volume)
                        .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
                    cumulative_volume: u64::try_from(cumulative_volume)
                        .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
                    base_price: None,
                    base_price_reason: StreamBasePriceReason::NotProvidedByChannel,
                    halted,
                    opening_class: "20".to_owned(),
                    hour_class: "0".to_owned(),
                    market_class: "2".to_owned(),
                    transaction_class: String::new(),
                })
            }
            _ => return Err(MarketStreamStorageError::DatabaseIntegrity),
        };
        let status = self
            .status_code
            .as_deref()
            .map(|code| {
                Ok(StreamStatus::new(
                    StreamStatusCode::parse(code)
                        .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    StreamConnectionState::parse(&self.connection_state)
                        .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    StreamMarketState::parse(&self.market_state)
                        .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    StreamFreshness::parse(&self.freshness)
                        .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    StreamAvailability::parse(&self.availability)
                        .ok_or(MarketStreamStorageError::DatabaseIntegrity)?,
                    self.gap_since.is_some(),
                    u64::try_from(self.gap_generation)
                        .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
                )?)
            })
            .transpose()?;
        Ok(StreamCacheRow {
            identity,
            row_generation: self.row_generation,
            quote,
            quote_version: u64::try_from(self.quote_version)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
            state_version: u64::try_from(self.state_version)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
            status,
            epoch: self.epoch,
            receive_ordinal: self
                .receive_ordinal
                .map(|value| {
                    u64::try_from(value).map_err(|_| MarketStreamStorageError::DatabaseIntegrity)
                })
                .transpose()?,
            received_at: self.received_at,
            committed_at: self.committed_at,
            gap_open: self.gap_since.is_some(),
            gap_generation: u64::try_from(self.gap_generation)
                .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        })
    }
}

async fn lock_owner_capacity(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
) -> Result<(), MarketStreamStorageError> {
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_xact_lock(
            pg_catalog.hashtextextended($1, 0)
        )",
    )
    .bind(format!("owner-market-stream-capacity:{owner_user_id}"))
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let policy_exists: Option<Uuid> = sqlx::query_scalar(
        "SELECT owner_user_id
           FROM public.owner_equity_universe_policies
          WHERE owner_user_id = $1",
    )
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if policy_exists != Some(owner_user_id) {
        return Err(MarketStreamStorageError::DatabaseIntegrity);
    }
    Ok(())
}

async fn lock_producer_slot(
    tx: &mut Transaction<'_, Postgres>,
    credential_slot_id: Uuid,
) -> Result<(), MarketStreamStorageError> {
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_xact_lock(
            pg_catalog.hashtextextended($1, 0)
        )",
    )
    .bind(format!("owner-market-stream-producer:{credential_slot_id}"))
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    Ok(())
}

async fn ensure_owner_identity_capacity(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
    fresh_now: DateTime<Utc>,
) -> Result<(), MarketStreamStorageError> {
    let distinct_symbols: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT item.instrument_id)::bigint
           FROM public.owner_market_stream_leases AS lease
           JOIN public.owner_market_stream_lease_items AS item
             ON item.lease_id = lease.id
            AND item.owner_user_id = lease.owner_user_id
           JOIN public.owner_equity_memberships AS membership
             ON membership.id = item.membership_id
            AND membership.owner_user_id = item.owner_user_id
            AND membership.instrument_id = item.instrument_id
            AND membership.state = 'READY'
           JOIN public.owner_equity_generation_admissions AS admission
             ON admission.generation_id = item.generation_id
            AND admission.owner_user_id = item.owner_user_id
            AND admission.membership_id = item.membership_id
            AND admission.instrument_id = item.instrument_id
            AND admission.generation = item.generation
          WHERE lease.owner_user_id = $1
            AND lease.state = 'ACTIVE'
            AND lease.lease_expires_at > $2
            AND public.owner_market_stream_session_valid(
                lease.session_hash, lease.owner_user_id
            )
            AND NOT EXISTS (
                SELECT 1
                  FROM public.owner_equity_generation_admissions AS newer
                 WHERE newer.owner_user_id = admission.owner_user_id
                   AND newer.membership_id = admission.membership_id
                   AND newer.instrument_id = admission.instrument_id
                   AND newer.generation > admission.generation
            )",
    )
    .bind(owner_user_id)
    .bind(fresh_now)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if distinct_symbols > STREAM_MAX_ACTIVE_IDENTITIES as i64 {
        return Err(MarketStreamStorageError::IdentityCapacity);
    }
    Ok(())
}

async fn prune_stream_cache(
    tx: &mut Transaction<'_, Postgres>,
    credential_slot_id: Uuid,
) -> Result<bool, MarketStreamStorageError> {
    let result = sqlx::query(
        "DELETE FROM public.owner_market_stream_cache
          WHERE credential_slot_id = $1
            AND COALESCE(received_at, status_at, updated_at)
                < pg_catalog.clock_timestamp() - INTERVAL '24 hours'",
    )
    .bind(credential_slot_id)
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    Ok(result.rows_affected() > 0)
}

async fn lock_requested_admissions(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
    session_hash: &str,
    request: &StreamLeaseRequest,
) -> Result<Vec<AdmissionDbRow>, MarketStreamStorageError> {
    let mut requested_identities: Vec<_> = request.identities.iter().collect();
    requested_identities.sort_by_key(|identity| identity.membership_id);
    let mut admissions = Vec::with_capacity(requested_identities.len());
    for requested in requested_identities {
        let row: Option<AdmissionDbRow> = sqlx::query_as(
            "SELECT owner_user_id, membership_id, generation_id,
                    instrument_id, generation
               FROM public.lock_owner_market_stream_app_admission($1, $2, $3, $4, $5)",
        )
        .bind(owner_user_id)
        .bind(session_hash)
        .bind(requested.membership_id)
        .bind(&requested.instrument_id)
        .bind(requested.generation_i64())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_database_error)?;
        admissions.push(row.ok_or(MarketStreamStorageError::MembershipNotReady)?);
    }
    Ok(admissions)
}

async fn insert_lease_items(
    tx: &mut Transaction<'_, Postgres>,
    lease: &LeaseDbRow,
    admissions: &[AdmissionDbRow],
) -> Result<(), MarketStreamStorageError> {
    for admission in admissions {
        sqlx::query(
            "INSERT INTO public.owner_market_stream_lease_items
                (lease_id, owner_user_id, membership_id, generation_id,
                 instrument_id, generation)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(lease.id)
        .bind(lease.owner_user_id)
        .bind(admission.membership_id)
        .bind(admission.generation_id)
        .bind(&admission.instrument_id)
        .bind(admission.generation)
        .execute(&mut **tx)
        .await
        .map_err(map_database_error)?;
    }
    Ok(())
}

async fn replace_lease_items(
    tx: &mut Transaction<'_, Postgres>,
    lease: &LeaseDbRow,
    admissions: &[AdmissionDbRow],
) -> Result<(), MarketStreamStorageError> {
    sqlx::query(
        "DELETE FROM public.owner_market_stream_lease_items
          WHERE lease_id = $1 AND owner_user_id = $2",
    )
    .bind(lease.id)
    .bind(lease.owner_user_id)
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    insert_lease_items(tx, lease, admissions).await
}

async fn lock_existing_stream_lease_items(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
    session_hash: &str,
    lease_id: Uuid,
) -> Result<(), MarketStreamStorageError> {
    let locked_count: i32 =
        sqlx::query_scalar("SELECT public.lock_owner_market_stream_app_lease_items($1, $2, $3)")
            .bind(owner_user_id)
            .bind(session_hash)
            .bind(lease_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_database_error)?;
    if !(1..=STREAM_MAX_IDENTITIES_PER_LEASE as i32).contains(&locked_count) {
        return Err(MarketStreamStorageError::DatabaseIntegrity);
    }
    Ok(())
}

async fn load_stream_lease(
    tx: &mut Transaction<'_, Postgres>,
    lease_id: Uuid,
    owner_user_id: Uuid,
) -> Result<StreamLease, MarketStreamStorageError> {
    let row: LeaseDbRow = sqlx::query_as(
        "SELECT id, owner_user_id, consumer_id, session_hash, kind,
                renewal_sequence, state, lease_expires_at, released_at,
                idempotency_key_sha256, request_sha256
           FROM public.owner_market_stream_leases
          WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(lease_id)
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?
    .ok_or(MarketStreamStorageError::LeaseNotFound)?;
    if row.kind != "BROWSER" || row.state != "ACTIVE" {
        return Err(MarketStreamStorageError::DatabaseIntegrity);
    }
    let item_rows: Vec<AdmissionDbRow> = sqlx::query_as(
        "SELECT item.owner_user_id, item.membership_id, item.generation_id,
                item.instrument_id, item.generation
           FROM public.owner_market_stream_lease_items AS item
          WHERE item.lease_id = $1 AND item.owner_user_id = $2
          ORDER BY item.membership_id",
    )
    .bind(lease_id)
    .bind(owner_user_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let identities = item_rows
        .into_iter()
        .map(AdmissionDbRow::into_identity)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StreamLease {
        lease_id: row.id,
        owner_user_id: row.owner_user_id,
        consumer_id: row.consumer_id,
        renewal_sequence: u64::try_from(row.renewal_sequence)
            .map_err(|_| MarketStreamStorageError::DatabaseIntegrity)?,
        lease_expires_at: row.lease_expires_at,
        renew_after_ms: STREAM_RENEW_AFTER_MS,
        identities,
    })
}

async fn load_producer_for_update(
    tx: &mut Transaction<'_, Postgres>,
    credential_slot_id: Uuid,
) -> Result<ProducerDbRow, MarketStreamStorageError> {
    sqlx::query_as(
        "SELECT credential_slot_id, grant_id, grant_revision, owner_user_id,
                holder_id, fencing_token, current_epoch, session_date,
                session_proof_id, session_proof_sha256, calendar_source,
                calendar_source_version, calendar_source_batch_id,
                calendar_content_sha256, window_contract_sha256,
                lease_expires_at, heartbeat_at, connection_state,
                gap_generation, state_version, updated_at
           FROM public.owner_market_stream_producers
          WHERE credential_slot_id = $1
          FOR UPDATE",
    )
    .bind(credential_slot_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?
    .ok_or(MarketStreamStorageError::ProducerLost)
}

async fn load_runtime_producer_for_update(
    tx: &mut Transaction<'_, Postgres>,
    credential_slot_id: Uuid,
) -> Result<RuntimeProducerDbRow, MarketStreamStorageError> {
    sqlx::query_as(
        "SELECT credential_slot_id, grant_id, grant_revision, owner_user_id,
                holder_id, fencing_token, current_epoch, session_date,
                session_proof_id, session_proof_sha256,
                calendar_source_batch_id, calendar_content_sha256,
                window_contract_sha256, lease_expires_at, heartbeat_at,
                connection_state, status_code, status_at, gap_since,
                session_has_gap, gap_generation, state_version, updated_at
           FROM public.owner_market_stream_producers
          WHERE credential_slot_id = $1
          FOR UPDATE",
    )
    .bind(credential_slot_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?
    .ok_or(MarketStreamStorageError::ProducerLost)
}

struct RuntimeDemandLocks {
    identities: Vec<StreamIdentity>,
}

/// Lock in the shared producer -> rights -> owner capacity -> admission ->
/// sessions -> demand -> subscriptions -> cache order. The caller already
/// holds the producer row and the rights/capacity locks.
async fn lock_runtime_demand_inputs(
    tx: &mut Transaction<'_, Postgres>,
    producer: &RuntimeProducerDbRow,
    lock_all_subscriptions: bool,
) -> Result<RuntimeDemandLocks, MarketStreamStorageError> {
    let now = fresh_database_time(tx).await?;
    let demand_rows: Vec<DesiredDbRow> = sqlx::query_as(
        "SELECT item.owner_user_id, item.membership_id, item.generation_id,
                item.instrument_id, item.generation,
                count(*)::bigint AS reference_count
           FROM public.owner_market_stream_leases AS lease
           JOIN public.owner_market_stream_lease_items AS item
             ON item.lease_id = lease.id AND item.owner_user_id = lease.owner_user_id
           JOIN public.owner_equity_memberships AS membership
             ON membership.owner_user_id = item.owner_user_id
            AND membership.id = item.membership_id
            AND membership.instrument_id = item.instrument_id
            AND membership.state = 'READY'
           JOIN public.owner_equity_generation_admissions AS admission
             ON admission.generation_id = item.generation_id
            AND admission.owner_user_id = item.owner_user_id
            AND admission.membership_id = item.membership_id
            AND admission.instrument_id = item.instrument_id
            AND admission.generation = item.generation
          WHERE item.owner_user_id = $1
            AND lease.state = 'ACTIVE'
            AND lease.lease_expires_at > $2
            AND public.owner_market_stream_session_valid(lease.session_hash, lease.owner_user_id)
            AND NOT EXISTS (
                SELECT 1 FROM public.owner_equity_generation_admissions AS newer
                 WHERE newer.owner_user_id = admission.owner_user_id
                   AND newer.membership_id = admission.membership_id
                   AND newer.instrument_id = admission.instrument_id
                   AND newer.generation > admission.generation
            )
          GROUP BY item.owner_user_id, item.membership_id, item.generation_id,
                   item.instrument_id, item.generation
          ORDER BY item.membership_id",
    )
    .bind(producer.owner_user_id)
    .bind(now)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if demand_rows.len() > STREAM_MAX_ACTIVE_IDENTITIES {
        return Err(MarketStreamStorageError::IdentityCapacity);
    }
    let identities = demand_rows
        .into_iter()
        .map(DesiredDbRow::into_item)
        .map(|row| row.map(|item| item.identity))
        .collect::<Result<Vec<_>, _>>()?;

    for identity in &identities {
        let locked: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_worker_admission(
                $1, $2, $3, $4, $5, $6, $7, $8
            )",
        )
        .bind(producer.credential_slot_id)
        .bind(producer.grant_id)
        .bind(producer.grant_revision)
        .bind(identity.owner_user_id)
        .bind(identity.membership_id)
        .bind(identity.generation_id)
        .bind(&identity.instrument_id)
        .bind(identity.generation_i64())
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)?;
        if !locked {
            return Err(MarketStreamStorageError::MembershipNotReady);
        }
    }

    if !identities.is_empty() {
        let membership_ids = identities
            .iter()
            .map(|identity| identity.membership_id)
            .collect::<Vec<_>>();
        let sessions: Vec<(String, Uuid)> = sqlx::query_as(
            "SELECT DISTINCT lease.session_hash, lease.owner_user_id
               FROM public.owner_market_stream_leases AS lease
               JOIN public.owner_market_stream_lease_items AS item
                 ON item.lease_id = lease.id AND item.owner_user_id = lease.owner_user_id
              WHERE lease.owner_user_id = $1
                AND lease.state = 'ACTIVE'
                AND lease.lease_expires_at > pg_catalog.clock_timestamp()
                AND item.membership_id = ANY($2::uuid[])
              ORDER BY lease.session_hash, lease.owner_user_id",
        )
        .bind(producer.owner_user_id)
        .bind(&membership_ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(map_database_error)?;
        for (session_hash, owner_user_id) in sessions {
            let locked: bool = sqlx::query_scalar(
                "SELECT public.lock_owner_market_stream_session($1, $2, $3, $4, $5)",
            )
            .bind(producer.credential_slot_id)
            .bind(producer.grant_id)
            .bind(producer.grant_revision)
            .bind(owner_user_id)
            .bind(&session_hash)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_database_error)?;
            if !locked {
                return Err(MarketStreamStorageError::SessionInvalid);
            }
        }
        let rows: Vec<LeaseItemLockDbRow> = sqlx::query_as(
            "SELECT * FROM public.lock_owner_market_stream_worker_demand($1, $2, $3, $4, $5)",
        )
        .bind(producer.credential_slot_id)
        .bind(producer.grant_id)
        .bind(producer.grant_revision)
        .bind(producer.owner_user_id)
        .bind(&membership_ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(map_database_error)?;
        if rows.iter().any(|row| {
            !identities.iter().any(|identity| {
                identity.membership_id == row.membership_id
                    && identity.generation_id == row.generation_id
                    && identity.instrument_id == row.instrument_id
                    && identity.generation == u64::try_from(row.generation).unwrap_or(0)
            })
        }) || identities.iter().any(|identity| {
            !rows.iter().any(|row| {
                identity.membership_id == row.membership_id
                    && identity.generation_id == row.generation_id
                    && identity.instrument_id == row.instrument_id
                    && identity.generation == u64::try_from(row.generation).unwrap_or(0)
            })
        }) {
            return Err(MarketStreamStorageError::DatabaseIntegrity);
        }

        if !lock_all_subscriptions {
            let symbols = identities
                .iter()
                .map(|identity| identity.symbol().to_owned())
                .collect::<BTreeSet<_>>();
            for symbol in symbols {
                let _: Option<SubscriptionDbRow> = sqlx::query_as(
                    "SELECT credential_slot_id, symbol, provider, environment, venue,
                            tr_id, grant_revision, epoch, state, pending_operation,
                            pending_ordinal, pending_reserved_at, pending_deadline,
                            acked_at, desired_reference_count, subscription_revision, updated_at
                       FROM public.owner_market_stream_subscriptions
                      WHERE credential_slot_id = $1 AND symbol = $2
                      FOR UPDATE",
                )
                .bind(producer.credential_slot_id)
                .bind(symbol)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_database_error)?;
            }
        }
    }
    if lock_all_subscriptions {
        let _: Vec<(String,)> = sqlx::query_as(
            "SELECT symbol FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 ORDER BY symbol FOR UPDATE",
        )
        .bind(producer.credential_slot_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(map_database_error)?;
    }
    for identity in &identities {
        let _: Option<Uuid> = sqlx::query_scalar(
            "SELECT row_generation FROM public.owner_market_stream_cache
              WHERE owner_user_id = $1 AND membership_id = $2
              FOR UPDATE",
        )
        .bind(identity.owner_user_id)
        .bind(identity.membership_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_database_error)?;
    }
    Ok(RuntimeDemandLocks { identities })
}

async fn update_runtime_cache_status(
    tx: &mut Transaction<'_, Postgres>,
    identity: &StreamIdentity,
    before: &RuntimeProducerDbRow,
    after: &RuntimeProducerDbRow,
    values: RuntimeTransitionValues,
    now: DateTime<Utc>,
) -> Result<(), MarketStreamStorageError> {
    let Some(session_date) = before.session_date else {
        return Ok(());
    };
    if after.state_version <= 0 {
        return Err(MarketStreamStorageError::DatabaseIntegrity);
    }
    let state_version: Option<i64> = sqlx::query_scalar(
        "SELECT state_version FROM public.owner_market_stream_cache
          WHERE owner_user_id = $1 AND membership_id = $2
            AND generation_id = $3 AND instrument_id = $4 AND generation = $5
            AND grant_id = $6 AND credential_slot_id = $7
            AND grant_revision = $8 AND session_date = $9
          FOR UPDATE",
    )
    .bind(identity.owner_user_id)
    .bind(identity.membership_id)
    .bind(identity.generation_id)
    .bind(&identity.instrument_id)
    .bind(identity.generation_i64())
    .bind(before.grant_id)
    .bind(before.credential_slot_id)
    .bind(before.grant_revision)
    .bind(session_date)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let Some(state_version) = state_version else {
        return Ok(());
    };
    if state_version == i64::MAX {
        return Err(MarketStreamStorageError::VersionExhausted);
    }
    let updated = sqlx::query(
        "UPDATE public.owner_market_stream_cache
            SET status_code = $6, status_at = $7,
                connection_state = $8, market_state = $9,
                freshness = $10, availability = $11,
                gap_since = CASE WHEN $12 THEN COALESCE(gap_since, $7)
                                 ELSE gap_since END,
                gap_generation = $13, state_version = state_version + 1,
                updated_at = $7
          WHERE owner_user_id = $1 AND membership_id = $2
            AND generation_id = $3 AND instrument_id = $4 AND generation = $5
            AND grant_id = $14 AND credential_slot_id = $15
            AND grant_revision = $16 AND session_date = $17
        ",
    )
    .bind(identity.owner_user_id)
    .bind(identity.membership_id)
    .bind(identity.generation_id)
    .bind(&identity.instrument_id)
    .bind(identity.generation_i64())
    .bind(values.code.as_str())
    .bind(now)
    .bind(values.connection.as_str())
    .bind(values.market_state.as_str())
    .bind(values.freshness.as_str())
    .bind(values.availability.as_str())
    .bind(after.gap_since.is_some())
    .bind(after.gap_generation)
    .bind(before.grant_id)
    .bind(before.credential_slot_id)
    .bind(before.grant_revision)
    .bind(session_date)
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if updated.rows_affected() != 1 {
        return Err(MarketStreamStorageError::DatabaseIntegrity);
    }
    Ok(())
}

fn runtime_retirement_values(reason: StreamStatusCode) -> RuntimeTransitionValues {
    use StreamAvailability as A;
    use StreamConnectionState as C;
    use StreamFreshness as F;
    use StreamMarketState as M;
    let opens_gap = matches!(
        reason,
        StreamStatusCode::ConnectionLost
            | StreamStatusCode::ReconnectGap
            | StreamStatusCode::SubscriptionAmbiguous
            | StreamStatusCode::PipelineLag
            | StreamStatusCode::WireSchemaMismatch
            | StreamStatusCode::ProviderResponseInvalid
            | StreamStatusCode::MarketClassUnsupported
            | StreamStatusCode::LocalIngressLimit
            | StreamStatusCode::ResyncRequired
    );
    RuntimeTransitionValues {
        code: reason,
        connection: C::Stopped,
        market_state: M::Unknown,
        freshness: if opens_gap { F::Stale } else { F::Unavailable },
        availability: if opens_gap {
            A::LastKnown
        } else {
            A::Unavailable
        },
        opens_gap,
    }
}

async fn fresh_database_time(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<DateTime<Utc>, MarketStreamStorageError> {
    sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)
}

async fn kst_database_date(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<NaiveDate, MarketStreamStorageError> {
    sqlx::query_scalar("SELECT (pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)
}

async fn notify_changed(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<(), MarketStreamStorageError> {
    sqlx::query("SELECT pg_catalog.pg_notify($1, $2)")
        .bind(NOTIFY_CHANNEL)
        .bind(NOTIFY_PAYLOAD)
        .execute(&mut **tx)
        .await
        .map_err(map_database_error)?;
    Ok(())
}

struct LockedPublication {
    producer: ProducerDbRow,
    admissions: BTreeMap<Uuid, AdmissionDbRow>,
    leases: Vec<LeaseItemLockDbRow>,
    subscriptions: BTreeMap<String, SubscriptionDbRow>,
    caches: BTreeMap<Uuid, Option<CacheDbRow>>,
    fresh_now: DateTime<Utc>,
}

async fn lock_publication_inputs(
    tx: &mut Transaction<'_, Postgres>,
    context: &StreamPublicationContext,
    require_ack: bool,
    resolved_day: Option<&ResolvedMarketStreamDay>,
) -> Result<LockedPublication, MarketStreamStorageError> {
    let producer = load_producer_for_update(tx, context.producer.credential_slot_id).await?;
    if !producer.matches_lease(&context.producer)
        || producer.session_date != Some(context.session.session_date)
    {
        return Err(MarketStreamStorageError::ProducerLost);
    }
    let rights: bool =
        sqlx::query_scalar("SELECT public.lock_owner_market_stream_rights($1, $2, $3, $4, $5)")
            .bind(producer.credential_slot_id)
            .bind(producer.grant_id)
            .bind(producer.grant_revision)
            .bind(producer.owner_user_id)
            .bind(context.session.session_date)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_database_error)?;
    if !rights {
        return Err(MarketStreamStorageError::RightsInvalid);
    }
    lock_owner_capacity(tx, producer.owner_user_id).await?;

    let mut ordered_items: Vec<_> = context.items.iter().collect();
    ordered_items.sort_by_key(|item| item.admission.membership_id);
    let mut admissions = BTreeMap::new();
    for item in &ordered_items {
        let locked: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_worker_admission(
                $1, $2, $3, $4, $5, $6, $7, $8
            )",
        )
        .bind(producer.credential_slot_id)
        .bind(producer.grant_id)
        .bind(producer.grant_revision)
        .bind(item.admission.owner_user_id)
        .bind(item.admission.membership_id)
        .bind(item.admission.generation_id)
        .bind(&item.admission.instrument_id)
        .bind(item.admission.generation_i64())
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)?;
        if !locked {
            return Err(MarketStreamStorageError::MembershipNotReady);
        }
        let row: AdmissionDbRow = sqlx::query_as(
            "SELECT admission.owner_user_id, admission.membership_id,
                    admission.generation_id, admission.instrument_id,
                    admission.generation
               FROM public.owner_equity_memberships AS membership
               JOIN public.owner_equity_generation_admissions AS admission
                 ON admission.owner_user_id = membership.owner_user_id
                AND admission.membership_id = membership.id
                AND admission.instrument_id = membership.instrument_id
                AND admission.generation = $4
              WHERE membership.owner_user_id = $1
                AND membership.id = $2
                AND membership.instrument_id = $3
                AND membership.state = 'READY'
                AND NOT EXISTS (
                    SELECT 1
                      FROM public.owner_equity_generation_admissions AS newer
                     WHERE newer.owner_user_id = admission.owner_user_id
                       AND newer.membership_id = admission.membership_id
                       AND newer.instrument_id = admission.instrument_id
                       AND newer.generation > admission.generation
                )
            ",
        )
        .bind(item.admission.owner_user_id)
        .bind(item.admission.membership_id)
        .bind(&item.admission.instrument_id)
        .bind(item.admission.generation_i64())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_database_error)?
        .ok_or(MarketStreamStorageError::MembershipNotReady)?;
        if admissions
            .insert(item.admission.membership_id, row)
            .is_some()
        {
            return Err(MarketStreamStorageError::InvalidInput);
        }
    }

    // The owner-capacity mutex above serializes this discovery with every
    // lease mutation.  Match complete current admission identities and only
    // currently live leases; revoked session hashes remain visible only on
    // this narrow active-demand surface so they can fail closed below.
    let membership_ids: Vec<Uuid> = ordered_items
        .iter()
        .map(|item| item.admission.membership_id)
        .collect();
    let generation_ids: Vec<Uuid> = ordered_items
        .iter()
        .map(|item| item.admission.generation_id)
        .collect();
    let instrument_ids: Vec<String> = ordered_items
        .iter()
        .map(|item| item.admission.instrument_id.clone())
        .collect();
    let generations: Vec<i64> = ordered_items
        .iter()
        .map(|item| item.admission.generation_i64())
        .collect();
    let session_rows: Vec<(String, Uuid)> = sqlx::query_as(
        "WITH requested AS (
             SELECT * FROM ROWS FROM (
                 pg_catalog.unnest($2::uuid[]),
                 pg_catalog.unnest($3::uuid[]),
                 pg_catalog.unnest($4::text[]),
                 pg_catalog.unnest($5::bigint[])
             ) AS requested(membership_id, generation_id, instrument_id, generation)
         )
         SELECT DISTINCT lease.session_hash, lease.owner_user_id
           FROM public.owner_market_stream_leases AS lease
           JOIN public.owner_market_stream_lease_items AS item
             ON item.lease_id = lease.id
            AND item.owner_user_id = lease.owner_user_id
           JOIN requested
             ON requested.membership_id = item.membership_id
            AND requested.generation_id = item.generation_id
            AND requested.instrument_id = item.instrument_id
            AND requested.generation = item.generation
          WHERE item.owner_user_id = $1
            AND lease.state = 'ACTIVE'
            AND lease.lease_expires_at > pg_catalog.clock_timestamp()
          ORDER BY lease.session_hash, lease.owner_user_id",
    )
    .bind(producer.owner_user_id)
    .bind(&membership_ids)
    .bind(&generation_ids)
    .bind(&instrument_ids)
    .bind(&generations)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let mut sessions = BTreeSet::new();
    for (session_hash, owner_user_id) in session_rows {
        if !sessions.insert(session_hash.clone()) {
            continue;
        }
        let valid: bool = sqlx::query_scalar(
            "SELECT public.lock_owner_market_stream_session($1, $2, $3, $4, $5)",
        )
        .bind(producer.credential_slot_id)
        .bind(producer.grant_id)
        .bind(producer.grant_revision)
        .bind(owner_user_id)
        .bind(&session_hash)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)?;
        if !valid {
            let still_demanded: bool = sqlx::query_scalar(
                "WITH requested AS (
                     SELECT * FROM ROWS FROM (
                         pg_catalog.unnest($3::uuid[]),
                         pg_catalog.unnest($4::uuid[]),
                         pg_catalog.unnest($5::text[]),
                         pg_catalog.unnest($6::bigint[])
                     ) AS requested(membership_id, generation_id, instrument_id, generation)
                 )
                 SELECT EXISTS (
                     SELECT 1
                       FROM public.owner_market_stream_leases AS lease
                       JOIN public.owner_market_stream_lease_items AS item
                         ON item.lease_id = lease.id
                        AND item.owner_user_id = lease.owner_user_id
                       JOIN requested
                         ON requested.membership_id = item.membership_id
                        AND requested.generation_id = item.generation_id
                        AND requested.instrument_id = item.instrument_id
                        AND requested.generation = item.generation
                      WHERE lease.owner_user_id = $1
                        AND lease.session_hash = $2
                        AND lease.state = 'ACTIVE'
                        AND lease.lease_expires_at > pg_catalog.clock_timestamp()
                 )",
            )
            .bind(owner_user_id)
            .bind(&session_hash)
            .bind(&membership_ids)
            .bind(&generation_ids)
            .bind(&instrument_ids)
            .bind(&generations)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_database_error)?;
            if still_demanded {
                return Err(MarketStreamStorageError::SessionInvalid);
            }
        }
    }

    let leases: Vec<LeaseItemLockDbRow> = sqlx::query_as(
        "SELECT * FROM public.lock_owner_market_stream_worker_demand($1, $2, $3, $4, $5)",
    )
    .bind(producer.credential_slot_id)
    .bind(producer.grant_id)
    .bind(producer.grant_revision)
    .bind(producer.owner_user_id)
    .bind(&membership_ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_database_error)?;

    let mut subscriptions = BTreeMap::new();
    let mut subscription_items = ordered_items.clone();
    subscription_items.sort_by_key(|item| item.admission.symbol());
    for item in &subscription_items {
        let row: Option<SubscriptionDbRow> = sqlx::query_as(
            "SELECT credential_slot_id, symbol, provider, environment,
                    venue, tr_id, grant_revision, epoch, state,
                    pending_operation, pending_ordinal, pending_reserved_at,
                    pending_deadline,
                    acked_at, desired_reference_count,
                    subscription_revision, updated_at
               FROM public.owner_market_stream_subscriptions
              WHERE credential_slot_id = $1 AND symbol = $2
              ORDER BY symbol
              FOR SHARE",
        )
        .bind(context.producer.credential_slot_id)
        .bind(item.admission.symbol())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_database_error)?;
        let row = row.ok_or(MarketStreamStorageError::SubscriptionInvalid)?;
        if require_ack
            && (row.state != "ACKED"
                || row.epoch != Some(context.epoch.epoch)
                || row.acked_at.is_none()
                || row.desired_reference_count <= 0)
        {
            return Err(MarketStreamStorageError::SubscriptionNotAcked);
        }
        subscriptions.insert(row.symbol.clone(), row);
    }

    let mut caches = BTreeMap::new();
    for item in &ordered_items {
        let row: Option<CacheDbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CACHE_SELECT}
               FROM public.owner_market_stream_cache AS cache
              WHERE cache.owner_user_id = $1
                AND cache.membership_id = $2
              FOR UPDATE"
        )))
        .bind(item.admission.owner_user_id)
        .bind(item.admission.membership_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_database_error)?;
        caches.insert(item.admission.membership_id, row);
    }
    let fresh_now = fresh_database_time(tx).await?;
    let mut checked_sessions = BTreeSet::new();
    for lease in &leases {
        if lease.state == "ACTIVE"
            && lease.lease_expires_at > fresh_now
            && checked_sessions.insert(lease.session_hash.clone())
        {
            let valid: bool =
                sqlx::query_scalar("SELECT public.owner_market_stream_session_valid($1, $2)")
                    .bind(&lease.session_hash)
                    .bind(lease.owner_user_id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(map_database_error)?;
            if !valid {
                return Err(MarketStreamStorageError::SessionInvalid);
            }
        }
    }
    if let Some(day) = resolved_day {
        if context.session != day.session() {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        validate_resolved_day(tx, day, fresh_now).await?;
    }
    Ok(LockedPublication {
        producer,
        admissions,
        leases,
        subscriptions,
        caches,
        fresh_now,
    })
}

async fn validate_resolved_day(
    tx: &mut Transaction<'_, Postgres>,
    day: &ResolvedMarketStreamDay,
    fresh_now: DateTime<Utc>,
) -> Result<(), MarketStreamStorageError> {
    validate_resolved_day_clock(day, fresh_now)?;
    let calendar = day.calendar();
    validate_session_lineage(tx, calendar, fresh_now)
        .await
        .map_err(|_| MarketStreamStorageError::SessionInvalid)
}

fn validate_resolved_day_clock(
    day: &ResolvedMarketStreamDay,
    fresh_now: DateTime<Utc>,
) -> Result<(), MarketStreamStorageError> {
    let session = day.session();
    let calendar = day.calendar();
    let kst_offset = FixedOffset::east_opt(KST_OFFSET_SECONDS)
        .ok_or(MarketStreamStorageError::SessionInvalid)?;
    if day.session_date() != session.session_date
        || day.session_date() != calendar.session_date
        || day.open_at() >= day.close_at()
        || fresh_now.with_timezone(&kst_offset).date_naive() != day.session_date()
        || !(day.open_at() <= fresh_now && fresh_now < day.close_at())
        || session.calendar_source_batch_id != calendar.calendar_source_batch_id
        || session.calendar_content_sha256 != calendar.calendar_content_sha256
        || session.window_contract_sha256 != calendar.window_contract_sha256
        || session.session_proof_sha256
            != canonical_day_proof_sha256(
                day.session_date(),
                calendar.calendar_source_batch_id,
                &calendar.calendar_content_sha256,
                &calendar.window_contract_sha256,
                day.open_at(),
                day.close_at(),
            )
    {
        return Err(MarketStreamStorageError::SessionInvalid);
    }
    Ok(())
}

async fn revalidate_resolved_commit_guard(
    tx: &mut Transaction<'_, Postgres>,
    lease: &StreamProducerLease,
    day: &ResolvedMarketStreamDay,
    expected_epoch: Uuid,
) -> Result<DateTime<Utc>, MarketStreamStorageError> {
    let lineage_now = fresh_database_time(tx).await?;
    validate_resolved_day(tx, day, lineage_now).await?;

    // Re-read the still-locked producer after lineage validation, then use a
    // fresh clock as the final database observation before the commit. No DB
    // await follows this clock read, so close/date and lease checks cannot be
    // based on the earlier post-lock timestamp.
    let producer = load_producer_for_update(tx, lease.credential_slot_id).await?;
    let commit_now = fresh_database_time(tx).await?;
    validate_resolved_day_clock(day, commit_now)?;
    if !producer.matches_lease(lease) || producer.current_epoch != Some(expected_epoch) {
        return Err(MarketStreamStorageError::ProducerLost);
    }
    let session = day.session();
    if producer.session_date != Some(day.session_date())
        || producer.session_proof_id != Some(session.session_proof_id)
        || producer.session_proof_sha256.as_deref() != Some(session.session_proof_sha256.as_str())
        || producer.calendar_source_batch_id != Some(session.calendar_source_batch_id)
        || producer.calendar_content_sha256.as_deref()
            != Some(session.calendar_content_sha256.as_str())
        || producer.window_contract_sha256.as_deref()
            != Some(session.window_contract_sha256.as_str())
    {
        return Err(MarketStreamStorageError::SessionInvalid);
    }
    if producer.lease_expires_at
        <= commit_now + chrono::Duration::seconds(STREAM_PRODUCER_RENEW_AFTER_SECONDS)
    {
        return Err(MarketStreamStorageError::ProducerLost);
    }
    Ok(commit_now)
}

async fn validate_locked_publication_context(
    locked: &LockedPublication,
    context: &StreamPublicationContext,
    fresh_now: DateTime<Utc>,
) -> Result<(), MarketStreamStorageError> {
    if !locked.producer.matches_lease(&context.producer)
        || locked.producer.current_epoch != Some(context.epoch.epoch)
        || locked.producer.session_date != Some(context.session.session_date)
        || locked.producer.lease_expires_at
            <= fresh_now + chrono::Duration::seconds(STREAM_PRODUCER_RENEW_AFTER_SECONDS)
    {
        return Err(MarketStreamStorageError::ProducerLost);
    }
    if locked.producer.session_proof_id != Some(context.session.session_proof_id)
        || locked.producer.session_proof_sha256.as_deref()
            != Some(context.session.session_proof_sha256.as_str())
        || locked.producer.calendar_source_batch_id
            != Some(context.session.calendar_source_batch_id)
        || locked.producer.calendar_content_sha256.as_deref()
            != Some(context.session.calendar_content_sha256.as_str())
        || locked.producer.window_contract_sha256.as_deref()
            != Some(context.session.window_contract_sha256.as_str())
    {
        return Err(MarketStreamStorageError::SessionInvalid);
    }
    let kst_offset = FixedOffset::east_opt(KST_OFFSET_SECONDS)
        .ok_or(MarketStreamStorageError::SessionInvalid)?;
    if fresh_now.with_timezone(&kst_offset).date_naive() != context.session.session_date
        || locked.producer.gap_generation
            != i64::try_from(context.epoch.gap_generation)
                .map_err(|_| MarketStreamStorageError::VersionExhausted)?
    {
        return Err(MarketStreamStorageError::SessionInvalid);
    }
    for item in &context.items {
        let admission = locked
            .admissions
            .get(&item.admission.membership_id)
            .ok_or(MarketStreamStorageError::MembershipNotReady)?;
        if admission.owner_user_id != item.admission.owner_user_id
            || admission.generation_id != item.admission.generation_id
            || admission.instrument_id != item.admission.instrument_id
            || admission.generation != item.admission.generation_i64()
        {
            return Err(MarketStreamStorageError::MembershipNotReady);
        }
        let subscription = locked
            .subscriptions
            .get(item.admission.symbol())
            .ok_or(MarketStreamStorageError::SubscriptionInvalid)?;
        if subscription.grant_revision != context.producer.grant_revision
            || subscription.epoch != Some(context.epoch.epoch)
            || subscription.subscription_revision != item.subscription.subscription_revision
            || subscription.state != "ACKED"
            || subscription.acked_at.is_none()
            || subscription.desired_reference_count <= 0
        {
            return Err(MarketStreamStorageError::SubscriptionNotAcked);
        }
        let has_demand = locked.leases.iter().any(|lease| {
            lease.owner_user_id == item.admission.owner_user_id
                && lease.membership_id == item.admission.membership_id
                && lease.generation_id == item.admission.generation_id
                && lease.instrument_id == item.admission.instrument_id
                && lease.generation == item.admission.generation_i64()
                && lease.state == "ACTIVE"
                && lease.lease_expires_at > fresh_now
        });
        if !has_demand {
            return Err(MarketStreamStorageError::RightsInvalid);
        }
    }
    Ok(())
}

async fn upsert_stream_quote(
    tx: &mut Transaction<'_, Postgres>,
    item: &StreamPublicationItem,
    context: &StreamPublicationContext,
    quote: &StreamQuote,
    receipt: &MarketReceipt,
    fresh_now: DateTime<Utc>,
    prior: Option<&CacheDbRow>,
) -> Result<CacheDbRow, MarketStreamStorageError> {
    if quote.base_price.is_some()
        || quote.base_price_reason != StreamBasePriceReason::NotProvidedByChannel
    {
        return Err(MarketStreamStorageError::ReceiptInvalid);
    }
    let fencing_token = i64::try_from(context.producer.fencing_token)
        .map_err(|_| MarketStreamStorageError::InvalidInput)?;
    let receive_ordinal = i64::try_from(receipt.receive_ordinal())
        .map_err(|_| MarketStreamStorageError::ReceiptInvalid)?;
    let trade_volume =
        i64::try_from(quote.trade_volume).map_err(|_| MarketStreamStorageError::ReceiptInvalid)?;
    let cumulative_volume = i64::try_from(quote.cumulative_volume)
        .map_err(|_| MarketStreamStorageError::ReceiptInvalid)?;
    let provider_trade_at = quote.provider_trade_at.with_timezone(&Utc);
    sqlx::query(
        "INSERT INTO public.owner_market_stream_cache
            (owner_user_id, membership_id, generation_id, instrument_id,
             generation, grant_id, credential_slot_id, grant_revision,
             source, wire_version, venue, currency, session_date,
             calendar_source, calendar_source_version, session_proof_id,
             session_proof_sha256, calendar_source_batch_id,
             calendar_content_sha256, window_contract_sha256, epoch,
             fencing_token, subscription_revision, receive_ordinal,
             price, base_price, change_amount, change_percent, direction,
             trade_volume, cumulative_volume, halted, business_date, trade_time,
             provider_trade_at, quote_version, state_version, status_code,
             status_at, connection_state, market_state, freshness, availability,
             gap_since, gap_generation, received_at, committed_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                 $9, $10, $11, $12, $13, 'kis',
                 'kis-chk-holiday-v1:schema-1', $14, $15, $16, $17, $18,
                 $19, $20, $21, $22, $23::numeric, NULL, $24::numeric,
                 $25::numeric, $26, $27, $28, $29, $30, $31, $32, 1, 1,
                 NULL, NULL, 'CONNECTED', 'OPEN', 'RECENT', 'LIVE', NULL,
                 $33, $34, $35, $35)
         ON CONFLICT (owner_user_id, membership_id) DO UPDATE
            SET row_generation = CASE
                    WHEN public.owner_market_stream_cache.generation_id IS DISTINCT FROM EXCLUDED.generation_id
                      OR public.owner_market_stream_cache.session_date IS DISTINCT FROM EXCLUDED.session_date
                    THEN pg_catalog.gen_random_uuid()
                    ELSE public.owner_market_stream_cache.row_generation
                END,
                generation_id = EXCLUDED.generation_id,
                instrument_id = EXCLUDED.instrument_id,
                generation = EXCLUDED.generation,
                grant_id = EXCLUDED.grant_id,
                credential_slot_id = EXCLUDED.credential_slot_id,
                grant_revision = EXCLUDED.grant_revision,
                source = EXCLUDED.source,
                wire_version = EXCLUDED.wire_version,
                venue = EXCLUDED.venue,
                currency = EXCLUDED.currency,
                session_date = EXCLUDED.session_date,
                calendar_source = EXCLUDED.calendar_source,
                calendar_source_version = EXCLUDED.calendar_source_version,
                session_proof_id = EXCLUDED.session_proof_id,
                session_proof_sha256 = EXCLUDED.session_proof_sha256,
                calendar_source_batch_id = EXCLUDED.calendar_source_batch_id,
                calendar_content_sha256 = EXCLUDED.calendar_content_sha256,
                window_contract_sha256 = EXCLUDED.window_contract_sha256,
                epoch = EXCLUDED.epoch,
                fencing_token = EXCLUDED.fencing_token,
                subscription_revision = EXCLUDED.subscription_revision,
                receive_ordinal = EXCLUDED.receive_ordinal,
                price = EXCLUDED.price,
                base_price = NULL,
                change_amount = EXCLUDED.change_amount,
                change_percent = EXCLUDED.change_percent,
                direction = EXCLUDED.direction,
                trade_volume = EXCLUDED.trade_volume,
                cumulative_volume = EXCLUDED.cumulative_volume,
                halted = EXCLUDED.halted,
                business_date = EXCLUDED.business_date,
                trade_time = EXCLUDED.trade_time,
                provider_trade_at = EXCLUDED.provider_trade_at,
                quote_version = CASE
                    WHEN public.owner_market_stream_cache.generation_id IS DISTINCT FROM EXCLUDED.generation_id
                      OR public.owner_market_stream_cache.session_date IS DISTINCT FROM EXCLUDED.session_date
                    THEN 1
                    ELSE public.owner_market_stream_cache.quote_version + 1
                END,
                state_version = CASE
                    WHEN public.owner_market_stream_cache.generation_id IS DISTINCT FROM EXCLUDED.generation_id
                      OR public.owner_market_stream_cache.session_date IS DISTINCT FROM EXCLUDED.session_date
                    THEN 1
                    ELSE public.owner_market_stream_cache.state_version + 1
                END,
                status_code = NULL, status_at = NULL,
                connection_state = EXCLUDED.connection_state,
                market_state = EXCLUDED.market_state,
                freshness = EXCLUDED.freshness,
                availability = EXCLUDED.availability,
                gap_since = NULL,
                gap_generation = EXCLUDED.gap_generation,
                received_at = EXCLUDED.received_at,
                committed_at = EXCLUDED.committed_at,
                updated_at = EXCLUDED.updated_at",
    )
    .bind(item.admission.owner_user_id)
    .bind(item.admission.membership_id)
    .bind(item.admission.generation_id)
    .bind(&item.admission.instrument_id)
    .bind(item.admission.generation_i64())
    .bind(context.producer.grant_id)
    .bind(context.producer.credential_slot_id)
    .bind(context.producer.grant_revision)
    .bind(STREAM_SOURCE)
    .bind(STREAM_WIRE_VERSION)
    .bind(STREAM_VENUE)
    .bind(STREAM_CURRENCY)
    .bind(context.session.session_date)
    .bind(context.session.session_proof_id)
    .bind(&context.session.session_proof_sha256)
    .bind(context.session.calendar_source_batch_id)
    .bind(&context.session.calendar_content_sha256)
    .bind(&context.session.window_contract_sha256)
    .bind(receipt.epoch())
    .bind(fencing_token)
    .bind(item.subscription.subscription_revision)
    .bind(receive_ordinal)
    .bind(&quote.price)
    .bind(&quote.change_from_previous_day)
    .bind(&quote.change_percent_from_previous_day)
    .bind(quote.direction.as_str())
    .bind(trade_volume)
    .bind(cumulative_volume)
    .bind(quote.halted)
    .bind(quote.business_date)
    .bind(quote.trade_time)
    .bind(provider_trade_at)
    .bind(
        i64::try_from(context.epoch.gap_generation)
            .map_err(|_| MarketStreamStorageError::VersionExhausted)?,
    )
    .bind(receipt_received_at(receipt)?)
    .bind(fresh_now)
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let row: CacheDbRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CACHE_SELECT}
           FROM public.owner_market_stream_cache AS cache
          WHERE cache.owner_user_id = $1 AND cache.membership_id = $2"
    )))
    .bind(item.admission.owner_user_id)
    .bind(item.admission.membership_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let _ = prior;
    Ok(row)
}

async fn upsert_stream_status(
    tx: &mut Transaction<'_, Postgres>,
    item: &StreamPublicationItem,
    context: &StreamPublicationContext,
    status: StreamStatus,
    fresh_now: DateTime<Utc>,
    prior: Option<&CacheDbRow>,
) -> Result<CacheDbRow, MarketStreamStorageError> {
    let gap_generation = i64::try_from(context.epoch.gap_generation)
        .map_err(|_| MarketStreamStorageError::InvalidInput)?;
    if let Some(prior) = prior
        && prior.generation_id == item.admission.generation_id
        && prior.session_date == context.session.session_date
    {
        sqlx::query(
            "UPDATE public.owner_market_stream_cache
                SET status_code = $3, status_at = $4,
                    connection_state = $5, market_state = $6,
                    freshness = $7, availability = $8,
                    gap_since = CASE WHEN $9 THEN COALESCE(gap_since, $4) ELSE NULL END,
                    gap_generation = $10, state_version = state_version + 1,
                    updated_at = $4
              WHERE owner_user_id = $1 AND membership_id = $2",
        )
        .bind(item.admission.owner_user_id)
        .bind(item.admission.membership_id)
        .bind(status.code.as_str())
        .bind(fresh_now)
        .bind(status.connection.as_str())
        .bind(status.market_state.as_str())
        .bind(status.freshness.as_str())
        .bind(status.availability.as_str())
        .bind(status.gap_open)
        .bind(gap_generation)
        .execute(&mut **tx)
        .await
        .map_err(map_database_error)?;
    } else {
        sqlx::query(
            "INSERT INTO public.owner_market_stream_cache
                (owner_user_id, membership_id, generation_id, instrument_id,
                 generation, grant_id, credential_slot_id, grant_revision,
                 source, wire_version, venue, currency, session_date,
                 calendar_source, calendar_source_version, session_proof_id,
                 session_proof_sha256, calendar_source_batch_id,
                 calendar_content_sha256, window_contract_sha256,
                 quote_version, state_version, status_code, status_at,
                 connection_state, market_state, freshness, availability,
                 gap_since, gap_generation, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                     $13, 'kis', 'kis-chk-holiday-v1:schema-1', $14, $15,
                     $16, $17, $18, 0, 1, $19, $20, $21, $22, $23, $24,
                     CASE WHEN $25 THEN $20 ELSE NULL END, $26, $20)
             ON CONFLICT (owner_user_id, membership_id) DO UPDATE
                SET row_generation = pg_catalog.gen_random_uuid(),
                    generation_id = EXCLUDED.generation_id,
                    instrument_id = EXCLUDED.instrument_id,
                    generation = EXCLUDED.generation,
                    grant_id = EXCLUDED.grant_id,
                    credential_slot_id = EXCLUDED.credential_slot_id,
                    grant_revision = EXCLUDED.grant_revision,
                    source = EXCLUDED.source,
                    wire_version = EXCLUDED.wire_version,
                    venue = EXCLUDED.venue,
                    currency = EXCLUDED.currency,
                    session_date = EXCLUDED.session_date,
                    calendar_source = EXCLUDED.calendar_source,
                    calendar_source_version = EXCLUDED.calendar_source_version,
                    session_proof_id = EXCLUDED.session_proof_id,
                    session_proof_sha256 = EXCLUDED.session_proof_sha256,
                    calendar_source_batch_id = EXCLUDED.calendar_source_batch_id,
                    calendar_content_sha256 = EXCLUDED.calendar_content_sha256,
                    window_contract_sha256 = EXCLUDED.window_contract_sha256,
                    epoch = NULL, fencing_token = NULL,
                    subscription_revision = NULL, receive_ordinal = NULL,
                    price = NULL, base_price = NULL, change_amount = NULL,
                    change_percent = NULL, direction = NULL,
                    trade_volume = NULL, cumulative_volume = NULL,
                    halted = NULL, business_date = NULL, trade_time = NULL,
                    provider_trade_at = NULL, quote_version = 0,
                    state_version = 1,
                    status_code = EXCLUDED.status_code,
                    status_at = EXCLUDED.status_at,
                    connection_state = EXCLUDED.connection_state,
                    market_state = EXCLUDED.market_state,
                    freshness = EXCLUDED.freshness,
                    availability = EXCLUDED.availability,
                    gap_since = EXCLUDED.gap_since,
                    gap_generation = EXCLUDED.gap_generation,
                    received_at = NULL, committed_at = NULL,
                    updated_at = EXCLUDED.updated_at",
        )
        .bind(item.admission.owner_user_id)
        .bind(item.admission.membership_id)
        .bind(item.admission.generation_id)
        .bind(&item.admission.instrument_id)
        .bind(item.admission.generation_i64())
        .bind(context.producer.grant_id)
        .bind(context.producer.credential_slot_id)
        .bind(context.producer.grant_revision)
        .bind(STREAM_SOURCE)
        .bind(STREAM_WIRE_VERSION)
        .bind(STREAM_VENUE)
        .bind(STREAM_CURRENCY)
        .bind(context.session.session_date)
        .bind(context.session.session_proof_id)
        .bind(&context.session.session_proof_sha256)
        .bind(context.session.calendar_source_batch_id)
        .bind(&context.session.calendar_content_sha256)
        .bind(&context.session.window_contract_sha256)
        .bind(status.code.as_str())
        .bind(fresh_now)
        .bind(status.connection.as_str())
        .bind(status.market_state.as_str())
        .bind(status.freshness.as_str())
        .bind(status.availability.as_str())
        .bind(status.gap_open)
        .bind(gap_generation)
        .execute(&mut **tx)
        .await
        .map_err(map_database_error)?;
    }
    let row: CacheDbRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CACHE_SELECT}
           FROM public.owner_market_stream_cache AS cache
          WHERE cache.owner_user_id = $1 AND cache.membership_id = $2"
    )))
    .bind(item.admission.owner_user_id)
    .bind(item.admission.membership_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    Ok(row)
}

fn receipt_received_at(receipt: &MarketReceipt) -> Result<DateTime<Utc>, MarketStreamStorageError> {
    DateTime::<Utc>::from_timestamp_millis(receipt.received_at_ms())
        .ok_or(MarketStreamStorageError::ReceiptInvalid)
}

fn validate_producer_lease(lease: &StreamProducerLease) -> Result<(), MarketStreamStorageError> {
    if lease.credential_slot_id.is_nil()
        || lease.grant_id.is_nil()
        || lease.grant_revision.is_nil()
        || lease.owner_user_id.is_nil()
        || lease.holder_id.is_nil()
        || lease.fencing_token == 0
        || lease.gap_generation > i64::MAX as u64
    {
        return Err(MarketStreamStorageError::InvalidInput);
    }
    Ok(())
}

fn yyyymmdd(date: NaiveDate) -> u32 {
    date.format("%Y%m%d")
        .to_string()
        .parse()
        .unwrap_or_default()
}

fn canonical_instrument(value: &str) -> bool {
    value.is_ascii()
        && value.len() == 10
        && value.as_bytes()[6] == b'.'
        && value.as_bytes()[..6]
            .iter()
            .all(|byte| byte.is_ascii_digit())
        && &value[7..] == "KRX"
}

fn is_six_ascii_digits(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn canonical_session_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_idempotency_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b':' && byte != b'\\')
}

fn canonical_unprefixed_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn canonical_prefixed_sha256(value: &str) -> bool {
    value.len() == 71 && value.starts_with("sha256:") && canonical_unprefixed_sha256(&value[7..])
}

fn prefixed_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn map_database_error(error: sqlx::Error) -> MarketStreamStorageError {
    match &error {
        sqlx::Error::Database(database) => match database.code().as_deref() {
            Some("42501") => MarketStreamStorageError::PermissionDenied,
            Some("23514" | "23503" | "23505" | "22P02") => {
                MarketStreamStorageError::DatabaseIntegrity
            }
            Some("57014" | "55P03" | "40001") => MarketStreamStorageError::DatabaseUnavailable,
            _ => MarketStreamStorageError::DatabaseUnavailable,
        },
        sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::Io(_)
        | sqlx::Error::Tls(_) => MarketStreamStorageError::DatabaseUnavailable,
        _ => MarketStreamStorageError::DatabaseUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_request_is_canonical_and_bounded() {
        let owner = Uuid::new_v4();
        let first = StreamLeaseIdentity::new(Uuid::new_v4(), "000660.KRX".to_owned(), 2).unwrap();
        let second = StreamLeaseIdentity::new(Uuid::new_v4(), "005930.KRX".to_owned(), 1).unwrap();
        let request = StreamLeaseRequest::new(
            owner,
            0,
            vec![second.clone(), first.clone()],
            "stream-test-1".to_owned(),
        )
        .unwrap();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(request.identities(), expected);
        assert!(format!("{request:?}").contains("<redacted>"));
        assert!(!format!("{request:?}").contains("stream-test-1"));
    }

    #[test]
    fn status_and_source_enums_are_closed() {
        assert_eq!(StreamStatusCode::AccessRevoked.as_str(), "ACCESS_REVOKED");
        assert_eq!(StreamConnectionState::Connected.as_str(), "CONNECTED");
        assert_eq!(StreamFreshness::Recent.as_str(), "RECENT");
        assert_eq!(STREAM_SOURCE, "KIS_MARKET_WS");
    }
}

const RUNTIME_DATABASE_CALL_DEADLINE: Duration = Duration::from_secs(1);

/// Deadline/cancellation state shared by clones of one private runtime adapter.
/// It deliberately has no operation lock: renewal and publication may proceed
/// independently while each owns its own bounded future.
#[derive(Clone)]
struct RuntimeTerminalLatch(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Default for RuntimeTerminalLatch {
    fn default() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }
}

impl RuntimeTerminalLatch {
    fn is_terminal(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    fn latch(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum RuntimeMarketStreamStorageError {
    #[error("MARKET_STREAM_RUNTIME_STORAGE")]
    Storage(MarketStreamStorageError),
    #[error("MARKET_STREAM_RUNTIME_DEADLINE_EXCEEDED")]
    DeadlineExceeded,
    #[error("MARKET_STREAM_RUNTIME_TERMINAL")]
    Terminal,
}

struct RuntimeDeadlineGuard {
    terminal: RuntimeTerminalLatch,
    pending: bool,
}

impl RuntimeDeadlineGuard {
    fn new(terminal: RuntimeTerminalLatch) -> Self {
        Self {
            terminal,
            pending: true,
        }
    }

    fn resolved(&mut self) {
        self.pending = false;
    }
}

impl Drop for RuntimeDeadlineGuard {
    fn drop(&mut self) {
        if self.pending {
            self.terminal.latch();
        }
    }
}

/// Owns one inner future and one deadline. Since this async body establishes
/// the deadline on its first poll before polling `operation`, pool acquisition
/// is included. Dropping a polled pending call drops the inner future and
/// leaves all adapter clones terminal.
async fn with_runtime_deadline<T, F>(
    terminal: RuntimeTerminalLatch,
    operation: F,
) -> Result<T, RuntimeMarketStreamStorageError>
where
    F: std::future::Future<Output = Result<T, MarketStreamStorageError>>,
{
    if terminal.is_terminal() {
        return Err(RuntimeMarketStreamStorageError::Terminal);
    }

    let deadline = tokio::time::Instant::now() + RUNTIME_DATABASE_CALL_DEADLINE;
    let mut guard = RuntimeDeadlineGuard::new(terminal.clone());
    match tokio::time::timeout_at(deadline, operation).await {
        Err(_) => {
            terminal.latch();
            guard.resolved();
            Err(RuntimeMarketStreamStorageError::DeadlineExceeded)
        }
        Ok(Ok(value)) => {
            if tokio::time::Instant::now() >= deadline {
                terminal.latch();
                guard.resolved();
                Err(RuntimeMarketStreamStorageError::DeadlineExceeded)
            } else if terminal.is_terminal() {
                guard.resolved();
                Err(RuntimeMarketStreamStorageError::Terminal)
            } else {
                guard.resolved();
                Ok(value)
            }
        }
        Ok(Err(error)) => {
            if tokio::time::Instant::now() >= deadline
                || error == MarketStreamStorageError::CommitUnknown
            {
                terminal.latch();
            }
            guard.resolved();
            Err(RuntimeMarketStreamStorageError::Storage(error))
        }
    }
}

/// Runtime-only bounded repository surface. The pool, policy repository and
/// terminal latch are private; no raw pool or inner repository is exposed.
#[derive(Clone)]
pub(super) struct RuntimeMarketStreamRepository {
    inner: OwnerMarketStreamRepository,
    terminal: RuntimeTerminalLatch,
}

impl RuntimeMarketStreamRepository {
    pub(super) async fn load_runtime_grant(
        &self,
        config: &OwnerMarketStreamRuntimeConfig,
    ) -> Result<RuntimeGrant, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(self.terminal.clone(), self.inner.load_runtime_grant(config)).await
    }

    pub(super) async fn read_stream_demand(
        &self,
        credential_slot_id: Uuid,
    ) -> Result<DesiredSet, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner.read_stream_demand(credential_slot_id),
        )
        .await
    }

    pub(super) async fn claim_stream_producer(
        &self,
        credential_slot_id: Uuid,
        holder_id: Uuid,
        grant_revision: Uuid,
    ) -> Result<StreamProducerLease, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner
                .claim_stream_producer(credential_slot_id, holder_id, grant_revision),
        )
        .await
    }

    pub(super) async fn renew_stream_producer(
        &self,
        lease: &StreamProducerLease,
    ) -> Result<StreamProducerLease, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner.renew_stream_producer(lease),
        )
        .await
    }

    pub(super) async fn start_stream_epoch_resolved(
        &self,
        lease: &StreamProducerLease,
        day: &ResolvedMarketStreamDay,
        epoch: Uuid,
    ) -> Result<StreamEpochProof, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner.start_stream_epoch_resolved(lease, day, epoch),
        )
        .await
    }

    pub(super) async fn set_subscription_desired(
        &self,
        lease: &StreamProducerLease,
        epoch: Uuid,
        symbol: &str,
        desired_reference_count: u32,
    ) -> Result<(), RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner
                .set_subscription_desired(lease, epoch, symbol, desired_reference_count),
        )
        .await
    }

    pub(super) async fn commit_prepared_subscription(
        &self,
        lease: &StreamProducerLease,
        prepared: &PreparedMarketSubscriptionCommand,
    ) -> Result<PendingSubscriptionCommit, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner.commit_prepared_subscription(lease, prepared),
        )
        .await
    }

    pub(super) async fn commit_subscription_ack(
        &self,
        lease: &StreamProducerLease,
        pending: PendingSubscriptionCommit,
        ack: MarketSubscriptionAck,
    ) -> Result<Option<SubscriptionProof>, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner.commit_subscription_ack(lease, pending, ack),
        )
        .await
    }

    pub(super) async fn publish_stream_latest_resolved(
        &self,
        context: &StreamPublicationContext,
        day: &ResolvedMarketStreamDay,
        observations: &[StreamPublicationObservation],
    ) -> Result<CommitResult, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner
                .publish_stream_latest_resolved(context, day, observations),
        )
        .await
    }

    pub(super) async fn record_stream_status(
        &self,
        context: &StreamPublicationContext,
        status: StreamStatus,
    ) -> Result<CommitResult, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner.record_stream_status(context, status),
        )
        .await
    }

    pub(super) async fn record_runtime_status(
        &self,
        lease: &StreamProducerLease,
        expected_epoch: Option<Uuid>,
        transition: RuntimeTransition,
    ) -> Result<RuntimeStatusCommit, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner
                .record_runtime_status(lease, expected_epoch, transition),
        )
        .await
    }

    pub(super) async fn retire_stream_producer(
        &self,
        lease: &StreamProducerLease,
        expected_epoch: Option<Uuid>,
        reason: StreamStatusCode,
    ) -> Result<RuntimeStatusCommit, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            self.inner
                .retire_stream_producer(lease, expected_epoch, reason),
        )
        .await
    }
}

#[cfg(test)]
mod runtime_deadline_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{Instant as TokioInstant, sleep, sleep_until};

    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn legacy_and_runtime_sql_limits_are_distinct() {
        let legacy = WorkerTransactionPolicy::Legacy.sql_limits();
        let runtime = WorkerTransactionPolicy::Runtime.sql_limits();
        assert_eq!(legacy.0, "SET LOCAL lock_timeout = '5s'");
        assert_eq!(legacy.1, "SET LOCAL statement_timeout = '30s'");
        assert_eq!(runtime.0, "SET LOCAL lock_timeout = '1s'");
        assert_eq!(runtime.1, "SET LOCAL statement_timeout = '1s'");
        assert_eq!(RUNTIME_DATABASE_CALL_DEADLINE, Duration::from_secs(1));
        for setting in [legacy.0, legacy.1, runtime.0, runtime.1] {
            assert!(setting.starts_with("SET LOCAL "));
            assert!(!setting.starts_with("SET SESSION "));
        }
    }

    #[tokio::test]
    async fn ready_result_and_known_error_keep_typed_outcomes() {
        let terminal = RuntimeTerminalLatch::default();
        let value = with_runtime_deadline(terminal.clone(), async {
            Ok::<_, MarketStreamStorageError>(17_u8)
        })
        .await;
        assert_eq!(value, Ok(17));
        assert!(!terminal.is_terminal());

        let error = with_runtime_deadline(terminal.clone(), async {
            Err::<(), _>(MarketStreamStorageError::RightsInvalid)
        })
        .await;
        assert_eq!(
            error,
            Err(RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::RightsInvalid,
            ))
        );
        assert!(!terminal.is_terminal());
    }

    #[tokio::test(start_paused = true)]
    async fn one_deadline_covers_all_stages() {
        let terminal = RuntimeTerminalLatch::default();
        let completed_stages = Arc::new(AtomicUsize::new(0));
        let stages = Arc::clone(&completed_stages);
        let result = with_runtime_deadline(terminal.clone(), async move {
            sleep(Duration::from_millis(600)).await;
            stages.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(600)).await;
            stages.fetch_add(1, Ordering::SeqCst);
            Ok::<_, MarketStreamStorageError>(())
        })
        .await;
        assert_eq!(
            result,
            Err(RuntimeMarketStreamStorageError::DeadlineExceeded)
        );
        assert_eq!(completed_stages.load(Ordering::SeqCst), 1);
        assert!(terminal.is_terminal());
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_equality_cannot_return_success() {
        let terminal = RuntimeTerminalLatch::default();
        let ready_at = TokioInstant::now() + RUNTIME_DATABASE_CALL_DEADLINE;
        let result = with_runtime_deadline(terminal.clone(), async move {
            sleep_until(ready_at).await;
            Ok::<_, MarketStreamStorageError>(())
        })
        .await;
        assert_eq!(
            result,
            Err(RuntimeMarketStreamStorageError::DeadlineExceeded)
        );
        assert!(terminal.is_terminal());
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_drops_future_and_latches_shared_state() {
        let terminal = RuntimeTerminalLatch::default();
        let shared = terminal.clone();
        let drops = Arc::new(AtomicUsize::new(0));
        let future_drops = Arc::clone(&drops);
        let result = with_runtime_deadline(terminal.clone(), async move {
            let _drop_counter = DropCounter(future_drops);
            std::future::pending::<Result<(), MarketStreamStorageError>>().await
        })
        .await;
        assert_eq!(
            result,
            Err(RuntimeMarketStreamStorageError::DeadlineExceeded)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(terminal.is_terminal());
        assert!(shared.is_terminal());
    }

    #[tokio::test]
    async fn external_cancellation_latches_shared_state() {
        let terminal = RuntimeTerminalLatch::default();
        let shared = terminal.clone();
        let drops = Arc::new(AtomicUsize::new(0));
        let future_drops = Arc::clone(&drops);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(with_runtime_deadline(terminal.clone(), async move {
            let _drop_counter = DropCounter(future_drops);
            let _ = entered_tx.send(());
            std::future::pending::<Result<(), MarketStreamStorageError>>().await
        }));
        entered_rx.await.expect("inner future was polled");
        task.abort();
        let joined = task.await;
        assert!(matches!(joined, Err(error) if error.is_cancelled()));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(terminal.is_terminal());
        assert!(shared.is_terminal());
    }

    #[tokio::test]
    async fn commit_unknown_latches_shared_state() {
        let terminal = RuntimeTerminalLatch::default();
        let shared = terminal.clone();
        let result = with_runtime_deadline(terminal.clone(), async {
            Err::<(), _>(MarketStreamStorageError::CommitUnknown)
        })
        .await;
        assert_eq!(
            result,
            Err(RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::CommitUnknown,
            ))
        );
        assert!(terminal.is_terminal());
        assert!(shared.is_terminal());

        let known_success = RuntimeTerminalLatch::default();
        assert_eq!(
            with_runtime_deadline(known_success.clone(), async {
                Ok::<_, MarketStreamStorageError>(())
            })
            .await,
            Ok(())
        );
        assert!(!known_success.is_terminal());
    }

    #[tokio::test]
    async fn terminal_state_never_polls_an_operation() {
        let terminal = RuntimeTerminalLatch::default();
        terminal.latch();
        let polls = Arc::new(AtomicUsize::new(0));
        let operation_polls = Arc::clone(&polls);
        let operation = std::future::poll_fn(move |_| {
            operation_polls.fetch_add(1, Ordering::SeqCst);
            std::task::Poll::Ready(Ok::<_, MarketStreamStorageError>(()))
        });
        let result = with_runtime_deadline(terminal, operation).await;
        assert_eq!(result, Err(RuntimeMarketStreamStorageError::Terminal));
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }
}

impl RuntimeMarketStreamRepository {
    pub(super) async fn resolve_runtime_day(
        &self,
        owner_user_id: Uuid,
        calendar: &super::intraday::OwnerIntradayQuoteRepository,
        windows_source: collectors::intraday_quotes::IntradaySessionWindowSource,
    ) -> Result<super::market_stream_runtime::DayResolution, RuntimeMarketStreamStorageError> {
        with_runtime_deadline(
            self.terminal.clone(),
            super::market_stream_runtime::resolve_day_for_runtime(
                owner_user_id,
                calendar.market_stream_calendar_reader(),
                windows_source,
            ),
        )
        .await
    }
}

#[cfg(test)]
mod runtime_calendar_deadline_tests {
    use super::{
        MarketStreamStorageError, RuntimeMarketStreamStorageError, RuntimeTerminalLatch,
        with_runtime_deadline,
    };
    use crate::owner_equity_v2::intraday::IntradayStorageError;
    use crate::owner_equity_v2::market_stream_runtime::map_runtime_calendar_storage_error;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn calendar_commit_uncertainty_latches_shared_runtime_deadline() {
        let terminal = RuntimeTerminalLatch::default();
        let shared = terminal.clone();
        let mapped = map_runtime_calendar_storage_error(IntradayStorageError::CommitUnknown);
        let result = with_runtime_deadline(terminal.clone(), async move {
            Err::<(), MarketStreamStorageError>(mapped)
        })
        .await;
        assert_eq!(
            result,
            Err(RuntimeMarketStreamStorageError::Storage(
                MarketStreamStorageError::CommitUnknown,
            ))
        );
        assert!(terminal.is_terminal());
        assert!(shared.is_terminal());

        let polls = Arc::new(AtomicUsize::new(0));
        let operation_polls = Arc::clone(&polls);
        let operation = std::future::poll_fn(move |_| {
            operation_polls.fetch_add(1, Ordering::SeqCst);
            std::task::Poll::Ready(Ok::<_, MarketStreamStorageError>(()))
        });
        let subsequent = with_runtime_deadline(shared, operation).await;
        assert_eq!(subsequent, Err(RuntimeMarketStreamStorageError::Terminal));
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod snapshot_delivery_age_tests {
    use super::*;

    pub(super) fn current_fixture(
        now: DateTime<Utc>,
    ) -> (
        CacheDbRow,
        StreamProducerDelivery,
        StreamSubscriptionDelivery,
    ) {
        let epoch = Uuid::from_u128(1);
        let revision = Uuid::from_u128(2);
        let proof = Uuid::from_u128(3);
        let batch = Uuid::from_u128(4);
        let proof_hash = "a".repeat(64);
        let calendar_hash = "b".repeat(64);
        let window_hash = "c".repeat(64);
        let row = CacheDbRow {
            owner_user_id: Uuid::from_u128(5),
            membership_id: Uuid::from_u128(6),
            row_generation: Uuid::from_u128(7),
            generation_id: Uuid::from_u128(8),
            instrument_id: "005930.KRX".to_owned(),
            generation: 1,
            grant_id: Uuid::from_u128(9),
            credential_slot_id: Uuid::from_u128(10),
            grant_revision: Uuid::from_u128(11),
            source: STREAM_SOURCE.to_owned(),
            wire_version: STREAM_WIRE_VERSION.to_owned(),
            venue: STREAM_VENUE.to_owned(),
            currency: STREAM_CURRENCY.to_owned(),
            session_date: now.date_naive(),
            calendar_source: "kis".to_owned(),
            calendar_source_version: "kis-chk-holiday-v1:schema-1".to_owned(),
            session_proof_id: proof,
            session_proof_sha256: proof_hash.clone(),
            calendar_source_batch_id: batch,
            calendar_content_sha256: calendar_hash.clone(),
            window_contract_sha256: window_hash.clone(),
            epoch: Some(epoch),
            fencing_token: Some(1),
            subscription_revision: Some(revision),
            receive_ordinal: Some(1),
            price: Some("100".to_owned()),
            base_price: None,
            change_amount: Some("1".to_owned()),
            change_percent: Some("1".to_owned()),
            direction: Some("UP".to_owned()),
            trade_volume: Some(1),
            cumulative_volume: Some(1),
            halted: Some(false),
            business_date: Some(now.date_naive()),
            trade_time: Some(
                now.with_timezone(&FixedOffset::east_opt(9 * 60 * 60).expect("KST offset"))
                    .time(),
            ),
            provider_trade_at: Some(now),
            quote_version: 1,
            state_version: 1,
            status_code: None,
            status_at: None,
            connection_state: StreamConnectionState::Connected.as_str().to_owned(),
            market_state: StreamMarketState::Open.as_str().to_owned(),
            freshness: StreamFreshness::Recent.as_str().to_owned(),
            availability: StreamAvailability::Live.as_str().to_owned(),
            gap_since: None,
            gap_generation: 0,
            received_at: Some(now),
            committed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        let producer = StreamProducerDelivery {
            connection: StreamConnectionState::Connected,
            reason: None,
            status_at: None,
            heartbeat_at: now,
            lease_expires_at: now + chrono::Duration::seconds(20),
            current_epoch: Some(epoch),
            session_date: Some(now.date_naive()),
            session_proof_id: Some(proof),
            session_proof_sha256: Some(proof_hash),
            calendar_source_batch_id: Some(batch),
            calendar_content_sha256: Some(calendar_hash),
            window_contract_sha256: Some(window_hash),
            gap_since: None,
            session_has_gap: false,
            gap_generation: 0,
            state_version: 1,
        };
        let subscription = StreamSubscriptionDelivery {
            state: StreamSubscriptionDeliveryState::Acked,
            epoch: Some(epoch),
            revision,
            updated_at: now,
        };
        (row, producer, subscription)
    }

    #[test]
    fn snapshot_delivery_uses_thirty_second_receipt_and_event_ages() {
        let now = DateTime::parse_from_rfc3339("2026-10-03T00:01:00Z")
            .expect("fixed test instant")
            .with_timezone(&Utc);
        let (mut row, producer, subscription) = current_fixture(now);
        let is_live = |row: &CacheDbRow| {
            cache_row_is_live(
                row,
                now,
                now.date_naive(),
                Some(&producer),
                Some(&subscription),
            )
        };
        assert!(is_live(&row), "fresh baseline must exercise the live path");

        row.received_at = Some(now - chrono::Duration::seconds(5));
        row.provider_trade_at = Some(now - chrono::Duration::seconds(5));
        row.committed_at = row.received_at;
        assert!(
            is_live(&row),
            "quiet valid observation became non-live before the 30-second display window"
        );

        row.received_at = Some(now - chrono::Duration::seconds(30));
        row.provider_trade_at = Some(now - chrono::Duration::seconds(30));
        row.committed_at = row.received_at;
        assert!(is_live(&row), "the inclusive 30-second boundary is recent");

        row.received_at = Some(now - chrono::Duration::milliseconds(30_001));
        assert!(!is_live(&row), "an old receipt cannot be recent");
        row.received_at = Some(now - chrono::Duration::seconds(1));
        row.provider_trade_at = Some(now - chrono::Duration::milliseconds(30_001));
        assert!(!is_live(&row), "recent receipt cannot freshen an old trade");

        row.provider_trade_at = None;
        assert!(!is_live(&row), "missing trade time is not recent evidence");
        row.provider_trade_at = Some(now + chrono::Duration::milliseconds(1));
        assert!(!is_live(&row), "a future trade must await its event time");
        row.provider_trade_at = Some(now);
        row.received_at = Some(now + chrono::Duration::milliseconds(1));
        assert!(!is_live(&row), "a future receipt cannot be recent");
        row.received_at = None;
        assert!(!is_live(&row), "missing receipt is not recent evidence");
    }
}

#[cfg(test)]
mod stream_delivery_evidence_tests {
    use super::*;

    struct Fixture {
        now: DateTime<Utc>,
        current_date: NaiveDate,
        row: CacheDbRow,
        producer: StreamProducerDelivery,
        identity: StreamIdentity,
        window: SnapshotWindowEvidence,
    }

    fn fixture() -> Fixture {
        let now = DateTime::parse_from_rfc3339("2026-10-03T00:01:00Z")
            .expect("fixed test instant")
            .with_timezone(&Utc);
        let current_date = now
            .with_timezone(&FixedOffset::east_opt(KST_OFFSET_SECONDS).expect("KST offset"))
            .date_naive();
        let (row, producer, _subscription) = snapshot_delivery_age_tests::current_fixture(now);
        let identity = StreamIdentity::new(
            row.owner_user_id,
            row.membership_id,
            row.generation_id,
            row.instrument_id.clone(),
            u64::try_from(row.generation).expect("positive fixture generation"),
        )
        .expect("canonical fixture identity");
        let window = window_fixture(
            current_date,
            now - chrono::Duration::minutes(1),
            now + chrono::Duration::minutes(1),
            row.window_contract_sha256.clone(),
        );
        Fixture {
            now,
            current_date,
            row,
            producer,
            identity,
            window,
        }
    }

    fn window_fixture(
        date: NaiveDate,
        open_at: DateTime<Utc>,
        close_at: DateTime<Utc>,
        contract_sha256: String,
    ) -> SnapshotWindowEvidence {
        super::super::market_stream_runtime::snapshot_window_fixture(
            date,
            open_at,
            close_at,
            contract_sha256,
        )
    }

    fn eligible(fixture: &Fixture, fresh_now: DateTime<Utc>) -> bool {
        snapshot_quote_is_eligible(
            &fixture.row,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fresh_now,
            fixture.current_date,
        )
    }

    #[test]
    fn consumer_release_binding_precedes_tombstone_replay() {
        let consumer = Uuid::from_u128(50);
        let other_consumer = Uuid::from_u128(51);
        let session = "a".repeat(64);
        for released in [false, true] {
            assert_eq!(
                release_binding_is_tombstone_replay(
                    &session,
                    &session,
                    consumer,
                    Some(consumer),
                    released,
                ),
                Ok(released)
            );
            assert_eq!(
                release_binding_is_tombstone_replay(
                    &session,
                    &session,
                    consumer,
                    Some(other_consumer),
                    released,
                ),
                Err(MarketStreamStorageError::LeaseNotFound)
            );
        }
        assert_eq!(
            release_binding_is_tombstone_replay(&session, &session, consumer, None, false,),
            Ok(false),
            "legacy same-session active release remains admitted"
        );
        assert_eq!(
            release_binding_is_tombstone_replay(
                &session,
                "b".repeat(64).as_str(),
                consumer,
                None,
                true,
            ),
            Err(MarketStreamStorageError::LeaseSessionMismatch)
        );
        assert_eq!(
            release_binding_is_tombstone_replay(
                &session,
                &session,
                consumer,
                Some(Uuid::nil()),
                false,
            ),
            Err(MarketStreamStorageError::InvalidInput)
        );
    }

    #[test]
    fn evidence_requires_validated_current_lineage_and_window() {
        let fixture = fixture();
        assert!(
            stream_delivery_evidence(
                fixture.now,
                fixture.current_date,
                Some(&fixture.producer),
                Some(&fixture.window),
                true,
                Some(&fixture.row),
                &fixture.identity,
            )
            .is_some()
        );
        assert!(
            stream_delivery_evidence(
                fixture.now,
                fixture.current_date,
                Some(&fixture.producer),
                Some(&fixture.window),
                false,
                Some(&fixture.row),
                &fixture.identity,
            )
            .is_none()
        );
        assert!(
            stream_delivery_evidence(
                fixture.now,
                fixture.current_date,
                Some(&fixture.producer),
                None,
                true,
                Some(&fixture.row),
                &fixture.identity,
            )
            .is_none()
        );
        let mut wrong_day = fixture.producer.clone();
        wrong_day.session_date = Some(fixture.current_date - chrono::Duration::days(1));
        assert!(
            stream_delivery_evidence(
                fixture.now,
                fixture.current_date,
                Some(&wrong_day),
                Some(&fixture.window),
                true,
                Some(&fixture.row),
                &fixture.identity,
            )
            .is_none()
        );
        let mut no_quote = fixture.row.clone();
        no_quote.price = None;
        no_quote.change_amount = None;
        no_quote.change_percent = None;
        no_quote.direction = None;
        no_quote.trade_volume = None;
        no_quote.cumulative_volume = None;
        no_quote.halted = None;
        no_quote.business_date = None;
        no_quote.trade_time = None;
        no_quote.provider_trade_at = None;
        no_quote.received_at = None;
        no_quote.committed_at = None;
        let evidence = stream_delivery_evidence(
            fixture.now,
            fixture.current_date,
            Some(&fixture.producer),
            Some(&fixture.window),
            true,
            Some(&no_quote),
            &fixture.identity,
        )
        .expect("valid lineage produces metadata without a quote");
        assert!(!evidence.quote_eligible);
    }

    #[test]
    fn fresh_and_closed_last_known_quotes_keep_original_capture() {
        let fixture = fixture();
        assert!(eligible(&fixture, fixture.now));

        let mut older = fixture;
        older.window = window_fixture(
            older.current_date,
            older.now - chrono::Duration::minutes(1),
            older.now + chrono::Duration::seconds(1),
            older.row.window_contract_sha256.clone(),
        );
        older.producer.current_epoch = Some(Uuid::from_u128(70));
        older.producer.session_proof_id = Some(Uuid::from_u128(71));
        older.producer.gap_generation = older.producer.gap_generation.saturating_add(1);
        let after_close = older.now + chrono::Duration::seconds(31);
        assert!(!older.window.contains(after_close));
        assert!(eligible(&older, after_close));
        let evidence = stream_delivery_evidence(
            after_close,
            older.current_date,
            Some(&older.producer),
            Some(&older.window),
            true,
            Some(&older.row),
            &older.identity,
        )
        .expect("same-day lineage remains available after close");
        assert!(evidence.quote_eligible);
        assert_eq!(evidence.observed_at, after_close);
        assert_eq!(evidence.open_at, older.window.open_at());
        assert_eq!(evidence.close_at, older.window.close_at());
    }

    #[test]
    fn grant_day_calendar_and_hash_mismatches_deny_quote() {
        let fixture = fixture();
        assert!(eligible(&fixture, fixture.now));

        let mut wrong_grant = fixture.row.clone();
        wrong_grant.grant_id = Uuid::nil();
        assert!(!snapshot_quote_is_eligible(
            &wrong_grant,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        let mut wrong_revision = fixture.row.clone();
        wrong_revision.grant_revision = Uuid::nil();
        assert!(!snapshot_quote_is_eligible(
            &wrong_revision,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        let mut wrong_day = fixture.row.clone();
        wrong_day.session_date -= chrono::Duration::days(1);
        assert!(!snapshot_quote_is_eligible(
            &wrong_day,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        for mutate in 0..4 {
            let mut wrong_lineage = fixture.row.clone();
            match mutate {
                0 => wrong_lineage.calendar_source = "other".to_owned(),
                1 => wrong_lineage.calendar_source_version = "schema-2".to_owned(),
                2 => wrong_lineage.calendar_content_sha256 = "d".repeat(64),
                _ => wrong_lineage.session_proof_sha256 = "e".repeat(64),
            }
            assert!(!snapshot_quote_is_eligible(
                &wrong_lineage,
                &fixture.identity,
                &fixture.producer,
                &fixture.window,
                fixture.now,
                fixture.current_date,
            ));
        }
        let mut wrong_window = fixture.row.clone();
        wrong_window.window_contract_sha256 = "f".repeat(64);
        assert!(!snapshot_quote_is_eligible(
            &wrong_window,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        let mut wrong_batch = fixture.row.clone();
        wrong_batch.calendar_source_batch_id = Uuid::from_u128(93);
        assert!(!snapshot_quote_is_eligible(
            &wrong_batch,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        let mut base_price = fixture.row.clone();
        base_price.base_price = Some("99".to_owned());
        assert!(!snapshot_quote_is_eligible(
            &base_price,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        let mut wrong_slot = fixture.row.clone();
        wrong_slot.credential_slot_id = Uuid::nil();
        assert!(!snapshot_quote_is_eligible(
            &wrong_slot,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        let mut wrong_owner = fixture.row.clone();
        wrong_owner.owner_user_id = Uuid::from_u128(94);
        assert!(!snapshot_quote_is_eligible(
            &wrong_owner,
            &fixture.identity,
            &fixture.producer,
            &fixture.window,
            fixture.now,
            fixture.current_date,
        ));
        // Cross-grant/revision substitution is rejected by the immutable
        // composite FK and app RLS, not by a forbidden private-table read.
        // Those boundaries require the separate actual-role DB acceptance.
    }

    #[test]
    fn capture_times_must_be_complete_current_and_bounded() {
        let fixture = fixture();
        let mut cases = Vec::new();
        for field in 0..3 {
            let mut row = fixture.row.clone();
            match field {
                0 => row.provider_trade_at = None,
                1 => row.received_at = None,
                _ => row.committed_at = None,
            }
            cases.push(row);
        }
        for field in 0..3 {
            let mut row = fixture.row.clone();
            match field {
                0 => row.provider_trade_at = Some(fixture.now + chrono::Duration::milliseconds(1)),
                1 => row.received_at = Some(fixture.now + chrono::Duration::milliseconds(1)),
                _ => row.committed_at = Some(fixture.now + chrono::Duration::milliseconds(1)),
            }
            cases.push(row);
        }
        let mut late_commit = fixture.row.clone();
        late_commit.received_at = Some(fixture.now - chrono::Duration::seconds(5));
        late_commit.provider_trade_at = late_commit.received_at;
        late_commit.committed_at = late_commit
            .received_at
            .map(|at| at + chrono::Duration::seconds(4));
        cases.push(late_commit);
        let mut future_provider = fixture.row.clone();
        future_provider.provider_trade_at = future_provider
            .received_at
            .map(|at| at + chrono::Duration::seconds(3));
        cases.push(future_provider);
        let mut wrong_business_day = fixture.row.clone();
        wrong_business_day.business_date = Some(fixture.current_date - chrono::Duration::days(1));
        cases.push(wrong_business_day);
        let mut wrong_kst_time = fixture.row.clone();
        wrong_kst_time.provider_trade_at = Some(fixture.now - chrono::Duration::minutes(1));
        cases.push(wrong_kst_time);
        for row in cases {
            assert!(!snapshot_quote_is_eligible(
                &row,
                &fixture.identity,
                &fixture.producer,
                &fixture.window,
                fixture.now,
                fixture.current_date,
            ));
        }

        let prior_kst_day = fixture.now - chrono::Duration::hours(10);
        let mut wrong_provider_date = fixture.row.clone();
        wrong_provider_date.provider_trade_at = Some(prior_kst_day);
        wrong_provider_date.received_at = Some(prior_kst_day);
        wrong_provider_date.committed_at = Some(prior_kst_day);
        wrong_provider_date.trade_time = Some(
            prior_kst_day
                .with_timezone(&FixedOffset::east_opt(KST_OFFSET_SECONDS).expect("KST offset"))
                .time(),
        );
        let wide_window = window_fixture(
            fixture.current_date,
            prior_kst_day - chrono::Duration::seconds(1),
            fixture.window.close_at(),
            fixture.row.window_contract_sha256.clone(),
        );
        assert!(!snapshot_quote_is_eligible(
            &wrong_provider_date,
            &fixture.identity,
            &fixture.producer,
            &wide_window,
            fixture.now,
            fixture.current_date,
        ));

        for outside in [
            fixture.window.open_at() - chrono::Duration::milliseconds(1),
            fixture.window.close_at(),
        ] {
            let mut row = fixture.row.clone();
            row.received_at = Some(outside);
            row.committed_at = Some(outside);
            row.provider_trade_at = Some(outside);
            row.business_date = Some(
                outside
                    .with_timezone(&FixedOffset::east_opt(KST_OFFSET_SECONDS).expect("KST offset"))
                    .date_naive(),
            );
            row.trade_time = Some(
                outside
                    .with_timezone(&FixedOffset::east_opt(KST_OFFSET_SECONDS).expect("KST offset"))
                    .time(),
            );
            assert!(!snapshot_quote_is_eligible(
                &row,
                &fixture.identity,
                &fixture.producer,
                &fixture.window,
                fixture.now,
                fixture.current_date,
            ));
        }
    }

    #[test]
    fn evidence_copies_validated_window_and_read_timestamp() {
        let fixture = fixture();
        let evidence = stream_delivery_evidence(
            fixture.now,
            fixture.current_date,
            Some(&fixture.producer),
            Some(&fixture.window),
            true,
            Some(&fixture.row),
            &fixture.identity,
        )
        .expect("current validated evidence");
        assert_eq!(evidence.observed_at, fixture.now);
        assert_eq!(evidence.session_date, fixture.current_date);
        assert_eq!(
            evidence.calendar_content_sha256,
            fixture.producer.calendar_content_sha256.unwrap()
        );
        assert_eq!(
            evidence.window_contract_sha256,
            fixture.window.contract_sha256()
        );
        assert!(fixture.window.contains(fixture.now));
        assert!(evidence.quote_eligible);
    }
}
