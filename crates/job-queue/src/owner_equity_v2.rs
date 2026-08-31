//! Durable orchestration for the owner-managed equity universe V2.
//!
//! The public API transaction creates the membership and its queue job.  This
//! module owns the other half of the boundary: a leased claim advances the
//! membership with compare-and-set transitions, delegates capture and
//! materialization to the WP-2 seams, computes the WP-3 snapshot, and commits
//! generation admission, exact snapshot publication, and queue settlement in
//! one transaction.  Filesystem/network work is never performed while a
//! database transaction is open.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use async_trait::async_trait;
use collectors::owner_equity_v2::{
    OwnerEquityCaptureOutcome, OwnerEquityCollectorError, capture_owner_equity_raw,
    check_owner_equity_from_raw,
};
use domain::{
    CodeCommit, ContentHash, InstrumentId, OwnerEquityAdmissionPins, OwnerEquityFailureCode,
    OwnerEquityGeneration, OwnerEquityMembershipState, OwnerEquityUniversePolicy, RetryDisposition,
    TradingDate,
};
use factor_engine::owner_equity_v2::{
    OwnerEquityAdmittedCandidate, OwnerEquitySignalSnapshotCandidate,
    compute_owner_equity_signal_snapshot,
};
use market_data::owner_equity_v2::{OwnerEquityCaptureIdentity, OwnerEquityGenerationCandidate};
use market_data::providers::kis::{KisProvider, KisRead};
use market_data::storage::RawStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{database_error_class, queue_error_class};
use crate::{
    ClaimedJob, ErrorClass, HeartbeatStatus, JobQueue, JobStatus, QueueError, SettleResult,
};

#[path = "owner_equity_v2/runtime.rs"]
mod runtime;
pub use runtime::{OwnerEquityPreflight, OwnerEquityRuntimeLimits, ProductionOwnerEquityAdapter};
#[path = "owner_equity_v2/runner.rs"]
mod runner;
pub use runner::{
    OwnerEquityRunnerConfig, OwnerEquityRunnerError, recover_owner_equity_claims,
    run_owner_equity_runner_once,
};
#[path = "owner_equity_v2/schedule.rs"]
mod schedule;
pub use schedule::{
    OwnerEquityScheduleError, OwnerEquitySchedulePins, OwnerEquityScheduleReport,
    eligible_schedule_date, run_owner_equity_schedule_cycle,
};

/// Dedicated type claimed by the V2 worker.  V1 jobs use different values.
pub const OWNER_EQUITY_V2_JOB_TYPE: &str = "owner_equity_v2";
/// Versioned JSON envelope stored in `jobs.payload_json`.
pub const OWNER_EQUITY_V2_JOB_SCHEMA_VERSION: u32 = 1;
/// Bounded queue attempts for provider/transient recovery.
pub const OWNER_EQUITY_V2_MAX_ATTEMPTS: i32 = 3;

const FAILURE_INVALID_JOB: &str = "OWNER_EQUITY_JOB_INVALID";
const FAILURE_DISABLED: &str = "MEMBERSHIP_DISABLED";
const FAILURE_STALE_GENERATION: &str = "GENERATION_MISMATCH";
const FAILURE_UNIVERSE_CHANGED: &str = "UNIVERSE_CHANGED";
const FAILURE_SNAPSHOT: &str = "SNAPSHOT_MISMATCH";
const FAILURE_ENTITLEMENT: &str = "ENTITLEMENT_MISMATCH";
const FAILURE_DATABASE: &str = "DATABASE_UNAVAILABLE";

/// Operation carried by one durable V2 job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnerEquityJobAction {
    Add,
    Retry,
    Incremental,
    DisableSnapshot,
    /// Durable idempotency receipt for an already-active duplicate add.  It is
    /// inserted terminal by the API and is never claimable.
    DuplicateReceipt,
}

impl OwnerEquityJobAction {
    pub const fn creates_generation(self) -> bool {
        matches!(self, Self::Add | Self::Retry | Self::Incremental)
    }
}

/// Exact body/job binding persisted by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerEquityJobPayload {
    pub schema_version: u32,
    pub action: OwnerEquityJobAction,
    pub membership_id: Uuid,
    pub instrument_id: String,
    pub expected_generation: Option<u64>,
    pub request_body_sha256: String,
    pub requested_through: TradingDate,
    pub max_active_instruments: u32,
    pub target_observed_sessions: u32,
    pub minimum_observed_sessions: u32,
    pub code_commit: String,
    pub entitlement_reference: String,
    pub entitlement_sha256: String,
}

impl OwnerEquityJobPayload {
    /// Validates every value before an adapter or database publisher sees it.
    pub fn validate(&self) -> Result<(), OwnerEquityWorkerError> {
        if self.schema_version != OWNER_EQUITY_V2_JOB_SCHEMA_VERSION
            || !canonical_instrument(&self.instrument_id)
            || !canonical_sha256_hex(&self.request_body_sha256)
            || CodeCommit::parse(&self.code_commit).is_err()
            || ContentHash::parse(&self.entitlement_sha256).is_err()
            || self.entitlement_reference.trim().is_empty()
            || self.entitlement_reference.len() > 512
            || self.entitlement_reference.chars().any(char::is_control)
            || OwnerEquityUniversePolicy::new(
                self.max_active_instruments,
                self.target_observed_sessions,
                self.minimum_observed_sessions,
            )
            .is_err()
        {
            return Err(OwnerEquityWorkerError::InvalidJob);
        }
        match (self.action.creates_generation(), self.expected_generation) {
            (true, Some(value)) if OwnerEquityGeneration::new(value).is_ok() => Ok(()),
            (false, None) => Ok(()),
            _ => Err(OwnerEquityWorkerError::InvalidJob),
        }
    }

    pub fn policy(&self) -> Result<OwnerEquityUniversePolicy, OwnerEquityWorkerError> {
        self.validate()?;
        OwnerEquityUniversePolicy::new(
            self.max_active_instruments,
            self.target_observed_sessions,
            self.minimum_observed_sessions,
        )
        .map_err(|_| OwnerEquityWorkerError::InvalidJob)
    }

    pub fn generation(&self) -> Result<Option<OwnerEquityGeneration>, OwnerEquityWorkerError> {
        self.validate()?;
        self.expected_generation
            .map(OwnerEquityGeneration::new)
            .transpose()
            .map_err(|_| OwnerEquityWorkerError::InvalidJob)
    }

    pub fn entitlement_hash(&self) -> Result<ContentHash, OwnerEquityWorkerError> {
        ContentHash::parse(&self.entitlement_sha256).map_err(|_| OwnerEquityWorkerError::InvalidJob)
    }

    pub fn code_revision(&self) -> Result<CodeCommit, OwnerEquityWorkerError> {
        CodeCommit::parse(&self.code_commit).map_err(|_| OwnerEquityWorkerError::InvalidJob)
    }
}

/// Hashes an untrusted public key into the queue's bounded, namespaced key.
/// The body binding remains separately visible in the payload.
pub fn durable_idempotency_key(public_key: &str) -> Result<String, OwnerEquityWorkerError> {
    let key = public_key.trim();
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b':' && byte != b'\\')
    {
        return Err(OwnerEquityWorkerError::InvalidIdempotencyKey);
    }
    let digest = Sha256::digest(key.as_bytes());
    Ok(format!("oev2:{digest:x}"))
}

/// Coverage persisted for either an admissible or insufficient generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityCoverage {
    pub observed_sessions: u32,
    pub first_session: Option<TradingDate>,
    pub last_session: Option<TradingDate>,
}

impl OwnerEquityCoverage {
    pub fn validate(
        &self,
        policy: OwnerEquityUniversePolicy,
    ) -> Result<(), OwnerEquityWorkerError> {
        let dates_valid = match (
            self.observed_sessions,
            self.first_session,
            self.last_session,
        ) {
            (0, None, None) => true,
            (count, Some(first), Some(last)) => count > 0 && first <= last,
            _ => false,
        };
        if !dates_valid || self.observed_sessions > policy.target_observed_sessions() {
            return Err(OwnerEquityWorkerError::EvidenceMismatch);
        }
        Ok(())
    }
}

/// Materialized and provider-free verified generation ready for admission.
#[derive(Debug, Clone)]
pub struct PreparedOwnerEquityGeneration {
    pub candidate: OwnerEquityGenerationCandidate,
    /// Hash of the immutable artifact manifest produced by the adapter, not a
    /// path and not the candidate bytes hash.
    pub artifact_manifest_sha256: ContentHash,
}

/// Adapter result keeps insufficient coverage typed and separate from errors.
#[derive(Debug, Clone)]
pub enum OwnerEquityMaterialization {
    Ready(Box<PreparedOwnerEquityGeneration>),
    InsufficientHistory(OwnerEquityCoverage),
}

/// Exact admitted generation descriptor used to load immutable candidates.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AdmittedGenerationDescriptor {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation_id: Uuid,
    pub instrument_id: String,
    pub generation: i64,
    pub raw_manifest_sha256: String,
    pub artifact_manifest_sha256: String,
    pub entitlement_sha256: String,
    pub capture_code_commit: String,
    pub materializer_code_commit: String,
}

/// Exact provider-free prior generation supplied to incremental adapters.
#[derive(Debug, Clone)]
pub struct OwnerEquityPriorCandidate {
    pub descriptor: AdmittedGenerationDescriptor,
    pub candidate: OwnerEquityGenerationCandidate,
}

/// Only this adapter may touch provider/Raw/artifact surfaces.  API code sees
/// none of these methods or values.  Implementations must build identities and
/// call the WP-2 functions; the helpers below provide the reviewed seams.
#[async_trait]
pub trait OwnerEquityWorkerAdapter: Send + Sync {
    async fn validate(&self, payload: &OwnerEquityJobPayload)
    -> Result<(), OwnerEquityWorkFailure>;
    async fn backfill(&self, payload: &OwnerEquityJobPayload)
    -> Result<(), OwnerEquityWorkFailure>;
    async fn materialize(
        &self,
        payload: &OwnerEquityJobPayload,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure>;
    async fn load_admitted_candidate(
        &self,
        descriptor: &AdmittedGenerationDescriptor,
    ) -> Result<OwnerEquityGenerationCandidate, OwnerEquityWorkFailure>;

    async fn validate_with_prior(
        &self,
        _owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        _prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<(), OwnerEquityWorkFailure> {
        self.validate(payload).await
    }

    async fn backfill_with_prior(
        &self,
        _owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        _prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<(), OwnerEquityWorkFailure> {
        self.backfill(payload).await
    }

    async fn materialize_with_prior(
        &self,
        _owner_user_id: Uuid,
        payload: &OwnerEquityJobPayload,
        _prior: Option<&OwnerEquityPriorCandidate>,
    ) -> Result<OwnerEquityMaterialization, OwnerEquityWorkFailure> {
        self.materialize(payload).await
    }
}

/// WP-2 network seam for runtime adapters.  It performs no queue/database work.
pub async fn capture_with_wp2<R: KisRead>(
    store: &RawStore,
    provider: &KisProvider<R>,
    identity: &OwnerEquityCaptureIdentity,
    retrieved_at: domain::UtcTimestamp,
) -> Result<OwnerEquityCaptureOutcome, OwnerEquityCollectorError> {
    capture_owner_equity_raw(store, provider, identity, retrieved_at).await
}

/// WP-2 provider-free replay seam.  A runtime adapter calls this only after it
/// has persisted the candidate artifact and obtained its manifest hash.
pub fn verify_materialized_with_wp2(
    store: &RawStore,
    identity: &OwnerEquityCaptureIdentity,
    materializer_code_commit: CodeCommit,
    candidate_bytes: &[u8],
    candidate_sha256: &ContentHash,
) -> Result<OwnerEquityGenerationCandidate, OwnerEquityCollectorError> {
    check_owner_equity_from_raw(
        store,
        identity,
        materializer_code_commit,
        candidate_bytes,
        candidate_sha256,
    )
}

/// Sanitized adapter failure.  Provider prose and response bodies cannot fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityWorkFailure {
    pub code: OwnerEquityFailureCode,
    pub retry: RetryDisposition,
}

impl OwnerEquityWorkFailure {
    pub fn new(code: &str, retry: RetryDisposition) -> Result<Self, OwnerEquityWorkerError> {
        Ok(Self {
            code: OwnerEquityFailureCode::parse(code)
                .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
            retry,
        })
    }

    pub fn from_collector(error: &OwnerEquityCollectorError) -> Self {
        Self {
            code: error.failure_code(),
            retry: error.retry_disposition(),
        }
    }
}

/// One run result for daemon metrics without sensitive fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerEquityRunOutcome {
    Idle,
    Published,
    InsufficientHistory,
    Retrying,
    Failed,
    Disabled,
    Canceled,
}

/// Value-free worker failures.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OwnerEquityWorkerError {
    InvalidIdempotencyKey,
    InvalidJob,
    InvalidLifecycle,
    StaleClaim,
    StaleGeneration,
    Disabled,
    EvidenceMismatch,
    EntitlementMismatch,
    UniverseChanged,
    SnapshotMismatch,
    DatabaseTransient,
    DatabaseIntegrity,
    QueueTransient,
    QueueIntegrity,
    CommitUnknown,
}

impl OwnerEquityWorkerError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidIdempotencyKey => "IDEMPOTENCY_KEY_INVALID",
            Self::InvalidJob => FAILURE_INVALID_JOB,
            Self::InvalidLifecycle => "LIFECYCLE_MISMATCH",
            Self::StaleClaim => "STALE_CLAIM",
            Self::StaleGeneration => FAILURE_STALE_GENERATION,
            Self::Disabled => FAILURE_DISABLED,
            Self::EvidenceMismatch => "EVIDENCE_MISMATCH",
            Self::EntitlementMismatch => FAILURE_ENTITLEMENT,
            Self::UniverseChanged => FAILURE_UNIVERSE_CHANGED,
            Self::SnapshotMismatch => FAILURE_SNAPSHOT,
            Self::DatabaseTransient => FAILURE_DATABASE,
            Self::DatabaseIntegrity => "DATABASE_CONTRACT_MISMATCH",
            Self::QueueTransient => "QUEUE_UNAVAILABLE",
            Self::QueueIntegrity => "QUEUE_CONTRACT_MISMATCH",
            Self::CommitUnknown => "COMMIT_OUTCOME_UNKNOWN",
        }
    }

    pub const fn class(self) -> ErrorClass {
        match self {
            Self::DatabaseTransient | Self::QueueTransient | Self::UniverseChanged => {
                ErrorClass::Transient
            }
            Self::InvalidJob | Self::InvalidIdempotencyKey => ErrorClass::Input,
            Self::Disabled | Self::EntitlementMismatch => ErrorClass::DataBlocked,
            Self::EvidenceMismatch | Self::SnapshotMismatch => ErrorClass::Determinism,
            Self::InvalidLifecycle
            | Self::StaleClaim
            | Self::StaleGeneration
            | Self::DatabaseIntegrity
            | Self::QueueIntegrity
            | Self::CommitUnknown => ErrorClass::Integrity,
        }
    }
}

impl fmt::Debug for OwnerEquityWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl fmt::Display for OwnerEquityWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for OwnerEquityWorkerError {}

/// Claim and execute at most one V2 job.
pub async fn run_owner_equity_once<A: OwnerEquityWorkerAdapter>(
    pool: &PgPool,
    queue: &JobQueue,
    worker_id: &str,
    adapter: &A,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    let Some(claim) = queue
        .claim_next_for(worker_id, OWNER_EQUITY_V2_JOB_TYPE)
        .await
        .map_err(map_queue_error)?
    else {
        return Ok(OwnerEquityRunOutcome::Idle);
    };
    process_owner_equity_claim(pool, queue, &claim, adapter).await
}

/// Execute a previously claimed job.  Each external phase is bracketed by a
/// lease checkpoint; stale/canceled workers stop before publication.
pub async fn process_owner_equity_claim<A: OwnerEquityWorkerAdapter>(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    adapter: &A,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    let payload = match parse_claim(claim) {
        Ok(payload) => payload,
        Err(error) => return settle_unbound_failure(queue, claim, error).await,
    };
    checkpoint(queue, claim).await?;
    let prior = if payload.action == OwnerEquityJobAction::Incremental {
        match load_prior_admitted_candidate(pool, claim.job.owner_user_id, &payload, adapter).await
        {
            Ok(prior) => Some(prior),
            Err(error) => return settle_bound_failure(pool, queue, claim, &payload, error).await,
        }
    } else {
        None
    };

    match payload.action {
        OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry => {
            let mut state = membership_state(pool, queue, claim, &payload).await?;
            if state == OwnerEquityMembershipState::Disabled {
                return settle_disabled(queue, claim).await;
            }
            if state == OwnerEquityMembershipState::Requested {
                transition_membership(
                    pool,
                    queue,
                    claim,
                    &payload,
                    OwnerEquityMembershipState::Requested,
                    OwnerEquityMembershipState::Validating,
                )
                .await?;
                state = OwnerEquityMembershipState::Validating;
            }
            if state == OwnerEquityMembershipState::Validating {
                checkpoint(queue, claim).await?;
                if let Err(failure) = adapter
                    .validate_with_prior(claim.job.owner_user_id, &payload, prior.as_ref())
                    .await
                {
                    return settle_work_failure(pool, queue, claim, &payload, failure).await;
                }
                transition_membership(
                    pool,
                    queue,
                    claim,
                    &payload,
                    OwnerEquityMembershipState::Validating,
                    OwnerEquityMembershipState::Backfilling,
                )
                .await?;
                state = OwnerEquityMembershipState::Backfilling;
            }
            if state == OwnerEquityMembershipState::Backfilling {
                checkpoint(queue, claim).await?;
                if let Err(failure) = adapter
                    .backfill_with_prior(claim.job.owner_user_id, &payload, prior.as_ref())
                    .await
                {
                    return settle_work_failure(pool, queue, claim, &payload, failure).await;
                }
                transition_membership(
                    pool,
                    queue,
                    claim,
                    &payload,
                    OwnerEquityMembershipState::Backfilling,
                    OwnerEquityMembershipState::Materializing,
                )
                .await?;
                state = OwnerEquityMembershipState::Materializing;
            }
            if state != OwnerEquityMembershipState::Materializing {
                return settle_bound_failure(
                    pool,
                    queue,
                    claim,
                    &payload,
                    OwnerEquityWorkerError::InvalidLifecycle,
                )
                .await;
            }
        }
        OwnerEquityJobAction::Incremental => {
            if membership_state(pool, queue, claim, &payload).await?
                != OwnerEquityMembershipState::Ready
            {
                return settle_bound_failure(
                    pool,
                    queue,
                    claim,
                    &payload,
                    OwnerEquityWorkerError::InvalidLifecycle,
                )
                .await;
            }
            checkpoint(queue, claim).await?;
            if let Err(failure) = adapter
                .validate_with_prior(claim.job.owner_user_id, &payload, prior.as_ref())
                .await
            {
                return settle_work_failure(pool, queue, claim, &payload, failure).await;
            }
            if let Err(failure) = adapter
                .backfill_with_prior(claim.job.owner_user_id, &payload, prior.as_ref())
                .await
            {
                return settle_work_failure(pool, queue, claim, &payload, failure).await;
            }
        }
        OwnerEquityJobAction::DisableSnapshot => {
            if membership_state(pool, queue, claim, &payload).await?
                != OwnerEquityMembershipState::Disabled
            {
                return settle_bound_failure(
                    pool,
                    queue,
                    claim,
                    &payload,
                    OwnerEquityWorkerError::InvalidLifecycle,
                )
                .await;
            }
            return build_and_publish(pool, queue, claim, &payload, adapter, None).await;
        }
        OwnerEquityJobAction::DuplicateReceipt => {
            return settle_unbound_failure(queue, claim, OwnerEquityWorkerError::InvalidJob).await;
        }
    }

    checkpoint(queue, claim).await?;
    match adapter
        .materialize_with_prior(claim.job.owner_user_id, &payload, prior.as_ref())
        .await
    {
        Ok(OwnerEquityMaterialization::Ready(prepared)) => {
            if let Err(error) = validate_prepared(&payload, &prepared) {
                return settle_bound_failure(pool, queue, claim, &payload, error).await;
            }
            build_and_publish(pool, queue, claim, &payload, adapter, Some(*prepared)).await
        }
        Ok(OwnerEquityMaterialization::InsufficientHistory(coverage)) => {
            match persist_insufficient(pool, queue, claim, &payload, coverage).await {
                Ok(outcome) => Ok(outcome),
                Err(error) => settle_publish_failure(pool, queue, claim, &payload, error).await,
            }
        }
        Err(failure) => settle_work_failure(pool, queue, claim, &payload, failure).await,
    }
}

fn parse_claim(claim: &ClaimedJob) -> Result<OwnerEquityJobPayload, OwnerEquityWorkerError> {
    if claim.job.job_type != OWNER_EQUITY_V2_JOB_TYPE
        || claim.job.owner_user_id == Uuid::nil()
        || claim.job.status != JobStatus::Running
    {
        return Err(OwnerEquityWorkerError::InvalidJob);
    }
    let payload: OwnerEquityJobPayload = serde_json::from_value(claim.job.payload_json.clone())
        .map_err(|_| OwnerEquityWorkerError::InvalidJob)?;
    payload.validate()?;
    Ok(payload)
}

async fn checkpoint(queue: &JobQueue, claim: &ClaimedJob) -> Result<(), OwnerEquityWorkerError> {
    match queue.heartbeat(claim).await.map_err(map_queue_error)? {
        HeartbeatStatus::Extended => Ok(()),
        HeartbeatStatus::Canceled | HeartbeatStatus::LeaseLost => {
            Err(OwnerEquityWorkerError::StaleClaim)
        }
    }
}

async fn membership_state(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
) -> Result<OwnerEquityMembershipState, OwnerEquityWorkerError> {
    let mut tx = begin_worker_tx(pool, claim.job.owner_user_id).await?;
    queue
        .lock_claim_in(&mut tx, claim)
        .await
        .map_err(map_queue_error)?;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM public.owner_equity_memberships
         WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3
         FOR SHARE",
    )
    .bind(payload.membership_id)
    .bind(claim.job.owner_user_id)
    .bind(&payload.instrument_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_database_error)?;
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)?;
    state
        .ok_or(OwnerEquityWorkerError::InvalidLifecycle)?
        .parse()
        .map_err(|_| OwnerEquityWorkerError::InvalidLifecycle)
}

async fn transition_membership(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    from: OwnerEquityMembershipState,
    to: OwnerEquityMembershipState,
) -> Result<(), OwnerEquityWorkerError> {
    from.transition_to(to)
        .map_err(|_| OwnerEquityWorkerError::InvalidLifecycle)?;
    let mut tx = begin_worker_tx(pool, claim.job.owner_user_id).await?;
    queue
        .lock_claim_in(&mut tx, claim)
        .await
        .map_err(map_queue_error)?;
    let changed = sqlx::query(
        "UPDATE public.owner_equity_memberships
         SET state = $4, transition_actor_user_id = $2,
             transition_code_commit = $5, transition_entitlement_sha256 = $6,
             error_code = NULL, error_retryable = NULL, disabled_at = NULL,
             updated_at = now()
         WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3 AND state = $7",
    )
    .bind(payload.membership_id)
    .bind(claim.job.owner_user_id)
    .bind(&payload.instrument_id)
    .bind(to.as_str())
    .bind(&payload.code_commit)
    .bind(&payload.entitlement_sha256)
    .bind(from.as_str())
    .execute(&mut *tx)
    .await
    .map_err(map_database_error)?;
    if changed.rows_affected() != 1 {
        tx.rollback().await.ok();
        return Err(OwnerEquityWorkerError::InvalidLifecycle);
    }
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)
}

fn validate_prepared(
    payload: &OwnerEquityJobPayload,
    prepared: &PreparedOwnerEquityGeneration,
) -> Result<(), OwnerEquityWorkerError> {
    let candidate = &prepared.candidate;
    let policy = payload.policy()?;
    if candidate.instrument_id.to_string() != payload.instrument_id
        || candidate.target_observed_sessions != policy.target_observed_sessions()
        || candidate.minimum_observed_sessions != policy.minimum_observed_sessions()
        || candidate.observed_sessions != candidate.bars.len() as u32
        || candidate.observed_sessions < policy.minimum_observed_sessions()
        || candidate.source_pins.entitlement_sha256.as_str() != payload.entitlement_sha256
        || candidate.source_pins.capture_code_commit.as_str() != payload.code_commit
        || candidate.source_pins.materializer_code_commit.as_str() != payload.code_commit
        || candidate
            .source_pins
            .raw_manifest_sha256
            .as_str()
            .is_empty()
        || prepared.artifact_manifest_sha256.as_str().is_empty()
    {
        return Err(OwnerEquityWorkerError::EvidenceMismatch);
    }
    Ok(())
}

async fn build_and_publish<A: OwnerEquityWorkerAdapter>(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    adapter: &A,
    prepared: Option<PreparedOwnerEquityGeneration>,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    let descriptors = match ready_descriptors(pool, claim.job.owner_user_id).await {
        Ok(value) => value,
        Err(error) => return settle_publish_failure(pool, queue, claim, payload, error).await,
    };
    let mut inputs = Vec::with_capacity(descriptors.len() + usize::from(prepared.is_some()));
    for descriptor in descriptors {
        if descriptor.membership_id == payload.membership_id && prepared.is_some() {
            continue;
        }
        let candidate = match adapter.load_admitted_candidate(&descriptor).await {
            Ok(value) => value,
            Err(failure) => {
                return settle_work_failure(pool, queue, claim, payload, failure).await;
            }
        };
        let input = match admitted_input(&descriptor, candidate) {
            Ok(value) => value,
            Err(error) => {
                return settle_bound_failure(pool, queue, claim, payload, error).await;
            }
        };
        inputs.push(input);
    }
    if let Some(prepared) = &prepared {
        let generation = match payload.generation() {
            Ok(Some(value)) => value,
            Ok(None) => {
                return settle_bound_failure(
                    pool,
                    queue,
                    claim,
                    payload,
                    OwnerEquityWorkerError::InvalidJob,
                )
                .await;
            }
            Err(error) => {
                return settle_bound_failure(pool, queue, claim, payload, error).await;
            }
        };
        let pins = match admission_pins(payload, prepared) {
            Ok(value) => value,
            Err(error) => {
                return settle_bound_failure(pool, queue, claim, payload, error).await;
            }
        };
        inputs.push(OwnerEquityAdmittedCandidate::active_ready(
            prepared.candidate.clone(),
            generation,
            pins,
        ));
    }
    let as_of = if inputs.is_empty() {
        payload.requested_through
    } else {
        match latest_common_session(&inputs) {
            Some(value) => value,
            None => {
                return settle_bound_failure(
                    pool,
                    queue,
                    claim,
                    payload,
                    OwnerEquityWorkerError::SnapshotMismatch,
                )
                .await;
            }
        }
    };
    let snapshot = match compute_owner_equity_signal_snapshot(&inputs, as_of) {
        Ok(value) => value,
        Err(_) => {
            return settle_bound_failure(
                pool,
                queue,
                claim,
                payload,
                OwnerEquityWorkerError::SnapshotMismatch,
            )
            .await;
        }
    };
    if snapshot.rows.len() != inputs.len()
        || snapshot
            .rows
            .iter()
            .enumerate()
            .any(|(index, row)| row.rank != index + 1)
    {
        return settle_bound_failure(
            pool,
            queue,
            claim,
            payload,
            OwnerEquityWorkerError::SnapshotMismatch,
        )
        .await;
    }
    match publish_snapshot(pool, queue, claim, payload, prepared.as_ref(), &snapshot).await {
        Ok(()) => Ok(OwnerEquityRunOutcome::Published),
        Err(error) => settle_publish_failure(pool, queue, claim, payload, error).await,
    }
}

async fn settle_publish_failure(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    error: OwnerEquityWorkerError,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    if matches!(
        error,
        OwnerEquityWorkerError::StaleClaim
            | OwnerEquityWorkerError::QueueTransient
            | OwnerEquityWorkerError::QueueIntegrity
            | OwnerEquityWorkerError::CommitUnknown
    ) {
        Err(error)
    } else if error == OwnerEquityWorkerError::Disabled {
        settle_disabled(queue, claim).await
    } else {
        settle_bound_failure(pool, queue, claim, payload, error).await
    }
}

fn admitted_input(
    descriptor: &AdmittedGenerationDescriptor,
    candidate: OwnerEquityGenerationCandidate,
) -> Result<OwnerEquityAdmittedCandidate, OwnerEquityWorkerError> {
    if candidate.instrument_id.to_string() != descriptor.instrument_id
        || candidate.source_pins.raw_manifest_sha256.as_str() != descriptor.raw_manifest_sha256
        || candidate.source_pins.entitlement_sha256.as_str() != descriptor.entitlement_sha256
        || candidate.source_pins.capture_code_commit.as_str() != descriptor.capture_code_commit
        || candidate.source_pins.materializer_code_commit.as_str()
            != descriptor.materializer_code_commit
    {
        return Err(OwnerEquityWorkerError::EvidenceMismatch);
    }
    let generation = OwnerEquityGeneration::new(
        u64::try_from(descriptor.generation)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?;
    let pins = OwnerEquityAdmissionPins {
        raw_manifest_sha256: ContentHash::parse(&descriptor.raw_manifest_sha256)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
        artifact_manifest_sha256: ContentHash::parse(&descriptor.artifact_manifest_sha256)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
        entitlement_sha256: ContentHash::parse(&descriptor.entitlement_sha256)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
        capture_code_commit: CodeCommit::parse(&descriptor.capture_code_commit)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
        materializer_code_commit: CodeCommit::parse(&descriptor.materializer_code_commit)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    };
    Ok(OwnerEquityAdmittedCandidate::active_ready(
        candidate, generation, pins,
    ))
}

fn admission_pins(
    payload: &OwnerEquityJobPayload,
    prepared: &PreparedOwnerEquityGeneration,
) -> Result<OwnerEquityAdmissionPins, OwnerEquityWorkerError> {
    Ok(OwnerEquityAdmissionPins {
        raw_manifest_sha256: prepared.candidate.source_pins.raw_manifest_sha256.clone(),
        artifact_manifest_sha256: prepared.artifact_manifest_sha256.clone(),
        entitlement_sha256: payload.entitlement_hash()?,
        capture_code_commit: prepared.candidate.source_pins.capture_code_commit.clone(),
        materializer_code_commit: prepared
            .candidate
            .source_pins
            .materializer_code_commit
            .clone(),
    })
}

fn latest_common_session(inputs: &[OwnerEquityAdmittedCandidate]) -> Option<TradingDate> {
    if inputs.is_empty() {
        return None;
    }
    let mut common = inputs[0]
        .candidate
        .bars
        .iter()
        .map(|bar| bar.session_date)
        .collect::<BTreeSet<_>>();
    for input in &inputs[1..] {
        let dates = input
            .candidate
            .bars
            .iter()
            .map(|bar| bar.session_date)
            .collect::<BTreeSet<_>>();
        common.retain(|date| dates.contains(date));
    }
    common.into_iter().next_back()
}

async fn load_prior_admitted_candidate<A: OwnerEquityWorkerAdapter>(
    pool: &PgPool,
    owner: Uuid,
    payload: &OwnerEquityJobPayload,
    adapter: &A,
) -> Result<OwnerEquityPriorCandidate, OwnerEquityWorkerError> {
    let expected = payload
        .expected_generation
        .and_then(|value| value.checked_sub(1))
        .filter(|value| *value > 0)
        .ok_or(OwnerEquityWorkerError::StaleGeneration)?;
    let expected = i64::try_from(expected).map_err(|_| OwnerEquityWorkerError::StaleGeneration)?;
    let mut tx = begin_worker_tx(pool, owner).await?;
    let descriptor: Option<AdmittedGenerationDescriptor> = sqlx::query_as(
        "SELECT a.owner_user_id, a.membership_id, a.generation_id,
                a.instrument_id, a.generation, a.raw_manifest_sha256,
                a.artifact_manifest_sha256, a.entitlement_sha256,
                a.capture_code_commit, a.materializer_code_commit
         FROM public.owner_equity_generation_admissions AS a
         WHERE a.membership_id = $1
           AND a.owner_user_id = $4
           AND a.instrument_id = $2
           AND a.generation = $3",
    )
    .bind(payload.membership_id)
    .bind(&payload.instrument_id)
    .bind(expected)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_database_error)?;
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)?;
    let descriptor = descriptor.ok_or(OwnerEquityWorkerError::StaleGeneration)?;
    let candidate = adapter
        .load_admitted_candidate(&descriptor)
        .await
        .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?;
    admitted_input(&descriptor, candidate.clone())?;
    Ok(OwnerEquityPriorCandidate {
        descriptor,
        candidate,
    })
}

async fn ready_descriptors(
    pool: &PgPool,
    owner: Uuid,
) -> Result<Vec<AdmittedGenerationDescriptor>, OwnerEquityWorkerError> {
    let mut tx = begin_worker_tx(pool, owner).await?;
    let rows = sqlx::query_as(
        "SELECT a.owner_user_id, a.membership_id, a.generation_id,
                a.instrument_id, a.generation, a.raw_manifest_sha256,
                a.artifact_manifest_sha256, a.entitlement_sha256,
                a.capture_code_commit, a.materializer_code_commit
         FROM public.owner_equity_memberships AS m
         JOIN LATERAL (
              SELECT admission.*
              FROM public.owner_equity_generation_admissions AS admission
              WHERE admission.membership_id = m.id
                AND admission.owner_user_id = m.owner_user_id
                AND admission.instrument_id = m.instrument_id
              ORDER BY admission.generation DESC
              LIMIT 1
         ) AS a ON true
         WHERE m.owner_user_id = $1 AND m.state = 'READY'
         ORDER BY m.instrument_id",
    )
    .bind(owner)
    .fetch_all(&mut *tx)
    .await
    .map_err(map_database_error)?;
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)?;
    Ok(rows)
}

async fn publish_snapshot(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    prepared: Option<&PreparedOwnerEquityGeneration>,
    snapshot: &OwnerEquitySignalSnapshotCandidate,
) -> Result<(), OwnerEquityWorkerError> {
    let owner = claim.job.owner_user_id;
    let mut tx = begin_worker_tx(pool, owner).await?;
    queue
        .lock_claim_in(&mut tx, claim)
        .await
        .map_err(map_queue_error)?;
    verify_policy_entitlement(&mut tx, owner, payload).await?;
    let memberships: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, instrument_id, state
         FROM public.owner_equity_memberships
         WHERE owner_user_id = $1
         ORDER BY id FOR UPDATE",
    )
    .bind(owner)
    .fetch_all(&mut *tx)
    .await
    .map_err(map_database_error)?;
    let rows = snapshot
        .rows
        .iter()
        .map(|row| row.instrument_id.to_string())
        .collect::<BTreeSet<_>>();
    let expected_current = expected_publish_state(payload.action)?;
    if let Err(error) = validate_locked_publication(
        payload,
        &memberships,
        prepared.is_some(),
        &rows,
        snapshot.rows.len(),
    ) {
        tx.rollback().await.ok();
        return Err(error);
    }

    if let Some(prepared) = prepared {
        insert_generation_and_admission(&mut tx, owner, payload, prepared).await?;
        if expected_current == "MATERIALIZING" {
            let updated = sqlx::query(
                "UPDATE public.owner_equity_memberships
                 SET state = 'READY', transition_actor_user_id = $2,
                     transition_code_commit = $4,
                     transition_entitlement_sha256 = $5,
                     error_code = NULL, error_retryable = NULL,
                     disabled_at = NULL, updated_at = now()
                 WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3
                   AND state = 'MATERIALIZING'",
            )
            .bind(payload.membership_id)
            .bind(owner)
            .bind(&payload.instrument_id)
            .bind(&payload.code_commit)
            .bind(&payload.entitlement_sha256)
            .execute(&mut *tx)
            .await
            .map_err(map_database_error)?;
            if updated.rows_affected() != 1 {
                return Err(OwnerEquityWorkerError::InvalidLifecycle);
            }
        }
    }

    let snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.owner_equity_signal_snapshots
         (id, owner_user_id, as_of_session, universe_sha256, row_count,
          signal_code_commit)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(snapshot_id)
    .bind(owner)
    .bind(snapshot.as_of.as_naive_date())
    .bind(snapshot.universe_sha256.as_str())
    .bind(i32::try_from(snapshot.rows.len()).map_err(|_| OwnerEquityWorkerError::SnapshotMismatch)?)
    .bind(&payload.code_commit)
    .execute(&mut *tx)
    .await
    .map_err(map_database_error)?;

    let descriptors = exact_snapshot_descriptors(&mut tx, owner).await?;
    for row in &snapshot.rows {
        let instrument = row.instrument_id.to_string();
        let descriptor = descriptors
            .get(&instrument)
            .ok_or(OwnerEquityWorkerError::UniverseChanged)?;
        if descriptor.generation != i64::try_from(row.generation.get()).unwrap_or(-1)
            || descriptor.raw_manifest_sha256 != row.admission_pins.raw_manifest_sha256.as_str()
            || descriptor.artifact_manifest_sha256
                != row.admission_pins.artifact_manifest_sha256.as_str()
        {
            return Err(OwnerEquityWorkerError::EvidenceMismatch);
        }
        sqlx::query(
            "INSERT INTO public.owner_equity_signal_snapshot_rows
             (snapshot_id, owner_user_id, instrument_id, membership_id,
              generation_id, generation, rank, signals_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(snapshot_id)
        .bind(owner)
        .bind(&instrument)
        .bind(descriptor.membership_id)
        .bind(descriptor.generation_id)
        .bind(descriptor.generation)
        .bind(i32::try_from(row.rank).map_err(|_| OwnerEquityWorkerError::SnapshotMismatch)?)
        .bind(serde_json::to_value(row).map_err(|_| OwnerEquityWorkerError::SnapshotMismatch)?)
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
    }
    let published = sqlx::query(
        "UPDATE public.owner_equity_signal_snapshots
         SET published_at = now()
         WHERE id = $1 AND owner_user_id = $2 AND published_at IS NULL",
    )
    .bind(snapshot_id)
    .bind(owner)
    .execute(&mut *tx)
    .await
    .map_err(map_database_error)?;
    if published.rows_affected() != 1 {
        return Err(OwnerEquityWorkerError::SnapshotMismatch);
    }
    match queue
        .settle_success_in(&mut tx, claim)
        .await
        .map_err(map_queue_error)?
    {
        SettleResult::Committed(job) if job.status == JobStatus::Succeeded => {}
        SettleResult::Canceled(_) => return Err(OwnerEquityWorkerError::Disabled),
        _ => return Err(OwnerEquityWorkerError::QueueIntegrity),
    }
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)
}

fn expected_publish_state(
    action: OwnerEquityJobAction,
) -> Result<&'static str, OwnerEquityWorkerError> {
    match action {
        OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry => Ok("MATERIALIZING"),
        OwnerEquityJobAction::Incremental => Ok("READY"),
        OwnerEquityJobAction::DisableSnapshot => Ok("DISABLED"),
        OwnerEquityJobAction::DuplicateReceipt => Err(OwnerEquityWorkerError::InvalidJob),
    }
}

fn validate_locked_publication(
    payload: &OwnerEquityJobPayload,
    memberships: &[(Uuid, String, String)],
    includes_prepared_generation: bool,
    snapshot_instruments: &BTreeSet<String>,
    snapshot_row_count: usize,
) -> Result<(), OwnerEquityWorkerError> {
    let expected_current = expected_publish_state(payload.action)?;
    let current = memberships
        .iter()
        .find(|(id, instrument, _)| {
            *id == payload.membership_id && instrument == &payload.instrument_id
        })
        .ok_or(OwnerEquityWorkerError::InvalidLifecycle)?;
    if current.2 != expected_current {
        return Err(if current.2 == "DISABLED" {
            OwnerEquityWorkerError::Disabled
        } else {
            OwnerEquityWorkerError::InvalidLifecycle
        });
    }

    let mut exact = memberships
        .iter()
        .filter(|(_, _, state)| state == "READY")
        .map(|(_, instrument, _)| instrument.clone())
        .collect::<BTreeSet<_>>();
    if includes_prepared_generation && expected_current == "MATERIALIZING" {
        exact.insert(payload.instrument_id.clone());
    }
    if exact != *snapshot_instruments || snapshot_row_count != exact.len() {
        return Err(OwnerEquityWorkerError::UniverseChanged);
    }
    Ok(())
}

async fn exact_snapshot_descriptors(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
) -> Result<BTreeMap<String, AdmittedGenerationDescriptor>, OwnerEquityWorkerError> {
    let rows: Vec<AdmittedGenerationDescriptor> = sqlx::query_as(
        "SELECT a.owner_user_id, a.membership_id, a.generation_id,
                a.instrument_id, a.generation, a.raw_manifest_sha256,
                a.artifact_manifest_sha256, a.entitlement_sha256,
                a.capture_code_commit, a.materializer_code_commit
         FROM public.owner_equity_memberships AS m
         JOIN LATERAL (
              SELECT admission.*
              FROM public.owner_equity_generation_admissions AS admission
              WHERE admission.membership_id = m.id
                AND admission.owner_user_id = m.owner_user_id
                AND admission.instrument_id = m.instrument_id
              ORDER BY admission.generation DESC LIMIT 1
         ) AS a ON true
         WHERE m.owner_user_id = $1 AND m.state = 'READY'
         ORDER BY m.instrument_id",
    )
    .bind(owner)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let map = rows
        .into_iter()
        .map(|row| (row.instrument_id.clone(), row))
        .collect::<BTreeMap<_, _>>();
    Ok(map)
}

async fn insert_generation_and_admission(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    payload: &OwnerEquityJobPayload,
    prepared: &PreparedOwnerEquityGeneration,
) -> Result<(), OwnerEquityWorkerError> {
    let generation = payload
        .generation()?
        .ok_or(OwnerEquityWorkerError::InvalidJob)?;
    let current: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(generation), 0)
         FROM public.owner_equity_instrument_generations
         WHERE membership_id = $1 AND owner_user_id = $2",
    )
    .bind(payload.membership_id)
    .bind(owner)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    require_next_generation(current, generation)?;
    let candidate = &prepared.candidate;
    let generation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations
         (id, membership_id, owner_user_id, instrument_id, generation,
          target_observed_sessions, minimum_observed_sessions, observed_sessions,
          first_session, last_session)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(generation_id)
    .bind(payload.membership_id)
    .bind(owner)
    .bind(&payload.instrument_id)
    .bind(i64::try_from(generation.get()).map_err(|_| OwnerEquityWorkerError::StaleGeneration)?)
    .bind(
        i32::try_from(candidate.target_observed_sessions)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .bind(
        i32::try_from(candidate.minimum_observed_sessions)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .bind(
        i32::try_from(candidate.observed_sessions)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .bind(candidate.first_observed_date.as_naive_date())
    .bind(candidate.last_observed_date.as_naive_date())
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    let pins = admission_pins(payload, prepared)?;
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions
         (generation_id, owner_user_id, membership_id, instrument_id, generation,
          raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
          capture_code_commit, materializer_code_commit)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(generation_id)
    .bind(owner)
    .bind(payload.membership_id)
    .bind(&payload.instrument_id)
    .bind(i64::try_from(generation.get()).map_err(|_| OwnerEquityWorkerError::StaleGeneration)?)
    .bind(pins.raw_manifest_sha256.as_str())
    .bind(pins.artifact_manifest_sha256.as_str())
    .bind(pins.entitlement_sha256.as_str())
    .bind(pins.capture_code_commit.as_str())
    .bind(pins.materializer_code_commit.as_str())
    .execute(&mut **tx)
    .await
    .map_err(map_database_error)?;
    Ok(())
}

async fn persist_insufficient(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    coverage: OwnerEquityCoverage,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    let policy = payload.policy()?;
    coverage.validate(policy)?;
    if coverage.observed_sessions >= policy.minimum_observed_sessions() {
        return Err(OwnerEquityWorkerError::EvidenceMismatch);
    }
    let owner = claim.job.owner_user_id;
    let generation = payload
        .generation()?
        .ok_or(OwnerEquityWorkerError::InvalidJob)?;
    let mut tx = begin_worker_tx(pool, owner).await?;
    queue
        .lock_claim_in(&mut tx, claim)
        .await
        .map_err(map_queue_error)?;
    verify_policy_entitlement(&mut tx, owner, payload).await?;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM public.owner_equity_memberships
         WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3
         FOR UPDATE",
    )
    .bind(payload.membership_id)
    .bind(owner)
    .bind(&payload.instrument_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_database_error)?;
    if state.as_deref() == Some("DISABLED") {
        return Err(OwnerEquityWorkerError::Disabled);
    }
    if state.as_deref() != Some("MATERIALIZING") {
        return Err(OwnerEquityWorkerError::InvalidLifecycle);
    }
    let current: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(generation), 0)
         FROM public.owner_equity_instrument_generations
         WHERE membership_id = $1 AND owner_user_id = $2",
    )
    .bind(payload.membership_id)
    .bind(owner)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_database_error)?;
    require_next_generation(current, generation)?;
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations
         (membership_id, owner_user_id, instrument_id, generation,
          target_observed_sessions, minimum_observed_sessions, observed_sessions,
          first_session, last_session)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(payload.membership_id)
    .bind(owner)
    .bind(&payload.instrument_id)
    .bind(i64::try_from(generation.get()).map_err(|_| OwnerEquityWorkerError::StaleGeneration)?)
    .bind(
        i32::try_from(policy.target_observed_sessions())
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .bind(
        i32::try_from(policy.minimum_observed_sessions())
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .bind(
        i32::try_from(coverage.observed_sessions)
            .map_err(|_| OwnerEquityWorkerError::EvidenceMismatch)?,
    )
    .bind(coverage.first_session.map(|date| date.as_naive_date()))
    .bind(coverage.last_session.map(|date| date.as_naive_date()))
    .execute(&mut *tx)
    .await
    .map_err(map_database_error)?;
    let changed = sqlx::query(
        "UPDATE public.owner_equity_memberships
         SET state = 'INSUFFICIENT_HISTORY', transition_actor_user_id = $2,
             transition_code_commit = $4, transition_entitlement_sha256 = $5,
             error_code = NULL, error_retryable = NULL, disabled_at = NULL,
             updated_at = now()
         WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3
           AND state = 'MATERIALIZING'",
    )
    .bind(payload.membership_id)
    .bind(owner)
    .bind(&payload.instrument_id)
    .bind(&payload.code_commit)
    .bind(&payload.entitlement_sha256)
    .execute(&mut *tx)
    .await
    .map_err(map_database_error)?;
    if changed.rows_affected() != 1 {
        return Err(OwnerEquityWorkerError::InvalidLifecycle);
    }
    match queue
        .settle_success_in(&mut tx, claim)
        .await
        .map_err(map_queue_error)?
    {
        SettleResult::Committed(job) if job.status == JobStatus::Succeeded => {}
        SettleResult::Canceled(_) => return Err(OwnerEquityWorkerError::StaleClaim),
        _ => return Err(OwnerEquityWorkerError::QueueIntegrity),
    }
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)?;
    Ok(OwnerEquityRunOutcome::InsufficientHistory)
}

fn require_next_generation(
    current: i64,
    expected: OwnerEquityGeneration,
) -> Result<(), OwnerEquityWorkerError> {
    let expected =
        i64::try_from(expected.get()).map_err(|_| OwnerEquityWorkerError::StaleGeneration)?;
    if current.checked_add(1) == Some(expected) {
        Ok(())
    } else {
        Err(OwnerEquityWorkerError::StaleGeneration)
    }
}

async fn settle_work_failure(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    failure: OwnerEquityWorkFailure,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    let error = match failure.retry {
        RetryDisposition::Retryable => OwnerEquityWorkerError::DatabaseTransient,
        RetryDisposition::Terminal => OwnerEquityWorkerError::EvidenceMismatch,
    };
    settle_bound_failure_with_code(pool, queue, claim, payload, error, failure.code.as_str()).await
}

async fn settle_bound_failure(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    error: OwnerEquityWorkerError,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    settle_bound_failure_with_code(pool, queue, claim, payload, error, error.code()).await
}

async fn settle_bound_failure_with_code(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    payload: &OwnerEquityJobPayload,
    error: OwnerEquityWorkerError,
    failure_code: &str,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    let mut tx = begin_worker_tx(pool, claim.job.owner_user_id).await?;
    let settled = queue
        .settle_failure_in(
            &mut tx,
            claim,
            error.class(),
            failure_code,
            "owner equity work failed",
        )
        .await
        .map_err(map_queue_error)?;
    let status = match &settled {
        SettleResult::Committed(job) | SettleResult::Canceled(job) => job.status,
    };
    if status == JobStatus::Failed
        && matches!(
            payload.action,
            OwnerEquityJobAction::Add | OwnerEquityJobAction::Retry
        )
    {
        let changed = sqlx::query(
            "UPDATE public.owner_equity_memberships
             SET state = 'FAILED', transition_actor_user_id = $2,
                 transition_code_commit = $4, transition_entitlement_sha256 = $5,
                 error_code = $6, error_retryable = $7, disabled_at = NULL,
                 updated_at = now()
             WHERE id = $1 AND owner_user_id = $2 AND instrument_id = $3
               AND state IN ('VALIDATING', 'BACKFILLING', 'MATERIALIZING')",
        )
        .bind(payload.membership_id)
        .bind(claim.job.owner_user_id)
        .bind(&payload.instrument_id)
        .bind(&payload.code_commit)
        .bind(&payload.entitlement_sha256)
        .bind(failure_code)
        .bind(error.class().retryable())
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
        // A newer retry or an Owner disable may have moved the membership
        // after this claim started.  The stale claim is still settled, but it
        // must never overwrite that newer lifecycle owner.
        let _membership_was_current = changed.rows_affected() == 1;
    }
    tx.commit()
        .await
        .map_err(|_| OwnerEquityWorkerError::CommitUnknown)?;
    Ok(match status {
        JobStatus::Queued => OwnerEquityRunOutcome::Retrying,
        JobStatus::Failed => OwnerEquityRunOutcome::Failed,
        JobStatus::Canceled => OwnerEquityRunOutcome::Canceled,
        _ => return Err(OwnerEquityWorkerError::QueueIntegrity),
    })
}

async fn settle_unbound_failure(
    queue: &JobQueue,
    claim: &ClaimedJob,
    error: OwnerEquityWorkerError,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    match queue
        .settle_failure(
            claim,
            error.class(),
            error.code(),
            "owner equity job rejected",
        )
        .await
        .map_err(map_queue_error)?
    {
        SettleResult::Committed(job) if job.status == JobStatus::Queued => {
            Ok(OwnerEquityRunOutcome::Retrying)
        }
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            Ok(OwnerEquityRunOutcome::Failed)
        }
        SettleResult::Canceled(_) => Ok(OwnerEquityRunOutcome::Canceled),
        _ => Err(OwnerEquityWorkerError::QueueIntegrity),
    }
}

async fn settle_disabled(
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<OwnerEquityRunOutcome, OwnerEquityWorkerError> {
    match queue
        .settle_failure(
            claim,
            ErrorClass::DataBlocked,
            FAILURE_DISABLED,
            "owner equity membership disabled",
        )
        .await
        .map_err(map_queue_error)?
    {
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            Ok(OwnerEquityRunOutcome::Disabled)
        }
        SettleResult::Canceled(_) => Ok(OwnerEquityRunOutcome::Canceled),
        _ => Err(OwnerEquityWorkerError::QueueIntegrity),
    }
}

async fn verify_policy_entitlement(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    payload: &OwnerEquityJobPayload,
) -> Result<(), OwnerEquityWorkerError> {
    // The policy is intentionally SELECT-only for `worker`; PostgreSQL row
    // locking would require an UPDATE privilege that migration 0053 does not
    // grant. Publication locks every owner membership below and uses
    // conditional UPDATEs for lifecycle CAS. This read only rejects a job
    // whose durable policy receipt no longer matches the current policy.
    let policy: Option<(i32, i32, i32)> = sqlx::query_as(
        "SELECT max_active_instruments, target_observed_sessions,
                minimum_observed_sessions
         FROM public.owner_equity_universe_policies
         WHERE owner_user_id = $1",
    )
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if policy
        != Some((
            i32::try_from(payload.max_active_instruments)
                .map_err(|_| OwnerEquityWorkerError::InvalidJob)?,
            i32::try_from(payload.target_observed_sessions)
                .map_err(|_| OwnerEquityWorkerError::InvalidJob)?,
            i32::try_from(payload.minimum_observed_sessions)
                .map_err(|_| OwnerEquityWorkerError::InvalidJob)?,
        ))
    {
        return Err(OwnerEquityWorkerError::InvalidJob);
    }
    let hash = payload
        .entitlement_sha256
        .strip_prefix("sha256:")
        .ok_or(OwnerEquityWorkerError::EntitlementMismatch)?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM public.data_entitlements
             WHERE contract_document_sha256 = $1
               AND contract_reference = $2
               AND status = 'ACTIVE'
               AND effective_from <= $3 AND effective_until >= $3
         )",
    )
    .bind(hash)
    .bind(&payload.entitlement_reference)
    .bind(payload.requested_through.as_naive_date())
    .fetch_one(&mut **tx)
    .await
    .map_err(map_database_error)?;
    if !active {
        return Err(OwnerEquityWorkerError::EntitlementMismatch);
    }
    Ok(())
}

async fn begin_worker_tx(
    pool: &PgPool,
    owner: Uuid,
) -> Result<Transaction<'_, Postgres>, OwnerEquityWorkerError> {
    let mut tx = pool.begin().await.map_err(map_database_error)?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_database_error)?;
    crate::paper_execution::set_paper_transaction_timeouts(&mut tx)
        .await
        .map_err(map_database_error)?;
    Ok(tx)
}

fn map_database_error(error: sqlx::Error) -> OwnerEquityWorkerError {
    match database_error_class(&error) {
        ErrorClass::Transient => OwnerEquityWorkerError::DatabaseTransient,
        _ => OwnerEquityWorkerError::DatabaseIntegrity,
    }
}

fn map_queue_error(error: QueueError) -> OwnerEquityWorkerError {
    match &error {
        QueueError::StaleClaim(_) => OwnerEquityWorkerError::StaleClaim,
        _ => match queue_error_class(&error) {
            ErrorClass::Transient => OwnerEquityWorkerError::QueueTransient,
            _ => OwnerEquityWorkerError::QueueIntegrity,
        },
    }
}

fn canonical_instrument(value: &str) -> bool {
    value.len() == 10
        && value.ends_with(".KRX")
        && value.as_bytes()[..6]
            .iter()
            .all(|byte| byte.is_ascii_digit())
        && InstrumentId::parse(value).is_ok()
}

fn canonical_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(action: OwnerEquityJobAction) -> OwnerEquityJobPayload {
        OwnerEquityJobPayload {
            schema_version: OWNER_EQUITY_V2_JOB_SCHEMA_VERSION,
            action,
            membership_id: Uuid::new_v4(),
            instrument_id: "005930.KRX".to_owned(),
            expected_generation: action.creates_generation().then_some(1),
            request_body_sha256: "a".repeat(64),
            requested_through: TradingDate::parse("2026-08-31").unwrap(),
            max_active_instruments: 73,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            code_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            entitlement_reference: "repo://docs/decisions/0005-kis-personal-use-entitlement.md"
                .to_owned(),
            entitlement_sha256: ContentHash::from_bytes(b"entitlement").to_string(),
        }
    }

    #[test]
    fn payload_is_exact_typed_and_policy_driven() {
        let value = payload(OwnerEquityJobAction::Add);
        assert!(value.validate().is_ok());
        assert_eq!(value.policy().unwrap().max_active_instruments(), 73);
        assert_eq!(value.generation().unwrap().unwrap().get(), 1);

        let mut fixed_assumption = value.clone();
        fixed_assumption.max_active_instruments = 0;
        assert_eq!(
            fixed_assumption.validate(),
            Err(OwnerEquityWorkerError::InvalidJob)
        );

        let mut unexpected = serde_json::to_value(&value).unwrap();
        unexpected["provider_url"] = serde_json::json!("https://example.invalid");
        assert!(serde_json::from_value::<OwnerEquityJobPayload>(unexpected).is_err());
    }

    #[test]
    fn idempotency_keys_are_bounded_ascii_and_namespaced() {
        let first = durable_idempotency_key("request-1").unwrap();
        let replay = durable_idempotency_key("request-1").unwrap();
        assert_eq!(first, replay);
        assert!(first.starts_with("oev2:"));
        assert_eq!(first.len(), 69);
        assert!(durable_idempotency_key("").is_err());
        assert!(durable_idempotency_key("한글").is_err());
        assert!(durable_idempotency_key(&"x".repeat(129)).is_err());
    }

    #[test]
    fn actions_bind_generation_discipline() {
        for action in [
            OwnerEquityJobAction::Add,
            OwnerEquityJobAction::Retry,
            OwnerEquityJobAction::Incremental,
        ] {
            assert!(payload(action).validate().is_ok());
        }
        for action in [
            OwnerEquityJobAction::DisableSnapshot,
            OwnerEquityJobAction::DuplicateReceipt,
        ] {
            assert!(payload(action).validate().is_ok());
        }
        let mut missing = payload(OwnerEquityJobAction::Add);
        missing.expected_generation = None;
        assert!(missing.validate().is_err());
        let mut invented = payload(OwnerEquityJobAction::DisableSnapshot);
        invented.expected_generation = Some(1);
        assert!(invented.validate().is_err());
    }

    #[test]
    fn coverage_fails_closed_on_shape_and_policy_mismatch() {
        let policy = OwnerEquityUniversePolicy::new(73, 261, 121).unwrap();
        let empty = OwnerEquityCoverage {
            observed_sessions: 0,
            first_session: None,
            last_session: None,
        };
        assert!(empty.validate(policy).is_ok());
        let malformed = OwnerEquityCoverage {
            observed_sessions: 1,
            first_session: None,
            last_session: None,
        };
        assert!(malformed.validate(policy).is_err());
        let over_target = OwnerEquityCoverage {
            observed_sessions: 262,
            first_session: Some(TradingDate::parse("2025-01-01").unwrap()),
            last_session: Some(TradingDate::parse("2026-08-31").unwrap()),
        };
        assert!(over_target.validate(policy).is_err());
    }

    #[test]
    fn failures_are_typed_and_never_accept_provider_prose() {
        assert!(
            OwnerEquityWorkFailure::new("PROVIDER_RETRYABLE", RetryDisposition::Retryable).is_ok()
        );
        assert!(
            OwnerEquityWorkFailure::new("provider said try later", RetryDisposition::Retryable)
                .is_err()
        );
        assert_eq!(
            OwnerEquityWorkerError::UniverseChanged.class(),
            ErrorClass::Transient
        );
        assert_eq!(
            OwnerEquityWorkerError::StaleGeneration.class(),
            ErrorClass::Integrity
        );
    }

    #[test]
    fn latest_common_session_requires_at_least_one_admitted_input() {
        assert_eq!(latest_common_session(&[]), None);
    }

    #[test]
    fn sql_publication_guards_stale_and_disabled_workers() {
        let source = include_str!("owner_equity_v2.rs");
        assert!(source.contains("FOR UPDATE"));
        assert!(source.contains("current.2 == \"DISABLED\""));
        assert!(source.contains("settle_success_in(&mut tx, claim)"));
        assert!(source.contains("exact != rows"));
        assert!(source.contains("effective_from <= $3 AND effective_until >= $3"));
    }

    #[test]
    fn locked_publication_rejects_disabled_stale_and_non_exact_workers() {
        let request = payload(OwnerEquityJobAction::Add);
        let current = request.membership_id;
        let other = Uuid::new_v4();
        let valid_memberships = vec![
            (
                current,
                request.instrument_id.clone(),
                "MATERIALIZING".into(),
            ),
            (other, "000660.KRX".into(), "READY".into()),
        ];
        let exact = BTreeSet::from([request.instrument_id.clone(), "000660.KRX".into()]);
        assert_eq!(
            validate_locked_publication(&request, &valid_memberships, true, &exact, 2),
            Ok(())
        );

        let disabled = vec![(current, request.instrument_id.clone(), "DISABLED".into())];
        assert_eq!(
            validate_locked_publication(&request, &disabled, true, &BTreeSet::new(), 0),
            Err(OwnerEquityWorkerError::Disabled)
        );

        let stale = vec![(current, request.instrument_id.clone(), "READY".into())];
        assert_eq!(
            validate_locked_publication(&request, &stale, true, &exact, 2),
            Err(OwnerEquityWorkerError::InvalidLifecycle)
        );

        let missing_other = BTreeSet::from([request.instrument_id.clone()]);
        assert_eq!(
            validate_locked_publication(&request, &valid_memberships, true, &missing_other, 1,),
            Err(OwnerEquityWorkerError::UniverseChanged)
        );
    }

    #[test]
    fn disable_snapshot_uses_the_exact_remaining_ready_universe() {
        let request = payload(OwnerEquityJobAction::DisableSnapshot);
        let memberships = vec![
            (
                request.membership_id,
                request.instrument_id.clone(),
                "DISABLED".into(),
            ),
            (Uuid::new_v4(), "000660.KRX".into(), "READY".into()),
        ];
        let exact = BTreeSet::from(["000660.KRX".into()]);
        assert_eq!(
            validate_locked_publication(&request, &memberships, false, &exact, 1),
            Ok(())
        );
    }

    #[test]
    fn generation_compare_and_set_is_strictly_monotonic() {
        assert_eq!(
            require_next_generation(2, OwnerEquityGeneration::new(3).unwrap()),
            Ok(())
        );
        assert_eq!(
            require_next_generation(2, OwnerEquityGeneration::new(2).unwrap()),
            Err(OwnerEquityWorkerError::StaleGeneration)
        );
        assert_eq!(
            require_next_generation(2, OwnerEquityGeneration::new(4).unwrap()),
            Err(OwnerEquityWorkerError::StaleGeneration)
        );
    }

    #[test]
    fn no_forbidden_surface_is_embedded() {
        let lower = include_str!("owner_equity_v2.rs").to_ascii_lowercase();
        let identifiers = lower
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .collect::<BTreeSet<_>>();
        for forbidden in [
            concat!("ca", "no"),
            concat!("acnt_", "prdt_cd"),
            concat!("buying_", "power"),
            concat!("sellable_", "quantity"),
            concat!("submit_", "order"),
            concat!("cancel_", "order"),
        ] {
            assert!(
                !identifiers.contains(forbidden),
                "forbidden surface: {forbidden}"
            );
        }
    }
}
