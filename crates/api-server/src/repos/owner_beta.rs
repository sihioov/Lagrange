//! Dedicated durable enqueue boundary for the owner-beta price-only route.
//!
//! This repository intentionally does not use [`RecommendationRepo`] or the
//! generic [`job_queue::JobQueue`].  The owner-beta queue payload and result
//! row have a separate schema contract, so the jobs row and the
//! `owner_beta_recommendation_runs` row are written by one actor-scoped
//! transaction and can never be mistaken for a normal recommendation.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::TenancyError;
use auth::entitlement::Actor;
use chrono::{Datelike, NaiveDate};
use job_queue::owner_beta::{
    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPriceRecommendationInput,
};
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

/// The public result needed by the HTTP response. `replay` is derived from
/// the durable queue row, never from the process-local idempotency cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerBetaPriceRecommendationRun {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub status: &'static str,
    pub replay: bool,
}

/// Static repository failures. Values from SQLx, the request body, the
/// artifact path, and approval pins intentionally cannot cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OwnerBetaPriceRecommendationError {
    #[error("resource not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("owner-beta recommendation capacity exceeded")]
    CapacityExceeded,
    #[error("idempotency key was already used with a different owner-beta request")]
    IdempotencyMismatch,
    #[error("internal error")]
    Internal,
}

impl From<TenancyError> for OwnerBetaPriceRecommendationError {
    fn from(error: TenancyError) -> Self {
        match error {
            TenancyError::NotFound => Self::NotFound,
            TenancyError::Forbidden => Self::Forbidden,
            TenancyError::Database(_)
            | TenancyError::NotImplemented
            | TenancyError::DatasetBlocked(_)
            | TenancyError::InvalidState(_)
            | TenancyError::ResultIntegrity(_) => Self::Internal,
        }
    }
}

/// Dedicated actor-scoped repository for the sealed owner-beta enqueue.
#[derive(Debug, Clone)]
pub struct OwnerBetaRecommendationRepo {
    pool: sqlx::PgPool,
}

#[derive(Clone, sqlx::FromRow)]
struct OwnerBetaRunBinding {
    id: Uuid,
    job_id: Uuid,
    owner_user_id: Uuid,
    strategy_config_id: Uuid,
    as_of: NaiveDate,
    #[allow(dead_code)]
    status: String,
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalPinStrings {
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

impl ApprovalPinStrings {
    fn from_artifact(artifact: &ApprovedHistoricalPriceOnlyArtifact) -> Self {
        let pins = artifact.pins();
        Self {
            candidate_content_sha256: pins.candidate_content_sha256().to_string(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256().to_string(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256().to_string(),
            action_manifest_sha256: pins.action_manifest_sha256().to_string(),
            approval_registry_sha256: pins.approval_registry_sha256().to_string(),
        }
    }

    fn from_input(input: &OwnerBetaPriceRecommendationInput) -> Self {
        let pins = input.pins();
        Self {
            candidate_content_sha256: pins.candidate_content_sha256().to_string(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256().to_string(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256().to_string(),
            action_manifest_sha256: pins.action_manifest_sha256().to_string(),
            approval_registry_sha256: pins.approval_registry_sha256().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayExpectation {
    owner_user_id: Uuid,
    strategy_config_id: Uuid,
    as_of: NaiveDate,
    as_of_trading: domain::TradingDate,
    pins: ApprovalPinStrings,
}

impl ReplayExpectation {
    fn from_approved_artifact(
        owner_user_id: Uuid,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
    ) -> Result<Self, OwnerBetaPriceRecommendationError> {
        let as_of_trading = domain::TradingDate::new(as_of.year(), as_of.month(), as_of.day())
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Ok(Self {
            owner_user_id,
            strategy_config_id,
            as_of,
            as_of_trading,
            pins: ApprovalPinStrings::from_artifact(artifact),
        })
    }
}

struct SubmissionProjection {
    payload_json: serde_json::Value,
    pins: ApprovalPinStrings,
}

impl SubmissionProjection {
    fn from_approved_artifact(
        run_id: Uuid,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
    ) -> Result<Self, OwnerBetaPriceRecommendationError> {
        let as_of = domain::TradingDate::new(as_of.year(), as_of.month(), as_of.day())
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Self::from_input(OwnerBetaPriceRecommendationInput::from_approved_artifact(
            run_id,
            strategy_config_id,
            as_of,
            artifact,
        ))
    }

    fn from_input(
        input: OwnerBetaPriceRecommendationInput,
    ) -> Result<Self, OwnerBetaPriceRecommendationError> {
        let pins = ApprovalPinStrings::from_input(&input);
        let payload_json =
            serde_json::to_value(input).map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Ok(Self { payload_json, pins })
    }
}

const JOB_INSERT_SQL: &str = "INSERT INTO jobs
        (id, owner_user_id, job_type, status, priority, idempotency_key,
         payload_json, max_attempts, available_at)
     VALUES ($1, $2, $3, 'QUEUED', 10, $4, $5, 3, now())";

const RUN_INSERT_SQL: &str = "INSERT INTO owner_beta_recommendation_runs
        (id, owner_user_id, strategy_config_id, job_id, as_of,
         candidate_content_sha256, artifact_manifest_sha256,
         stage5_manifest_sha256, action_manifest_sha256,
         approval_registry_sha256)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";

impl OwnerBetaRecommendationRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// Enqueue one owner-beta price-only recommendation or return the exact
    /// durable replay for its namespaced client key.
    ///
    /// The caller must have approved `artifact` immediately before entering
    /// this method.  The artifact is borrowed only long enough to derive the
    /// sealed payload; no artifact bytes or filesystem path enter SQL.
    pub async fn submit(
        &self,
        actor: &Actor,
        strategy_config_id: Uuid,
        as_of: NaiveDate,
        client_key: &str,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        max_jobs_per_owner: u32,
    ) -> Result<OwnerBetaPriceRecommendationRun, OwnerBetaPriceRecommendationError> {
        let owner = actor_uuid(actor).map_err(OwnerBetaPriceRecommendationError::from)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(OwnerBetaPriceRecommendationError::from)?;

        // All job producers share this per-owner advisory lock. The lock is
        // held through the replay probe, capacity count, and both inserts.
        crate::repos::lock_owner_job_capacity(&mut tx, owner)
            .await
            .map_err(OwnerBetaPriceRecommendationError::from)?;

        let queue_key = format!("owner-beta:price-only:v1:{client_key}");
        let replay_expectation =
            ReplayExpectation::from_approved_artifact(owner, strategy_config_id, as_of, artifact)?;
        let replay = match durable_replay(&mut tx, &queue_key, &replay_expectation).await {
            Ok(replay) => replay,
            Err(TenancyError::InvalidState(_)) => {
                return Err(OwnerBetaPriceRecommendationError::IdempotencyMismatch);
            }
            Err(error) => return Err(OwnerBetaPriceRecommendationError::from(error)),
        };
        if let Some(replay) = replay {
            tx.commit()
                .await
                .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
            return Ok(replay);
        }

        // RLS makes a foreign config invisible. Keep the explicit owner
        // predicate as a second invariant, then require the active row lock.
        let config: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT owner_user_id, is_active
               FROM user_strategy_configs
              WHERE id = $1
                AND owner_user_id = $2
              FOR SHARE",
        )
        .bind(strategy_config_id)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        if config != Some((owner, true)) {
            return Err(OwnerBetaPriceRecommendationError::NotFound);
        }

        let active_jobs: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM jobs
              WHERE owner_user_id = $1
                AND status IN ('QUEUED', 'RUNNING')",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        if active_jobs >= max_jobs_per_owner as i64 {
            return Err(OwnerBetaPriceRecommendationError::CapacityExceeded);
        }

        let run_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let projection = SubmissionProjection::from_approved_artifact(
            run_id,
            strategy_config_id,
            as_of,
            artifact,
        )?;

        // Keep this insert first: a failure in the run insert must roll back
        // the queue row together with it.
        sqlx::query(JOB_INSERT_SQL)
            .bind(job_id)
            .bind(owner)
            .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
            .bind(&queue_key)
            .bind(projection.payload_json)
            .execute(&mut *tx)
            .await
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;

        let pins = projection.pins;
        sqlx::query(RUN_INSERT_SQL)
            .bind(run_id)
            .bind(owner)
            .bind(strategy_config_id)
            .bind(job_id)
            .bind(as_of)
            .bind(pins.candidate_content_sha256)
            .bind(pins.artifact_manifest_sha256)
            .bind(pins.stage5_manifest_sha256)
            .bind(pins.action_manifest_sha256)
            .bind(pins.approval_registry_sha256)
            .execute(&mut *tx)
            .await
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;

        tx.commit()
            .await
            .map_err(|_| OwnerBetaPriceRecommendationError::Internal)?;
        Ok(OwnerBetaPriceRecommendationRun {
            run_id,
            job_id,
            status: "PENDING",
            replay: false,
        })
    }
}

async fn durable_replay(
    tx: &mut Transaction<'_, Postgres>,
    queue_key: &str,
    expected: &ReplayExpectation,
) -> Result<Option<OwnerBetaPriceRecommendationRun>, TenancyError> {
    let Some((job_id, job_type, payload_json)) =
        sqlx::query_as::<_, (Uuid, String, serde_json::Value)>(
            "SELECT id, job_type, payload_json
           FROM jobs
          WHERE owner_user_id = $1
            AND idempotency_key = $2
          FOR SHARE",
        )
        .bind(expected.owner_user_id)
        .bind(queue_key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(TenancyError::from_sqlx)?
    else {
        return Ok(None);
    };

    let input = serde_json::from_value::<OwnerBetaPriceRecommendationInput>(payload_json).ok();
    let Some(input) = input else {
        return Err(TenancyError::InvalidState(
            "idempotency mismatch".to_owned(),
        ));
    };
    let row: Option<OwnerBetaRunBinding> = sqlx::query_as(
        "SELECT id, job_id, owner_user_id, strategy_config_id, as_of, status,
                candidate_content_sha256, artifact_manifest_sha256,
                stage5_manifest_sha256, action_manifest_sha256,
                approval_registry_sha256
           FROM owner_beta_recommendation_runs
          WHERE id = $1
            AND owner_user_id = $2
            AND job_id = $3
          FOR SHARE",
    )
    .bind(input.run_id())
    .bind(expected.owner_user_id)
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(TenancyError::from_sqlx)?;
    let Some(binding) = row else {
        return Err(TenancyError::InvalidState(
            "idempotency mismatch".to_owned(),
        ));
    };
    if !replay_binding_matches(job_id, &job_type, &input, &binding, expected) {
        return Err(TenancyError::InvalidState(
            "idempotency mismatch".to_owned(),
        ));
    }
    Ok(Some(OwnerBetaPriceRecommendationRun {
        run_id: binding.id,
        job_id,
        // The enqueue contract always returns the fixed pending response,
        // including a durable replay after a worker has settled the run.
        status: "PENDING",
        replay: true,
    }))
}

fn replay_binding_matches(
    job_id: Uuid,
    job_type: &str,
    input: &OwnerBetaPriceRecommendationInput,
    binding: &OwnerBetaRunBinding,
    expected: &ReplayExpectation,
) -> bool {
    let input_pins = ApprovalPinStrings::from_input(input);
    job_type == OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE
        && input.strategy_config_id() == expected.strategy_config_id
        && input.as_of() == expected.as_of_trading
        && input_pins == expected.pins
        && binding.id == input.run_id()
        && binding.job_id == job_id
        && binding.owner_user_id == expected.owner_user_id
        && binding.strategy_config_id == expected.strategy_config_id
        && binding.as_of == expected.as_of
        && binding.candidate_content_sha256 == input_pins.candidate_content_sha256
        && binding.artifact_manifest_sha256 == input_pins.artifact_manifest_sha256
        && binding.stage5_manifest_sha256 == input_pins.stage5_manifest_sha256
        && binding.action_manifest_sha256 == input_pins.action_manifest_sha256
        && binding.approval_registry_sha256 == input_pins.approval_registry_sha256
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    fn hash(value: u8) -> String {
        format!("sha256:{value:064x}")
    }

    fn input_value() -> Value {
        json!({
            "run_id": "00000000-0000-4000-8000-000000000001",
            "strategy_config_id": "00000000-0000-4000-8000-000000000002",
            "as_of": "2026-08-19",
            "pins": {
                "candidate_content_sha256": hash(1),
                "artifact_manifest_sha256": hash(2),
                "stage5_manifest_sha256": hash(3),
                "action_manifest_sha256": hash(4),
                "approval_registry_sha256": hash(5),
            }
        })
    }

    fn input_from(value: Value) -> OwnerBetaPriceRecommendationInput {
        serde_json::from_value(value).expect("valid sealed input fixture")
    }

    fn replay_fixture() -> (
        Uuid,
        OwnerBetaPriceRecommendationInput,
        OwnerBetaRunBinding,
        ReplayExpectation,
    ) {
        let input = input_from(input_value());
        let job_id = Uuid::parse_str("00000000-0000-4000-8000-000000000003").unwrap();
        let owner = Uuid::parse_str("00000000-0000-4000-8000-000000000004").unwrap();
        let as_of = NaiveDate::from_ymd_opt(2026, 8, 19).unwrap();
        let pins = ApprovalPinStrings::from_input(&input);
        let binding = OwnerBetaRunBinding {
            id: input.run_id(),
            job_id,
            owner_user_id: owner,
            strategy_config_id: input.strategy_config_id(),
            as_of,
            status: "PENDING".to_owned(),
            candidate_content_sha256: pins.candidate_content_sha256.clone(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256.clone(),
            action_manifest_sha256: pins.action_manifest_sha256.clone(),
            approval_registry_sha256: pins.approval_registry_sha256.clone(),
        };
        let expected = ReplayExpectation {
            owner_user_id: owner,
            strategy_config_id: input.strategy_config_id(),
            as_of,
            as_of_trading: domain::TradingDate::parse("2026-08-19").unwrap(),
            pins,
        };
        (job_id, input, binding, expected)
    }

    #[test]
    fn replay_requires_exact_job_request_row_and_all_five_pins() {
        let (job_id, input, binding, expected) = replay_fixture();
        assert!(replay_binding_matches(
            job_id,
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            &input,
            &binding,
            &expected,
        ));

        assert!(!replay_binding_matches(
            job_id,
            "recommendation",
            &input,
            &binding,
            &expected,
        ));
        let mut changed = binding.clone();
        changed.job_id = Uuid::new_v4();
        assert!(!replay_binding_matches(
            job_id,
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            &input,
            &changed,
            &expected,
        ));
        for mutate in [
            |row: &mut OwnerBetaRunBinding| row.id = Uuid::new_v4(),
            |row: &mut OwnerBetaRunBinding| row.owner_user_id = Uuid::new_v4(),
            |row: &mut OwnerBetaRunBinding| row.strategy_config_id = Uuid::new_v4(),
            |row: &mut OwnerBetaRunBinding| {
                row.as_of = NaiveDate::from_ymd_opt(2026, 8, 18).unwrap()
            },
        ] {
            let mut changed = binding.clone();
            mutate(&mut changed);
            assert!(!replay_binding_matches(
                job_id,
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                &input,
                &changed,
                &expected,
            ));
        }

        for field in [
            "candidate_content_sha256",
            "artifact_manifest_sha256",
            "stage5_manifest_sha256",
            "action_manifest_sha256",
            "approval_registry_sha256",
        ] {
            let mut changed_value = input_value();
            changed_value["pins"][field] = json!(hash(9));
            let changed_input = input_from(changed_value);
            assert!(
                !replay_binding_matches(
                    job_id,
                    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    &changed_input,
                    &binding,
                    &expected,
                ),
                "input {field} mismatch must fail"
            );

            let mut changed_row = binding.clone();
            match field {
                "candidate_content_sha256" => changed_row.candidate_content_sha256 = hash(9),
                "artifact_manifest_sha256" => changed_row.artifact_manifest_sha256 = hash(9),
                "stage5_manifest_sha256" => changed_row.stage5_manifest_sha256 = hash(9),
                "action_manifest_sha256" => changed_row.action_manifest_sha256 = hash(9),
                "approval_registry_sha256" => changed_row.approval_registry_sha256 = hash(9),
                _ => unreachable!(),
            }
            assert!(
                !replay_binding_matches(
                    job_id,
                    OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                    &input,
                    &changed_row,
                    &expected,
                ),
                "run row {field} mismatch must fail"
            );
        }

        for (field, value) in [
            ("run_id", json!("00000000-0000-4000-8000-000000000098")),
            (
                "strategy_config_id",
                json!("00000000-0000-4000-8000-000000000099"),
            ),
            ("as_of", json!("2026-08-18")),
        ] {
            let mut changed_value = input_value();
            changed_value[field] = value;
            let changed_input = input_from(changed_value);
            assert!(!replay_binding_matches(
                job_id,
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
                &changed_input,
                &binding,
                &expected,
            ));
        }
    }

    #[test]
    fn submission_projection_is_exact_and_targets_only_dedicated_persistence() {
        let input = input_from(input_value());
        let projection = SubmissionProjection::from_input(input).expect("projection");
        let object = projection.payload_json.as_object().expect("payload object");
        assert_eq!(
            object.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "as_of".to_owned(),
                "pins".to_owned(),
                "run_id".to_owned(),
                "strategy_config_id".to_owned(),
            ])
        );
        assert_eq!(
            object["pins"]
                .as_object()
                .expect("pins object")
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "action_manifest_sha256".to_owned(),
                "approval_registry_sha256".to_owned(),
                "artifact_manifest_sha256".to_owned(),
                "candidate_content_sha256".to_owned(),
                "stage5_manifest_sha256".to_owned(),
            ])
        );
        assert!(JOB_INSERT_SQL.starts_with("INSERT INTO jobs"));
        assert!(RUN_INSERT_SQL.starts_with("INSERT INTO owner_beta_recommendation_runs"));
        assert!(!RUN_INSERT_SQL.contains("INSERT INTO recommendation_runs"));
        assert!(!RUN_INSERT_SQL.contains("target_portfolios"));
        assert!(!RUN_INSERT_SQL.contains("paper"));
    }

    #[test]
    fn owner_beta_job_type_is_excluded_from_all_existing_typed_workers() {
        for existing_worker_type in [
            "recommendation",
            "backtest",
            "candidate_compute",
            "paper_rebalance_preview",
        ] {
            assert_ne!(
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, existing_worker_type,
                "existing typed worker must not claim owner-beta jobs"
            );
        }
    }
}
