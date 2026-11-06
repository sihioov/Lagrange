//! Strict actor-scoped repository for the Owner-beta Paper v1 ledger ABI.
//!
//! This is separate from generic DatasetPin previews. Economic requests retain
//! the real API runtime and ProtectedCurrent scope across their transaction.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::TenancyError;
use crate::http::dto::{
    OwnerBetaPaperApplyDto, OwnerBetaPaperConsentDto, OwnerBetaPaperLineageDto,
    OwnerBetaPaperPreviewDto, OwnerBetaPaperSessionDetailDto, OwnerBetaPaperSessionState,
    OwnerBetaPaperSessionSummaryDto, OwnerBetaPaperSettlementKind, OwnerBetaPaperStateDto,
    OwnerBetaPaperStateKind,
};
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use job_queue::owner_beta::paper::action::{
    BoundPaperActionError, BoundPaperActionErrorKind, PaperActionContext, PaperActionFuture,
    PaperActionRefusal, PaperActionWork, run_bound_paper_action,
};
use job_queue::owner_beta::paper::runtime::PaperProcessRole;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{FromRow, Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

const PAPER_CONSENT_VERSION: &str = "owner-beta-paper-v1-draft-1";
const PAPER_CONTRACT_SHA256: &str =
    "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0";
const ACTION_SCHEMA: &str = "owner-beta-paper-action-v1";
const STATE_SCHEMA: &str = "owner-beta-paper-state-v1";
const PREVIEW_SCHEMA: &str = "owner-beta-paper-preview-v1";
const SESSION_SCHEMA: &str = "owner-beta-paper-session-v1";
const CONSENT_SCHEMA: &str = "owner-beta-paper-consent-v1";
const APPLY_SCHEMA: &str = "owner-beta-paper-apply-v1";
const MAX_STATE_SESSIONS: i64 = 20;
const ECONOMIC_ACTION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerBetaPaperRepoError {
    Unbound,
    NotFound,
    Forbidden,
    Stale,
    Conflict,
    NotReady,
    RightsBlocked,
    SourceBlocked,
    IdempotencyMismatch,
    CommitUnknown,
    InvalidParameter,
    Integrity,
    Internal,
}

#[derive(Debug, Clone)]
pub(crate) struct OwnerBetaPaperSubmission {
    pub preview_id: Uuid,
    pub job_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaperActionEnvelope {
    schema: String,
    action: String,
    outcome: String,
    control_epoch: Option<i64>,
    account_id: Option<Uuid>,
    data: Value,
}

impl PaperActionEnvelope {
    fn decode(value: Value, expected_action: &str) -> Result<Self, OwnerBetaPaperRepoError> {
        let envelope: Self = serde_json::from_value(value.clone())
            .map_err(|_| OwnerBetaPaperRepoError::Integrity)?;
        if envelope.schema != ACTION_SCHEMA
            || envelope.action != expected_action
            || envelope.control_epoch.is_some_and(|epoch| epoch < 0)
            || envelope.account_id.is_some_and(|id| id.is_nil())
        {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
        if envelope
            .account_id
            .is_some_and(|account_id| !canonical_uuid_matches(value.get("account_id"), account_id))
        {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
        Ok(envelope)
    }

    fn account_and_epoch(&self) -> Result<(Uuid, i64), OwnerBetaPaperRepoError> {
        let account_id = self.account_id.ok_or(OwnerBetaPaperRepoError::Integrity)?;
        let control_epoch = self
            .control_epoch
            .ok_or(OwnerBetaPaperRepoError::Integrity)?;
        Ok((account_id, control_epoch))
    }
}

#[derive(Debug)]
struct StateSnapshot {
    envelope: PaperActionEnvelope,
    state: Option<OwnerBetaPaperStateKind>,
    strategy_config_id: Option<Uuid>,
    consent_present: bool,
    daily_cohort_sha256: Option<String>,
    process_binding_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateData {
    control: StateControl,
    #[allow(dead_code)]
    sessions: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateControl {
    state: String,
    control_epoch: i64,
    strategy_config_id: Uuid,
    consent_event_id: Option<Uuid>,
    cohort_sha256: Option<String>,
    process_binding_sha256: Option<String>,
}

#[derive(Debug, FromRow)]
struct StrategyIdentityRow {
    strategy_id: String,
    strategy_version: String,
}

#[derive(Debug, FromRow)]
struct SessionSummaryRow {
    pending_target_id: Uuid,
    effective_session: NaiveDate,
    state: String,
    settlement_kind: Option<String>,
    reason_code: Option<String>,
    source_original_retrieved_at: Option<DateTime<Utc>>,
    input_admitted_at: Option<DateTime<Utc>>,
    usable_at: Option<DateTime<Utc>>,
    settled_at: Option<DateTime<Utc>>,
    valued_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct PriorConsentRow {
    id: Uuid,
    kind: String,
    control_epoch: i64,
    payload: Value,
}

#[derive(Debug, FromRow)]
struct AppliedIdentityRow {
    preview_id: Uuid,
    job_id: Uuid,
    account_id: Uuid,
    recommendation_run_id: Uuid,
    control_epoch: i64,
}

#[derive(Debug, FromRow)]
struct ApplyPreviewIdentityRow {
    account_id: Uuid,
    status: String,
    beta_lineage_id: Option<Uuid>,
    pending_target_id: Option<Uuid>,
    proposed_effective_date: NaiveDate,
}

#[derive(Debug, Clone)]
enum EconomicRequest {
    Submit {
        recommendation_run_id: Uuid,
        expected_epoch: i64,
        request_key: String,
        max_jobs_per_owner: u32,
    },
    Apply {
        preview_id: Uuid,
        preview_token: String,
        expected_epoch: i64,
    },
}

struct EconomicWork {
    pool: sqlx::PgPool,
    actor: Actor,
    request: EconomicRequest,
}

struct StateProbeExpectation {
    owner_user_id: Uuid,
    account_id: Uuid,
    strategy_config_id: Uuid,
    control_epoch: i64,
    daily_cohort_sha256: Option<String>,
    process_binding_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateProbeFailure {
    CurrentRefused,
    ExpectedBindingMissing,
    RuntimeBindingMismatch,
}

struct StateProbeWork {
    expected: StateProbeExpectation,
}

impl PaperActionWork<(), StateProbeFailure> for StateProbeWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, (), StateProbeFailure> {
        Box::pin(async move {
            context
                .revalidate()
                .map_err(|_| StateProbeFailure::CurrentRefused)?;
            let runtime = context.runtime();
            let matches_state = validate_state_runtime_binding(
                &self.expected,
                runtime.owner_user_id(),
                runtime.account_id(),
                runtime.strategy_config_id(),
                runtime.control_epoch(),
                runtime.binding().daily_cohort_sha256(),
                runtime.config_sha256(),
            );
            context
                .revalidate()
                .map_err(|_| StateProbeFailure::CurrentRefused)?;
            matches_state
        })
    }
}

impl PaperActionWork<PaperActionEnvelope, OwnerBetaPaperRepoError> for EconomicWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, PaperActionEnvelope, OwnerBetaPaperRepoError> {
        Box::pin(async move { self.run_scoped(context).await })
    }
}

impl EconomicWork {
    async fn run_scoped(
        self,
        context: &PaperActionContext<'_, '_, '_>,
    ) -> Result<PaperActionEnvelope, OwnerBetaPaperRepoError> {
        let runtime = context.runtime();
        let owner_id = Uuid::parse_str(runtime.owner_user_id())
            .map_err(|_| OwnerBetaPaperRepoError::Unbound)?;
        let account_id =
            Uuid::parse_str(runtime.account_id()).map_err(|_| OwnerBetaPaperRepoError::Unbound)?;
        let actor_id = actor_uuid(&self.actor).map_err(|_| OwnerBetaPaperRepoError::Forbidden)?;
        let expected_epoch = match &self.request {
            EconomicRequest::Submit { expected_epoch, .. }
            | EconomicRequest::Apply { expected_epoch, .. } => *expected_epoch,
        };
        if owner_id.is_nil()
            || account_id.is_nil()
            || owner_id != actor_id
            || expected_epoch != runtime.control_epoch()
        {
            return Err(OwnerBetaPaperRepoError::Unbound);
        }
        context.revalidate().map_err(map_action_refusal)?;

        let mut tx = begin_actor_tx(&self.pool, &self.actor)
            .await
            .map_err(map_tenancy_error)?;
        let action_result = match self.request {
            EconomicRequest::Submit {
                recommendation_run_id,
                expected_epoch,
                request_key,
                max_jobs_per_owner,
            } => {
                submit_in_transaction(
                    &mut tx,
                    context,
                    &self.actor,
                    owner_id,
                    account_id,
                    recommendation_run_id,
                    expected_epoch,
                    &request_key,
                    max_jobs_per_owner,
                    runtime.config_sha256(),
                )
                .await
            }
            EconomicRequest::Apply {
                preview_id,
                preview_token,
                expected_epoch,
            } => {
                apply_in_transaction(
                    &mut tx,
                    context,
                    &self.actor,
                    owner_id,
                    account_id,
                    preview_id,
                    &preview_token,
                    expected_epoch,
                    runtime.config_sha256(),
                )
                .await
            }
        };

        match action_result {
            Ok(envelope) => {
                if let Err(refusal) = context.revalidate() {
                    return match tx.rollback().await {
                        Ok(()) => Err(map_action_refusal(refusal)),
                        Err(_) => Err(OwnerBetaPaperRepoError::CommitUnknown),
                    };
                }
                tx.commit()
                    .await
                    .map_err(|_| OwnerBetaPaperRepoError::CommitUnknown)?;
                Ok(envelope)
            }
            Err(error) => match tx.rollback().await {
                Ok(()) => Err(error),
                Err(_) => Err(OwnerBetaPaperRepoError::CommitUnknown),
            },
        }
    }
}

impl super::RebalancePreviewRepo {
    pub(crate) async fn owner_beta_paper_state(
        &self,
        actor: &Actor,
    ) -> Result<OwnerBetaPaperStateDto, OwnerBetaPaperRepoError> {
        let owner_id = actor_uuid(actor).map_err(|_| OwnerBetaPaperRepoError::Forbidden)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy_error)?;
        let snapshot = read_state_snapshot(&mut tx).await?;
        let Some((account_id, epoch)) = snapshot
            .envelope
            .account_id
            .zip(snapshot.envelope.control_epoch)
        else {
            tx.commit()
                .await
                .map_err(|_| OwnerBetaPaperRepoError::Internal)?;
            return Ok(unbound_state());
        };
        let state = snapshot.state.ok_or(OwnerBetaPaperRepoError::Integrity)?;
        let strategy_config_id = snapshot
            .strategy_config_id
            .ok_or(OwnerBetaPaperRepoError::Integrity)?;
        if epoch < 0 || account_id.is_nil() || strategy_config_id.is_nil() {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
        let strategy = sqlx::query_as::<_, StrategyIdentityRow>(
            "SELECT strategy_id,strategy_version FROM account_strategy_bindings \
             WHERE account_id=$1 AND owner_user_id=$2 AND strategy_config_id=$3 \
               AND unbound_at IS NULL",
        )
        .bind(account_id)
        .bind(owner_id)
        .bind(strategy_config_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or(OwnerBetaPaperRepoError::Integrity)?;
        let probe_expectation = StateProbeExpectation {
            owner_user_id: owner_id,
            account_id,
            strategy_config_id,
            control_epoch: epoch,
            daily_cohort_sha256: snapshot.daily_cohort_sha256.clone(),
            process_binding_sha256: snapshot.process_binding_sha256.clone(),
        };
        let sessions = sqlx::query_as::<_, SessionSummaryRow>(
            "SELECT pending_target_id,effective_session,state,settlement_kind,reason_code, \
                    source_original_retrieved_at,input_admitted_at,usable_at,settled_at,valued_at \
             FROM owner_beta_paper_sessions \
             WHERE owner_user_id=$1 AND account_id=$2 \
             ORDER BY effective_session DESC LIMIT $3",
        )
        .bind(owner_id)
        .bind(account_id)
        .bind(MAX_STATE_SESSIONS)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .into_iter()
        .map(|row| {
            Ok::<_, OwnerBetaPaperRepoError>(OwnerBetaPaperSessionSummaryDto {
                pending_target_id: row.pending_target_id,
                effective_session: row.effective_session,
                state: parse_session_state(&row.state)?,
                settlement_kind: row
                    .settlement_kind
                    .as_deref()
                    .map(parse_settlement_kind)
                    .transpose()?,
                reason_code: row.reason_code,
                source_original_retrieved_at: row.source_original_retrieved_at,
                input_admitted_at: row.input_admitted_at,
                usable_at: row.usable_at,
                settled_at: row.settled_at,
                valued_at: row.valued_at,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaPaperRepoError::Internal)?;
        let mut state_dto = OwnerBetaPaperStateDto {
            schema: STATE_SCHEMA.to_owned(),
            state,
            account_id: Some(account_id),
            strategy_config_id: Some(strategy_config_id),
            strategy_id: Some(strategy.strategy_id),
            strategy_version: Some(strategy.strategy_version),
            control_epoch: Some(epoch),
            consent_present: snapshot.consent_present,
            daily_cohort_sha256: snapshot.daily_cohort_sha256,
            process_binding_sha256: snapshot.process_binding_sha256,
            sessions,
        };
        if state == OwnerBetaPaperStateKind::Enabled {
            let probe = run_bound_paper_action(
                PaperProcessRole::Api,
                ECONOMIC_ACTION_TIMEOUT,
                None,
                StateProbeWork {
                    expected: probe_expectation,
                },
            )
            .await;
            state_dto.state = project_state_probe(state_dto.state, probe);
        }
        Ok(state_dto)
    }

    pub(crate) async fn owner_beta_paper_consent(
        &self,
        actor: &Actor,
        opt_in: bool,
        expected_epoch: i64,
        idempotency_key: &str,
    ) -> Result<OwnerBetaPaperConsentDto, OwnerBetaPaperRepoError> {
        let owner_id = actor_uuid(actor).map_err(|_| OwnerBetaPaperRepoError::Forbidden)?;
        let request_key = format!("api-paper-consent:{idempotency_key}");
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy_error)?;
        let initial = read_state_snapshot(&mut tx).await?;
        let (account_id, _) = initial
            .envelope
            .account_id
            .zip(initial.envelope.control_epoch)
            .ok_or(OwnerBetaPaperRepoError::Unbound)?;
        let prior = sqlx::query_as::<_, PriorConsentRow>(
            "SELECT id,kind,control_epoch,payload FROM owner_beta_paper_events \
             WHERE owner_user_id=$1 AND account_id=$2 AND request_key=$3",
        )
        .bind(owner_id)
        .bind(account_id)
        .bind(&request_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let (event_id, event_opt_in, replayed) = if let Some(prior) = prior {
            let prior_opt_in = prior.payload.get("opt_in").and_then(Value::as_bool);
            let prior_epoch = prior.payload.get("expected_epoch").and_then(Value::as_i64);
            let expected_kind = if opt_in {
                "CONSENT_GRANTED"
            } else {
                "CONSENT_REVOKED"
            };
            let prior_event_epoch = if opt_in {
                expected_epoch
            } else {
                expected_epoch
                    .checked_add(1)
                    .ok_or(OwnerBetaPaperRepoError::Stale)?
            };
            if prior.kind != expected_kind
                || prior_opt_in != Some(opt_in)
                || prior_epoch != Some(expected_epoch)
                || prior.control_epoch != prior_event_epoch
                || prior.payload.get("consent_version").and_then(Value::as_str)
                    != Some(PAPER_CONSENT_VERSION)
                || prior.payload.get("contract_sha256").and_then(Value::as_str)
                    != Some(PAPER_CONTRACT_SHA256)
            {
                return Err(OwnerBetaPaperRepoError::IdempotencyMismatch);
            }
            (
                prior.id,
                prior_opt_in.ok_or(OwnerBetaPaperRepoError::Integrity)?,
                true,
            )
        } else {
            let raw: Value =
                sqlx::query_scalar("SELECT record_owner_beta_paper_consent_v1($1,$2,$3,$4,$5,$6)")
                    .bind(account_id)
                    .bind(expected_epoch)
                    .bind(PAPER_CONSENT_VERSION)
                    .bind(PAPER_CONTRACT_SHA256)
                    .bind(opt_in)
                    .bind(&request_key)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_consent_sqlx_error)?;
            let envelope = PaperActionEnvelope::decode(raw, "CONSENT")?;
            if envelope.outcome == "NOT_FOUND" {
                return Err(OwnerBetaPaperRepoError::Unbound);
            }
            let consent_epoch = if opt_in {
                expected_epoch
            } else {
                expected_epoch
                    .checked_add(1)
                    .ok_or(OwnerBetaPaperRepoError::Stale)?
            };
            check_envelope_identity(&envelope, account_id, consent_epoch, "CONSENT")?;
            match envelope.outcome.as_str() {
                "OK" | "REPLAY" => {}
                "NOT_FOUND" => return Err(OwnerBetaPaperRepoError::Unbound),
                _ => return Err(map_action_outcome("CONSENT", &envelope.outcome)),
            }
            let returned_event_id = value_uuid(&envelope.data, "event_id")?;
            let event = sqlx::query_as::<_, PriorConsentRow>(
                "SELECT id,kind,control_epoch,payload FROM owner_beta_paper_events \
                 WHERE owner_user_id=$1 AND account_id=$2 AND request_key=$3",
            )
            .bind(owner_id)
            .bind(account_id)
            .bind(&request_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?
            .ok_or(OwnerBetaPaperRepoError::Integrity)?;
            let expected_kind = if opt_in {
                "CONSENT_GRANTED"
            } else {
                "CONSENT_REVOKED"
            };
            if event.id != returned_event_id
                || event.kind != expected_kind
                || event.payload.get("opt_in").and_then(Value::as_bool) != Some(opt_in)
                || event.payload.get("expected_epoch").and_then(Value::as_i64)
                    != Some(expected_epoch)
                || event.payload.get("consent_version").and_then(Value::as_str)
                    != Some(PAPER_CONSENT_VERSION)
                || event.payload.get("contract_sha256").and_then(Value::as_str)
                    != Some(PAPER_CONTRACT_SHA256)
                || event.control_epoch != consent_epoch
            {
                return Err(OwnerBetaPaperRepoError::Integrity);
            }
            (event.id, opt_in, envelope.outcome == "REPLAY")
        };

        let current = read_state_snapshot(&mut tx).await?;
        let (current_account, current_epoch) = current
            .envelope
            .account_id
            .zip(current.envelope.control_epoch)
            .ok_or(OwnerBetaPaperRepoError::Integrity)?;
        if current_account != account_id {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
        tx.commit()
            .await
            .map_err(|_| OwnerBetaPaperRepoError::CommitUnknown)?;
        Ok(OwnerBetaPaperConsentDto {
            schema: CONSENT_SCHEMA.to_owned(),
            account_id,
            event_id,
            control_epoch: current_epoch,
            opt_in: event_opt_in,
            consent_present: current.consent_present,
            replayed,
        })
    }

    pub(crate) async fn owner_beta_paper_preview(
        &self,
        actor: &Actor,
        preview_id: Uuid,
    ) -> Result<OwnerBetaPaperPreviewDto, OwnerBetaPaperRepoError> {
        let owner_id = actor_uuid(actor).map_err(|_| OwnerBetaPaperRepoError::Forbidden)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy_error)?;
        let snapshot = read_state_snapshot(&mut tx).await?;
        let account_id = snapshot
            .envelope
            .account_id
            .ok_or(OwnerBetaPaperRepoError::Unbound)?;
        let value = sqlx::query_scalar::<_, Value>(PREVIEW_READ_SQL)
            .bind(preview_id)
            .bind(owner_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?
            .ok_or(OwnerBetaPaperRepoError::NotFound)?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaPaperRepoError::Internal)?;
        let preview: OwnerBetaPaperPreviewDto =
            serde_json::from_value(value).map_err(|_| OwnerBetaPaperRepoError::Integrity)?;
        validate_preview(&preview)?;
        Ok(preview)
    }

    pub(crate) async fn owner_beta_paper_session(
        &self,
        actor: &Actor,
        pending_target_id: Uuid,
    ) -> Result<OwnerBetaPaperSessionDetailDto, OwnerBetaPaperRepoError> {
        let owner_id = actor_uuid(actor).map_err(|_| OwnerBetaPaperRepoError::Forbidden)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy_error)?;
        let snapshot = read_state_snapshot(&mut tx).await?;
        let account_id = snapshot
            .envelope
            .account_id
            .ok_or(OwnerBetaPaperRepoError::Unbound)?;
        let value = sqlx::query_scalar::<_, Value>(SESSION_READ_SQL)
            .bind(pending_target_id)
            .bind(owner_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?
            .ok_or(OwnerBetaPaperRepoError::NotFound)?;
        tx.commit()
            .await
            .map_err(|_| OwnerBetaPaperRepoError::Internal)?;
        let detail: OwnerBetaPaperSessionDetailDto =
            serde_json::from_value(value).map_err(|_| OwnerBetaPaperRepoError::Integrity)?;
        if detail.schema != SESSION_SCHEMA
            || detail.pending_target_id != pending_target_id
            || detail.signal_date != detail.lineage.signal_date
            || detail.effective_session != detail.lineage.effective_session
        {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
        validate_lineage(&detail.lineage)?;
        Ok(detail)
    }

    pub(crate) async fn owner_beta_paper_submit(
        &self,
        actor: &Actor,
        recommendation_run_id: Uuid,
        expected_epoch: i64,
        request_key: String,
        max_jobs_per_owner: u32,
    ) -> Result<OwnerBetaPaperSubmission, OwnerBetaPaperRepoError> {
        let request = EconomicRequest::Submit {
            recommendation_run_id,
            expected_epoch,
            request_key,
            max_jobs_per_owner,
        };
        let envelope = run_economic_action(self.pool.clone(), actor.clone(), request).await?;
        match envelope.outcome.as_str() {
            "OK" | "REPLAY" => {
                let preview_id = value_uuid(&envelope.data, "preview_id")?;
                let job_id = value_uuid(&envelope.data, "job_id")?;
                Ok(OwnerBetaPaperSubmission {
                    preview_id,
                    job_id,
                    replayed: envelope.outcome == "REPLAY",
                })
            }
            _ => Err(map_action_outcome("SUBMIT_PREVIEW", &envelope.outcome)),
        }
    }

    pub(crate) async fn owner_beta_paper_apply(
        &self,
        actor: &Actor,
        preview_id: Uuid,
        preview_token: String,
        expected_epoch: i64,
    ) -> Result<OwnerBetaPaperApplyDto, OwnerBetaPaperRepoError> {
        let request = EconomicRequest::Apply {
            preview_id,
            preview_token,
            expected_epoch,
        };
        let envelope = run_economic_action(self.pool.clone(), actor.clone(), request).await?;
        match envelope.outcome.as_str() {
            "OK" | "REPLAY" => {
                let (_, control_epoch) = envelope.account_and_epoch()?;
                let data = &envelope.data;
                Ok(OwnerBetaPaperApplyDto {
                    schema: APPLY_SCHEMA.to_owned(),
                    preview_id,
                    pending_target_id: value_uuid(data, "pending_target_id")?,
                    lineage_id: value_uuid(data, "lineage_id")?,
                    effective_session: value_date(data, "effective_session")?,
                    control_epoch,
                    status: "APPLIED".to_owned(),
                    replayed: envelope.outcome == "REPLAY",
                })
            }
            _ => Err(map_action_outcome("APPLY_PREVIEW", &envelope.outcome)),
        }
    }
}

async fn read_state_snapshot(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<StateSnapshot, OwnerBetaPaperRepoError> {
    // The SQL function currently emits all historical session summaries. Strip
    // that array before it crosses the wire; the repository performs a bounded
    // owner/account-scoped read for the fields this API actually publishes.
    let raw: Value = sqlx::query_scalar(
        "SELECT CASE WHEN action->>'outcome'='OK' \
                    THEN jsonb_set(action,'{data,sessions}','[]'::jsonb,true) \
                    ELSE action END \
         FROM (SELECT read_owner_beta_paper_state_v1() AS action) AS state_read",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx_error)?;
    let envelope = PaperActionEnvelope::decode(raw, "READ_STATE")?;
    if envelope.outcome == "NOT_FOUND" {
        return Ok(StateSnapshot {
            envelope,
            state: None,
            strategy_config_id: None,
            consent_present: false,
            daily_cohort_sha256: None,
            process_binding_sha256: None,
        });
    }
    if envelope.outcome != "OK" {
        return Err(map_action_outcome("READ_STATE", &envelope.outcome));
    }
    let (_, epoch) = envelope.account_and_epoch()?;
    let data: StateData = serde_json::from_value(envelope.data.clone())
        .map_err(|_| OwnerBetaPaperRepoError::Integrity)?;
    if data.control.control_epoch != epoch || data.control.control_epoch < 0 {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    let state = paper_state_kind(&data.control.state)?;
    if data.control.strategy_config_id.is_nil()
        || data.control.consent_event_id.is_some_and(|id| id.is_nil())
        || !canonical_uuid_matches(
            envelope.data.pointer("/control/strategy_config_id"),
            data.control.strategy_config_id,
        )
        || data
            .control
            .cohort_sha256
            .as_deref()
            .is_some_and(|hash| !is_sha256(hash))
        || data
            .control
            .process_binding_sha256
            .as_deref()
            .is_some_and(|hash| !is_sha256(hash))
    {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    Ok(StateSnapshot {
        envelope,
        state: Some(state),
        strategy_config_id: Some(data.control.strategy_config_id),
        consent_present: data.control.consent_event_id.is_some(),
        daily_cohort_sha256: data.control.cohort_sha256,
        process_binding_sha256: data.control.process_binding_sha256,
    })
}

fn unbound_state() -> OwnerBetaPaperStateDto {
    OwnerBetaPaperStateDto {
        schema: STATE_SCHEMA.to_owned(),
        state: OwnerBetaPaperStateKind::Unbound,
        account_id: None,
        strategy_config_id: None,
        strategy_id: None,
        strategy_version: None,
        control_epoch: None,
        consent_present: false,
        daily_cohort_sha256: None,
        process_binding_sha256: None,
        sessions: Vec::new(),
    }
}

fn validate_state_runtime_binding(
    expected: &StateProbeExpectation,
    runtime_owner_user_id: &str,
    runtime_account_id: &str,
    runtime_strategy_config_id: &str,
    runtime_control_epoch: i64,
    runtime_daily_cohort_sha256: &str,
    runtime_config_sha256: &str,
) -> Result<(), StateProbeFailure> {
    let expected_cohort = expected
        .daily_cohort_sha256
        .as_deref()
        .ok_or(StateProbeFailure::ExpectedBindingMissing)?;
    let expected_process_config = expected
        .process_binding_sha256
        .as_deref()
        .ok_or(StateProbeFailure::ExpectedBindingMissing)?;
    let runtime_owner_user_id = Uuid::parse_str(runtime_owner_user_id)
        .map_err(|_| StateProbeFailure::RuntimeBindingMismatch)?;
    let runtime_account_id = Uuid::parse_str(runtime_account_id)
        .map_err(|_| StateProbeFailure::RuntimeBindingMismatch)?;
    let runtime_strategy_config_id = Uuid::parse_str(runtime_strategy_config_id)
        .map_err(|_| StateProbeFailure::RuntimeBindingMismatch)?;
    if runtime_owner_user_id != expected.owner_user_id
        || runtime_account_id != expected.account_id
        || runtime_strategy_config_id != expected.strategy_config_id
        || runtime_control_epoch != expected.control_epoch
        || runtime_daily_cohort_sha256 != expected_cohort
        || runtime_config_sha256 != expected_process_config
    {
        return Err(StateProbeFailure::RuntimeBindingMismatch);
    }
    Ok(())
}

fn project_state_probe<E>(
    state: OwnerBetaPaperStateKind,
    probe: Result<(), E>,
) -> OwnerBetaPaperStateKind {
    if state == OwnerBetaPaperStateKind::Enabled && probe.is_err() {
        OwnerBetaPaperStateKind::Blocked
    } else {
        state
    }
}

fn paper_state_kind(value: &str) -> Result<OwnerBetaPaperStateKind, OwnerBetaPaperRepoError> {
    match value {
        "OFF" => Ok(OwnerBetaPaperStateKind::Off),
        "ENABLED" => Ok(OwnerBetaPaperStateKind::Enabled),
        "DRAINING" => Ok(OwnerBetaPaperStateKind::Draining),
        "RESTORED_OFF" => Ok(OwnerBetaPaperStateKind::RestoredOff),
        "BLOCKED" => Ok(OwnerBetaPaperStateKind::Blocked),
        _ => Err(OwnerBetaPaperRepoError::Integrity),
    }
}

fn parse_session_state(value: &str) -> Result<OwnerBetaPaperSessionState, OwnerBetaPaperRepoError> {
    match value {
        "PENDING" => Ok(OwnerBetaPaperSessionState::Pending),
        "WAITING_PRICE" => Ok(OwnerBetaPaperSessionState::WaitingPrice),
        "WAITING_ADMISSION" => Ok(OwnerBetaPaperSessionState::WaitingAdmission),
        "HELD" => Ok(OwnerBetaPaperSessionState::Held),
        "BLOCKED" => Ok(OwnerBetaPaperSessionState::Blocked),
        "RECOVERY_REQUIRED" => Ok(OwnerBetaPaperSessionState::RecoveryRequired),
        "SETTLED_TRADE" => Ok(OwnerBetaPaperSessionState::SettledTrade),
        "SETTLED_NO_TRADE" => Ok(OwnerBetaPaperSessionState::SettledNoTrade),
        "VALUED" => Ok(OwnerBetaPaperSessionState::Valued),
        _ => Err(OwnerBetaPaperRepoError::Integrity),
    }
}

fn parse_settlement_kind(
    value: &str,
) -> Result<OwnerBetaPaperSettlementKind, OwnerBetaPaperRepoError> {
    match value {
        "TRADE" => Ok(OwnerBetaPaperSettlementKind::Trade),
        "NO_TRADE" => Ok(OwnerBetaPaperSettlementKind::NoTrade),
        _ => Err(OwnerBetaPaperRepoError::Integrity),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "The SQL action boundary keeps its protected context and economic inputs explicit."
)]
async fn submit_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    context: &PaperActionContext<'_, '_, '_>,
    actor: &Actor,
    owner_id: Uuid,
    account_id: Uuid,
    recommendation_run_id: Uuid,
    expected_epoch: i64,
    request_key: &str,
    max_jobs_per_owner: u32,
    process_config_sha256: &str,
) -> Result<PaperActionEnvelope, OwnerBetaPaperRepoError> {
    crate::repos::lock_owner_job_capacity(tx, owner_id)
        .await
        .map_err(map_tenancy_error)?;
    let queue_key = format!("owner-beta-paper-preview:{account_id}:{request_key}");
    let existing: Option<AppliedIdentityRow> = sqlx::query_as(
        "SELECT p.id AS preview_id,p.job_id,p.account_id,l.recommendation_run_id,l.control_epoch \
         FROM jobs j JOIN paper_rebalance_previews p ON p.job_id=j.id \
         JOIN owner_beta_paper_lineage l ON l.id=p.beta_lineage_id \
         WHERE j.owner_user_id=$1 AND j.idempotency_key=$2 \
           AND p.owner_user_id=$1 AND p.account_id=$3 AND p.input_kind='OWNER_BETA_DAILY_V1'",
    )
    .bind(owner_id)
    .bind(&queue_key)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx_error)?;
    if let Some(existing) = &existing {
        if existing.account_id != account_id
            || existing.recommendation_run_id != recommendation_run_id
            || existing.control_epoch != expected_epoch
        {
            return Err(OwnerBetaPaperRepoError::IdempotencyMismatch);
        }
    } else {
        let active_jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE owner_user_id=$1 AND status IN ('QUEUED','RUNNING')",
        )
        .bind(owner_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_sqlx_error)?;
        if active_jobs >= i64::from(max_jobs_per_owner) {
            return Err(OwnerBetaPaperRepoError::NotReady);
        }
    }
    context.revalidate().map_err(map_action_refusal)?;
    let raw: Value =
        sqlx::query_scalar("SELECT submit_owner_beta_paper_preview_v1($1,$2,$3,$4,$5)")
            .bind(account_id)
            .bind(recommendation_run_id)
            .bind(expected_epoch)
            .bind(request_key)
            .bind(process_config_sha256)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_economic_sqlx_error)?;
    let envelope = PaperActionEnvelope::decode(raw, "SUBMIT_PREVIEW")?;
    if envelope.outcome == "NOT_FOUND" {
        return Err(OwnerBetaPaperRepoError::NotFound);
    }
    check_envelope_identity(&envelope, account_id, expected_epoch, "SUBMIT_PREVIEW")?;
    match envelope.outcome.as_str() {
        "OK" | "REPLAY" => {
            let preview_id = value_uuid(&envelope.data, "preview_id")?;
            let job_id = value_uuid(&envelope.data, "job_id")?;
            let actual: AppliedIdentityRow = sqlx::query_as(
                "SELECT p.id AS preview_id,p.job_id,p.account_id,l.recommendation_run_id,l.control_epoch \
                 FROM paper_rebalance_previews p \
                 JOIN owner_beta_paper_lineage l ON l.id=p.beta_lineage_id \
                 WHERE p.id=$1 AND p.job_id=$2 AND p.owner_user_id=$3 \
                   AND p.account_id=$4 AND p.input_kind='OWNER_BETA_DAILY_V1'",
            )
            .bind(preview_id)
            .bind(job_id)
            .bind(owner_id)
            .bind(account_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx_error)?
            .ok_or(OwnerBetaPaperRepoError::Integrity)?;
            if actual.preview_id != preview_id
                || actual.job_id != job_id
                || actual.recommendation_run_id != recommendation_run_id
                || actual.control_epoch != expected_epoch
            {
                return Err(OwnerBetaPaperRepoError::IdempotencyMismatch);
            }
            if existing.is_some() != (envelope.outcome == "REPLAY") {
                return Err(OwnerBetaPaperRepoError::Integrity);
            }
        }
        _ => return Err(map_action_outcome("SUBMIT_PREVIEW", &envelope.outcome)),
    }
    let _ = actor;
    Ok(envelope)
}

#[allow(
    clippy::too_many_arguments,
    reason = "The SQL action boundary keeps its protected context and economic inputs explicit."
)]
async fn apply_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    context: &PaperActionContext<'_, '_, '_>,
    actor: &Actor,
    owner_id: Uuid,
    account_id: Uuid,
    preview_id: Uuid,
    preview_token: &str,
    expected_epoch: i64,
    process_config_sha256: &str,
) -> Result<PaperActionEnvelope, OwnerBetaPaperRepoError> {
    let identity: Option<ApplyPreviewIdentityRow> = sqlx::query_as(
        "SELECT account_id,status,beta_lineage_id,pending_target_id,proposed_effective_date FROM paper_rebalance_previews \
         WHERE id=$1 AND owner_user_id=$2 AND account_id=$3 \
           AND input_kind='OWNER_BETA_DAILY_V1'",
    )
    .bind(preview_id)
    .bind(owner_id)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx_error)?;
    let initial_identity = identity.ok_or(OwnerBetaPaperRepoError::NotFound)?;
    if initial_identity.account_id != account_id || initial_identity.beta_lineage_id.is_none() {
        return Err(OwnerBetaPaperRepoError::NotFound);
    }
    context.revalidate().map_err(map_action_refusal)?;
    let raw: Value = sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_v1($1,$2,$3,$4)")
        .bind(preview_id)
        .bind(preview_token)
        .bind(expected_epoch)
        .bind(process_config_sha256)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_economic_sqlx_error)?;
    let mut envelope = PaperActionEnvelope::decode(raw, "APPLY_PREVIEW")?;
    if envelope.outcome == "NOT_FOUND" {
        return Err(OwnerBetaPaperRepoError::NotFound);
    }
    check_envelope_identity(&envelope, account_id, expected_epoch, "APPLY_PREVIEW")?;
    match envelope.outcome.as_str() {
        "OK" | "REPLAY" => {
            if envelope.outcome == "OK"
                && (initial_identity.status != "READY"
                    || initial_identity.pending_target_id.is_some())
            {
                return Err(OwnerBetaPaperRepoError::Integrity);
            }
            let pending_target_id = value_uuid(&envelope.data, "pending_target_id")?;
            let effective_session = value_date(&envelope.data, "effective_session")?;
            let actual: ApplyPreviewIdentityRow = sqlx::query_as(
                "SELECT account_id,status,beta_lineage_id,pending_target_id,proposed_effective_date \
                 FROM paper_rebalance_previews \
                 WHERE id=$1 AND owner_user_id=$2 AND account_id=$3 \
                   AND input_kind='OWNER_BETA_DAILY_V1'",
            )
            .bind(preview_id)
            .bind(owner_id)
            .bind(account_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(map_sqlx_error)?
            .ok_or(OwnerBetaPaperRepoError::Integrity)?;
            let lineage_id = actual
                .beta_lineage_id
                .ok_or(OwnerBetaPaperRepoError::Integrity)?;
            if actual.status != "APPLIED"
                || actual.pending_target_id != Some(pending_target_id)
                || lineage_id != initial_identity.beta_lineage_id.unwrap()
                || actual.proposed_effective_date != effective_session
                || initial_identity.proposed_effective_date != effective_session
                || (envelope.outcome == "OK"
                    && value_uuid(&envelope.data, "lineage_id")? != lineage_id)
            {
                return Err(OwnerBetaPaperRepoError::Integrity);
            }
            if envelope.outcome == "REPLAY" {
                let data = envelope
                    .data
                    .as_object_mut()
                    .ok_or(OwnerBetaPaperRepoError::Integrity)?;
                data.insert(
                    "lineage_id".to_owned(),
                    Value::String(lineage_id.to_string()),
                );
            }
        }
        _ => return Err(map_action_outcome("APPLY_PREVIEW", &envelope.outcome)),
    }
    let _ = actor;
    Ok(envelope)
}

async fn run_economic_action(
    pool: sqlx::PgPool,
    actor: Actor,
    request: EconomicRequest,
) -> Result<PaperActionEnvelope, OwnerBetaPaperRepoError> {
    let result = run_bound_paper_action(
        PaperProcessRole::Api,
        ECONOMIC_ACTION_TIMEOUT,
        None,
        EconomicWork {
            pool,
            actor,
            request,
        },
    )
    .await;
    match result {
        Ok(envelope) => Ok(envelope),
        Err(error) => Err(map_bound_action_error(error)),
    }
}

fn map_bound_action_error(
    error: BoundPaperActionError<OwnerBetaPaperRepoError>,
) -> OwnerBetaPaperRepoError {
    if matches!(
        error.kind(),
        BoundPaperActionErrorKind::CommitUnknown | BoundPaperActionErrorKind::CompletionUnclean
    ) {
        return OwnerBetaPaperRepoError::CommitUnknown;
    }
    if let Some(work_error) = error.work_error() {
        return *work_error;
    }
    if let Some(refusal) = error.refusal() {
        return map_action_refusal(refusal);
    }
    OwnerBetaPaperRepoError::Internal
}

fn map_action_refusal(refusal: PaperActionRefusal) -> OwnerBetaPaperRepoError {
    match refusal {
        PaperActionRefusal::RuntimeUnbound | PaperActionRefusal::RuntimeRejected => {
            OwnerBetaPaperRepoError::Unbound
        }
        PaperActionRefusal::BindingMismatch | PaperActionRefusal::CurrentNotCurrent => {
            OwnerBetaPaperRepoError::Stale
        }
        PaperActionRefusal::CurrentUnavailable => OwnerBetaPaperRepoError::SourceBlocked,
        PaperActionRefusal::Canceled
        | PaperActionRefusal::DeadlineExceeded
        | PaperActionRefusal::ExecutorUnavailable
        | PaperActionRefusal::CallbackPanicked => OwnerBetaPaperRepoError::SourceBlocked,
    }
}

fn check_envelope_identity(
    envelope: &PaperActionEnvelope,
    account_id: Uuid,
    expected_epoch: i64,
    action: &str,
) -> Result<(), OwnerBetaPaperRepoError> {
    if envelope.action != action
        || envelope.account_id != Some(account_id)
        || envelope.control_epoch != Some(expected_epoch)
    {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    Ok(())
}

fn map_action_outcome(action: &str, outcome: &str) -> OwnerBetaPaperRepoError {
    match (action, outcome) {
        (_, "NOT_FOUND") => OwnerBetaPaperRepoError::NotFound,
        (_, "STALE") => OwnerBetaPaperRepoError::Stale,
        (_, "CONFLICT") => OwnerBetaPaperRepoError::Conflict,
        (_, "WAITING_CALENDAR" | "WAITING_EARLIER_SESSION" | "NOT_READY") => {
            OwnerBetaPaperRepoError::NotReady
        }
        (_, "CONSENT_REQUIRED") => OwnerBetaPaperRepoError::RightsBlocked,
        (_, "BLOCKED" | "DATA_BLOCKED" | "SOURCE_BLOCKED") => {
            OwnerBetaPaperRepoError::SourceBlocked
        }
        ("READ_STATE", "NOT_CONFIGURED") | ("SUBMIT_PREVIEW", "UNBOUND") => {
            OwnerBetaPaperRepoError::Unbound
        }
        _ => OwnerBetaPaperRepoError::Integrity,
    }
}

fn map_sqlx_error(error: sqlx::Error) -> OwnerBetaPaperRepoError {
    match error.as_database_error().and_then(|db| db.code()) {
        Some(code) if code == "42501" => OwnerBetaPaperRepoError::Forbidden,
        Some(code) if code == "P0002" => OwnerBetaPaperRepoError::NotFound,
        Some(code) if code == "40001" => OwnerBetaPaperRepoError::Stale,
        Some(code) if code == "23505" => OwnerBetaPaperRepoError::Conflict,
        Some(code) if code == "23514" => OwnerBetaPaperRepoError::SourceBlocked,
        Some(code) if code == "22023" => OwnerBetaPaperRepoError::InvalidParameter,
        Some(code) if code == "22003" => OwnerBetaPaperRepoError::Conflict,
        Some(code) if code == "55000" => OwnerBetaPaperRepoError::NotReady,
        _ => OwnerBetaPaperRepoError::Internal,
    }
}

fn map_tenancy_error(error: TenancyError) -> OwnerBetaPaperRepoError {
    match error {
        TenancyError::NotFound => OwnerBetaPaperRepoError::NotFound,
        TenancyError::Forbidden => OwnerBetaPaperRepoError::Forbidden,
        TenancyError::Database(error) => map_sqlx_error(error),
        TenancyError::InvalidState(_) | TenancyError::DatasetBlocked(_) => {
            OwnerBetaPaperRepoError::SourceBlocked
        }
        TenancyError::ResultIntegrity(_) => OwnerBetaPaperRepoError::Integrity,
        TenancyError::NotImplemented => OwnerBetaPaperRepoError::Internal,
    }
}

fn value_uuid(data: &Value, key: &str) -> Result<Uuid, OwnerBetaPaperRepoError> {
    let text = data
        .get(key)
        .and_then(Value::as_str)
        .ok_or(OwnerBetaPaperRepoError::Integrity)?;
    let value = Uuid::parse_str(text).map_err(|_| OwnerBetaPaperRepoError::Integrity)?;
    if value.is_nil() || value.to_string() != text {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    Ok(value)
}

fn canonical_uuid_matches(value: Option<&Value>, parsed: Uuid) -> bool {
    let Some(raw) = value.and_then(Value::as_str) else {
        return false;
    };
    parsed.to_string() == raw
}

fn value_date(data: &Value, key: &str) -> Result<NaiveDate, OwnerBetaPaperRepoError> {
    let text = data
        .get(key)
        .and_then(Value::as_str)
        .ok_or(OwnerBetaPaperRepoError::Integrity)?;
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| OwnerBetaPaperRepoError::Integrity)?;
    if date.format("%Y-%m-%d").to_string() != text {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    Ok(date)
}

fn validate_preview(preview: &OwnerBetaPaperPreviewDto) -> Result<(), OwnerBetaPaperRepoError> {
    if preview.schema != PREVIEW_SCHEMA
        || preview.preview_id.is_nil()
        || preview.job_id.is_nil()
        || preview.account_id.is_nil()
        || preview.control_epoch < 0
        || preview.config_epoch < 0
        || preview.daily_activation_epoch < 0
        || preview.signal_date != preview.lineage.signal_date
        || preview.effective_session != preview.lineage.effective_session
        || preview.effective_session <= preview.signal_date
        || preview.control_epoch != preview.lineage.control_epoch
        || preview.config_epoch != preview.lineage.config_epoch
        || preview.daily_activation_epoch != preview.lineage.daily_activation_epoch
        || preview.error.is_some()
            != (preview.status == crate::http::dto::OwnerBetaPaperPreviewStatus::Failed)
        || preview
            .error
            .as_ref()
            .is_some_and(|error| !is_error_code(&error.code))
    {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    validate_lineage(&preview.lineage)?;
    if preview
        .preview_token
        .as_deref()
        .is_some_and(|token| !is_lower_hex(token, 64))
    {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    let has_token = preview.preview_token.is_some();
    let published = matches!(
        preview.status,
        crate::http::dto::OwnerBetaPaperPreviewStatus::Ready
            | crate::http::dto::OwnerBetaPaperPreviewStatus::Applied
    );
    if has_token != published || preview.result.is_some() != published {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    if let Some(result) = &preview.result {
        if result.schema != "owner-beta-paper-preview-result-v1"
            || result.cost_profile_id != "KRX_ETF_DEFAULT"
            || result.cost_profile_version != 1
            || result.target_snapshot_sha256 != preview.lineage.target_snapshot_sha256
            || result.signal_allocation_sha256 != preview.lineage.signal_allocation_sha256
            || (result.plan_status == crate::http::dto::OwnerBetaPaperPlanStatus::NoTrade
                && (!result.orders.is_empty()
                    || !is_zero_decimal(&result.buy_notional)
                    || !is_zero_decimal(&result.sell_notional)
                    || !is_zero_decimal(&result.explicit_fees)
                    || !is_zero_decimal(&result.informational_slippage)))
        {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
        for amount in [
            &result.equity,
            &result.cash_before,
            &result.available_cash,
            &result.leftover_cash,
            &result.buy_notional,
            &result.sell_notional,
            &result.explicit_fees,
            &result.informational_slippage,
        ] {
            if !is_decimal(amount) {
                return Err(OwnerBetaPaperRepoError::Integrity);
            }
        }
        if result.price_observations.len() > 11
            || result.price_observations.iter().any(|observation| {
                observation.instrument_id.is_empty()
                    || observation.session_date != preview.signal_date
                    || observation.observed_at > Utc::now()
            })
        {
            return Err(OwnerBetaPaperRepoError::Integrity);
        }
    }
    Ok(())
}

fn is_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn validate_lineage(lineage: &OwnerBetaPaperLineageDto) -> Result<(), OwnerBetaPaperRepoError> {
    if lineage.recommendation_run_id.is_nil()
        || lineage.strategy_config_id.is_nil()
        || lineage.control_epoch < 0
        || lineage.config_epoch < 0
        || lineage.daily_activation_epoch < 0
        || !is_sha256(&lineage.cohort_sha256)
        || !is_sha256(&lineage.daily_pins.admission_sha256)
        || !is_sha256(&lineage.daily_pins.content_sha256)
        || !is_sha256(&lineage.daily_pins.manifest_sha256)
        || !is_sha256(&lineage.daily_pins.policy_sha256)
        || !is_sha256(&lineage.factor_snapshot_sha256)
        || !is_sha256(&lineage.target_snapshot_sha256)
        || !is_sha256(&lineage.signal_allocation_sha256)
    {
        return Err(OwnerBetaPaperRepoError::Integrity);
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| is_lower_hex(hex, 64))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_decimal(value: &str) -> bool {
    let mut dot = false;
    let mut digits = 0usize;
    for byte in value.bytes() {
        if byte.is_ascii_digit() {
            digits += 1;
        } else if byte == b'.' && !dot {
            dot = true;
        } else {
            return false;
        }
    }
    digits > 0 && !value.starts_with('.') && !value.ends_with('.')
}

fn is_zero_decimal(value: &str) -> bool {
    is_decimal(value) && value.bytes().all(|byte| byte == b'0' || byte == b'.')
}

const PREVIEW_READ_SQL: &str = r#"
SELECT jsonb_build_object(
  'schema','owner-beta-paper-preview-v1',
  'preview_id',p.id,'job_id',p.job_id,'account_id',p.account_id,'status',p.status,
  'signal_date',l.signal_date,'effective_session',l.effective_session,
  'control_epoch',l.control_epoch,'config_epoch',l.config_epoch,
  'daily_activation_epoch',l.daily_activation_epoch,
  'lineage',jsonb_build_object(
    'recommendation_run_id',l.recommendation_run_id,
    'strategy_config_id',l.strategy_config_id,
    'signal_date',l.signal_date,'effective_session',l.effective_session,
    'control_epoch',l.control_epoch,'config_epoch',l.config_epoch,
    'daily_activation_epoch',l.daily_activation_epoch,
    'cohort_sha256',l.cohort_sha256,
    'daily_pins',jsonb_build_object(
      'admission_sha256',a.admission_sha256,'content_sha256',a.content_sha256,
      'manifest_sha256',a.manifest_sha256,'policy_sha256',a.policy_sha256),
    'factor_snapshot_sha256',l.factor_snapshot_sha256,
    'target_snapshot_sha256',l.target_snapshot_sha256,
    'signal_allocation_sha256',p.signal_allocation_sha256,
    'target_weights',l.target_weights_json),
  'preview_token',p.preview_token,'cost_profile_id',p.cost_profile_id,
  'cost_profile_version',p.cost_profile_version,
  'result',p.result_json,
  'error',CASE WHEN p.error_json IS NULL THEN NULL
               ELSE jsonb_build_object('code',p.error_json->>'code') END,
  'created_at',p.created_at,'started_at',p.started_at,
  'completed_at',p.completed_at,'applied_at',p.applied_at,'updated_at',p.updated_at)
FROM paper_rebalance_previews p
JOIN owner_beta_paper_lineage l
  ON l.id=p.beta_lineage_id AND l.owner_user_id=p.owner_user_id
 AND l.account_id=p.account_id AND l.strategy_config_id=p.strategy_config_id
JOIN owner_beta_daily_admissions a
  ON a.admission_sha256=l.signal_admission_sha256 AND a.owner_user_id=p.owner_user_id
WHERE p.id=$1 AND p.owner_user_id=$2 AND p.account_id=$3
  AND p.input_kind='OWNER_BETA_DAILY_V1'
"#;

const SESSION_READ_SQL: &str = r#"
SELECT jsonb_build_object(
  'schema','owner-beta-paper-session-v1',
  'pending_target_id',s.pending_target_id,'state',s.state,
  'signal_date',l.signal_date,'effective_session',l.effective_session,
  'processing_origin',s.processing_origin,
  'lineage',jsonb_build_object(
    'recommendation_run_id',l.recommendation_run_id,
    'strategy_config_id',l.strategy_config_id,
    'signal_date',l.signal_date,'effective_session',l.effective_session,
    'control_epoch',l.control_epoch,'config_epoch',l.config_epoch,
    'daily_activation_epoch',l.daily_activation_epoch,
    'cohort_sha256',l.cohort_sha256,
    'daily_pins',jsonb_build_object(
      'admission_sha256',a.admission_sha256,'content_sha256',a.content_sha256,
      'manifest_sha256',a.manifest_sha256,'policy_sha256',a.policy_sha256),
    'factor_snapshot_sha256',l.factor_snapshot_sha256,
    'target_snapshot_sha256',l.target_snapshot_sha256,
    'signal_allocation_sha256',(SELECT p.signal_allocation_sha256
      FROM paper_rebalance_previews p
      WHERE p.pending_target_id=s.pending_target_id
        AND p.owner_user_id=s.owner_user_id AND p.account_id=s.account_id
        AND p.beta_lineage_id=l.id AND p.input_kind='OWNER_BETA_DAILY_V1'),
    'target_weights',l.target_weights_json),
  'source_original_retrieved_at',s.source_original_retrieved_at,
  'input_admitted_at',s.input_admitted_at,'usable_at',s.usable_at,
  'settled_at',s.settled_at,'valued_at',s.valued_at,
  'settlement_kind',s.settlement_kind,'reason_code',s.reason_code,
  'amounts',jsonb_build_object(
    'cash_after',s.receipt_json #>> '{settlement,cash_after}',
    'valuation_cash',s.receipt_json #>> '{valuation,cash}',
    'positions_value',s.receipt_json #>> '{valuation,positions_value}',
    'equity',s.receipt_json #>> '{valuation,equity}'),
  'order_references',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('order_ref',entry->>'order_ref',
                                        'fill_ref',entry->>'fill_ref') ORDER BY ordinal)
    FROM jsonb_array_elements(COALESCE(s.receipt_json #> '{settlement,orders}','[]'::jsonb))
         WITH ORDINALITY AS order_rows(entry,ordinal)), '[]'::jsonb),
  'comparison',jsonb_build_object(
    'disposition','NOT_COMPARABLE',
    'reason','NO_MATCHED_BACKTEST_EXECUTION_CONTRACT'))
FROM owner_beta_paper_sessions s
JOIN pending_targets t ON t.id=s.pending_target_id AND t.owner_user_id=s.owner_user_id
  AND t.account_id=s.account_id AND t.beta_lineage_id IS NOT NULL
JOIN owner_beta_paper_lineage l ON l.id=t.beta_lineage_id
  AND l.owner_user_id=s.owner_user_id AND l.account_id=s.account_id
JOIN owner_beta_daily_admissions a ON a.admission_sha256=l.signal_admission_sha256
  AND a.owner_user_id=s.owner_user_id
WHERE s.pending_target_id=$1 AND s.owner_user_id=$2 AND s.account_id=$3
"#;

fn map_economic_sqlx_error(error: sqlx::Error) -> OwnerBetaPaperRepoError {
    match error.as_database_error().and_then(|db| db.code()) {
        Some(code) if code == "42501" => OwnerBetaPaperRepoError::SourceBlocked,
        other => map_sqlx_error_code(other.as_deref()),
    }
}

fn map_sqlx_error_code(code: Option<&str>) -> OwnerBetaPaperRepoError {
    match code {
        Some("42501") => OwnerBetaPaperRepoError::Forbidden,
        Some("P0002") => OwnerBetaPaperRepoError::NotFound,
        Some("40001") => OwnerBetaPaperRepoError::Stale,
        Some("23505") => OwnerBetaPaperRepoError::Conflict,
        Some("23514") => OwnerBetaPaperRepoError::SourceBlocked,
        Some("22023") => OwnerBetaPaperRepoError::InvalidParameter,
        Some("22003") => OwnerBetaPaperRepoError::Conflict,
        Some("55000") => OwnerBetaPaperRepoError::NotReady,
        _ => OwnerBetaPaperRepoError::Internal,
    }
}

fn map_consent_sqlx_error(error: sqlx::Error) -> OwnerBetaPaperRepoError {
    if error
        .as_database_error()
        .and_then(|db| db.code())
        .as_deref()
        == Some("23505")
    {
        OwnerBetaPaperRepoError::IdempotencyMismatch
    } else {
        map_sqlx_error(error)
    }
}

#[cfg(test)]
mod state_probe_tests {
    use super::*;

    fn expectation() -> StateProbeExpectation {
        StateProbeExpectation {
            owner_user_id: Uuid::from_u128(1),
            account_id: Uuid::from_u128(2),
            strategy_config_id: Uuid::from_u128(3),
            control_epoch: 7,
            daily_cohort_sha256: Some(format!("sha256:{}", "a".repeat(64))),
            process_binding_sha256: Some(format!("sha256:{}", "b".repeat(64))),
        }
    }

    #[test]
    fn runtime_probe_requires_every_state_identity_and_digest_to_match() {
        let expected = expectation();
        let owner = expected.owner_user_id.to_string();
        let account = expected.account_id.to_string();
        let strategy = expected.strategy_config_id.to_string();
        let cohort = expected.daily_cohort_sha256.as_deref().unwrap();
        let process_config = expected.process_binding_sha256.as_deref().unwrap();
        let validate = |owner: &str,
                        account: &str,
                        strategy: &str,
                        epoch: i64,
                        cohort: &str,
                        process_config: &str| {
            validate_state_runtime_binding(
                &expected,
                owner,
                account,
                strategy,
                epoch,
                cohort,
                process_config,
            )
        };

        assert_eq!(
            validate(&owner, &account, &strategy, 7, cohort, process_config),
            Ok(())
        );
        assert_eq!(
            validate(
                &Uuid::from_u128(4).to_string(),
                &account,
                &strategy,
                7,
                cohort,
                process_config,
            ),
            Err(StateProbeFailure::RuntimeBindingMismatch)
        );
        assert_eq!(
            validate(
                &owner,
                &Uuid::from_u128(4).to_string(),
                &strategy,
                7,
                cohort,
                process_config,
            ),
            Err(StateProbeFailure::RuntimeBindingMismatch)
        );
        assert_eq!(
            validate(
                &owner,
                &account,
                &Uuid::from_u128(4).to_string(),
                7,
                cohort,
                process_config,
            ),
            Err(StateProbeFailure::RuntimeBindingMismatch)
        );
        assert_eq!(
            validate(&owner, &account, &strategy, 8, cohort, process_config),
            Err(StateProbeFailure::RuntimeBindingMismatch)
        );
        assert_eq!(
            validate(
                &owner,
                &account,
                &strategy,
                7,
                &format!("sha256:{}", "c".repeat(64)),
                process_config,
            ),
            Err(StateProbeFailure::RuntimeBindingMismatch)
        );
        assert_eq!(
            validate(
                &owner,
                &account,
                &strategy,
                7,
                cohort,
                &format!("sha256:{}", "d".repeat(64)),
            ),
            Err(StateProbeFailure::RuntimeBindingMismatch)
        );
    }

    #[test]
    fn missing_enabled_state_runtime_pins_fail_closed() {
        let mut expected = expectation();
        expected.daily_cohort_sha256 = None;
        assert_eq!(
            validate_state_runtime_binding(
                &expected,
                &expected.owner_user_id.to_string(),
                &expected.account_id.to_string(),
                &expected.strategy_config_id.to_string(),
                expected.control_epoch,
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            Err(StateProbeFailure::ExpectedBindingMissing)
        );
    }

    #[test]
    fn failed_runtime_probe_projects_only_enabled_state_to_blocked() {
        assert_eq!(
            project_state_probe(
                OwnerBetaPaperStateKind::Enabled,
                Err(StateProbeFailure::RuntimeBindingMismatch)
            ),
            OwnerBetaPaperStateKind::Blocked
        );
        assert_eq!(
            project_state_probe(
                OwnerBetaPaperStateKind::Enabled,
                Ok::<(), StateProbeFailure>(()),
            ),
            OwnerBetaPaperStateKind::Enabled
        );
        assert_eq!(
            project_state_probe(
                OwnerBetaPaperStateKind::Off,
                Err(StateProbeFailure::CurrentRefused)
            ),
            OwnerBetaPaperStateKind::Off
        );
        assert_eq!(
            project_state_probe(
                OwnerBetaPaperStateKind::Unbound,
                Err(StateProbeFailure::ExpectedBindingMissing)
            ),
            OwnerBetaPaperStateKind::Unbound
        );
        for state in [
            OwnerBetaPaperStateKind::Disabled,
            OwnerBetaPaperStateKind::Draining,
            OwnerBetaPaperStateKind::RestoredOff,
            OwnerBetaPaperStateKind::Blocked,
        ] {
            assert_eq!(
                project_state_probe(state, Err(StateProbeFailure::CurrentRefused)),
                state
            );
        }
    }
}
