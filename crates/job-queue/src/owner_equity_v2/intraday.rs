//! Typed, transactional storage for owner-private intraday quote state.
//!
//! This module is deliberately provider-free.  The future WP-3B2 loop brings
//! a parsed [`market_data::intraday_quotes::IntradayQuote`], an explicit
//! session proof, and an attempt reservation to this boundary.  Every write
//! then rechecks the producer fence, current 0053 admission, active demand,
//! and the database calendar lineage in one worker transaction.

use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use market_data::intraday_quotes::{IntradayQuote, IntradayQuoteDirection};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::database_error_class;
use crate::types::ErrorClass;

pub const INTRADAY_SCHEMA_VERSION: u32 = 1;
pub const DEMAND_LEASE_SECONDS: i64 = 30;
pub const DEMAND_RENEW_AFTER_MS: i64 = 15_000;
pub const MAX_ACTIVE_CONSUMERS: i64 = 20;
pub const MAX_ACTIVE_IDENTITIES: i64 = 5;
pub const PRODUCER_LEASE_SECONDS: i64 = 20;
pub const PRODUCER_HEARTBEAT_SECONDS: i64 = 5;
pub const CACHE_RETENTION_SECONDS: i64 = 24 * 60 * 60;
pub const SESSION_PROOF_MAX_AGE_HOURS: i64 = 36;

const KRX_SOURCE: &str = "kis";
const KRX_SOURCE_VERSION: &str = "kis-chk-holiday-v1:schema-1";

/// Typed storage failures.  No variant carries SQL text, provider prose, a
/// response body, or a caller credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IntradayStorageError {
    #[error("INTRADAY_INPUT_INVALID")]
    InvalidInput,
    #[error("INTRADAY_DEMAND_NOT_FOUND")]
    DemandNotFound,
    #[error("INTRADAY_DEMAND_RELEASED")]
    DemandReleased,
    #[error("IDEMPOTENCY_MISMATCH")]
    IdempotencyMismatch,
    #[error("QUOTE_DEMAND_SEQUENCE_CONFLICT")]
    SequenceConflict,
    #[error("INTRADAY_DEMAND_IDENTITY_MISMATCH")]
    IdentityMismatch,
    #[error("MEMBERSHIP_NOT_READY")]
    MembershipNotReady,
    #[error("QUOTE_DEMAND_CAPACITY")]
    DemandCapacity,
    #[error("QUOTE_DEMAND_IDENTITY_CAPACITY")]
    IdentityCapacity,
    #[error("INTRADAY_POLICY_UNAVAILABLE")]
    PolicyUnavailable,
    #[error("INTRADAY_PRODUCER_LEASE_HELD")]
    ProducerLeaseHeld,
    #[error("INTRADAY_PRODUCER_LEASE_LOST")]
    ProducerLeaseLost,
    #[error("INTRADAY_ACTIVE_DEMAND_REQUIRED")]
    ActiveDemandRequired,
    #[error("INTRADAY_SESSION_PROOF_INVALID")]
    SessionProofInvalid,
    #[error("INTRADAY_CALENDAR_PROOF_UNAVAILABLE")]
    CalendarProofUnavailable,
    #[error("INTRADAY_BUDGET_PROOF_INVALID")]
    BudgetProofInvalid,
    #[error("INTRADAY_RECEIPT_INVALID")]
    ReceiptInvalid,
    #[error("INTRADAY_QUOTE_INVALID")]
    QuoteInvalid,
    #[error("INTRADAY_QUOTE_RECEIPT_STALE")]
    QuoteReceiptStale,
    #[error("INTRADAY_CACHE_NOT_FOUND")]
    CacheNotFound,
    #[error("INTRADAY_QUOTE_VERSION_EXHAUSTED")]
    QuoteVersionExhausted,
    #[error("INTRADAY_PRODUCER_FENCE_EXHAUSTED")]
    ProducerFenceExhausted,
    #[error("INTRADAY_DATABASE_UNAVAILABLE")]
    DatabaseUnavailable,
    #[error("INTRADAY_DATABASE_INTEGRITY")]
    DatabaseIntegrity,
    #[error("INTRADAY_PERMISSION_DENIED")]
    PermissionDenied,
    #[error("INTRADAY_COMMIT_OUTCOME_UNKNOWN")]
    CommitUnknown,
}

impl IntradayStorageError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "INTRADAY_INPUT_INVALID",
            Self::DemandNotFound => "INTRADAY_DEMAND_NOT_FOUND",
            Self::DemandReleased => "INTRADAY_DEMAND_RELEASED",
            Self::IdempotencyMismatch => "IDEMPOTENCY_MISMATCH",
            Self::SequenceConflict => "QUOTE_DEMAND_SEQUENCE_CONFLICT",
            Self::IdentityMismatch => "INTRADAY_DEMAND_IDENTITY_MISMATCH",
            Self::MembershipNotReady => "MEMBERSHIP_NOT_READY",
            Self::DemandCapacity => "QUOTE_DEMAND_CAPACITY",
            Self::IdentityCapacity => "QUOTE_DEMAND_IDENTITY_CAPACITY",
            Self::PolicyUnavailable => "INTRADAY_POLICY_UNAVAILABLE",
            Self::ProducerLeaseHeld => "INTRADAY_PRODUCER_LEASE_HELD",
            Self::ProducerLeaseLost => "INTRADAY_PRODUCER_LEASE_LOST",
            Self::ActiveDemandRequired => "INTRADAY_ACTIVE_DEMAND_REQUIRED",
            Self::SessionProofInvalid => "INTRADAY_SESSION_PROOF_INVALID",
            Self::CalendarProofUnavailable => "INTRADAY_CALENDAR_PROOF_UNAVAILABLE",
            Self::BudgetProofInvalid => "INTRADAY_BUDGET_PROOF_INVALID",
            Self::ReceiptInvalid => "INTRADAY_RECEIPT_INVALID",
            Self::QuoteInvalid => "INTRADAY_QUOTE_INVALID",
            Self::QuoteReceiptStale => "INTRADAY_QUOTE_RECEIPT_STALE",
            Self::CacheNotFound => "INTRADAY_CACHE_NOT_FOUND",
            Self::QuoteVersionExhausted => "INTRADAY_QUOTE_VERSION_EXHAUSTED",
            Self::ProducerFenceExhausted => "INTRADAY_PRODUCER_FENCE_EXHAUSTED",
            Self::DatabaseUnavailable => "INTRADAY_DATABASE_UNAVAILABLE",
            Self::DatabaseIntegrity => "INTRADAY_DATABASE_INTEGRITY",
            Self::PermissionDenied => "INTRADAY_PERMISSION_DENIED",
            Self::CommitUnknown => "INTRADAY_COMMIT_OUTCOME_UNKNOWN",
        }
    }

    pub const fn class(self) -> ErrorClass {
        match self {
            Self::DatabaseUnavailable | Self::CommitUnknown => ErrorClass::Transient,
            Self::DemandCapacity
            | Self::IdentityCapacity
            | Self::ProducerLeaseHeld
            | Self::ProducerLeaseLost
            | Self::ActiveDemandRequired
            | Self::CalendarProofUnavailable => ErrorClass::DataBlocked,
            Self::InvalidInput
            | Self::DemandNotFound
            | Self::DemandReleased
            | Self::IdempotencyMismatch
            | Self::SequenceConflict
            | Self::IdentityMismatch
            | Self::MembershipNotReady
            | Self::PolicyUnavailable
            | Self::SessionProofInvalid
            | Self::BudgetProofInvalid
            | Self::ReceiptInvalid
            | Self::QuoteInvalid
            | Self::QuoteReceiptStale
            | Self::CacheNotFound
            | Self::QuoteVersionExhausted
            | Self::ProducerFenceExhausted
            | Self::DatabaseIntegrity
            | Self::PermissionDenied => ErrorClass::Integrity,
        }
    }
}

/// The exact owner/membership/generation identity admitted by migration 0053.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayQuoteIdentity {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
}

impl IntradayQuoteIdentity {
    pub fn new(
        owner_user_id: Uuid,
        membership_id: Uuid,
        generation_id: Uuid,
        instrument_id: String,
        generation: u64,
    ) -> Result<Self, IntradayStorageError> {
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

    fn validate(&self) -> Result<(), IntradayStorageError> {
        if self.owner_user_id.is_nil()
            || self.membership_id.is_nil()
            || self.generation_id.is_nil()
            || !canonical_instrument(&self.instrument_id)
            || self.generation == 0
            || i64::try_from(self.generation).is_err()
        {
            return Err(IntradayStorageError::InvalidInput);
        }
        Ok(())
    }

    pub fn symbol(&self) -> &str {
        self.instrument_id.get(..6).unwrap_or("")
    }

    fn generation_i64(&self) -> i64 {
        // `validate` proves this conversion before any SQL call.
        i64::try_from(self.generation).expect("validated generation fits PostgreSQL bigint")
    }
}

/// Input for POST create/renew.  The raw idempotency key is transient and is
/// never placed in the database; only its SHA-256 digest is stored.
pub struct IntradayQuoteDemandRequest {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub membership_id: Uuid,
    pub generation: u64,
    pub renewal_sequence: u64,
    idempotency_key: String,
}

impl IntradayQuoteDemandRequest {
    pub fn new(
        consumer_id: Uuid,
        membership_id: Uuid,
        generation: u64,
        renewal_sequence: u64,
        idempotency_key: String,
    ) -> Result<Self, IntradayStorageError> {
        let request = Self {
            schema_version: INTRADAY_SCHEMA_VERSION,
            consumer_id,
            membership_id,
            generation,
            renewal_sequence,
            idempotency_key,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn with_schema_version(
        schema_version: u32,
        consumer_id: Uuid,
        membership_id: Uuid,
        generation: u64,
        renewal_sequence: u64,
        idempotency_key: String,
    ) -> Result<Self, IntradayStorageError> {
        let request = Self {
            schema_version,
            consumer_id,
            membership_id,
            generation,
            renewal_sequence,
            idempotency_key,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    fn validate(&self) -> Result<(), IntradayStorageError> {
        if self.schema_version != INTRADAY_SCHEMA_VERSION
            || self.consumer_id.is_nil()
            || self.membership_id.is_nil()
            || self.generation == 0
            || i64::try_from(self.generation).is_err()
            || i64::try_from(self.renewal_sequence).is_err()
            || !valid_idempotency_key(&self.idempotency_key)
        {
            return Err(IntradayStorageError::InvalidInput);
        }
        Ok(())
    }

    fn idempotency_digest(&self) -> String {
        prefixed_sha256(self.idempotency_key.as_bytes())
    }

    fn request_digest(&self) -> String {
        prefixed_sha256(
            format!(
                "{{\"schema_version\":{},\"consumer_id\":\"{}\",\"membership_id\":\"{}\",\"generation\":{},\"renewal_sequence\":{}}}",
                self.schema_version,
                self.consumer_id,
                self.membership_id,
                self.generation,
                self.renewal_sequence
            )
            .as_bytes(),
        )
    }

    fn generation_i64(&self) -> i64 {
        i64::try_from(self.generation).expect("validated generation fits PostgreSQL bigint")
    }

    fn sequence_i64(&self) -> i64 {
        i64::try_from(self.renewal_sequence)
            .expect("validated renewal sequence fits PostgreSQL bigint")
    }
}

impl fmt::Debug for IntradayQuoteDemandRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntradayQuoteDemandRequest")
            .field("schema_version", &self.schema_version)
            .field("consumer_id", &self.consumer_id)
            .field("membership_id", &self.membership_id)
            .field("generation", &self.generation)
            .field("renewal_sequence", &self.renewal_sequence)
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

/// Input for DELETE.  As with POST, only the idempotency digest is durable.
pub struct IntradayQuoteReleaseRequest {
    pub schema_version: u32,
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
    idempotency_key: String,
}

impl IntradayQuoteReleaseRequest {
    pub fn new(
        consumer_id: Uuid,
        renewal_sequence: u64,
        idempotency_key: String,
    ) -> Result<Self, IntradayStorageError> {
        let request = Self {
            schema_version: INTRADAY_SCHEMA_VERSION,
            consumer_id,
            renewal_sequence,
            idempotency_key,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), IntradayStorageError> {
        if self.schema_version != INTRADAY_SCHEMA_VERSION
            || self.consumer_id.is_nil()
            || i64::try_from(self.renewal_sequence).is_err()
            || !valid_idempotency_key(&self.idempotency_key)
        {
            return Err(IntradayStorageError::InvalidInput);
        }
        Ok(())
    }

    fn idempotency_digest(&self) -> String {
        prefixed_sha256(self.idempotency_key.as_bytes())
    }

    fn request_digest(&self) -> String {
        prefixed_sha256(
            format!(
                "{{\"schema_version\":{},\"consumer_id\":\"{}\",\"renewal_sequence\":{}}}",
                self.schema_version, self.consumer_id, self.renewal_sequence
            )
            .as_bytes(),
        )
    }

    fn sequence_i64(&self) -> i64 {
        i64::try_from(self.renewal_sequence)
            .expect("validated renewal sequence fits PostgreSQL bigint")
    }
}

impl fmt::Debug for IntradayQuoteReleaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntradayQuoteReleaseRequest")
            .field("schema_version", &self.schema_version)
            .field("consumer_id", &self.consumer_id)
            .field("renewal_sequence", &self.renewal_sequence)
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandMutationKind {
    Created,
    Renewed,
    Replayed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayQuoteDemandLease {
    pub demand_id: Uuid,
    pub owner_user_id: Uuid,
    pub consumer_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
    pub renewal_sequence: u64,
    pub lease_expires_at: DateTime<Utc>,
    pub renew_after_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemandMutationOutcome {
    pub kind: DemandMutationKind,
    pub lease: IntradayQuoteDemandLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandReleaseKind {
    Released,
    Replayed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemandReleaseOutcome {
    pub kind: DemandReleaseKind,
    pub demand_id: Uuid,
}

/// The exact KIS calendar lineage and the explicit, commit-pinned session
/// window digest required by the future producer.  The storage layer never
/// invents a default window digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradaySessionProof {
    pub session_date: NaiveDate,
    pub calendar_source_batch_id: Uuid,
    pub calendar_content_sha256: String,
    pub window_contract_sha256: String,
}

impl IntradaySessionProof {
    pub fn new(
        session_date: NaiveDate,
        calendar_source_batch_id: Uuid,
        calendar_content_sha256: String,
        window_contract_sha256: String,
    ) -> Result<Self, IntradayStorageError> {
        let proof = Self {
            session_date,
            calendar_source_batch_id,
            calendar_content_sha256,
            window_contract_sha256,
        };
        proof.validate()?;
        Ok(proof)
    }

    fn validate(&self) -> Result<(), IntradayStorageError> {
        if self.calendar_source_batch_id.is_nil()
            || !canonical_unprefixed_sha256(&self.calendar_content_sha256)
            || !canonical_prefixed_sha256(&self.window_contract_sha256)
        {
            return Err(IntradayStorageError::SessionProofInvalid);
        }
        Ok(())
    }

    pub fn calendar_source(&self) -> &'static str {
        KRX_SOURCE
    }

    pub fn calendar_source_version(&self) -> &'static str {
        KRX_SOURCE_VERSION
    }
}

/// A reservation created by the shared read/budget boundary.  It is
/// intentionally required by every fenced publication method; this module
/// does not fabricate a budget success or maintain a second quota ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayAttemptReservation {
    pub reservation_id: Uuid,
    pub session_date: NaiveDate,
    pub attempt_number: u64,
}

impl IntradayAttemptReservation {
    pub fn new(
        reservation_id: Uuid,
        session_date: NaiveDate,
        attempt_number: u64,
    ) -> Result<Self, IntradayStorageError> {
        if reservation_id.is_nil() || attempt_number == 0 {
            return Err(IntradayStorageError::BudgetProofInvalid);
        }
        Ok(Self {
            reservation_id,
            session_date,
            attempt_number,
        })
    }

    fn validate(&self) -> Result<(), IntradayStorageError> {
        if self.reservation_id.is_nil() || self.attempt_number == 0 {
            return Err(IntradayStorageError::BudgetProofInvalid);
        }
        Ok(())
    }
}

/// Receipt timestamp captured by the future transport caller after complete
/// response bytes arrive.  It is not generated by this repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntradayQuoteReceipt {
    pub received_at: DateTime<Utc>,
}

impl IntradayQuoteReceipt {
    pub fn captured(received_at: DateTime<Utc>) -> Self {
        Self { received_at }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradayQuoteFailureCode {
    NoActiveDemand,
    QuotePending,
    QuoteStale,
    ProviderTimeout,
    ProviderRateLimited,
    ProviderUnavailable,
    ProviderResponseInvalid,
    QuoteValueInvalid,
    QuoteBudgetExhausted,
    CalendarUnavailable,
    SessionWindowUnavailable,
    SessionClosed,
    InstrumentHalted,
    ProducerUnavailable,
    FeatureDisabled,
}

impl IntradayQuoteFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoActiveDemand => "NO_ACTIVE_DEMAND",
            Self::QuotePending => "QUOTE_PENDING",
            Self::QuoteStale => "QUOTE_STALE",
            Self::ProviderTimeout => "PROVIDER_TIMEOUT",
            Self::ProviderRateLimited => "PROVIDER_RATE_LIMITED",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::ProviderResponseInvalid => "PROVIDER_RESPONSE_INVALID",
            Self::QuoteValueInvalid => "QUOTE_VALUE_INVALID",
            Self::QuoteBudgetExhausted => "QUOTE_BUDGET_EXHAUSTED",
            Self::CalendarUnavailable => "CALENDAR_UNAVAILABLE",
            Self::SessionWindowUnavailable => "SESSION_WINDOW_UNAVAILABLE",
            Self::SessionClosed => "SESSION_CLOSED",
            Self::InstrumentHalted => "INSTRUMENT_HALTED",
            Self::ProducerUnavailable => "PRODUCER_UNAVAILABLE",
            Self::FeatureDisabled => "FEATURE_DISABLED",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "NO_ACTIVE_DEMAND" => Self::NoActiveDemand,
            "QUOTE_PENDING" => Self::QuotePending,
            "QUOTE_STALE" => Self::QuoteStale,
            "PROVIDER_TIMEOUT" => Self::ProviderTimeout,
            "PROVIDER_RATE_LIMITED" => Self::ProviderRateLimited,
            "PROVIDER_UNAVAILABLE" => Self::ProviderUnavailable,
            "PROVIDER_RESPONSE_INVALID" => Self::ProviderResponseInvalid,
            "QUOTE_VALUE_INVALID" => Self::QuoteValueInvalid,
            "QUOTE_BUDGET_EXHAUSTED" => Self::QuoteBudgetExhausted,
            "CALENDAR_UNAVAILABLE" => Self::CalendarUnavailable,
            "SESSION_WINDOW_UNAVAILABLE" => Self::SessionWindowUnavailable,
            "SESSION_CLOSED" => Self::SessionClosed,
            "INSTRUMENT_HALTED" => Self::InstrumentHalted,
            "PRODUCER_UNAVAILABLE" => Self::ProducerUnavailable,
            "FEATURE_DISABLED" => Self::FeatureDisabled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerLease {
    pub owner_user_id: Uuid,
    pub holder_id: Uuid,
    pub fencing_token: u64,
    pub lease_expires_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProducerClaimKind {
    Acquired,
    AlreadyHeld,
    TakenOver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerClaimOutcome {
    pub kind: ProducerClaimKind,
    pub lease: ProducerLease,
}

/// All prerequisites for one success/failure publication.  The constructors
/// make the session and budget proofs explicit in the WP-3B2 call graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayPublicationContext {
    pub producer: ProducerLease,
    pub identity: IntradayQuoteIdentity,
    pub session: IntradaySessionProof,
    pub attempt: IntradayAttemptReservation,
}

impl IntradayPublicationContext {
    pub fn new(
        producer: ProducerLease,
        identity: IntradayQuoteIdentity,
        session: IntradaySessionProof,
        attempt: IntradayAttemptReservation,
    ) -> Result<Self, IntradayStorageError> {
        identity.validate()?;
        session.validate()?;
        attempt.validate()?;
        validate_lease(&producer)?;
        if producer.owner_user_id != identity.owner_user_id
            || attempt.session_date != session.session_date
        {
            return Err(IntradayStorageError::InvalidInput);
        }
        let context = Self {
            producer,
            identity,
            session,
            attempt,
        };
        context.validate()?;
        Ok(context)
    }

    fn validate(&self) -> Result<(), IntradayStorageError> {
        self.identity.validate()?;
        self.session.validate()?;
        self.attempt.validate()?;
        validate_lease(&self.producer)?;
        if self.producer.owner_user_id != self.identity.owner_user_id
            || self.attempt.session_date != self.session.session_date
        {
            return Err(IntradayStorageError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayCacheRecord {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
    pub session_date: Option<NaiveDate>,
    pub calendar_source: Option<String>,
    pub calendar_source_version: Option<String>,
    pub calendar_source_batch_id: Option<Uuid>,
    pub calendar_content_sha256: Option<String>,
    pub window_contract_sha256: Option<String>,
    pub price: Option<String>,
    pub base_price: Option<String>,
    pub change_amount: Option<String>,
    pub change_percent: Option<String>,
    pub direction: Option<IntradayQuoteDirection>,
    pub halted: Option<bool>,
    pub received_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub quote_version: u64,
    pub last_attempt_at: DateTime<Utc>,
    pub last_failure_code: Option<IntradayQuoteFailureCode>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub producer_fence: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntradayGcReport {
    pub demand_rows_deleted: u64,
    pub cache_rows_deleted: u64,
}

#[derive(Debug, Clone)]
pub struct OwnerIntradayQuoteRepository {
    pool: PgPool,
}

impl OwnerIntradayQuoteRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a new sequence-zero demand or renew the exact next sequence.
    /// Exact replay returns the original database expiry without extending it.
    pub async fn create_or_renew_demand(
        &self,
        owner_user_id: Uuid,
        request: &IntradayQuoteDemandRequest,
    ) -> Result<DemandMutationOutcome, IntradayStorageError> {
        if owner_user_id.is_nil() {
            return Err(IntradayStorageError::InvalidInput);
        }
        request.validate()?;
        let key_digest = request.idempotency_digest();
        let body_digest = request.request_digest();
        let mut tx = self.begin_actor_transaction(owner_user_id).await?;

        lock_owner_policy_for_demand(&mut tx, owner_user_id).await?;
        let existing: Option<DemandDbRow> = sqlx::query_as(
            "SELECT id, owner_user_id, consumer_id, membership_id, generation_id,
                    instrument_id, generation, state, renewal_sequence,
                    lease_expires_at, idempotency_key_sha256, request_sha256
             FROM public.owner_intraday_quote_demands
             WHERE owner_user_id = $1 AND consumer_id = $2
             FOR UPDATE
             ",
        )
        .bind(owner_user_id)
        .bind(request.consumer_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;

        if let Some(row) = existing.as_ref() {
            if row.state == "RELEASED" {
                if row.idempotency_key_sha256 == key_digest
                    && row.request_sha256 == body_digest
                    && row.renewal_sequence == request.sequence_i64()
                {
                    return Err(IntradayStorageError::DemandReleased);
                }
                if row.idempotency_key_sha256 == key_digest && row.request_sha256 != body_digest {
                    return Err(IntradayStorageError::IdempotencyMismatch);
                }
                return Err(IntradayStorageError::DemandReleased);
            }

            if row.idempotency_key_sha256 == key_digest {
                if row.request_sha256 != body_digest {
                    return Err(IntradayStorageError::IdempotencyMismatch);
                }
                if row.renewal_sequence == request.sequence_i64() {
                    let lease = row.to_lease()?;
                    tx.commit()
                        .await
                        .map_err(|_| IntradayStorageError::CommitUnknown)?;
                    return Ok(DemandMutationOutcome {
                        kind: DemandMutationKind::Replayed,
                        lease,
                    });
                }
            }

            let admission = lock_ready_admission(
                &mut tx,
                owner_user_id,
                request.membership_id,
                request.generation_i64(),
            )
            .await?;
            if row.membership_id != admission.membership_id
                || row.generation_id != admission.generation_id
                || row.instrument_id != admission.instrument_id
                || row.generation != admission.generation
            {
                return Err(IntradayStorageError::IdentityMismatch);
            }
            let expected_next_sequence = row
                .renewal_sequence
                .checked_add(1)
                .ok_or(IntradayStorageError::SequenceConflict)?;
            if request.sequence_i64() != expected_next_sequence {
                return Err(IntradayStorageError::SequenceConflict);
            }

            let fresh_now = fresh_database_time(&mut tx).await?;
            let still_active = row.lease_expires_at > fresh_now;
            if !still_active {
                ensure_active_capacity(&mut tx, owner_user_id, Some(row.id), &admission, fresh_now)
                    .await?;
            }

            let updated: DemandDbRow = sqlx::query_as(
                "UPDATE public.owner_intraday_quote_demands
                    SET renewal_sequence = $3,
                        lease_expires_at = $6 + INTERVAL '30 seconds',
                        idempotency_key_sha256 = $4,
                        request_sha256 = $5,
                        updated_at = $6
                  WHERE id = $1 AND owner_user_id = $2 AND state = 'ACTIVE'
                  RETURNING id, owner_user_id, consumer_id, membership_id,
                            generation_id, instrument_id, generation, state,
                            renewal_sequence, lease_expires_at,
                            idempotency_key_sha256, request_sha256",
            )
            .bind(row.id)
            .bind(owner_user_id)
            .bind(request.sequence_i64())
            .bind(&key_digest)
            .bind(&body_digest)
            .bind(fresh_now)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database_error)?;
            let lease = updated.to_lease()?;
            tx.commit()
                .await
                .map_err(|_| IntradayStorageError::CommitUnknown)?;
            return Ok(DemandMutationOutcome {
                kind: DemandMutationKind::Renewed,
                lease,
            });
        }

        if request.renewal_sequence != 0 {
            return Err(IntradayStorageError::DemandNotFound);
        }
        let admission = lock_ready_admission(
            &mut tx,
            owner_user_id,
            request.membership_id,
            request.generation_i64(),
        )
        .await?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        ensure_active_capacity(&mut tx, owner_user_id, None, &admission, fresh_now).await?;

        let inserted: DemandDbRow = sqlx::query_as(
            "INSERT INTO public.owner_intraday_quote_demands
                (owner_user_id, consumer_id, membership_id, generation_id,
                 instrument_id, generation, state, renewal_sequence,
                 lease_expires_at, idempotency_key_sha256, request_sha256)
             VALUES ($1, $2, $3, $4, $5, $6, 'ACTIVE', 0,
                     $9 + INTERVAL '30 seconds', $7, $8)
             RETURNING id, owner_user_id, consumer_id, membership_id,
                       generation_id, instrument_id, generation, state,
                       renewal_sequence, lease_expires_at,
                       idempotency_key_sha256, request_sha256",
        )
        .bind(owner_user_id)
        .bind(request.consumer_id)
        .bind(admission.membership_id)
        .bind(admission.generation_id)
        .bind(&admission.instrument_id)
        .bind(admission.generation)
        .bind(&key_digest)
        .bind(&body_digest)
        .bind(fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let lease = inserted.to_lease()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(DemandMutationOutcome {
            kind: DemandMutationKind::Created,
            lease,
        })
    }

    /// Release only the addressed demand row.  A release never deletes a
    /// sibling consumer and a RELEASED row is never resurrected.
    pub async fn release_demand(
        &self,
        owner_user_id: Uuid,
        demand_id: Uuid,
        request: &IntradayQuoteReleaseRequest,
    ) -> Result<DemandReleaseOutcome, IntradayStorageError> {
        if owner_user_id.is_nil() || demand_id.is_nil() {
            return Err(IntradayStorageError::InvalidInput);
        }
        request.validate()?;
        let key_digest = request.idempotency_digest();
        let body_digest = request.request_digest();
        let mut tx = self.begin_actor_transaction(owner_user_id).await?;
        lock_owner_policy_for_demand(&mut tx, owner_user_id).await?;
        let row: Option<DemandDbRow> = sqlx::query_as(
            "SELECT id, owner_user_id, consumer_id, membership_id, generation_id,
                    instrument_id, generation, state, renewal_sequence,
                    lease_expires_at, idempotency_key_sha256, request_sha256
             FROM public.owner_intraday_quote_demands
             WHERE id = $1 AND owner_user_id = $2
             FOR UPDATE
             ",
        )
        .bind(demand_id)
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let row = row.ok_or(IntradayStorageError::DemandNotFound)?;
        if row.consumer_id != request.consumer_id {
            return Err(IntradayStorageError::DemandNotFound);
        }

        if row.state == "RELEASED" {
            if row.idempotency_key_sha256 == key_digest
                && row.request_sha256 == body_digest
                && row.renewal_sequence == request.sequence_i64()
            {
                tx.commit()
                    .await
                    .map_err(|_| IntradayStorageError::CommitUnknown)?;
                return Ok(DemandReleaseOutcome {
                    kind: DemandReleaseKind::Replayed,
                    demand_id,
                });
            }
            if row.idempotency_key_sha256 == key_digest && row.request_sha256 != body_digest {
                return Err(IntradayStorageError::IdempotencyMismatch);
            }
            return Err(IntradayStorageError::DemandReleased);
        }
        if row.idempotency_key_sha256 == key_digest && row.request_sha256 != body_digest {
            return Err(IntradayStorageError::IdempotencyMismatch);
        }
        if row.renewal_sequence != request.sequence_i64() {
            return Err(IntradayStorageError::SequenceConflict);
        }

        let fresh_now = fresh_database_time(&mut tx).await?;
        sqlx::query(
            "UPDATE public.owner_intraday_quote_demands
                SET state = 'RELEASED', released_at = $5,
                    idempotency_key_sha256 = $3, request_sha256 = $4,
                    updated_at = $5
              WHERE id = $1 AND owner_user_id = $2 AND state = 'ACTIVE'",
        )
        .bind(demand_id)
        .bind(owner_user_id)
        .bind(&key_digest)
        .bind(&body_digest)
        .bind(fresh_now)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(DemandReleaseOutcome {
            kind: DemandReleaseKind::Released,
            demand_id,
        })
    }

    /// Merge all non-expired ACTIVE consumers into exact current identities.
    /// The worker's future scheduler supplies session evidence separately; no
    /// provider or session assumption is made by this demand read.
    pub async fn active_demand_identities(
        &self,
        owner_user_id: Uuid,
    ) -> Result<Vec<IntradayQuoteIdentity>, IntradayStorageError> {
        if owner_user_id.is_nil() {
            return Err(IntradayStorageError::InvalidInput);
        }
        let mut tx = self.begin_actor_transaction(owner_user_id).await?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        let rows: Vec<ActiveDemandIdentityDbRow> = sqlx::query_as(
            "SELECT DISTINCT demand.owner_user_id, demand.membership_id,
                    demand.generation_id, demand.instrument_id, demand.generation
               FROM public.owner_intraday_quote_demands AS demand
               JOIN public.owner_equity_memberships AS membership
                 ON membership.owner_user_id = demand.owner_user_id
                AND membership.id = demand.membership_id
                AND membership.instrument_id = demand.instrument_id
                AND membership.state = 'READY'
               JOIN public.owner_equity_generation_admissions AS admission
                 ON admission.owner_user_id = demand.owner_user_id
                AND admission.membership_id = demand.membership_id
                AND admission.generation_id = demand.generation_id
                AND admission.instrument_id = demand.instrument_id
                AND admission.generation = demand.generation
              WHERE demand.owner_user_id = $1
                AND demand.state = 'ACTIVE'
                AND demand.lease_expires_at > $2
                AND NOT EXISTS (
                    SELECT 1
                      FROM public.owner_equity_generation_admissions AS newer
                     WHERE newer.owner_user_id = admission.owner_user_id
                       AND newer.membership_id = admission.membership_id
                       AND newer.instrument_id = admission.instrument_id
                       AND newer.generation > admission.generation
                )
              ORDER BY demand.instrument_id, demand.membership_id,
                       demand.generation",
        )
        .bind(owner_user_id)
        .bind(fresh_now)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let identities = rows
            .into_iter()
            .map(ActiveDemandIdentityDbRow::into_identity)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(identities)
    }

    /// Claim or take over the one producer lease for an owner.  A live lease
    /// held by another UUID is never stolen; takeover increments the fence.
    pub async fn claim_producer(
        &self,
        owner_user_id: Uuid,
        holder_id: Uuid,
    ) -> Result<ProducerClaimOutcome, IntradayStorageError> {
        if owner_user_id.is_nil() || holder_id.is_nil() {
            return Err(IntradayStorageError::InvalidInput);
        }
        let mut tx = self.begin_actor_transaction(owner_user_id).await?;
        let insert_now = fresh_database_time(&mut tx).await?;
        let inserted = sqlx::query(
            "INSERT INTO public.owner_intraday_quote_producers
                (owner_user_id, holder_id, fencing_token,
                 lease_expires_at, heartbeat_at, updated_at)
             VALUES ($1, $2, 1,
                     $3 + INTERVAL '20 seconds', $3, $3)
             ON CONFLICT (owner_user_id) DO NOTHING",
        )
        .bind(owner_user_id)
        .bind(holder_id)
        .bind(insert_now)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let was_created = inserted.rows_affected() == 1;

        let mut row: ProducerDbRow = sqlx::query_as(
            "SELECT owner_user_id, holder_id, fencing_token,
                    lease_expires_at, heartbeat_at,
                    FALSE AS live
             FROM public.owner_intraday_quote_producers
             WHERE owner_user_id = $1
             FOR UPDATE",
        )
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?
        .ok_or(IntradayStorageError::DatabaseIntegrity)?;
        let fresh_now = fresh_database_time(&mut tx).await?;

        if was_created {
            // The unique-index insert can have waited for an uncommitted
            // competitor. Refresh our own placeholder from the post-lock
            // clock without treating a delayed first claim as a takeover.
            if row.holder_id != holder_id || row.fencing_token != 1 {
                return Err(IntradayStorageError::DatabaseIntegrity);
            }
            let mut refreshed: ProducerDbRow = sqlx::query_as(
                "UPDATE public.owner_intraday_quote_producers
                    SET lease_expires_at = $3 + INTERVAL '20 seconds',
                        heartbeat_at = $3, updated_at = $3
                  WHERE owner_user_id = $1 AND holder_id = $2
                    AND fencing_token = 1
                  RETURNING owner_user_id, holder_id, fencing_token,
                            lease_expires_at, heartbeat_at, FALSE AS live",
            )
            .bind(owner_user_id)
            .bind(holder_id)
            .bind(fresh_now)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database_error)?;
            refreshed.live = refreshed.lease_expires_at > fresh_now;
            let lease = refreshed.to_lease()?;
            tx.commit()
                .await
                .map_err(|_| IntradayStorageError::CommitUnknown)?;
            return Ok(ProducerClaimOutcome {
                kind: ProducerClaimKind::Acquired,
                lease,
            });
        }
        row.live = row.lease_expires_at > fresh_now;

        if row.live && row.holder_id != holder_id {
            return Err(IntradayStorageError::ProducerLeaseHeld);
        }
        if row.live && row.holder_id == holder_id {
            let lease = row.to_lease()?;
            tx.commit()
                .await
                .map_err(|_| IntradayStorageError::CommitUnknown)?;
            return Ok(ProducerClaimOutcome {
                kind: ProducerClaimKind::AlreadyHeld,
                lease,
            });
        }
        if row.fencing_token == i64::MAX {
            return Err(IntradayStorageError::ProducerFenceExhausted);
        }
        let mut updated: ProducerDbRow = sqlx::query_as(
            "UPDATE public.owner_intraday_quote_producers
                SET holder_id = $2, fencing_token = fencing_token + 1,
                    lease_expires_at = $3 + INTERVAL '20 seconds',
                    heartbeat_at = $3, updated_at = $3
              WHERE owner_user_id = $1
              RETURNING owner_user_id, holder_id, fencing_token,
                        lease_expires_at, heartbeat_at, FALSE AS live",
        )
        .bind(owner_user_id)
        .bind(holder_id)
        .bind(fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        updated.live = updated.lease_expires_at > fresh_now;
        let lease = updated.to_lease()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(ProducerClaimOutcome {
            kind: ProducerClaimKind::TakenOver,
            lease,
        })
    }

    /// Extend a lease only when holder and fence still match and the lease has
    /// not expired according to a fresh post-lock database clock.
    pub async fn heartbeat_producer(
        &self,
        lease: &ProducerLease,
    ) -> Result<ProducerLease, IntradayStorageError> {
        validate_lease(lease)?;
        let mut tx = self.begin_actor_transaction(lease.owner_user_id).await?;
        let row: Option<ProducerDbRow> = sqlx::query_as(
            "SELECT owner_user_id, holder_id, fencing_token,
                    lease_expires_at, heartbeat_at, FALSE AS live
               FROM public.owner_intraday_quote_producers
              WHERE owner_user_id = $1
              FOR UPDATE",
        )
        .bind(lease.owner_user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let mut row = row.ok_or(IntradayStorageError::ProducerLeaseLost)?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        row.live = row.lease_expires_at > fresh_now;
        if !row.live
            || row.holder_id != lease.holder_id
            || row.fencing_token
                != i64::try_from(lease.fencing_token)
                    .map_err(|_| IntradayStorageError::InvalidInput)?
        {
            return Err(IntradayStorageError::ProducerLeaseLost);
        }
        let mut updated: ProducerDbRow = sqlx::query_as(
            "UPDATE public.owner_intraday_quote_producers
                SET lease_expires_at = $4 + INTERVAL '20 seconds',
                    heartbeat_at = $4, updated_at = $4
              WHERE owner_user_id = $1 AND holder_id = $2
                AND fencing_token = $3 AND lease_expires_at > $4
              RETURNING owner_user_id, holder_id, fencing_token,
                        lease_expires_at, heartbeat_at, FALSE AS live",
        )
        .bind(lease.owner_user_id)
        .bind(lease.holder_id)
        .bind(i64::try_from(lease.fencing_token).map_err(|_| IntradayStorageError::InvalidInput)?)
        .bind(fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        updated.live = updated.lease_expires_at > fresh_now;
        let result = updated.to_lease()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(result)
    }

    /// Publish one validated quote behind all current producer/session/demand
    /// fences.  The only timestamp accepted for a successful quote is the
    /// explicit receipt supplied by the future transport caller.
    pub async fn publish_success(
        &self,
        context: &IntradayPublicationContext,
        quote: &IntradayQuote,
        receipt: IntradayQuoteReceipt,
    ) -> Result<IntradayCacheRecord, IntradayStorageError> {
        context.validate()?;
        validate_quote(context.identity.symbol(), quote)?;
        let mut tx = self
            .begin_actor_transaction(context.identity.owner_user_id)
            .await?;
        let locked = lock_publication_inputs(&mut tx, context).await?;
        validate_receipt(
            &mut tx,
            receipt.received_at,
            context.session.session_date,
            locked.fresh_now,
        )
        .await?;
        if let Some(prior) = locked.prior_cache.as_ref() {
            if prior.quote_version == i64::MAX {
                return Err(IntradayStorageError::QuoteVersionExhausted);
            }
            if prior.matches_context(context)
                && prior
                    .received_at
                    .is_some_and(|previous| previous > receipt.received_at)
            {
                return Err(IntradayStorageError::QuoteReceiptStale);
            }
        }

        let cache: CacheDbRow = sqlx::query_as(
            "INSERT INTO public.owner_intraday_quote_cache
                (owner_user_id, membership_id, generation_id, instrument_id,
                 generation, session_date, calendar_source,
                 calendar_source_version, calendar_source_batch_id,
                 calendar_content_sha256, window_contract_sha256,
                 price, base_price, change_amount, change_percent,
                 direction, halted, received_at, last_success_at,
                 quote_version, last_attempt_at, last_failure_code,
                 last_failure_at, producer_fence)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                     $12::numeric, $13::numeric, $14::numeric, $15::numeric,
                     $16, $17, $18, $18, 1, $20, NULL, NULL, $19)
             ON CONFLICT (owner_user_id, membership_id) DO UPDATE
                SET generation_id = EXCLUDED.generation_id,
                    instrument_id = EXCLUDED.instrument_id,
                    generation = EXCLUDED.generation,
                    session_date = EXCLUDED.session_date,
                    calendar_source = EXCLUDED.calendar_source,
                    calendar_source_version = EXCLUDED.calendar_source_version,
                    calendar_source_batch_id = EXCLUDED.calendar_source_batch_id,
                    calendar_content_sha256 = EXCLUDED.calendar_content_sha256,
                    window_contract_sha256 = EXCLUDED.window_contract_sha256,
                    price = EXCLUDED.price,
                    base_price = EXCLUDED.base_price,
                    change_amount = EXCLUDED.change_amount,
                    change_percent = EXCLUDED.change_percent,
                    direction = EXCLUDED.direction,
                    halted = EXCLUDED.halted,
                    received_at = EXCLUDED.received_at,
                    last_success_at = EXCLUDED.last_success_at,
                    quote_version = public.owner_intraday_quote_cache.quote_version + 1,
                    last_attempt_at = EXCLUDED.last_attempt_at,
                    last_failure_code = NULL,
                    last_failure_at = NULL,
                    producer_fence = EXCLUDED.producer_fence,
                    updated_at = EXCLUDED.last_attempt_at
             RETURNING owner_user_id, membership_id, generation_id,
                       instrument_id, generation, session_date,
                       calendar_source, calendar_source_version,
                       calendar_source_batch_id, calendar_content_sha256,
                       window_contract_sha256, price::text AS price,
                       base_price::text AS base_price,
                       change_amount::text AS change_amount,
                       change_percent::text AS change_percent,
                       direction, halted, received_at, last_success_at,
                       quote_version, last_attempt_at, last_failure_code,
                       last_failure_at, producer_fence, created_at, updated_at",
        )
        .bind(context.identity.owner_user_id)
        .bind(context.identity.membership_id)
        .bind(context.identity.generation_id)
        .bind(&context.identity.instrument_id)
        .bind(context.identity.generation_i64())
        .bind(context.session.session_date)
        .bind(context.session.calendar_source())
        .bind(context.session.calendar_source_version())
        .bind(context.session.calendar_source_batch_id)
        .bind(&context.session.calendar_content_sha256)
        .bind(&context.session.window_contract_sha256)
        .bind(&quote.price)
        .bind(&quote.base_price)
        .bind(&quote.change_from_previous_day)
        .bind(&quote.change_percent_from_previous_day)
        .bind(quote.direction.as_str())
        .bind(quote.halted)
        .bind(receipt.received_at)
        .bind(
            i64::try_from(context.producer.fencing_token)
                .map_err(|_| IntradayStorageError::InvalidInput)?,
        )
        .bind(locked.fresh_now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let record = cache.into_record()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(record)
    }

    /// Record only attempt/failure fields.  A same-identity/session last-good
    /// quote, version, and success timestamps are left untouched.
    pub async fn record_failure(
        &self,
        context: &IntradayPublicationContext,
        failure: IntradayQuoteFailureCode,
    ) -> Result<IntradayCacheRecord, IntradayStorageError> {
        context.validate()?;
        let mut tx = self
            .begin_actor_transaction(context.identity.owner_user_id)
            .await?;
        let locked = lock_publication_inputs(&mut tx, context).await?;
        let prior = locked.prior_cache;

        let same_identity = prior
            .as_ref()
            .is_some_and(|row| row.matches_context(context));
        let cache: CacheDbRow = if same_identity {
            sqlx::query_as(
                "UPDATE public.owner_intraday_quote_cache
                    SET last_attempt_at = $5,
                        last_failure_code = $3, last_failure_at = $5,
                        producer_fence = $4, updated_at = $5
                  WHERE owner_user_id = $1 AND membership_id = $2
                  RETURNING owner_user_id, membership_id, generation_id,
                            instrument_id, generation, session_date,
                            calendar_source, calendar_source_version,
                            calendar_source_batch_id, calendar_content_sha256,
                            window_contract_sha256, price::text AS price,
                            base_price::text AS base_price,
                            change_amount::text AS change_amount,
                            change_percent::text AS change_percent,
                            direction, halted, received_at, last_success_at,
                            quote_version, last_attempt_at, last_failure_code,
                            last_failure_at, producer_fence, created_at, updated_at",
            )
            .bind(context.identity.owner_user_id)
            .bind(context.identity.membership_id)
            .bind(failure.as_str())
            .bind(
                i64::try_from(context.producer.fencing_token)
                    .map_err(|_| IntradayStorageError::InvalidInput)?,
            )
            .bind(locked.fresh_now)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database_error)?
        } else {
            sqlx::query_as(
                "INSERT INTO public.owner_intraday_quote_cache
                    (owner_user_id, membership_id, generation_id, instrument_id,
                     generation, session_date, calendar_source,
                     calendar_source_version, calendar_source_batch_id,
                     calendar_content_sha256, window_contract_sha256,
                     quote_version, last_attempt_at, last_failure_code,
                     last_failure_at, producer_fence)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                         0, $14, $12, $14, $13)
                 ON CONFLICT (owner_user_id, membership_id) DO UPDATE
                    SET generation_id = EXCLUDED.generation_id,
                        instrument_id = EXCLUDED.instrument_id,
                        generation = EXCLUDED.generation,
                        session_date = EXCLUDED.session_date,
                        calendar_source = EXCLUDED.calendar_source,
                        calendar_source_version = EXCLUDED.calendar_source_version,
                        calendar_source_batch_id = EXCLUDED.calendar_source_batch_id,
                        calendar_content_sha256 = EXCLUDED.calendar_content_sha256,
                        window_contract_sha256 = EXCLUDED.window_contract_sha256,
                        price = NULL, base_price = NULL,
                        change_amount = NULL, change_percent = NULL,
                        direction = NULL, halted = NULL,
                        received_at = NULL, last_success_at = NULL,
                        quote_version = 0,
                        last_attempt_at = EXCLUDED.last_attempt_at,
                        last_failure_code = EXCLUDED.last_failure_code,
                        last_failure_at = EXCLUDED.last_failure_at,
                        producer_fence = EXCLUDED.producer_fence,
                        updated_at = EXCLUDED.last_attempt_at
                 RETURNING owner_user_id, membership_id, generation_id,
                           instrument_id, generation, session_date,
                           calendar_source, calendar_source_version,
                           calendar_source_batch_id, calendar_content_sha256,
                           window_contract_sha256, price::text AS price,
                           base_price::text AS base_price,
                           change_amount::text AS change_amount,
                           change_percent::text AS change_percent,
                           direction, halted, received_at, last_success_at,
                           quote_version, last_attempt_at, last_failure_code,
                           last_failure_at, producer_fence, created_at, updated_at",
            )
            .bind(context.identity.owner_user_id)
            .bind(context.identity.membership_id)
            .bind(context.identity.generation_id)
            .bind(&context.identity.instrument_id)
            .bind(context.identity.generation_i64())
            .bind(context.session.session_date)
            .bind(context.session.calendar_source())
            .bind(context.session.calendar_source_version())
            .bind(context.session.calendar_source_batch_id)
            .bind(&context.session.calendar_content_sha256)
            .bind(&context.session.window_contract_sha256)
            .bind(failure.as_str())
            .bind(
                i64::try_from(context.producer.fencing_token)
                    .map_err(|_| IntradayStorageError::InvalidInput)?,
            )
            .bind(locked.fresh_now)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database_error)?
        };
        let record = cache.into_record()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(record)
    }

    /// Read only a current, exact identity/session cache row.  A new database
    /// session date or proof digest makes a prior-session quote invisible.
    pub async fn read_current_cache(
        &self,
        owner_user_id: Uuid,
        membership_id: Uuid,
        generation: u64,
        session: &IntradaySessionProof,
    ) -> Result<Option<IntradayCacheRecord>, IntradayStorageError> {
        if owner_user_id.is_nil() || membership_id.is_nil() || generation == 0 {
            return Err(IntradayStorageError::InvalidInput);
        }
        session.validate()?;
        let generation =
            i64::try_from(generation).map_err(|_| IntradayStorageError::InvalidInput)?;
        let mut tx = self.begin_actor_transaction(owner_user_id).await?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        validate_session_lineage(&mut tx, session, fresh_now).await?;
        let row: Option<CacheDbRow> = sqlx::query_as(
            "SELECT cache.owner_user_id, cache.membership_id, cache.generation_id,
                    cache.instrument_id, cache.generation, cache.session_date,
                    cache.calendar_source, cache.calendar_source_version,
                    cache.calendar_source_batch_id, cache.calendar_content_sha256,
                    cache.window_contract_sha256, cache.price::text AS price,
                    cache.base_price::text AS base_price,
                    cache.change_amount::text AS change_amount,
                    cache.change_percent::text AS change_percent,
                    cache.direction, cache.halted, cache.received_at,
                    cache.last_success_at, cache.quote_version,
                    cache.last_attempt_at, cache.last_failure_code,
                    cache.last_failure_at, cache.producer_fence,
                    cache.created_at, cache.updated_at
             FROM public.owner_intraday_quote_cache AS cache
             JOIN public.owner_equity_memberships AS membership
               ON membership.id = cache.membership_id
              AND membership.owner_user_id = cache.owner_user_id
              AND membership.instrument_id = cache.instrument_id
              AND membership.state = 'READY'
             JOIN public.owner_equity_generation_admissions AS admission
               ON admission.generation_id = cache.generation_id
              AND admission.owner_user_id = cache.owner_user_id
              AND admission.membership_id = cache.membership_id
              AND admission.instrument_id = cache.instrument_id
              AND admission.generation = cache.generation
             WHERE cache.owner_user_id = $1
               AND cache.membership_id = $2
               AND cache.generation = $3
               AND NOT EXISTS (
                    SELECT 1
                      FROM public.owner_equity_generation_admissions AS newer
                     WHERE newer.owner_user_id = admission.owner_user_id
                       AND newer.membership_id = admission.membership_id
                       AND newer.instrument_id = admission.instrument_id
                       AND newer.generation > admission.generation
               )
               AND cache.session_date = $4
               AND cache.calendar_source = $5
               AND cache.calendar_source_version = $6
               AND cache.calendar_source_batch_id = $7
               AND cache.calendar_content_sha256 = $8
               AND cache.window_contract_sha256 = $9
               AND cache.last_attempt_at <= $10
               AND cache.last_attempt_at >= $10
                    - INTERVAL '24 hours'
               AND (cache.last_success_at IS NULL OR (
                    cache.last_success_at <= $10
                AND cache.last_success_at >= $10
                    - INTERVAL '24 hours'))",
        )
        .bind(owner_user_id)
        .bind(membership_id)
        .bind(generation)
        .bind(session.session_date)
        .bind(session.calendar_source())
        .bind(session.calendar_source_version())
        .bind(session.calendar_source_batch_id)
        .bind(&session.calendar_content_sha256)
        .bind(&session.window_contract_sha256)
        .bind(fresh_now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let record = row.map(CacheDbRow::into_record).transpose()?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(record)
    }

    /// Delete only old demand tombstones/expired rows and old latest cache
    /// rows.  Producer rows are retained so a future takeover never reuses a
    /// fencing token.
    pub async fn gc_expired(
        &self,
        owner_user_id: Uuid,
    ) -> Result<IntradayGcReport, IntradayStorageError> {
        if owner_user_id.is_nil() {
            return Err(IntradayStorageError::InvalidInput);
        }
        let mut tx = self.begin_actor_transaction(owner_user_id).await?;
        lock_owner_policy_for_demand(&mut tx, owner_user_id).await?;
        let fresh_now = fresh_database_time(&mut tx).await?;
        let demands = sqlx::query(
            "DELETE FROM public.owner_intraday_quote_demands
              WHERE owner_user_id = $1
                AND ((state = 'RELEASED'
                      AND released_at < $2 - INTERVAL '24 hours')
                  OR (state = 'ACTIVE'
                      AND lease_expires_at < $2
                          - INTERVAL '24 hours'))",
        )
        .bind(owner_user_id)
        .bind(fresh_now)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        let cache = sqlx::query(
            "DELETE FROM public.owner_intraday_quote_cache
              WHERE owner_user_id = $1
                AND last_attempt_at < $2 - INTERVAL '24 hours'
                AND (last_success_at IS NULL
                  OR last_success_at < $2 - INTERVAL '24 hours')",
        )
        .bind(owner_user_id)
        .bind(fresh_now)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        tx.commit()
            .await
            .map_err(|_| IntradayStorageError::CommitUnknown)?;
        Ok(IntradayGcReport {
            demand_rows_deleted: demands.rows_affected(),
            cache_rows_deleted: cache.rows_affected(),
        })
    }

    async fn begin_actor_transaction(
        &self,
        owner_user_id: Uuid,
    ) -> Result<Transaction<'_, Postgres>, IntradayStorageError> {
        let mut tx = self.pool.begin().await.map_err(map_database_error)?;
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(owner_user_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
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

async fn fresh_database_time(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<DateTime<Utc>, IntradayStorageError> {
    sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)
}

#[derive(Debug, FromRow)]
struct DemandDbRow {
    id: Uuid,
    owner_user_id: Uuid,
    consumer_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    state: String,
    renewal_sequence: i64,
    lease_expires_at: DateTime<Utc>,
    idempotency_key_sha256: String,
    request_sha256: String,
}

impl DemandDbRow {
    fn to_lease(&self) -> Result<IntradayQuoteDemandLease, IntradayStorageError> {
        if self.state != "ACTIVE"
            || self.generation <= 0
            || self.renewal_sequence < 0
            || self.owner_user_id.is_nil()
            || self.consumer_id.is_nil()
            || self.membership_id.is_nil()
            || self.generation_id.is_nil()
            || !canonical_instrument(&self.instrument_id)
        {
            return Err(IntradayStorageError::DatabaseIntegrity);
        }
        Ok(IntradayQuoteDemandLease {
            demand_id: self.id,
            owner_user_id: self.owner_user_id,
            consumer_id: self.consumer_id,
            membership_id: self.membership_id,
            generation_id: self.generation_id,
            instrument_id: self.instrument_id.clone(),
            generation: u64::try_from(self.generation)
                .map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
            renewal_sequence: u64::try_from(self.renewal_sequence)
                .map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
            lease_expires_at: self.lease_expires_at,
            renew_after_ms: DEMAND_RENEW_AFTER_MS,
        })
    }
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
struct ActiveDemandIdentityDbRow {
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
}

impl ActiveDemandIdentityDbRow {
    fn into_identity(self) -> Result<IntradayQuoteIdentity, IntradayStorageError> {
        IntradayQuoteIdentity::new(
            self.owner_user_id,
            self.membership_id,
            self.generation_id,
            self.instrument_id,
            u64::try_from(self.generation).map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
        )
        .map_err(|_| IntradayStorageError::DatabaseIntegrity)
    }
}

#[derive(Debug, FromRow)]
struct ProducerDbRow {
    owner_user_id: Uuid,
    holder_id: Uuid,
    fencing_token: i64,
    lease_expires_at: DateTime<Utc>,
    heartbeat_at: DateTime<Utc>,
    live: bool,
}

impl ProducerDbRow {
    fn to_lease(&self) -> Result<ProducerLease, IntradayStorageError> {
        if self.owner_user_id.is_nil()
            || self.holder_id.is_nil()
            || self.fencing_token <= 0
            || !self.live
        {
            return Err(IntradayStorageError::ProducerLeaseLost);
        }
        Ok(ProducerLease {
            owner_user_id: self.owner_user_id,
            holder_id: self.holder_id,
            fencing_token: u64::try_from(self.fencing_token)
                .map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
            lease_expires_at: self.lease_expires_at,
            heartbeat_at: self.heartbeat_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct CacheIdentityDbRow {
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    session_date: Option<NaiveDate>,
    calendar_source: Option<String>,
    calendar_source_version: Option<String>,
    calendar_source_batch_id: Option<Uuid>,
    calendar_content_sha256: Option<String>,
    window_contract_sha256: Option<String>,
    received_at: Option<DateTime<Utc>>,
    quote_version: i64,
}

impl CacheIdentityDbRow {
    fn matches_context(&self, context: &IntradayPublicationContext) -> bool {
        self.generation_id == context.identity.generation_id
            && self.instrument_id == context.identity.instrument_id
            && self.generation == context.identity.generation_i64()
            && self.session_date == Some(context.session.session_date)
            && self.calendar_source.as_deref() == Some(context.session.calendar_source())
            && self.calendar_source_version.as_deref()
                == Some(context.session.calendar_source_version())
            && self.calendar_source_batch_id == Some(context.session.calendar_source_batch_id)
            && self.calendar_content_sha256.as_deref()
                == Some(context.session.calendar_content_sha256.as_str())
            && self.window_contract_sha256.as_deref()
                == Some(context.session.window_contract_sha256.as_str())
    }
}

struct LockedPublicationInputs {
    fresh_now: DateTime<Utc>,
    prior_cache: Option<CacheIdentityDbRow>,
}

#[derive(Debug, FromRow)]
struct CacheDbRow {
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation_id: Uuid,
    instrument_id: String,
    generation: i64,
    session_date: Option<NaiveDate>,
    calendar_source: Option<String>,
    calendar_source_version: Option<String>,
    calendar_source_batch_id: Option<Uuid>,
    calendar_content_sha256: Option<String>,
    window_contract_sha256: Option<String>,
    price: Option<String>,
    base_price: Option<String>,
    change_amount: Option<String>,
    change_percent: Option<String>,
    direction: Option<String>,
    halted: Option<bool>,
    received_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    quote_version: i64,
    last_attempt_at: DateTime<Utc>,
    last_failure_code: Option<String>,
    last_failure_at: Option<DateTime<Utc>>,
    producer_fence: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl CacheDbRow {
    fn into_record(self) -> Result<IntradayCacheRecord, IntradayStorageError> {
        if self.owner_user_id.is_nil()
            || self.membership_id.is_nil()
            || self.generation_id.is_nil()
            || !canonical_instrument(&self.instrument_id)
            || self.generation <= 0
            || self.quote_version < 0
            || self.producer_fence < 0
        {
            return Err(IntradayStorageError::DatabaseIntegrity);
        }
        let direction = self.direction.as_deref().map(parse_direction).transpose()?;
        let last_failure_code = match self.last_failure_code.as_deref() {
            None => None,
            Some(value) => Some(
                IntradayQuoteFailureCode::parse(value)
                    .ok_or(IntradayStorageError::DatabaseIntegrity)?,
            ),
        };
        Ok(IntradayCacheRecord {
            owner_user_id: self.owner_user_id,
            membership_id: self.membership_id,
            generation_id: self.generation_id,
            instrument_id: self.instrument_id,
            generation: u64::try_from(self.generation)
                .map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
            session_date: self.session_date,
            calendar_source: self.calendar_source,
            calendar_source_version: self.calendar_source_version,
            calendar_source_batch_id: self.calendar_source_batch_id,
            calendar_content_sha256: self.calendar_content_sha256,
            window_contract_sha256: self.window_contract_sha256,
            price: self.price,
            base_price: self.base_price,
            change_amount: self.change_amount,
            change_percent: self.change_percent,
            direction,
            halted: self.halted,
            received_at: self.received_at,
            last_success_at: self.last_success_at,
            quote_version: u64::try_from(self.quote_version)
                .map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
            last_attempt_at: self.last_attempt_at,
            last_failure_code,
            last_failure_at: self.last_failure_at,
            producer_fence: u64::try_from(self.producer_fence)
                .map_err(|_| IntradayStorageError::DatabaseIntegrity)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

async fn lock_owner_policy_for_demand(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
) -> Result<(), IntradayStorageError> {
    // 0053 grants app/worker SELECT, not UPDATE, on the policy row.  The
    // owner-scoped transaction advisory lock is therefore the frozen capacity
    // mutex; this SELECT is only the policy-existence anchor.  It does not
    // lock policy administration and does not use a privilege bypass.
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_xact_lock(
                    pg_catalog.hashtextextended($1, 0))",
    )
    .bind(format!("owner-intraday-demand-cap|{owner_user_id}"))
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let policy: Option<Uuid> = sqlx::query_scalar(
        "SELECT owner_user_id
           FROM public.owner_equity_universe_policies
          WHERE owner_user_id = $1
          ",
    )
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if policy != Some(owner_user_id) {
        return Err(IntradayStorageError::PolicyUnavailable);
    }
    Ok(())
}

async fn ensure_active_capacity(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
    excluded_demand_id: Option<Uuid>,
    requested_identity: &AdmissionDbRow,
    fresh_now: DateTime<Utc>,
) -> Result<(), IntradayStorageError> {
    let consumers: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1
            AND state = 'ACTIVE'
            AND lease_expires_at > $3::timestamptz
            AND ($2::uuid IS NULL OR id <> $2)",
    )
    .bind(owner_user_id)
    .bind(excluded_demand_id)
    .bind(fresh_now)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if consumers >= MAX_ACTIVE_CONSUMERS {
        return Err(IntradayStorageError::DemandCapacity);
    }
    let identities: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM (
                SELECT DISTINCT membership_id, generation_id,
                       instrument_id, generation
                  FROM public.owner_intraday_quote_demands
                 WHERE owner_user_id = $1
                   AND state = 'ACTIVE'
                   AND lease_expires_at > $3::timestamptz
                   AND ($2::uuid IS NULL OR id <> $2)
           ) AS active_identities",
    )
    .bind(owner_user_id)
    .bind(excluded_demand_id)
    .bind(fresh_now)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if identities >= MAX_ACTIVE_IDENTITIES {
        let requested_identity_is_active: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                    SELECT 1
                      FROM public.owner_intraday_quote_demands
                     WHERE owner_user_id = $1
                       AND membership_id = $2
                       AND generation_id = $3
                       AND instrument_id = $4
                       AND generation = $5
                       AND state = 'ACTIVE'
                       AND lease_expires_at > $6::timestamptz
                       AND ($7::uuid IS NULL OR id <> $7)
                )",
        )
        .bind(owner_user_id)
        .bind(requested_identity.membership_id)
        .bind(requested_identity.generation_id)
        .bind(&requested_identity.instrument_id)
        .bind(requested_identity.generation)
        .bind(fresh_now)
        .bind(excluded_demand_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_database_error)?;
        if !requested_identity_is_active {
            return Err(IntradayStorageError::IdentityCapacity);
        }
    }
    Ok(())
}

async fn lock_ready_admission(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation: i64,
) -> Result<AdmissionDbRow, IntradayStorageError> {
    let row: Option<AdmissionDbRow> = sqlx::query_as(
        "SELECT admission.owner_user_id, admission.membership_id,
                admission.generation_id, admission.instrument_id,
                admission.generation
           FROM public.owner_equity_memberships AS membership
           JOIN public.owner_equity_generation_admissions AS admission
             ON admission.owner_user_id = membership.owner_user_id
            AND admission.membership_id = membership.id
            AND admission.instrument_id = membership.instrument_id
            AND admission.generation = $3
          WHERE membership.owner_user_id = $1
            AND membership.id = $2
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
    .bind(owner_user_id)
    .bind(membership_id)
    .bind(generation)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let row = row.ok_or(IntradayStorageError::MembershipNotReady)?;
    if row.owner_user_id != owner_user_id {
        return Err(IntradayStorageError::IdentityMismatch);
    }
    Ok(row)
}

async fn lock_publication_inputs(
    tx: &mut Transaction<'_, Postgres>,
    context: &IntradayPublicationContext,
) -> Result<LockedPublicationInputs, IntradayStorageError> {
    let fence: Option<ProducerDbRow> = sqlx::query_as(
        "SELECT owner_user_id, holder_id, fencing_token,
                lease_expires_at, heartbeat_at,
                FALSE AS live
           FROM public.owner_intraday_quote_producers
          WHERE owner_user_id = $1
          FOR UPDATE",
    )
    .bind(context.identity.owner_user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let mut fence = fence.ok_or(IntradayStorageError::ProducerLeaseLost)?;
    let admission = lock_ready_admission_by_identity(tx, &context.identity).await?;
    if admission.generation_id != context.identity.generation_id {
        return Err(IntradayStorageError::IdentityMismatch);
    }
    let active_demand_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM public.owner_intraday_quote_demands
          WHERE owner_user_id = $1 AND membership_id = $2
            AND generation_id = $3 AND instrument_id = $4
            AND generation = $5 AND state = 'ACTIVE'
          FOR SHARE",
    )
    .bind(context.identity.owner_user_id)
    .bind(context.identity.membership_id)
    .bind(context.identity.generation_id)
    .bind(&context.identity.instrument_id)
    .bind(context.identity.generation_i64())
    .fetch_all(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if active_demand_ids.is_empty() {
        return Err(IntradayStorageError::ActiveDemandRequired);
    }
    let prior_cache: Option<CacheIdentityDbRow> = sqlx::query_as(
        "SELECT generation_id, instrument_id, generation, session_date,
                calendar_source, calendar_source_version,
                calendar_source_batch_id, calendar_content_sha256,
                window_contract_sha256, received_at, quote_version
           FROM public.owner_intraday_quote_cache
          WHERE owner_user_id = $1 AND membership_id = $2
          FOR UPDATE",
    )
    .bind(context.identity.owner_user_id)
    .bind(context.identity.membership_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;

    let fresh_now = fresh_database_time(tx).await?;
    fence.live = fence.lease_expires_at > fresh_now;
    if !fence.live
        || fence.holder_id != context.producer.holder_id
        || fence.fencing_token
            != i64::try_from(context.producer.fencing_token)
                .map_err(|_| IntradayStorageError::InvalidInput)?
    {
        return Err(IntradayStorageError::ProducerLeaseLost);
    }
    let active_demand: bool = sqlx::query_scalar(
        "SELECT EXISTS (
                SELECT 1
                  FROM public.owner_intraday_quote_demands
                 WHERE owner_user_id = $1 AND membership_id = $2
                   AND generation_id = $3 AND instrument_id = $4
                   AND generation = $5 AND state = 'ACTIVE'
                   AND lease_expires_at > $6::timestamptz
            )",
    )
    .bind(context.identity.owner_user_id)
    .bind(context.identity.membership_id)
    .bind(context.identity.generation_id)
    .bind(&context.identity.instrument_id)
    .bind(context.identity.generation_i64())
    .bind(fresh_now)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if !active_demand {
        return Err(IntradayStorageError::ActiveDemandRequired);
    }
    validate_session_lineage(tx, &context.session, fresh_now).await?;
    Ok(LockedPublicationInputs {
        fresh_now,
        prior_cache,
    })
}

async fn lock_ready_admission_by_identity(
    tx: &mut Transaction<'_, Postgres>,
    identity: &IntradayQuoteIdentity,
) -> Result<AdmissionDbRow, IntradayStorageError> {
    let row: Option<AdmissionDbRow> = sqlx::query_as(
        "SELECT admission.owner_user_id, admission.membership_id,
                admission.generation_id, admission.instrument_id,
                admission.generation
           FROM public.owner_equity_memberships AS membership
           JOIN public.owner_equity_generation_admissions AS admission
             ON admission.owner_user_id = membership.owner_user_id
            AND admission.membership_id = membership.id
            AND admission.instrument_id = membership.instrument_id
            AND admission.generation_id = $3
            AND admission.generation = $5
          WHERE membership.owner_user_id = $1
            AND membership.id = $2
            AND membership.instrument_id = $4
            AND membership.state = 'READY'
            AND NOT EXISTS (
                SELECT 1 FROM public.owner_equity_generation_admissions AS newer
                 WHERE newer.owner_user_id = admission.owner_user_id
                   AND newer.membership_id = admission.membership_id
                   AND newer.instrument_id = admission.instrument_id
                   AND newer.generation > admission.generation
            )
          FOR SHARE OF membership",
    )
    .bind(identity.owner_user_id)
    .bind(identity.membership_id)
    .bind(identity.generation_id)
    .bind(&identity.instrument_id)
    .bind(identity.generation_i64())
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let row = row.ok_or(IntradayStorageError::MembershipNotReady)?;
    if row.owner_user_id != identity.owner_user_id {
        return Err(IntradayStorageError::IdentityMismatch);
    }
    Ok(row)
}

async fn validate_session_lineage(
    tx: &mut Transaction<'_, Postgres>,
    session: &IntradaySessionProof,
    fresh_now: DateTime<Utc>,
) -> Result<(), IntradayStorageError> {
    session.validate()?;
    let row: Option<(Uuid, i64)> = sqlx::query_as(
        "SELECT calendar.id, version.id
           FROM public.trading_calendars AS calendar
           JOIN public.trading_calendar_versions AS version
            ON version.exchange = calendar.exchange
            AND version.session_date = calendar.session_date
            AND version.session_type = calendar.session_type
            AND version.source_version = calendar.source_version
            AND version.source = $2
            AND version.timezone = 'Asia/Seoul'
            AND version.source_batch_id = calendar.source_batch_id
            AND version.content_sha256 = calendar.content_sha256
          JOIN public.data_batches AS batch
            ON batch.id = calendar.source_batch_id
           AND batch.provider = 'KIS'
           AND batch.market = 'KR'
           AND batch.kind = 'CALENDAR'
           AND batch.batch_date = calendar.session_date
           AND batch.content_sha256 = calendar.content_sha256
          WHERE calendar.exchange = 'KRX'
            AND calendar.session_date = $1
            AND calendar.session_type = 'TRADING'
            AND calendar.timezone = 'Asia/Seoul'
            AND calendar.source = $2
            AND calendar.source_version = $3
            AND calendar.source_batch_id = $4
            AND calendar.content_sha256 = $5
            AND calendar.retrieved_at <= $6
            AND calendar.retrieved_at >= $6
                - INTERVAL '36 hours'
            AND version.retrieved_at <= $6
            AND version.retrieved_at >= $6
                - INTERVAL '36 hours'
            AND batch.retrieved_at <= $6
            AND batch.retrieved_at >= $6
                - INTERVAL '36 hours'
          ",
    )
    .bind(session.session_date)
    .bind(session.calendar_source())
    .bind(session.calendar_source_version())
    .bind(session.calendar_source_batch_id)
    .bind(&session.calendar_content_sha256)
    .bind(fresh_now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if row.is_none() {
        return Err(IntradayStorageError::CalendarProofUnavailable);
    }
    let current_date: NaiveDate =
        sqlx::query_scalar("SELECT ($1::timestamptz AT TIME ZONE 'Asia/Seoul')::date")
            .bind(fresh_now)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_database_error)?;
    if current_date != session.session_date {
        return Err(IntradayStorageError::CalendarProofUnavailable);
    }
    Ok(())
}

async fn validate_receipt(
    tx: &mut Transaction<'_, Postgres>,
    received_at: DateTime<Utc>,
    session_date: NaiveDate,
    fresh_now: DateTime<Utc>,
) -> Result<(), IntradayStorageError> {
    let valid: bool = sqlx::query_scalar(
        "SELECT $1::timestamptz <= $3::timestamptz
             AND ($1::timestamptz AT TIME ZONE 'Asia/Seoul')::date = $2",
    )
    .bind(received_at)
    .bind(session_date)
    .bind(fresh_now)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if valid {
        Ok(())
    } else {
        Err(IntradayStorageError::ReceiptInvalid)
    }
}

fn validate_lease(lease: &ProducerLease) -> Result<(), IntradayStorageError> {
    if lease.owner_user_id.is_nil()
        || lease.holder_id.is_nil()
        || lease.fencing_token == 0
        || lease.lease_expires_at <= lease.heartbeat_at
    {
        return Err(IntradayStorageError::InvalidInput);
    }
    Ok(())
}

fn validate_quote(
    requested_symbol: &str,
    quote: &IntradayQuote,
) -> Result<(), IntradayStorageError> {
    if quote.symbol != requested_symbol
        || !is_six_ascii_digits(&quote.symbol)
        || !is_positive_decimal(&quote.price)
        || !is_positive_decimal(&quote.base_price)
    {
        return Err(IntradayStorageError::QuoteInvalid);
    }
    let amount = decimal_kind(&quote.change_from_previous_day);
    let percent = decimal_kind(&quote.change_percent_from_previous_day);
    let (Some(amount), Some(percent)) = (amount, percent) else {
        return Err(IntradayStorageError::QuoteInvalid);
    };
    let signs_match = match quote.direction {
        IntradayQuoteDirection::LimitUp | IntradayQuoteDirection::Up => {
            amount == DecimalKind::Positive && percent == DecimalKind::Positive
        }
        IntradayQuoteDirection::Flat => amount == DecimalKind::Zero && percent == DecimalKind::Zero,
        IntradayQuoteDirection::LimitDown | IntradayQuoteDirection::Down => {
            amount == DecimalKind::Negative && percent == DecimalKind::Negative
        }
    };
    if signs_match {
        Ok(())
    } else {
        Err(IntradayStorageError::QuoteInvalid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecimalKind {
    Positive,
    Zero,
    Negative,
}

fn decimal_kind(value: &str) -> Option<DecimalKind> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let negative = bytes[0] == b'-';
    let integer_start = usize::from(negative);
    if integer_start == bytes.len() {
        return None;
    }
    let mut integer_end = integer_start;
    match bytes[integer_start] {
        b'0' => {
            integer_end += 1;
            if integer_end < bytes.len() && bytes[integer_end].is_ascii_digit() {
                return None;
            }
        }
        byte if (b'1'..=b'9').contains(&byte) => {
            integer_end += 1;
            while integer_end < bytes.len() && bytes[integer_end].is_ascii_digit() {
                integer_end += 1;
            }
        }
        _ => return None,
    }
    if integer_end - integer_start > 12 {
        return None;
    }
    let mut fraction_start = integer_end;
    if integer_end < bytes.len() {
        if bytes[integer_end] != b'.' {
            return None;
        }
        fraction_start += 1;
        let mut fraction_end = fraction_start;
        while fraction_end < bytes.len() && bytes[fraction_end].is_ascii_digit() {
            fraction_end += 1;
        }
        if !(1..=8).contains(&(fraction_end - fraction_start)) || fraction_end != bytes.len() {
            return None;
        }
    }
    let integer_zero = bytes[integer_start] == b'0';
    let fraction_zero = bytes[fraction_start..].iter().all(|byte| *byte == b'0');
    if negative && integer_zero && fraction_zero {
        return None;
    }
    if integer_zero && fraction_zero {
        Some(DecimalKind::Zero)
    } else if negative {
        Some(DecimalKind::Negative)
    } else {
        Some(DecimalKind::Positive)
    }
}

fn is_positive_decimal(value: &str) -> bool {
    decimal_kind(value) == Some(DecimalKind::Positive)
}

fn parse_direction(value: &str) -> Result<IntradayQuoteDirection, IntradayStorageError> {
    match value {
        "LIMIT_UP" => Ok(IntradayQuoteDirection::LimitUp),
        "UP" => Ok(IntradayQuoteDirection::Up),
        "FLAT" => Ok(IntradayQuoteDirection::Flat),
        "LIMIT_DOWN" => Ok(IntradayQuoteDirection::LimitDown),
        "DOWN" => Ok(IntradayQuoteDirection::Down),
        _ => Err(IntradayStorageError::DatabaseIntegrity),
    }
}

fn valid_idempotency_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b':' && byte != b'\\')
}

fn canonical_instrument(value: &str) -> bool {
    value.len() == 10
        && value.ends_with(".KRX")
        && value.as_bytes()[..6]
            .iter()
            .all(|byte| byte.is_ascii_digit())
}

fn is_six_ascii_digits(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn canonical_unprefixed_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_prefixed_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(canonical_unprefixed_sha256)
}

fn prefixed_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn map_database_error(error: sqlx::Error) -> IntradayStorageError {
    if matches!(&error, sqlx::Error::Database(database) if database.code().as_deref() == Some("42501"))
    {
        return IntradayStorageError::PermissionDenied;
    }
    match database_error_class(&error) {
        ErrorClass::Transient => IntradayStorageError::DatabaseUnavailable,
        _ => IntradayStorageError::DatabaseIntegrity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn identity() -> IntradayQuoteIdentity {
        IntradayQuoteIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "005930.KRX".to_owned(),
            1,
        )
        .unwrap()
    }

    #[test]
    fn request_hashes_are_prefixed_and_debug_redacts_key() {
        let request = IntradayQuoteDemandRequest::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            0,
            "synthetic-idempotency".to_owned(),
        )
        .unwrap();
        assert!(request.idempotency_digest().starts_with("sha256:"));
        assert!(request.request_digest().starts_with("sha256:"));
        assert!(!format!("{request:?}").contains("synthetic-idempotency"));
    }

    #[test]
    fn decimal_storage_validation_matches_frozen_grammar() {
        assert!(is_positive_decimal("72500"));
        assert!(is_positive_decimal("0.00000001"));
        assert!(!is_positive_decimal("0"));
        assert!(!is_positive_decimal("-0.1"));
        assert!(!is_positive_decimal("1e3"));
        assert!(!is_positive_decimal("01"));
        assert!(decimal_kind("-1.25") == Some(DecimalKind::Negative));
        assert!(decimal_kind("0.00") == Some(DecimalKind::Zero));
    }

    #[test]
    fn session_and_budget_proofs_are_explicit() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 8).unwrap();
        let session = IntradaySessionProof::new(
            date,
            Uuid::new_v4(),
            "a".repeat(64),
            format!("sha256:{}", "b".repeat(64)),
        )
        .unwrap();
        let budget = IntradayAttemptReservation::new(Uuid::new_v4(), date, 1).unwrap();
        let holder = Uuid::new_v4();
        let lease = ProducerLease {
            owner_user_id: identity().owner_user_id,
            holder_id: holder,
            fencing_token: 1,
            lease_expires_at: Utc.with_ymd_and_hms(2026, 9, 8, 1, 0, 20).unwrap(),
            heartbeat_at: Utc.with_ymd_and_hms(2026, 9, 8, 1, 0, 0).unwrap(),
        };
        let id = IntradayQuoteIdentity {
            owner_user_id: lease.owner_user_id,
            ..identity()
        };
        assert!(IntradayPublicationContext::new(lease, id, session, budget).is_ok());
    }
}
