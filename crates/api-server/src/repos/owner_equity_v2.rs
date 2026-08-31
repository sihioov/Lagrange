//! Owner-scoped persistence for the managed equity universe V2.
//!
//! Every mutation binds an owner membership and a durable `jobs` receipt in
//! one actor-scoped transaction.  The queue key is namespaced and hashed, and
//! the canonical body hash remains in the payload so process restarts cannot
//! weaken replay mismatch detection.

use chrono::{DateTime, NaiveDate, Utc};
use domain::{OwnerEquityMembershipState, TradingDate};
use factor_engine::owner_equity_v2::OwnerEquitySignalRow;
use job_queue::owner_equity_v2::{
    OWNER_EQUITY_V2_JOB_SCHEMA_VERSION, OWNER_EQUITY_V2_JOB_TYPE, OWNER_EQUITY_V2_MAX_ATTEMPTS,
    OwnerEquityJobAction, OwnerEquityJobPayload, durable_idempotency_key,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use auth::entitlement::Actor;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OwnerEquityPolicyRecord {
    pub max_active_instruments: i32,
    pub active_instruments: i64,
    pub target_observed_sessions: i32,
    pub minimum_observed_sessions: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OwnerEquityMembershipRecord {
    pub id: Uuid,
    pub instrument_id: String,
    pub state: String,
    pub error_code: Option<String>,
    pub error_retryable: Option<bool>,
    pub requested_at: DateTime<Utc>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub generation: i64,
    pub observed_sessions: i32,
    pub target_observed_sessions: i32,
    pub minimum_observed_sessions: i32,
    pub first_session: Option<NaiveDate>,
    pub last_session: Option<NaiveDate>,
}

impl OwnerEquityMembershipRecord {
    pub fn lifecycle(&self) -> Result<OwnerEquityMembershipState, OwnerEquityRepoError> {
        self.state
            .parse()
            .map_err(|_| OwnerEquityRepoError::Integrity)
    }
}

#[derive(Debug, Clone)]
pub struct OwnerEquityMutationPins {
    pub code_commit: String,
    pub entitlement_reference: String,
    pub entitlement_sha256: String,
    pub requested_through: TradingDate,
}

#[derive(Debug, Clone)]
pub struct OwnerEquityMutationResult {
    pub membership: OwnerEquityMembershipRecord,
    pub job_id: Uuid,
    pub replayed: bool,
    pub duplicate_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OwnerEquitySnapshotRecord {
    pub id: Uuid,
    pub as_of_session: NaiveDate,
    pub universe_sha256: String,
    pub row_count: i32,
    pub created_at: DateTime<Utc>,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct OwnerEquitySnapshotRowRecord {
    pub instrument_id: String,
    pub generation: i64,
    pub rank: i32,
    pub signal: OwnerEquitySignalRow,
}

#[derive(Debug, Clone)]
pub struct OwnerEquityLatestSnapshot {
    pub snapshot: OwnerEquitySnapshotRecord,
    pub rows: Vec<OwnerEquitySnapshotRowRecord>,
}

#[derive(Debug, Error)]
pub enum OwnerEquityRepoError {
    #[error("invalid owner equity request")]
    InvalidRequest,
    #[error("owner equity idempotency key is invalid")]
    InvalidIdempotencyKey,
    #[error("owner equity idempotency key has a different body")]
    IdempotencyMismatch,
    #[error("owner equity policy is unavailable")]
    PolicyUnavailable,
    #[error("owner equity policy capacity is exhausted")]
    CapacityExceeded,
    #[error("owner equity membership is not found")]
    NotFound,
    #[error("owner equity membership is not in the required state")]
    InvalidState,
    #[error("owner equity entitlement is unavailable")]
    EntitlementUnavailable,
    #[error("owner equity stored evidence is invalid")]
    Integrity,
    #[error("owner equity admitted snapshot is unavailable")]
    SnapshotUnavailable,
    #[error("owner equity database is unavailable")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct OwnerEquityV2Repo {
    pool: sqlx::PgPool,
}

impl OwnerEquityV2Repo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn list(
        &self,
        actor: &Actor,
    ) -> Result<(OwnerEquityPolicyRecord, Vec<OwnerEquityMembershipRecord>), OwnerEquityRepoError>
    {
        let owner = owner_actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy)?;
        let policy = policy_in(&mut tx, owner).await?;
        let memberships = sqlx::query_as(membership_select(
            "WHERE membership.owner_user_id = $1 ORDER BY membership.requested_at, membership.id",
        ))
        .bind(owner)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((policy, memberships))
    }

    pub async fn get(
        &self,
        actor: &Actor,
        membership_id: Uuid,
    ) -> Result<(OwnerEquityPolicyRecord, OwnerEquityMembershipRecord), OwnerEquityRepoError> {
        let owner = owner_actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy)?;
        let policy = policy_in(&mut tx, owner).await?;
        let membership = membership_by_id(&mut tx, owner, membership_id).await?;
        tx.commit().await?;
        Ok((policy, membership))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add(
        &self,
        actor: &Actor,
        instrument_code: &str,
        public_idempotency_key: &str,
        request_body_sha256: &str,
        pins: &OwnerEquityMutationPins,
    ) -> Result<OwnerEquityMutationResult, OwnerEquityRepoError> {
        if !canonical_code(instrument_code) || !canonical_body_hash(request_body_sha256) {
            return Err(OwnerEquityRepoError::InvalidRequest);
        }
        validate_pins(pins)?;
        let owner = owner_actor_uuid(actor)?;
        let queue_key = durable_idempotency_key(public_idempotency_key)
            .map_err(|_| OwnerEquityRepoError::InvalidIdempotencyKey)?;
        let instrument_id = format!("{instrument_code}.KRX");
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy)?;
        lock_owner_mutation_in(&mut tx, owner).await?;

        if let Some(replay) = replay_in(
            &mut tx,
            owner,
            &queue_key,
            request_body_sha256,
            OwnerEquityJobAction::Add,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(replay);
        }
        let policy = policy_in(&mut tx, owner).await?;
        require_entitlement_in(&mut tx, pins).await?;

        let existing: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM public.owner_equity_memberships
             WHERE owner_user_id = $1 AND instrument_id = $2 AND state <> 'DISABLED'",
        )
        .bind(owner)
        .bind(&instrument_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((membership_id,)) = existing {
            let payload = job_payload(
                OwnerEquityJobAction::DuplicateReceipt,
                membership_id,
                &instrument_id,
                None,
                request_body_sha256,
                &policy,
                pins,
            )?;
            let job_id = insert_job_in(&mut tx, owner, &queue_key, &payload, true).await?;
            let membership = membership_by_id(&mut tx, owner, membership_id).await?;
            tx.commit().await?;
            return Ok(OwnerEquityMutationResult {
                membership,
                job_id,
                replayed: false,
                duplicate_active: true,
            });
        }

        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.owner_equity_memberships
             WHERE owner_user_id = $1 AND state <> 'DISABLED'",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await?;
        if capacity_available(active, policy.max_active_instruments).is_err() {
            tx.rollback().await.ok();
            return Err(OwnerEquityRepoError::CapacityExceeded);
        }
        let membership_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO public.owner_equity_memberships
             (id, owner_user_id, instrument_id, transition_actor_user_id,
              transition_code_commit, transition_entitlement_sha256)
             VALUES ($1, $2, $3, $2, $4, $5)",
        )
        .bind(membership_id)
        .bind(owner)
        .bind(&instrument_id)
        .bind(&pins.code_commit)
        .bind(&pins.entitlement_sha256)
        .execute(&mut *tx)
        .await
        .map_err(map_insert_error)?;
        let payload = job_payload(
            OwnerEquityJobAction::Add,
            membership_id,
            &instrument_id,
            Some(1),
            request_body_sha256,
            &policy,
            pins,
        )?;
        let job_id = insert_job_in(&mut tx, owner, &queue_key, &payload, false).await?;
        let membership = membership_by_id(&mut tx, owner, membership_id).await?;
        tx.commit().await?;
        Ok(OwnerEquityMutationResult {
            membership,
            job_id,
            replayed: false,
            duplicate_active: false,
        })
    }

    pub async fn retry(
        &self,
        actor: &Actor,
        membership_id: Uuid,
        public_idempotency_key: &str,
        request_body_sha256: &str,
        pins: &OwnerEquityMutationPins,
    ) -> Result<OwnerEquityMutationResult, OwnerEquityRepoError> {
        self.transition_job(
            actor,
            membership_id,
            public_idempotency_key,
            request_body_sha256,
            pins,
            OwnerEquityJobAction::Retry,
        )
        .await
    }

    pub async fn disable(
        &self,
        actor: &Actor,
        membership_id: Uuid,
        public_idempotency_key: &str,
        request_body_sha256: &str,
        pins: &OwnerEquityMutationPins,
    ) -> Result<OwnerEquityMutationResult, OwnerEquityRepoError> {
        self.transition_job(
            actor,
            membership_id,
            public_idempotency_key,
            request_body_sha256,
            pins,
            OwnerEquityJobAction::DisableSnapshot,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn transition_job(
        &self,
        actor: &Actor,
        membership_id: Uuid,
        public_idempotency_key: &str,
        request_body_sha256: &str,
        pins: &OwnerEquityMutationPins,
        action: OwnerEquityJobAction,
    ) -> Result<OwnerEquityMutationResult, OwnerEquityRepoError> {
        if !canonical_body_hash(request_body_sha256) {
            return Err(OwnerEquityRepoError::InvalidRequest);
        }
        validate_pins(pins)?;
        let owner = owner_actor_uuid(actor)?;
        let queue_key = durable_idempotency_key(public_idempotency_key)
            .map_err(|_| OwnerEquityRepoError::InvalidIdempotencyKey)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy)?;
        lock_owner_mutation_in(&mut tx, owner).await?;
        if let Some(replay) =
            replay_in(&mut tx, owner, &queue_key, request_body_sha256, action).await?
        {
            tx.commit().await?;
            return Ok(replay);
        }
        let policy = policy_in(&mut tx, owner).await?;
        require_entitlement_in(&mut tx, pins).await?;
        // The app role is intentionally SELECT-only on memberships.  The
        // SECURITY DEFINER retry/disable function below performs a
        // conditional UPDATE, which obtains the row lock and validates the
        // lifecycle atomically after this owner-serialized read.
        let before = membership_by_id(&mut tx, owner, membership_id).await?;
        let expected_generation = if action == OwnerEquityJobAction::Retry {
            let state = before.lifecycle()?;
            if !retry_allowed(state, before.error_retryable) {
                return Err(OwnerEquityRepoError::InvalidState);
            }
            let next: i64 = sqlx::query_scalar(
                "SELECT COALESCE(max(generation), 0) + 1
                 FROM public.owner_equity_instrument_generations
                 WHERE membership_id = $1 AND owner_user_id = $2",
            )
            .bind(membership_id)
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query("SELECT public.retry_owner_equity_membership($1, $2, $3)")
                .bind(membership_id)
                .bind(&pins.code_commit)
                .bind(&pins.entitlement_sha256)
                .execute(&mut *tx)
                .await
                .map_err(map_transition_error)?;
            Some(u64::try_from(next).map_err(|_| OwnerEquityRepoError::Integrity)?)
        } else {
            if before.lifecycle()? == OwnerEquityMembershipState::Disabled {
                return Err(OwnerEquityRepoError::InvalidState);
            }
            sqlx::query("SELECT public.disable_owner_equity_membership($1, $2, $3)")
                .bind(membership_id)
                .bind(&pins.code_commit)
                .bind(&pins.entitlement_sha256)
                .execute(&mut *tx)
                .await
                .map_err(map_transition_error)?;
            None
        };
        let payload = job_payload(
            action,
            membership_id,
            &before.instrument_id,
            expected_generation,
            request_body_sha256,
            &policy,
            pins,
        )?;
        let job_id = insert_job_in(&mut tx, owner, &queue_key, &payload, false).await?;
        let membership = membership_by_id(&mut tx, owner, membership_id).await?;
        tx.commit().await?;
        Ok(OwnerEquityMutationResult {
            membership,
            job_id,
            replayed: false,
            duplicate_active: false,
        })
    }

    pub async fn latest_snapshot(
        &self,
        actor: &Actor,
    ) -> Result<Option<OwnerEquityLatestSnapshot>, OwnerEquityRepoError> {
        let owner = owner_actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy)?;
        let snapshot: Option<OwnerEquitySnapshotRecord> = sqlx::query_as(
            "SELECT id, as_of_session, universe_sha256, row_count, created_at,
                    published_at
             FROM public.owner_equity_signal_snapshots
             WHERE owner_user_id = $1 AND published_at IS NOT NULL
             ORDER BY published_at DESC, id DESC LIMIT 1",
        )
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(snapshot) = snapshot else {
            tx.commit().await?;
            return Ok(None);
        };
        let raw: Vec<(String, i64, i32, Value)> = sqlx::query_as(
            "SELECT instrument_id, generation, rank, signals_json
             FROM public.owner_equity_signal_snapshot_rows
             WHERE owner_user_id = $1 AND snapshot_id = $2
             ORDER BY rank",
        )
        .bind(owner)
        .bind(snapshot.id)
        .fetch_all(&mut *tx)
        .await?;
        if raw.len() != usize::try_from(snapshot.row_count).unwrap_or(usize::MAX) {
            return Err(OwnerEquityRepoError::Integrity);
        }
        let mut rows = Vec::with_capacity(raw.len());
        for (instrument_id, generation, rank, value) in raw {
            let signal: OwnerEquitySignalRow =
                serde_json::from_value(value).map_err(|_| OwnerEquityRepoError::Integrity)?;
            if signal.instrument_id.to_string() != instrument_id
                || i64::try_from(signal.generation.get()).unwrap_or(-1) != generation
                || i32::try_from(signal.rank).unwrap_or(-1) != rank
            {
                return Err(OwnerEquityRepoError::Integrity);
            }
            rows.push(OwnerEquitySnapshotRowRecord {
                instrument_id,
                generation,
                rank,
                signal,
            });
        }
        tx.commit().await?;
        Ok(Some(OwnerEquityLatestSnapshot { snapshot, rows }))
    }
}

fn membership_select(where_clause: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!(
        "SELECT membership.id, membership.instrument_id, membership.state,
                membership.error_code, membership.error_retryable,
                membership.requested_at, membership.disabled_at, membership.updated_at,
                COALESCE(generation.generation, 0) AS generation,
                COALESCE(generation.observed_sessions, 0) AS observed_sessions,
                policy.target_observed_sessions, policy.minimum_observed_sessions,
                generation.first_session, generation.last_session
         FROM public.owner_equity_memberships AS membership
         JOIN public.owner_equity_universe_policies AS policy
           ON policy.owner_user_id = membership.owner_user_id
         LEFT JOIN LATERAL (
              SELECT item.generation, item.observed_sessions,
                     item.first_session, item.last_session
              FROM public.owner_equity_instrument_generations AS item
              WHERE item.membership_id = membership.id
                AND item.owner_user_id = membership.owner_user_id
              ORDER BY item.generation DESC LIMIT 1
         ) AS generation ON true {where_clause}"
    ))
}

async fn membership_by_id(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    membership_id: Uuid,
) -> Result<OwnerEquityMembershipRecord, OwnerEquityRepoError> {
    sqlx::query_as(membership_select(
        "WHERE membership.id = $1 AND membership.owner_user_id = $2",
    ))
    .bind(membership_id)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(OwnerEquityRepoError::NotFound)
}

async fn policy_in(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
) -> Result<OwnerEquityPolicyRecord, OwnerEquityRepoError> {
    let query = sqlx::AssertSqlSafe(
        "SELECT policy.max_active_instruments,
                (SELECT count(*)
                 FROM public.owner_equity_memberships AS membership
                 WHERE membership.owner_user_id = policy.owner_user_id
                   AND membership.state <> 'DISABLED') AS active_instruments,
                policy.target_observed_sessions,
                policy.minimum_observed_sessions
         FROM public.owner_equity_universe_policies AS policy
         WHERE policy.owner_user_id = $1"
            .to_owned(),
    );
    sqlx::query_as(query)
        .bind(owner)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(OwnerEquityRepoError::PolicyUnavailable)
}

async fn lock_owner_mutation_in(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
) -> Result<(), OwnerEquityRepoError> {
    // Policies and memberships are intentionally SELECT-only for `app`.
    // Every mutation, including an idempotent replay, takes this same
    // transaction-scoped owner lock before observing either table. Hash
    // collisions can only over-serialize different owners; identical owner
    // UUIDs always map to the same lock key.
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_xact_lock(\
             pg_catalog.hashtextextended($1::text, 0))",
    )
    .bind(owner.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn require_entitlement_in(
    tx: &mut Transaction<'_, Postgres>,
    pins: &OwnerEquityMutationPins,
) -> Result<(), OwnerEquityRepoError> {
    let hash = pins
        .entitlement_sha256
        .strip_prefix("sha256:")
        .ok_or(OwnerEquityRepoError::EntitlementUnavailable)?;
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
    .bind(&pins.entitlement_reference)
    .bind(pins.requested_through.as_naive_date())
    .fetch_one(&mut **tx)
    .await?;
    if active {
        Ok(())
    } else {
        Err(OwnerEquityRepoError::EntitlementUnavailable)
    }
}

async fn replay_in(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    queue_key: &str,
    body_hash: &str,
    requested_action: OwnerEquityJobAction,
) -> Result<Option<OwnerEquityMutationResult>, OwnerEquityRepoError> {
    let existing: Option<(Uuid, Value)> = sqlx::query_as(
        "SELECT id, payload_json FROM public.jobs
         WHERE owner_user_id = $1 AND idempotency_key = $2 FOR UPDATE",
    )
    .bind(owner)
    .bind(queue_key)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((job_id, value)) = existing else {
        return Ok(None);
    };
    let payload: OwnerEquityJobPayload =
        serde_json::from_value(value).map_err(|_| OwnerEquityRepoError::Integrity)?;
    payload
        .validate()
        .map_err(|_| OwnerEquityRepoError::Integrity)?;
    validate_replay_binding(&payload, body_hash, requested_action)?;
    let membership = membership_by_id(tx, owner, payload.membership_id).await?;
    Ok(Some(OwnerEquityMutationResult {
        membership,
        job_id,
        replayed: true,
        duplicate_active: payload.action == OwnerEquityJobAction::DuplicateReceipt,
    }))
}

async fn insert_job_in(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    queue_key: &str,
    payload: &OwnerEquityJobPayload,
    terminal_receipt: bool,
) -> Result<Uuid, OwnerEquityRepoError> {
    payload
        .validate()
        .map_err(|_| OwnerEquityRepoError::InvalidRequest)?;
    let status = if terminal_receipt {
        "SUCCEEDED"
    } else {
        "QUEUED"
    };
    let job_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO public.jobs
         (id, owner_user_id, job_type, status, priority, idempotency_key,
          payload_json, max_attempts, finished_at)
         VALUES ($1, $2, $3, $4, 20, $5, $6, $7,
                 CASE WHEN $4 = 'SUCCEEDED' THEN now() ELSE NULL END)",
    )
    .bind(job_id)
    .bind(owner)
    .bind(OWNER_EQUITY_V2_JOB_TYPE)
    .bind(status)
    .bind(queue_key)
    .bind(serde_json::to_value(payload).map_err(|_| OwnerEquityRepoError::InvalidRequest)?)
    .bind(OWNER_EQUITY_V2_MAX_ATTEMPTS)
    .execute(&mut **tx)
    .await
    .map_err(map_insert_error)?;
    Ok(job_id)
}

#[allow(clippy::too_many_arguments)]
fn job_payload(
    action: OwnerEquityJobAction,
    membership_id: Uuid,
    instrument_id: &str,
    expected_generation: Option<u64>,
    request_body_sha256: &str,
    policy: &OwnerEquityPolicyRecord,
    pins: &OwnerEquityMutationPins,
) -> Result<OwnerEquityJobPayload, OwnerEquityRepoError> {
    let payload = OwnerEquityJobPayload {
        schema_version: OWNER_EQUITY_V2_JOB_SCHEMA_VERSION,
        action,
        membership_id,
        instrument_id: instrument_id.to_owned(),
        expected_generation,
        request_body_sha256: request_body_sha256.to_owned(),
        requested_through: pins.requested_through,
        max_active_instruments: u32::try_from(policy.max_active_instruments)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
        target_observed_sessions: u32::try_from(policy.target_observed_sessions)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
        minimum_observed_sessions: u32::try_from(policy.minimum_observed_sessions)
            .map_err(|_| OwnerEquityRepoError::Integrity)?,
        code_commit: pins.code_commit.clone(),
        entitlement_reference: pins.entitlement_reference.clone(),
        entitlement_sha256: pins.entitlement_sha256.clone(),
    };
    payload
        .validate()
        .map_err(|_| OwnerEquityRepoError::InvalidRequest)?;
    Ok(payload)
}

fn validate_pins(pins: &OwnerEquityMutationPins) -> Result<(), OwnerEquityRepoError> {
    if domain::CodeCommit::parse(&pins.code_commit).is_err()
        || domain::ContentHash::parse(&pins.entitlement_sha256).is_err()
        || pins.entitlement_reference.trim().is_empty()
        || pins.entitlement_reference.len() > 512
        || pins.entitlement_reference.chars().any(char::is_control)
    {
        return Err(OwnerEquityRepoError::InvalidRequest);
    }
    Ok(())
}

fn canonical_code(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn owner_actor_uuid(actor: &Actor) -> Result<Uuid, OwnerEquityRepoError> {
    if !actor.is_owner() {
        return Err(OwnerEquityRepoError::NotFound);
    }
    actor_uuid(actor).map_err(|_| OwnerEquityRepoError::NotFound)
}

fn canonical_body_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_replay_binding(
    payload: &OwnerEquityJobPayload,
    body_hash: &str,
    requested_action: OwnerEquityJobAction,
) -> Result<(), OwnerEquityRepoError> {
    let action_matches = payload.action == requested_action
        || (requested_action == OwnerEquityJobAction::Add
            && payload.action == OwnerEquityJobAction::DuplicateReceipt);
    if action_matches && payload.request_body_sha256 == body_hash {
        Ok(())
    } else {
        Err(OwnerEquityRepoError::IdempotencyMismatch)
    }
}

fn capacity_available(active: i64, maximum: i32) -> Result<(), OwnerEquityRepoError> {
    if active >= 0 && maximum > 0 && active < i64::from(maximum) {
        Ok(())
    } else {
        Err(OwnerEquityRepoError::CapacityExceeded)
    }
}

fn retry_allowed(state: OwnerEquityMembershipState, retryable: Option<bool>) -> bool {
    state == OwnerEquityMembershipState::InsufficientHistory
        || (state == OwnerEquityMembershipState::Failed && retryable == Some(true))
}

fn map_insert_error(error: sqlx::Error) -> OwnerEquityRepoError {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("23505") => return OwnerEquityRepoError::Integrity,
            Some("23514") => return OwnerEquityRepoError::Integrity,
            Some("42501") => return OwnerEquityRepoError::NotFound,
            _ => {}
        }
    }
    OwnerEquityRepoError::Database(error)
}

fn map_transition_error(error: sqlx::Error) -> OwnerEquityRepoError {
    if let sqlx::Error::Database(database) = &error
        && database.code().as_deref() == Some("42501")
    {
        return OwnerEquityRepoError::InvalidState;
    }
    OwnerEquityRepoError::Database(error)
}

fn map_tenancy(error: crate::error::TenancyError) -> OwnerEquityRepoError {
    match error {
        crate::error::TenancyError::NotFound | crate::error::TenancyError::Forbidden => {
            OwnerEquityRepoError::NotFound
        }
        crate::error::TenancyError::Database(error) => OwnerEquityRepoError::Database(error),
        _ => OwnerEquityRepoError::Integrity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replay_payload(action: OwnerEquityJobAction) -> OwnerEquityJobPayload {
        OwnerEquityJobPayload {
            schema_version: OWNER_EQUITY_V2_JOB_SCHEMA_VERSION,
            action,
            membership_id: Uuid::new_v4(),
            instrument_id: "005930.KRX".into(),
            expected_generation: action.creates_generation().then_some(1),
            request_body_sha256: "a".repeat(64),
            requested_through: TradingDate::parse("2026-08-31").unwrap(),
            max_active_instruments: 73,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            code_commit: "0123456789abcdef0123456789abcdef01234567".into(),
            entitlement_reference: "repo://entitlement".into(),
            entitlement_sha256: domain::ContentHash::from_bytes(b"entitlement").to_string(),
        }
    }

    #[test]
    fn canonical_input_is_exact_ascii_six_digits() {
        assert!(canonical_code("005930"));
        for invalid in ["5930", "005930.KRX", "００５９３０", "00593a", " 005930"] {
            assert!(!canonical_code(invalid), "accepted {invalid:?}");
        }
    }

    #[test]
    fn repository_rejects_member_actor_before_sql() {
        let id = Uuid::new_v4().to_string();
        assert!(owner_actor_uuid(&Actor::owner(id.clone())).is_ok());
        assert!(matches!(
            owner_actor_uuid(&Actor::member(id)),
            Err(OwnerEquityRepoError::NotFound)
        ));
    }

    #[test]
    fn payload_never_contains_http_or_provider_output_fields() {
        let source = include_str!("owner_equity_v2.rs");
        let identifiers = source
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(!identifiers.contains(concat!("response_", "body")));
        assert!(!identifiers.contains(concat!("provider_", "message")));
        assert!(!identifiers.contains(concat!("arbitrary_", "url")));
        assert!(source.contains("request_body_sha256"));
        assert!(source.contains("DuplicateReceipt"));
    }

    #[test]
    fn latest_snapshot_query_is_publication_only() {
        let source = include_str!("owner_equity_v2.rs");
        assert!(source.contains("published_at IS NOT NULL"));
        assert!(source.contains("ORDER BY published_at DESC, id DESC LIMIT 1"));
        assert!(source.contains("serde_json::from_value(value)"));
    }

    #[test]
    fn replay_body_action_capacity_and_retry_contracts_fail_closed() {
        let add = replay_payload(OwnerEquityJobAction::Add);
        assert!(validate_replay_binding(&add, &"a".repeat(64), OwnerEquityJobAction::Add).is_ok());
        assert!(matches!(
            validate_replay_binding(&add, &"b".repeat(64), OwnerEquityJobAction::Add),
            Err(OwnerEquityRepoError::IdempotencyMismatch)
        ));
        assert!(matches!(
            validate_replay_binding(&add, &"a".repeat(64), OwnerEquityJobAction::Retry),
            Err(OwnerEquityRepoError::IdempotencyMismatch)
        ));
        let duplicate = replay_payload(OwnerEquityJobAction::DuplicateReceipt);
        assert!(
            validate_replay_binding(&duplicate, &"a".repeat(64), OwnerEquityJobAction::Add).is_ok()
        );

        assert!(capacity_available(72, 73).is_ok());
        assert!(capacity_available(73, 73).is_err());
        assert!(capacity_available(-1, 73).is_err());
        assert!(retry_allowed(
            OwnerEquityMembershipState::InsufficientHistory,
            None
        ));
        assert!(retry_allowed(
            OwnerEquityMembershipState::Failed,
            Some(true)
        ));
        assert!(!retry_allowed(
            OwnerEquityMembershipState::Failed,
            Some(false)
        ));
        assert!(!retry_allowed(OwnerEquityMembershipState::Ready, None));
    }
}
