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
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
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
        if owner_user_id.is_nil()
            || lease_id.is_nil()
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
        if row.session_hash.as_str() != session_hash {
            return Err(MarketStreamStorageError::LeaseSessionMismatch);
        }
        if row.state == "RELEASED" {
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
                            current_epoch = NULL, session_date = NULL,
                            session_proof_id = NULL, session_proof_sha256 = NULL,
                            calendar_source = NULL, calendar_source_version = NULL,
                            calendar_source_batch_id = NULL,
                            calendar_content_sha256 = NULL,
                            window_contract_sha256 = NULL,
                            gap_generation = CASE WHEN $5 THEN $6 ELSE gap_generation END,
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
                    .bind(
                        row.gap_generation
                            .checked_add(1)
                            .ok_or(MarketStreamStorageError::FenceExhausted)?,
                    )
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
        if kst_database_date(&mut tx).await? != session.session_date {
            return Err(MarketStreamStorageError::SessionInvalid);
        }
        let fresh_now = fresh_database_time(&mut tx).await?;
        if row.lease_expires_at <= fresh_now + chrono::Duration::seconds(5) {
            return Err(MarketStreamStorageError::ProducerLost);
        }
        let next_gap = row
            .gap_generation
            .checked_add(1)
            .ok_or(MarketStreamStorageError::VersionExhausted)?;
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
                    connection_state = 'CONNECTED', gap_generation = $9,
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
        .bind(next_gap)
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
            gap_generation: u64::try_from(next_gap)
                .map_err(|_| MarketStreamStorageError::VersionExhausted)?,
        };
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
        let locked = lock_publication_inputs(&mut tx, context, true).await?;
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
            rows.push(row.into_public()?);
            changed = true;
        }
        if changed {
            notify_changed(&mut tx).await?;
        }
        let precommit_now = fresh_database_time(&mut tx).await?;
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
        match tx.commit().await {
            Ok(()) => Ok(CommitResult { changed, rows }),
            Err(_) => {
                // A commit error is indeterminate.  Do one read of committed
                // epoch/ordinal/version state before reporting uncertainty;
                // never fan out the in-memory value or blindly retry.
                match self.reread_publication_outcome(context, observations).await {
                    Ok(Some(committed)) => Ok(CommitResult {
                        changed: false,
                        rows: committed,
                    }),
                    Ok(None) | Err(_) => Err(MarketStreamStorageError::CommitUnknown),
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
        let locked = lock_publication_inputs(&mut tx, context, false).await?;
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
        if owner_user_id.is_nil() || lease_id.is_nil() || !canonical_session_hash(session_hash) {
            return Err(MarketStreamStorageError::InvalidInput);
        }
        let mut tx = self
            .begin_actor_transaction(owner_user_id, session_hash)
            .await?;
        let lease: LeaseDbRow = sqlx::query_as(
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
        .await
        .map_err(map_database_error)?
        .ok_or(MarketStreamStorageError::LeaseNotFound)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        if lease.state != "ACTIVE" {
            return Err(MarketStreamStorageError::LeaseReleased);
        }
        if lease.lease_expires_at <= fresh_now {
            return Err(MarketStreamStorageError::LeaseExpired);
        }
        let rows: Vec<CacheDbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CACHE_SELECT}
               FROM public.owner_market_stream_cache AS cache
               JOIN public.owner_market_stream_lease_items AS item
                 ON item.owner_user_id = cache.owner_user_id
                AND item.membership_id = cache.membership_id
                AND item.generation_id = cache.generation_id
                AND item.instrument_id = cache.instrument_id
                AND item.generation = cache.generation
              WHERE item.lease_id = $1 AND item.owner_user_id = $2
              ORDER BY item.instrument_id, item.membership_id"
        )))
        .bind(lease_id)
        .bind(owner_user_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let rows = rows
            .into_iter()
            .map(CacheDbRow::into_public)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit()
            .await
            .map_err(|_| MarketStreamStorageError::CommitUnknown)?;
        Ok(StreamSnapshot {
            lease_id,
            lease_expires_at: lease.lease_expires_at,
            rows,
        })
    }

    async fn reread_publication_outcome(
        &self,
        context: &StreamPublicationContext,
        observations: &[StreamPublicationObservation],
    ) -> Result<Option<Vec<StreamCacheRow>>, MarketStreamStorageError> {
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

    async fn begin_worker_transaction(
        &self,
    ) -> Result<Transaction<'_, Postgres>, MarketStreamStorageError> {
        let mut tx = self.pool.begin().await.map_err(map_database_error)?;
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
    Ok(LockedPublication {
        producer,
        admissions,
        leases,
        subscriptions,
        caches,
        fresh_now,
    })
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
