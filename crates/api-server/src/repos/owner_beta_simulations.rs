//! Actor-scoped API persistence for the dedicated owner-beta simulation.
//!
//! All operations use the app pool, actor-local RLS transaction, explicit
//! owner predicates, and the migration-0063 run/job/pair binding. Successful
//! metadata is returned only after the entire immutable pair has passed the
//! shared checker.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::http::dto::{
    OwnerBetaSimulationPairDto, OwnerBetaSimulationProfileDto, OwnerBetaSimulationRunDto, PageDto,
};
use crate::http::pagination::Cursor;
use auth::entitlement::{AccessRequest, Actor, CalendarDate, DatasetId, EntitlementService, KrUse};
use chrono::{DateTime, NaiveDate, Utc};
use domain::TradingDate;
use job_queue::owner_beta::OwnerBetaStrategySnapshot;
use job_queue::owner_beta::simulation::v2::V2PreparedInput;
use job_queue::owner_beta::simulation::{EngineIdentity, v2::V2Strategy};
use job_queue::owner_beta::simulation_execution::{
    OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE, OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA,
    SimulationContractVersion, SimulationJobInput, V2SimulationJobInput,
    VersionedSimulationJobInput, check_stored_versioned_simulation_pair,
};
use job_queue::paper_execution::set_paper_transaction_timeouts;
use job_queue::resolver::ResolvedConfig;
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

const IDEMPOTENCY_PREFIX: &str = "owner-beta:price-only-simulation:v1:";
const STRATEGY_ID: &str = "trend_following";
const STRATEGY_VERSION: &str = "1.0.0";
const STRATEGY_CONFIG_SHA256: &str =
    "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0";
const MAX_JOB_ATTEMPTS: i32 = 3;
const BACKTEST_DATASET: &str = "krx_eod_bars";

const RUN_SELECT: &str = "SELECT
    run.id, run.owner_user_id, run.strategy_config_id, run.job_id,
    run.strategy_id, run.strategy_version, run.strategy_config_json,
    run.strategy_config_sha256, run.evaluation_start, run.evaluation_end,
    run.session_count, run.input_kind, run.capability, run.audience,
    run.vendor_snapshot, run.strict_pit, run.action_count,
    run.candidate_content_sha256, run.artifact_manifest_sha256,
    run.stage5_manifest_sha256, run.action_manifest_sha256,
    run.approval_registry_sha256, run.contract_document_sha256,
    run.bars_sha256, run.status, run.error_code, run.engine_json,
    run.published_attempt_id, run.created_at, run.started_at,
    run.finished_at, run.updated_at,
    job.owner_user_id AS job_owner_user_id, job.job_type AS job_type,
    job.status AS job_status, job.idempotency_key AS job_idempotency_key,
    job.payload_json AS job_payload
  FROM public.owner_beta_simulation_runs AS run
  JOIN public.jobs AS job ON job.id = run.job_id
 WHERE run.id = $1 AND run.owner_user_id = $2
   AND job.owner_user_id = $2
   AND job.job_type = 'owner_beta_price_only_simulation'";

const RUN_LIST_SELECT: &str = "SELECT
    run.id, run.owner_user_id, run.strategy_config_id, run.job_id,
    run.strategy_id, run.strategy_version, run.strategy_config_json,
    run.strategy_config_sha256, run.evaluation_start, run.evaluation_end,
    run.session_count, run.input_kind, run.capability, run.audience,
    run.vendor_snapshot, run.strict_pit, run.action_count,
    run.candidate_content_sha256, run.artifact_manifest_sha256,
    run.stage5_manifest_sha256, run.action_manifest_sha256,
    run.approval_registry_sha256, run.contract_document_sha256,
    run.bars_sha256, run.status, run.error_code, run.engine_json,
    run.published_attempt_id, run.created_at, run.started_at,
    run.finished_at, run.updated_at,
    job.owner_user_id AS job_owner_user_id, job.job_type AS job_type,
    job.status AS job_status, job.idempotency_key AS job_idempotency_key,
    job.payload_json AS job_payload
  FROM public.owner_beta_simulation_runs AS run
  JOIN public.jobs AS job ON job.id = run.job_id
 WHERE run.owner_user_id = $1 AND job.owner_user_id = $1
   AND job.job_type = 'owner_beta_price_only_simulation'";

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OwnerBetaSimulationRepoError {
    #[error("owner-beta simulation was not found")]
    NotFound,
    #[error("owner-beta simulation entitlement is required")]
    EntitlementDenied,
    #[error("owner-beta simulation capacity is exhausted")]
    CapacityExceeded,
    #[error("owner-beta simulation idempotency intent differs")]
    IdempotencyMismatch,
    #[error("owner-beta simulation strategy is unsupported")]
    StrategyUnsupported,
    #[error("owner-beta simulation window is invalid")]
    WindowInvalid,
    #[error("owner-beta simulation result integrity failed")]
    Integrity,
    #[error("owner-beta simulation storage is unavailable")]
    Internal,
    #[error("owner-beta simulation result is not ready")]
    NotReady,
}

#[derive(Debug, Clone, Copy)]
pub struct EnqueueResult {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub replay: bool,
}

#[derive(Clone)]
pub struct SimulationRunHeader {
    pub input: VersionedSimulationJobInput,
}

/// Information read before v2 artifact approval. An existing queue payload
/// supplies its frozen strategy snapshot; only a new queue key reads the
/// mutable active configuration.
pub enum V2SubmissionPreflight {
    Replay(Box<V2SimulationJobInput>),
    New(ResolvedConfig),
}

/// Production-only-by-construction admission value. Its constructor accepts
/// the opaque approved input and calls the shared actual-readiness seam;
/// callers cannot make one from queue JSON alone.
pub(crate) struct PreparedV2Admission {
    input: V2SimulationJobInput,
    resolved: ResolvedConfig,
}

impl PreparedV2Admission {
    pub(crate) fn from_prepared(
        prepared: &V2PreparedInput,
        run_id: Uuid,
        strategy_config_id: Uuid,
        resolved: ResolvedConfig,
        evaluation_start: Option<TradingDate>,
        evaluation_end: Option<TradingDate>,
    ) -> Result<Self, job_queue::owner_beta::simulation_execution::SimulationExecutionError> {
        let input = V2SimulationJobInput::from_prepared(
            prepared,
            run_id,
            strategy_config_id,
            &resolved,
            evaluation_start,
            evaluation_end,
        )?;
        Ok(Self { input, resolved })
    }

    pub(crate) fn input(&self) -> &V2SimulationJobInput {
        &self.input
    }
}

#[derive(Clone, sqlx::FromRow)]
struct SimulationDbRow {
    id: Uuid,
    owner_user_id: Uuid,
    strategy_config_id: Uuid,
    job_id: Uuid,
    strategy_id: String,
    strategy_version: String,
    strategy_config_json: Value,
    strategy_config_sha256: String,
    evaluation_start: NaiveDate,
    evaluation_end: NaiveDate,
    session_count: i32,
    input_kind: String,
    capability: String,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    action_count: i32,
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
    contract_document_sha256: String,
    bars_sha256: String,
    status: String,
    error_code: Option<String>,
    engine_json: Option<Value>,
    published_attempt_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    job_owner_user_id: Uuid,
    job_type: String,
    job_status: String,
    job_idempotency_key: String,
    job_payload: Value,
}

#[derive(sqlx::FromRow)]
struct PairDbRow {
    illustrative_canonical: Vec<u8>,
    illustrative_sha256: String,
    zero_cost_canonical: Vec<u8>,
    zero_cost_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEngineJson {
    git_commit: String,
    binary_sha256: String,
    toolchain: String,
    target: String,
    profile: String,
}

pub struct OwnerBetaSimulationRepo {
    pool: sqlx::PgPool,
}

impl OwnerBetaSimulationRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    async fn begin_read_snapshot(
        &self,
        actor: &Actor,
    ) -> Result<Transaction<'static, Postgres>, OwnerBetaSimulationRepoError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        crate::actor_tx::set_actor_guc(&mut tx, actor)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(tx)
    }

    /// Enqueue only from the approved artifact passed by the production
    /// handler. Durable replay is probed before the mutable strategy config,
    /// so later config edits cannot rewrite the original admitted binding.
    // Keep the approved artifact and each persisted intent field explicit.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_approved(
        &self,
        actor: &Actor,
        strategy_config_id: Uuid,
        start_date: Option<TradingDate>,
        end_date: Option<TradingDate>,
        client_key: &str,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        max_jobs_per_owner: u32,
    ) -> Result<EnqueueResult, OwnerBetaSimulationRepoError> {
        let expected_input = SimulationJobInput::from_approved_artifact(
            Uuid::nil(),
            strategy_config_id,
            artifact,
            start_date,
            end_date,
            &fixed_resolved_config(),
        )
        .map_err(map_input_error)?;
        let expected = VersionedSimulationJobInput::V1(expected_input.clone());
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        crate::repos::lock_owner_job_capacity(&mut tx, owner)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;

        let queue_key = queue_key(client_key);
        reject_other_version_key(&mut tx, owner, SimulationContractVersion::V1, client_key).await?;
        if let Some(replay) = durable_replay(&mut tx, owner, &queue_key, &expected).await? {
            tx.commit()
                .await
                .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
            return Ok(replay);
        }

        let resolved = lock_fixed_active_config(&mut tx, owner, strategy_config_id).await?;
        ensure_capacity(&mut tx, owner, max_jobs_per_owner).await?;

        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let input = SimulationJobInput::from_approved_artifact(
            run_id,
            strategy_config_id,
            artifact,
            start_date,
            end_date,
            &resolved,
        )
        .map_err(map_input_error)?;
        insert_enqueue(&mut tx, owner, job_id, &queue_key, &input).await?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(EnqueueResult {
            run_id,
            job_id,
            replay: false,
        })
    }

    /// Probe the durable queue key before consulting the mutable current
    /// config. Replays return their frozen input; only a new key reads the
    /// active config snapshot used for later transaction revalidation.
    pub async fn preflight_v2_submission(
        &self,
        actor: &Actor,
        strategy_config_id: Uuid,
        client_key: &str,
    ) -> Result<V2SubmissionPreflight, OwnerBetaSimulationRepoError> {
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let mut tx = self.begin_read_snapshot(actor).await?;
        if queue_key_exists(
            &mut tx,
            owner,
            &queue_key_for(SimulationContractVersion::V1, client_key),
        )
        .await?
        {
            return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
        }
        let queue_key = queue_key_for(SimulationContractVersion::V2, client_key);
        let job: Option<(Uuid, String, Value)> = sqlx::query_as(
            "SELECT id, job_type, payload_json FROM public.jobs
              WHERE owner_user_id = $1 AND idempotency_key = $2",
        )
        .bind(owner)
        .bind(&queue_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        if let Some((job_id, job_type, payload)) = job {
            if job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE {
                return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
            }
            let bytes = serde_json::to_vec(&payload)
                .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
            let stored = VersionedSimulationJobInput::from_json(&bytes)
                .map_err(|_| OwnerBetaSimulationRepoError::IdempotencyMismatch)?;
            let VersionedSimulationJobInput::V2(stored) = stored else {
                return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
            };
            let row = fetch_header(&mut tx, stored.run_id(), owner).await?;
            let (validated, _) = validate_header(&row, owner)?;
            let VersionedSimulationJobInput::V2(validated) = validated else {
                return Err(OwnerBetaSimulationRepoError::Integrity);
            };
            if row.job_id != job_id || validated != stored {
                return Err(OwnerBetaSimulationRepoError::Integrity);
            }
            tx.commit()
                .await
                .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
            return Ok(V2SubmissionPreflight::Replay(Box::new(validated)));
        }

        let resolved = load_active_config(&mut tx, owner, strategy_config_id).await?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(V2SubmissionPreflight::New(resolved))
    }

    /// The production v2 enqueue path accepts only the value produced by the
    /// opaque prepared-input seam above. Durable replay is rechecked before
    /// current config access in the final short transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn submit_v2_approved(
        &self,
        actor: &Actor,
        admission: &PreparedV2Admission,
        client_key: &str,
        max_jobs_per_owner: u32,
        entitlements: &EntitlementService,
    ) -> Result<EnqueueResult, OwnerBetaSimulationRepoError> {
        self.submit_v2_input(
            actor,
            &admission.input,
            &admission.resolved,
            client_key,
            max_jobs_per_owner,
            entitlements,
        )
        .await
    }

    /// A typed wire-only seam for library tests. It exercises queue and row
    /// validation without claiming that reconstructed dates prove sealed
    /// factor readiness; production cannot call this method.
    #[cfg(test)]
    pub(crate) async fn submit_test_v2_input(
        &self,
        actor: &Actor,
        input: &V2SimulationJobInput,
        client_key: &str,
        max_jobs_per_owner: u32,
        entitlements: &EntitlementService,
    ) -> Result<EnqueueResult, OwnerBetaSimulationRepoError> {
        let resolved = resolved_from_v2_input(input);
        self.submit_v2_input(
            actor,
            input,
            &resolved,
            client_key,
            max_jobs_per_owner,
            entitlements,
        )
        .await
    }

    async fn submit_v2_input(
        &self,
        actor: &Actor,
        input: &V2SimulationJobInput,
        resolved: &ResolvedConfig,
        client_key: &str,
        max_jobs_per_owner: u32,
        entitlements: &EntitlementService,
    ) -> Result<EnqueueResult, OwnerBetaSimulationRepoError> {
        let versioned = VersionedSimulationJobInput::V2(input.clone());
        validate_versioned_input_shape(&versioned)
            .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        crate::repos::lock_owner_job_capacity(&mut tx, owner)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let queue_key = queue_key_for(SimulationContractVersion::V2, client_key);
        reject_other_version_key(&mut tx, owner, SimulationContractVersion::V2, client_key).await?;
        if let Some(replay) = durable_replay(&mut tx, owner, &queue_key, &versioned).await? {
            authorize_schedule(entitlements, actor, &versioned)?;
            tx.commit()
                .await
                .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
            return Ok(replay);
        }

        let current = lock_active_config(&mut tx, owner, input.strategy_config_id()).await?;
        if &current != resolved || resolved != &resolved_from_v2_input(input) {
            return Err(OwnerBetaSimulationRepoError::StrategyUnsupported);
        }
        ensure_capacity(&mut tx, owner, max_jobs_per_owner).await?;
        authorize_schedule(entitlements, actor, &versioned)?;
        let job_id = Uuid::new_v4();
        insert_enqueue_versioned(&mut tx, owner, job_id, &queue_key, &versioned).await?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(EnqueueResult {
            run_id: input.run_id(),
            job_id,
            replay: false,
        })
    }

    /// A library-only seam for real-router PostgreSQL tests. It accepts an
    /// already-validated synthetic job input; no production build contains
    /// this method or any path to call it.
    #[cfg(test)]
    pub(crate) async fn submit_test_input(
        &self,
        actor: &Actor,
        input: &SimulationJobInput,
        client_key: &str,
        max_jobs_per_owner: u32,
    ) -> Result<EnqueueResult, OwnerBetaSimulationRepoError> {
        validate_input_shape(input).map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        crate::repos::lock_owner_job_capacity(&mut tx, owner)
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let queue_key = queue_key(client_key);
        reject_other_version_key(&mut tx, owner, SimulationContractVersion::V1, client_key).await?;
        let versioned = VersionedSimulationJobInput::V1(input.clone());
        if let Some(replay) = durable_replay(&mut tx, owner, &queue_key, &versioned).await? {
            tx.commit()
                .await
                .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
            return Ok(replay);
        }
        lock_fixed_active_config(&mut tx, owner, input.strategy_config_id()).await?;
        ensure_capacity(&mut tx, owner, max_jobs_per_owner).await?;
        let job_id = Uuid::new_v4();
        insert_enqueue(&mut tx, owner, job_id, &queue_key, input).await?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(EnqueueResult {
            run_id: input.run_id(),
            job_id,
            replay: false,
        })
    }

    /// Resolve only an actor-owned header before the handler loads current
    /// rights or reads result content. Foreign and missing IDs are identical.
    pub async fn get_header(
        &self,
        actor: &Actor,
        run_id: Uuid,
    ) -> Result<SimulationRunHeader, OwnerBetaSimulationRepoError> {
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::NotFound)?;
        let mut tx = self.begin_read_snapshot(actor).await?;
        let row = fetch_header(&mut tx, run_id, owner).await?;
        let (input, _) = validate_header(&row, owner)?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(SimulationRunHeader { input })
    }

    pub async fn get(
        &self,
        actor: &Actor,
        run_id: Uuid,
        entitlements: &EntitlementService,
    ) -> Result<OwnerBetaSimulationRunDto, OwnerBetaSimulationRepoError> {
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::NotFound)?;
        let mut tx = self.begin_read_snapshot(actor).await?;
        let row = fetch_header(&mut tx, run_id, owner).await?;
        let (input, status) = validate_header(&row, owner)?;
        authorize_schedule(entitlements, actor, &input)?;
        if status == "SUCCEEDED" {
            validate_whole_pair(&mut tx, &row, &input, owner).await?;
        }
        let dto = run_dto(&row, status, &input);
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(dto)
    }

    pub async fn get_pair(
        &self,
        actor: &Actor,
        run_id: Uuid,
        entitlements: &EntitlementService,
    ) -> Result<OwnerBetaSimulationPairDto, OwnerBetaSimulationRepoError> {
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::NotFound)?;
        let mut tx = self.begin_read_snapshot(actor).await?;
        let row = fetch_header(&mut tx, run_id, owner).await?;
        let (input, status) = validate_header(&row, owner)?;
        authorize_schedule(entitlements, actor, &input)?;
        if status != "SUCCEEDED" {
            return Err(OwnerBetaSimulationRepoError::NotReady);
        }
        let pair = load_and_check_pair(&mut tx, &row, &input, owner).await?;
        let dto = OwnerBetaSimulationPairDto {
            run_id: row.id.to_string(),
            status: "SUCCEEDED",
            illustrative_10bp_per_side: pair.illustrative,
            zero_cost_reference: pair.zero_cost,
        };
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(dto)
    }

    pub async fn list(
        &self,
        actor: &Actor,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
        cursor_secret: &[u8; 32],
        entitlements: &EntitlementService,
        current_date: NaiveDate,
    ) -> Result<PageDto<OwnerBetaSimulationRunDto>, OwnerBetaSimulationRepoError> {
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        authorize_date(entitlements, actor, current_date)?;
        let limit = limit.clamp(1, 100);
        let mut tx = self.begin_read_snapshot(actor).await?;
        // Only the fixed SQL prefix and literal suffixes are interpolated;
        // owner, cursor values and limit remain bind parameters.
        let mut rows = match cursor {
            Some((created_at, id)) => {
                sqlx::query_as::<_, SimulationDbRow>(sqlx::AssertSqlSafe(format!(
                    "{RUN_LIST_SELECT} AND (run.created_at, run.id) < ($2, $3)
                     ORDER BY run.created_at DESC, run.id DESC LIMIT $4"
                )))
                .bind(owner)
                .bind(created_at)
                .bind(id)
                .bind((limit + 1) as i64)
                .fetch_all(&mut *tx)
                .await
            }
            None => {
                sqlx::query_as::<_, SimulationDbRow>(sqlx::AssertSqlSafe(format!(
                    "{RUN_LIST_SELECT} ORDER BY run.created_at DESC, run.id DESC
                     LIMIT $2"
                )))
                .bind(owner)
                .bind((limit + 1) as i64)
                .fetch_all(&mut *tx)
                .await
            }
        }
        .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let (input, status) = validate_header(row, owner)?;
            authorize_schedule(entitlements, actor, &input)?;
            if status == "SUCCEEDED" {
                // Pair bytes are fetched and dropped one row at a time; list
                // never holds multiple large canonical pairs simultaneously.
                validate_whole_pair(&mut tx, row, &input, owner).await?;
            }
            items.push(run_dto(row, status, &input));
        }
        let next_cursor = if has_more {
            rows.last().map(|row| {
                Cursor {
                    k: row.created_at.to_rfc3339(),
                    i: row.id.to_string(),
                }
                .encode(cursor_secret)
            })
        } else {
            None
        };
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(PageDto::new(items, next_cursor))
    }

    /// Lookup the actor-owned run/job binding before any cancel cache replay.
    pub async fn cancel_target(
        &self,
        actor: &Actor,
        run_id: Uuid,
    ) -> Result<(Uuid, Uuid), OwnerBetaSimulationRepoError> {
        let owner = actor_uuid(actor).map_err(|_| OwnerBetaSimulationRepoError::NotFound)?;
        let mut tx = self.begin_read_snapshot(actor).await?;
        let row = fetch_header(&mut tx, run_id, owner).await?;
        validate_header(&row, owner)?;
        let result = (row.id, row.job_id);
        tx.commit()
            .await
            .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
        Ok(result)
    }
}

fn queue_key(client_key: &str) -> String {
    format!("{IDEMPOTENCY_PREFIX}{client_key}")
}

fn queue_key_for(version: SimulationContractVersion, client_key: &str) -> String {
    format!("{}{client_key}", version.idempotency_prefix())
}

fn fixed_resolved_config() -> ResolvedConfig {
    ResolvedConfig {
        strategy_id: STRATEGY_ID.to_owned(),
        strategy_version: STRATEGY_VERSION.to_owned(),
        config: json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 50,
            "slow_ma": 200
        }),
    }
}

async fn lock_fixed_active_config(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    strategy_config_id: Uuid,
) -> Result<ResolvedConfig, OwnerBetaSimulationRepoError> {
    let row: Option<(Uuid, bool, String, String, Value)> = sqlx::query_as(
        "SELECT owner_user_id, is_active, strategy_id, strategy_version, config_json
           FROM public.user_strategy_configs
          WHERE id = $1 AND owner_user_id = $2
          FOR SHARE",
    )
    .bind(strategy_config_id)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    let Some((config_owner, true, strategy_id, strategy_version, config)) = row else {
        return Err(OwnerBetaSimulationRepoError::NotFound);
    };
    if config_owner != owner {
        return Err(OwnerBetaSimulationRepoError::NotFound);
    }
    let resolved = ResolvedConfig {
        strategy_id,
        strategy_version,
        config,
    };
    if resolved != fixed_resolved_config() {
        return Err(OwnerBetaSimulationRepoError::StrategyUnsupported);
    }
    Ok(resolved)
}

async fn load_active_config(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    strategy_config_id: Uuid,
) -> Result<ResolvedConfig, OwnerBetaSimulationRepoError> {
    let row: Option<(Uuid, bool, String, String, Value)> = sqlx::query_as(
        "SELECT owner_user_id, is_active, strategy_id, strategy_version, config_json
           FROM public.user_strategy_configs
          WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(strategy_config_id)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    active_resolved_config(row, owner)
}

async fn lock_active_config(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    strategy_config_id: Uuid,
) -> Result<ResolvedConfig, OwnerBetaSimulationRepoError> {
    let row: Option<(Uuid, bool, String, String, Value)> = sqlx::query_as(
        "SELECT owner_user_id, is_active, strategy_id, strategy_version, config_json
           FROM public.user_strategy_configs
          WHERE id = $1 AND owner_user_id = $2
          FOR SHARE",
    )
    .bind(strategy_config_id)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    active_resolved_config(row, owner)
}

fn active_resolved_config(
    row: Option<(Uuid, bool, String, String, Value)>,
    owner: Uuid,
) -> Result<ResolvedConfig, OwnerBetaSimulationRepoError> {
    let Some((config_owner, true, strategy_id, strategy_version, config)) = row else {
        return Err(OwnerBetaSimulationRepoError::NotFound);
    };
    if config_owner != owner {
        return Err(OwnerBetaSimulationRepoError::NotFound);
    }
    Ok(ResolvedConfig {
        strategy_id,
        strategy_version,
        config,
    })
}

async fn ensure_capacity(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    max_jobs_per_owner: u32,
) -> Result<(), OwnerBetaSimulationRepoError> {
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM public.jobs
          WHERE owner_user_id = $1 AND status IN ('QUEUED', 'RUNNING')",
    )
    .bind(owner)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    if active >= i64::from(max_jobs_per_owner) {
        return Err(OwnerBetaSimulationRepoError::CapacityExceeded);
    }
    Ok(())
}

async fn queue_key_exists(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    key: &str,
) -> Result<bool, OwnerBetaSimulationRepoError> {
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM public.jobs
              WHERE owner_user_id = $1 AND idempotency_key = $2
         )",
    )
    .bind(owner)
    .bind(key)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)
}

async fn reject_other_version_key(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    version: SimulationContractVersion,
    client_key: &str,
) -> Result<(), OwnerBetaSimulationRepoError> {
    let other = match version {
        SimulationContractVersion::V1 => SimulationContractVersion::V2,
        SimulationContractVersion::V2 => SimulationContractVersion::V1,
    };
    if queue_key_exists(tx, owner, &queue_key_for(other, client_key)).await? {
        return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
    }
    Ok(())
}

async fn insert_enqueue(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    job_id: Uuid,
    queue_key: &str,
    input: &SimulationJobInput,
) -> Result<(), OwnerBetaSimulationRepoError> {
    validate_input_shape(input).map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let payload =
        serde_json::to_value(input).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    let pins = input.pins();
    let schedule = input.schedule();
    let session_count = schedule.warmup_sessions().len() + schedule.evaluation_sessions().len();
    if !(202..=2452).contains(&session_count) {
        return Err(OwnerBetaSimulationRepoError::WindowInvalid);
    }

    // Jobs precede runs because migration-0063's binding trigger resolves the
    // immutable owner/job relation during the run insert.
    sqlx::query(
        "INSERT INTO public.jobs
            (id, owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts, available_at)
         VALUES ($1, $2, $3, 'QUEUED', 10, $4, $5, $6, pg_catalog.now())",
    )
    .bind(job_id)
    .bind(owner)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .bind(queue_key)
    .bind(payload)
    .bind(MAX_JOB_ATTEMPTS)
    .execute(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;

    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_runs
            (id, owner_user_id, strategy_config_id, job_id,
             strategy_id, strategy_version, strategy_config_json,
             strategy_config_sha256, evaluation_start, evaluation_end,
             session_count, candidate_content_sha256, artifact_manifest_sha256,
             stage5_manifest_sha256, action_manifest_sha256,
             approval_registry_sha256, contract_document_sha256, bars_sha256)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 $14, $15, $16, $17, $18)",
    )
    .bind(input.run_id())
    .bind(owner)
    .bind(input.strategy_config_id())
    .bind(job_id)
    .bind(STRATEGY_ID)
    .bind(STRATEGY_VERSION)
    .bind(json!({
        "benchmark_instrument": "069500.KRX",
        "fast_ma": 50,
        "slow_ma": 200
    }))
    .bind(STRATEGY_CONFIG_SHA256)
    .bind(input.evaluation_start().as_naive_date())
    .bind(input.evaluation_end().as_naive_date())
    .bind(session_count as i32)
    .bind(pins.candidate_content_sha256().as_str())
    .bind(pins.artifact_manifest_sha256().as_str())
    .bind(pins.stage5_manifest_sha256().as_str())
    .bind(pins.action_manifest_sha256().as_str())
    .bind(pins.approval_registry_sha256().as_str())
    .bind(input.contract_document_sha256().as_str())
    .bind(input.bars_sha256().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    Ok(())
}

async fn insert_enqueue_versioned(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    job_id: Uuid,
    queue_key: &str,
    input: &VersionedSimulationJobInput,
) -> Result<(), OwnerBetaSimulationRepoError> {
    if input.version() != SimulationContractVersion::V2
        || queue_key
            .strip_prefix(SimulationContractVersion::V2.idempotency_prefix())
            .is_none_or(str::is_empty)
    {
        return Err(OwnerBetaSimulationRepoError::Integrity);
    }
    validate_versioned_input_shape(input).map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let payload =
        serde_json::to_value(input).map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    let pins = input.pins();
    let session_count = input.required_dates().len();
    if !(202..=2452).contains(&session_count) {
        return Err(OwnerBetaSimulationRepoError::WindowInvalid);
    }
    let strategy = input.strategy();

    // Preserve the migration-defined jobs-before-runs binding order.
    sqlx::query(
        "INSERT INTO public.jobs
            (id, owner_user_id, job_type, status, priority, idempotency_key,
             payload_json, max_attempts, available_at)
         VALUES ($1, $2, $3, 'QUEUED', 10, $4, $5, $6, pg_catalog.now())",
    )
    .bind(job_id)
    .bind(owner)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .bind(queue_key)
    .bind(payload)
    .bind(MAX_JOB_ATTEMPTS)
    .execute(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;

    sqlx::query(
        "INSERT INTO public.owner_beta_simulation_runs
            (id, owner_user_id, strategy_config_id, job_id,
             strategy_id, strategy_version, strategy_config_json,
             strategy_config_sha256, evaluation_start, evaluation_end,
             session_count, candidate_content_sha256, artifact_manifest_sha256,
             stage5_manifest_sha256, action_manifest_sha256,
             approval_registry_sha256, contract_document_sha256, bars_sha256)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 $14, $15, $16, $17, $18)",
    )
    .bind(input.run_id())
    .bind(owner)
    .bind(input.strategy_config_id())
    .bind(job_id)
    .bind(strategy.strategy_id())
    .bind(strategy.strategy_version())
    .bind(strategy.config_json().clone())
    .bind(strategy.config_sha256().as_str())
    .bind(input.evaluation_start().as_naive_date())
    .bind(input.evaluation_end().as_naive_date())
    .bind(session_count as i32)
    .bind(pins.candidate_content_sha256().as_str())
    .bind(pins.artifact_manifest_sha256().as_str())
    .bind(pins.stage5_manifest_sha256().as_str())
    .bind(pins.action_manifest_sha256().as_str())
    .bind(pins.approval_registry_sha256().as_str())
    .bind(input.contract_document_sha256().as_str())
    .bind(input.bars_sha256().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    Ok(())
}

async fn durable_replay(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    queue_key: &str,
    expected: &VersionedSimulationJobInput,
) -> Result<Option<EnqueueResult>, OwnerBetaSimulationRepoError> {
    let job: Option<(Uuid, String, Value)> = sqlx::query_as(
        "SELECT id, job_type, payload_json FROM public.jobs
          WHERE owner_user_id = $1 AND idempotency_key = $2
          FOR SHARE",
    )
    .bind(owner)
    .bind(queue_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    let Some((job_id, job_type, payload)) = job else {
        return Ok(None);
    };
    if job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE {
        return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
    }
    let bytes =
        serde_json::to_vec(&payload).map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let stored_input = VersionedSimulationJobInput::from_json(&bytes)
        .map_err(|_| OwnerBetaSimulationRepoError::IdempotencyMismatch)?;
    if stored_input.version() != expected.version() {
        return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
    }
    let row = fetch_header(tx, stored_input.run_id(), owner).await?;
    let (validated, _) = validate_header(&row, owner)?;
    if row.job_id != job_id || !same_original_intent(expected, &stored_input) {
        return Err(OwnerBetaSimulationRepoError::IdempotencyMismatch);
    }
    if !same_original_intent(&validated, &stored_input) {
        return Err(OwnerBetaSimulationRepoError::Integrity);
    }
    Ok(Some(EnqueueResult {
        run_id: row.id,
        job_id,
        // Replays return the original enqueue envelope; GET provides current
        // job/run state even after this operation became terminal.
        replay: true,
    }))
}

fn same_original_intent(
    expected: &VersionedSimulationJobInput,
    stored: &VersionedSimulationJobInput,
) -> bool {
    match (expected, stored) {
        (VersionedSimulationJobInput::V1(expected), VersionedSimulationJobInput::V1(stored)) => {
            expected.strategy_config_id() == stored.strategy_config_id()
                && expected.evaluation_start() == stored.evaluation_start()
                && expected.evaluation_end() == stored.evaluation_end()
                && expected.pins() == stored.pins()
                && expected.contract_document_sha256() == stored.contract_document_sha256()
                && expected.bars_sha256() == stored.bars_sha256()
                && expected.strategy() == stored.strategy()
        }
        (VersionedSimulationJobInput::V2(expected), VersionedSimulationJobInput::V2(stored)) => {
            expected.strategy_config_id() == stored.strategy_config_id()
                && expected.evaluation_start() == stored.evaluation_start()
                && expected.evaluation_end() == stored.evaluation_end()
                && expected.required_dates() == stored.required_dates()
                && expected.pins() == stored.pins()
                && expected.contract_document_sha256() == stored.contract_document_sha256()
                && expected.bars_sha256() == stored.bars_sha256()
                && expected.strategy() == stored.strategy()
        }
        _ => false,
    }
}

async fn fetch_header(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    owner: Uuid,
) -> Result<SimulationDbRow, OwnerBetaSimulationRepoError> {
    sqlx::query_as::<_, SimulationDbRow>(RUN_SELECT)
        .bind(run_id)
        .bind(owner)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| OwnerBetaSimulationRepoError::Internal)?
        .ok_or(OwnerBetaSimulationRepoError::NotFound)
}

fn validate_header(
    row: &SimulationDbRow,
    owner: Uuid,
) -> Result<(VersionedSimulationJobInput, &'static str), OwnerBetaSimulationRepoError> {
    if row.owner_user_id != owner
        || row.job_owner_user_id != owner
        || row.job_type != OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE
    {
        return Err(OwnerBetaSimulationRepoError::Integrity);
    }
    let payload = serde_json::to_vec(&row.job_payload)
        .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let input = VersionedSimulationJobInput::from_json(&payload)
        .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    validate_versioned_input_shape(&input).map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let required_dates = input.required_dates();
    let strategy = input.strategy();
    let version = input.version();
    let pins = input.pins();
    if input.run_id() != row.id
        || input.strategy_config_id() != row.strategy_config_id
        || input.evaluation_start().as_naive_date() != row.evaluation_start
        || input.evaluation_end().as_naive_date() != row.evaluation_end
        || required_dates.len() != row.session_count as usize
        || row.strategy_id != strategy.strategy_id()
        || row.strategy_version != strategy.strategy_version()
        || row.strategy_config_json != strategy.config_json().clone()
        || row.strategy_config_sha256 != strategy.config_sha256().as_str()
        || row.job_idempotency_key.is_empty()
        || row
            .job_idempotency_key
            .strip_prefix(version.idempotency_prefix())
            .is_none_or(str::is_empty)
        || row.input_kind != "owner_beta_historical_price_only_v1"
        || row.capability != "PRICE_RETURN_ONLY"
        || row.audience != "OWNER_ONLY"
        || !row.vendor_snapshot
        || row.strict_pit
        || row.action_count != 0
        || row.candidate_content_sha256 != pins.candidate_content_sha256().as_str()
        || row.artifact_manifest_sha256 != pins.artifact_manifest_sha256().as_str()
        || row.stage5_manifest_sha256 != pins.stage5_manifest_sha256().as_str()
        || row.action_manifest_sha256 != pins.action_manifest_sha256().as_str()
        || row.approval_registry_sha256 != pins.approval_registry_sha256().as_str()
        || row.contract_document_sha256 != input.contract_document_sha256().as_str()
        || row.bars_sha256 != input.bars_sha256().as_str()
        || row.created_at > row.updated_at
        || row
            .started_at
            .is_some_and(|at| at < row.created_at || at > row.updated_at)
        || row
            .finished_at
            .is_some_and(|at| at < row.created_at || at > row.updated_at)
        || matches!((row.started_at, row.finished_at), (Some(start), Some(end)) if end < start)
    {
        return Err(OwnerBetaSimulationRepoError::Integrity);
    }
    let public_status = match (row.status.as_str(), row.job_status.as_str()) {
        ("PENDING", "QUEUED" | "RUNNING")
            if row.error_code.is_none()
                && row.engine_json.is_none()
                && row.published_attempt_id.is_none()
                && row.finished_at.is_none() =>
        {
            if row.job_status == "QUEUED" {
                "PENDING"
            } else {
                "RUNNING"
            }
        }
        ("PENDING", "CANCELED")
            if row.error_code.is_none()
                && row.engine_json.is_none()
                && row.published_attempt_id.is_none()
                && row.finished_at.is_none() =>
        {
            "CANCELED"
        }
        ("SUCCEEDED", "SUCCEEDED")
            if row.error_code.is_none()
                && row.engine_json.is_some()
                && row.published_attempt_id.is_some()
                && row.started_at.is_some()
                && row.finished_at.is_some() =>
        {
            "SUCCEEDED"
        }
        ("FAILED", "FAILED")
            if row.error_code.as_deref().is_some_and(is_known_run_error)
                && row.engine_json.is_none()
                && row.published_attempt_id.is_none()
                && row.finished_at.is_some() =>
        {
            "FAILED"
        }
        ("CANCELED", "CANCELED")
            if row.error_code.as_deref().is_some_and(is_known_run_error)
                && row.engine_json.is_none()
                && row.published_attempt_id.is_none()
                && row.finished_at.is_some() =>
        {
            "CANCELED"
        }
        _ => return Err(OwnerBetaSimulationRepoError::Integrity),
    };
    Ok((input, public_status))
}

fn is_known_run_error(code: &str) -> bool {
    matches!(
        code,
        "SIMULATION_INPUT_INVALID"
            | "SIMULATION_ENTITLEMENT_DENIED"
            | "SIMULATION_PROVENANCE_INVALID"
            | "SIMULATION_RESULT_INVALID"
            | "SIMULATION_COMPUTATION_TIMED_OUT"
            | "SIMULATION_COMPUTATION_FAILED"
            | "SIMULATION_INFRASTRUCTURE_UNAVAILABLE"
            | "SIMULATION_ATTEMPTS_EXHAUSTED"
            | "CANCELED"
    )
}

fn validate_input_shape(input: &SimulationJobInput) -> Result<(), ()> {
    let expected = fixed_resolved_config();
    let strategy_json = serde_json::to_value(input.strategy()).map_err(|_| ())?;
    let expected_strategy =
        OwnerBetaStrategySnapshot::from_resolved_config(&expected).map_err(|_| ())?;
    if input.schema() != OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA
        || input.strategy() != &expected_strategy
        || strategy_json.get("strategy_id").and_then(Value::as_str) != Some(STRATEGY_ID)
        || input.schedule().warmup_sessions().len() < 200
        || input.schedule().evaluation_sessions().len() < 2
    {
        return Err(());
    }
    Ok(())
}

fn validate_versioned_input_shape(input: &VersionedSimulationJobInput) -> Result<(), ()> {
    match input {
        VersionedSimulationJobInput::V1(input) => validate_input_shape(input),
        VersionedSimulationJobInput::V2(input) => {
            let strategy = V2Strategy::ALL
                .into_iter()
                .find(|strategy| strategy.as_str() == input.strategy().strategy_id())
                .ok_or(())?;
            let resolved = resolved_from_v2_input(input);
            let expected =
                OwnerBetaStrategySnapshot::from_resolved_config(&resolved).map_err(|_| ())?;
            let required_dates = input.required_dates();
            if input.schema()
                != job_queue::owner_beta::simulation_execution::
                    OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2
                || input.strategy() != &expected
                || input.schedule().strategy() != strategy
                || input.schedule().preparation_sessions().len() < 200
                || input.schedule().evaluation_sessions().len() < 2
                || !(202..=2452).contains(&required_dates.len())
            {
                return Err(());
            }
            Ok(())
        }
    }
}

fn resolved_from_v2_input(input: &V2SimulationJobInput) -> ResolvedConfig {
    ResolvedConfig {
        strategy_id: input.strategy().strategy_id().to_owned(),
        strategy_version: input.strategy().strategy_version().to_owned(),
        config: input.strategy().config_json().clone(),
    }
}

fn authorize_schedule(
    service: &EntitlementService,
    actor: &Actor,
    input: &VersionedSimulationJobInput,
) -> Result<(), OwnerBetaSimulationRepoError> {
    for date in input.required_dates() {
        authorize_date(service, actor, date.as_naive_date())?;
    }
    Ok(())
}

fn authorize_date(
    service: &EntitlementService,
    actor: &Actor,
    date: NaiveDate,
) -> Result<(), OwnerBetaSimulationRepoError> {
    let iso = date.format("%Y-%m-%d").to_string();
    let request = AccessRequest {
        actor: actor.clone(),
        dataset: DatasetId::new(BACKTEST_DATASET),
        as_of: CalendarDate::parse(&iso).map_err(|_| OwnerBetaSimulationRepoError::Integrity)?,
    };
    service
        .authorize_use(KrUse::Backtest, &request)
        .map(|_| ())
        .map_err(|_| OwnerBetaSimulationRepoError::EntitlementDenied)
}

fn run_dto(
    row: &SimulationDbRow,
    status: &str,
    input: &VersionedSimulationJobInput,
) -> OwnerBetaSimulationRunDto {
    let version = input.version();
    let evaluation_session_count = match input {
        VersionedSimulationJobInput::V1(input) => input.schedule().evaluation_sessions().len(),
        VersionedSimulationJobInput::V2(input) => input.schedule().evaluation_sessions().len(),
    };
    OwnerBetaSimulationRunDto {
        id: row.id.to_string(),
        job_id: row.job_id.to_string(),
        strategy_config_id: row.strategy_config_id.to_string(),
        strategy_id: row.strategy_id.clone(),
        strategy_version: row.strategy_version.clone(),
        start_date: row.evaluation_start,
        end_date: row.evaluation_end,
        evaluation_session_count: evaluation_session_count as u32,
        status: status.to_owned(),
        audience: "OWNER_ONLY",
        capability: "PRICE_RETURN_ONLY",
        vendor_snapshot: true,
        strict_pit: false,
        contract_version: version.contract_version(),
        contract_document_sha256: row.contract_document_sha256.clone(),
        result_schema_version: version.result_schema(),
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
        error_code: row.error_code.clone(),
    }
}

struct CheckedPairDto {
    illustrative: OwnerBetaSimulationProfileDto,
    zero_cost: OwnerBetaSimulationProfileDto,
}

async fn validate_whole_pair(
    tx: &mut Transaction<'_, Postgres>,
    row: &SimulationDbRow,
    input: &VersionedSimulationJobInput,
    owner: Uuid,
) -> Result<(), OwnerBetaSimulationRepoError> {
    load_and_check_pair(tx, row, input, owner).await.map(|_| ())
}

async fn load_and_check_pair(
    tx: &mut Transaction<'_, Postgres>,
    row: &SimulationDbRow,
    input: &VersionedSimulationJobInput,
    owner: Uuid,
) -> Result<CheckedPairDto, OwnerBetaSimulationRepoError> {
    let pair: Option<PairDbRow> = sqlx::query_as(
        "SELECT pair.illustrative_canonical, pair.illustrative_sha256,
                pair.zero_cost_canonical, pair.zero_cost_sha256
           FROM public.owner_beta_simulation_result_pairs AS pair
           JOIN public.owner_beta_simulation_runs AS run ON run.id = pair.run_id
           JOIN public.jobs AS job ON job.id = run.job_id
          WHERE pair.run_id = $1 AND pair.owner_user_id = $2
            AND run.owner_user_id = $2 AND job.owner_user_id = $2
            AND job.job_type = $3",
    )
    .bind(row.id)
    .bind(owner)
    .bind(OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| OwnerBetaSimulationRepoError::Internal)?;
    let Some(pair) = pair else {
        return Err(OwnerBetaSimulationRepoError::Integrity);
    };
    let engine_value = row
        .engine_json
        .clone()
        .ok_or(OwnerBetaSimulationRepoError::Integrity)?;
    let engine_wire: StoredEngineJson = serde_json::from_value(engine_value)
        .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let engine = EngineIdentity::new(
        &engine_wire.git_commit,
        &engine_wire.binary_sha256,
        &engine_wire.toolchain,
        &engine_wire.target,
        &engine_wire.profile,
    )
    .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let checked = check_stored_versioned_simulation_pair(
        &pair.illustrative_canonical,
        &pair.illustrative_sha256,
        &pair.zero_cost_canonical,
        &pair.zero_cost_sha256,
        input,
        &engine,
    )
    .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let illustrative_result = serde_json::from_slice(checked.illustrative_bytes())
        .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    let zero_cost_result = serde_json::from_slice(checked.zero_cost_bytes())
        .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?;
    Ok(CheckedPairDto {
        illustrative: OwnerBetaSimulationProfileDto {
            result_content_sha256: checked.illustrative_sha256().as_str().to_owned(),
            result: illustrative_result,
        },
        zero_cost: OwnerBetaSimulationProfileDto {
            result_content_sha256: checked.zero_cost_sha256().as_str().to_owned(),
            result: zero_cost_result,
        },
    })
}

fn map_input_error(
    error: job_queue::owner_beta::simulation_execution::SimulationExecutionError,
) -> OwnerBetaSimulationRepoError {
    use job_queue::owner_beta::simulation_execution::SimulationExecutionError as InputError;
    match error {
        InputError::WindowInvalid => OwnerBetaSimulationRepoError::WindowInvalid,
        InputError::StrategyInvalid => OwnerBetaSimulationRepoError::StrategyUnsupported,
        InputError::InputInvalid
        | InputError::PairInvalid
        | InputError::ProvenanceMismatch
        | InputError::SizeLimitExceeded => OwnerBetaSimulationRepoError::Integrity,
    }
}
