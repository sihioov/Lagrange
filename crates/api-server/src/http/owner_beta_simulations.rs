//! Dedicated authenticated HTTP surface for owner-beta price-only simulations.
//!
//! Production enqueue always starts from the existing sealed artifact
//! approval. The cfg(test)-only synthetic input is private to the library
//! harness and cannot be called from an integration crate or production.

use crate::http::dto::{
    EmptyBody, OwnerBetaSimulationEnqueuedDto, OwnerBetaSimulationRunBody,
    SimulationContractVersionField, SimulationDateField,
};
use crate::http::error::{api_error, code_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{PageParams, Session, require_csrf};
use crate::http::state::{ApiState, OwnerBetaAccessMode};
use crate::http::{JsonBody, audit, idempotent};
use crate::repos::owner_beta_simulations::{
    OwnerBetaSimulationRepoError, PreparedV2Admission, V2SubmissionPreflight,
};
use auth::entitlement::{AccessRequest, CalendarDate, DatasetId, EntitlementService, KrUse};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use domain::TradingDate;
#[cfg(test)]
use job_queue::owner_beta::simulation_execution::SimulationJobInput;
use job_queue::owner_beta::simulation_execution::{
    SimulationContractVersion, V2SimulationJobInput, VersionedSimulationJobInput,
};
use job_queue::resolver::ResolvedConfig;
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use serde_json::json;
use uuid::Uuid;

const BASE_PATH: &str = "/api/v1/backtests/owner-beta/price-only";
const BACKTEST_DATASET: &str = "krx_eod_bars";

pub async fn create_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<OwnerBetaSimulationRunBody>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    if !state.cfg.owner_beta_price_input.is_enabled() {
        return code_error(
            "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
            "owner-beta price input unavailable",
            &rid,
        );
    }
    if let Err(response) = require_csrf(&headers, &session.0) {
        return response;
    }
    let client_key = match client_key(&headers, &rid) {
        Ok(key) => key,
        Err(response) => return response,
    };
    let strategy_config_id = match Uuid::parse_str(&body.strategy_config_id) {
        Ok(id) => id,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "strategy_config_id must be a uuid",
                &rid,
                None,
            );
        }
    };
    let start_date = match parse_date_field(body.start_date, "start_date", &rid) {
        Ok(date) => date,
        Err(response) => return response,
    };
    let end_date = match parse_date_field(body.end_date, "end_date", &rid) {
        Ok(date) => date,
        Err(response) => return response,
    };
    let contract_version = match parse_contract_version(body.contract_version, &rid) {
        Ok(version) => version,
        Err(response) => return response,
    };
    if contract_version == SimulationContractVersion::V2 {
        return create_v2_run(
            state,
            session,
            headers,
            strategy_config_id,
            (start_date, end_date),
            client_key,
            rid,
        )
        .await;
    }

    #[cfg(test)]
    let (schedule, synthetic_input, artifact) = match state.take_owner_beta_simulation_test_input()
    {
        Some(input) => {
            if input.strategy_config_id() != strategy_config_id
                || start_date.is_none_or(|date| date != input.evaluation_start())
                || end_date.is_none_or(|date| date != input.evaluation_end())
            {
                return code_error(
                    "INVALID_PARAMETER",
                    "synthetic test input must match the closed request",
                    &rid,
                );
            }
            (input.schedule().clone(), Some(input), None)
        }
        None => {
            let artifact = match approve_artifact(&state).await {
                Ok(artifact) => artifact,
                Err(()) => {
                    return code_error(
                        "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
                        "owner-beta price input unavailable",
                        &rid,
                    );
                }
            };
            let schedule =
                match job_queue::owner_beta::simulation::prepare_approved_simulation_window(
                    &artifact, start_date, end_date,
                ) {
                    Ok(schedule) => schedule,
                    Err(_) => {
                        return code_error(
                            "INVALID_DATE",
                            "requested dates are outside the approved trading calendar",
                            &rid,
                        );
                    }
                };
            (schedule, None, Some(artifact))
        }
    };

    #[cfg(not(test))]
    let (schedule, artifact) = {
        let artifact = match approve_artifact(&state).await {
            Ok(artifact) => artifact,
            Err(()) => {
                return code_error(
                    "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
                    "owner-beta price input unavailable",
                    &rid,
                );
            }
        };
        let schedule = match job_queue::owner_beta::simulation::prepare_approved_simulation_window(
            &artifact, start_date, end_date,
        ) {
            Ok(schedule) => schedule,
            Err(_) => {
                return code_error(
                    "INVALID_DATE",
                    "requested dates are outside the approved trading calendar",
                    &rid,
                );
            }
        };
        (schedule, artifact)
    };

    let entitlements = match fresh_entitlements(&state, &rid).await {
        Ok(service) => service,
        Err(response) => return response,
    };
    if let Err(error) = authorize_schedule(&entitlements, &session, &schedule) {
        return map_repo_error(error, &rid);
    }

    let evaluation_start = schedule
        .evaluation_sessions()
        .first()
        .copied()
        .expect("validated simulation schedule has evaluation sessions");
    let evaluation_end = schedule
        .evaluation_sessions()
        .last()
        .copied()
        .expect("validated simulation schedule has evaluation sessions");
    let body_hash = crate::http::idempotency::body_hash(&json!({
        "method": "POST",
        "route": format!("{BASE_PATH}/runs"),
        "strategy_config_id": strategy_config_id,
        "start_date": evaluation_start.to_string(),
        "end_date": evaluation_end.to_string(),
    }));
    let actor = session.actor();
    let repo = state.owner_beta_simulations();
    let max_jobs_per_owner = state.cfg.max_jobs_per_owner;
    let durable_replay = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let replay_flag = durable_replay.clone();
    let result = idempotent(&state, &session, &headers, &body_hash, async {
        #[cfg(test)]
        let enqueue = if let Some(input) = synthetic_input.as_ref() {
            repo.submit_test_input(&actor, input, &client_key, max_jobs_per_owner)
                .await
        } else {
            let Some(artifact) = artifact.as_ref() else {
                return code_error("INTERNAL", "internal error", &rid);
            };
            repo.submit_approved(
                &actor,
                strategy_config_id,
                Some(evaluation_start),
                Some(evaluation_end),
                &client_key,
                artifact,
                max_jobs_per_owner,
            )
            .await
        };

        #[cfg(not(test))]
        let enqueue = repo
            .submit_approved(
                &actor,
                strategy_config_id,
                Some(evaluation_start),
                Some(evaluation_end),
                &client_key,
                &artifact,
                max_jobs_per_owner,
            )
            .await;
        let result = match enqueue {
            Ok(result) => result,
            Err(error) => return map_repo_error(error, &rid),
        };
        replay_flag.store(result.replay, std::sync::atomic::Ordering::Release);
        audit(
            &state,
            &session,
            &headers,
            "owner_beta.simulation.enqueue",
            "owner_beta_simulation_run",
            &result.run_id.to_string(),
            None,
            Some(json!({
                "run_id": result.run_id,
                "job_id": result.job_id,
                "strategy_config_id": strategy_config_id,
                "start_date": evaluation_start.to_string(),
                "end_date": evaluation_end.to_string(),
            })),
            None,
        )
        .await;
        (
            StatusCode::ACCEPTED,
            Json(OwnerBetaSimulationEnqueuedDto {
                run_id: result.run_id.to_string(),
                job_id: result.job_id.to_string(),
                status: "PENDING",
            }),
        )
            .into_response()
    })
    .await;
    let mut result = result;
    if durable_replay.load(std::sync::atomic::Ordering::Acquire) {
        result.headers_mut().insert(
            "X-Idempotent-Replay",
            axum::http::HeaderValue::from_static("true"),
        );
    }
    result
}

async fn create_v2_run(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    strategy_config_id: Uuid,
    (requested_start, requested_end): (Option<TradingDate>, Option<TradingDate>),
    client_key: String,
    rid: String,
) -> Response {
    let actor = session.actor();
    let repo = state.owner_beta_simulations();
    let preflight = match repo
        .preflight_v2_submission(&actor, strategy_config_id, &client_key)
        .await
    {
        Ok(preflight) => preflight,
        Err(error) => return map_repo_error(error, &rid),
    };
    let (resolved, replay, evaluation_start, evaluation_end) = match preflight {
        V2SubmissionPreflight::Replay(input) => {
            if input.strategy_config_id() != strategy_config_id
                || requested_start.is_some_and(|date| date != input.evaluation_start())
                || requested_end.is_some_and(|date| date != input.evaluation_end())
            {
                return map_repo_error(OwnerBetaSimulationRepoError::IdempotencyMismatch, &rid);
            }
            let resolved = ResolvedConfig {
                strategy_id: input.strategy().strategy_id().to_owned(),
                strategy_version: input.strategy().strategy_version().to_owned(),
                config: input.strategy().config_json().clone(),
            };
            let start = Some(input.evaluation_start());
            let end = Some(input.evaluation_end());
            (resolved, Some(input), start, end)
        }
        V2SubmissionPreflight::New(resolved) => (resolved, None, requested_start, requested_end),
    };

    let admission = match prepare_v2_admission(
        &state,
        strategy_config_id,
        &resolved,
        evaluation_start,
        evaluation_end,
        replay.as_ref(),
    )
    .await
    {
        Ok(admission) => admission,
        Err(V2AdmissionFailure::Approval) => {
            return code_error(
                "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
                "owner-beta price input unavailable",
                &rid,
            );
        }
        Err(V2AdmissionFailure::InvalidInput) => {
            return map_repo_error(OwnerBetaSimulationRepoError::Integrity, &rid);
        }
        Err(V2AdmissionFailure::Window) => {
            return map_repo_error(OwnerBetaSimulationRepoError::WindowInvalid, &rid);
        }
        Err(V2AdmissionFailure::Strategy) => {
            return map_repo_error(OwnerBetaSimulationRepoError::StrategyUnsupported, &rid);
        }
    };
    let input = admission.input().clone();
    let versioned = VersionedSimulationJobInput::V2(input.clone());
    let entitlements = match fresh_entitlements(&state, &rid).await {
        Ok(service) => service,
        Err(response) => return response,
    };
    if let Err(error) = authorize_versioned_input(&entitlements, &actor, &versioned) {
        return map_repo_error(error, &rid);
    }

    let body_hash = crate::http::idempotency::body_hash(&json!({
        "method": "POST",
        "route": format!("{BASE_PATH}/runs"),
        "strategy_config_id": strategy_config_id,
        "contract_version": SimulationContractVersion::V2.contract_version(),
        "start_date": input.evaluation_start().to_string(),
        "end_date": input.evaluation_end().to_string(),
    }));
    let max_jobs_per_owner = state.cfg.max_jobs_per_owner;
    let durable_replay = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let replay_flag = durable_replay.clone();
    let result = idempotent(&state, &session, &headers, &body_hash, async {
        let enqueue = match &admission {
            V2AdmissionSource::Prepared(admission) => {
                repo.submit_v2_approved(
                    &actor,
                    admission,
                    &client_key,
                    max_jobs_per_owner,
                    &entitlements,
                )
                .await
            }
            #[cfg(test)]
            V2AdmissionSource::Synthetic(input) => {
                repo.submit_test_v2_input(
                    &actor,
                    input,
                    &client_key,
                    max_jobs_per_owner,
                    &entitlements,
                )
                .await
            }
        };
        let result = match enqueue {
            Ok(result) => result,
            Err(error) => return map_repo_error(error, &rid),
        };
        replay_flag.store(result.replay, std::sync::atomic::Ordering::Release);
        audit(
            &state,
            &session,
            &headers,
            "owner_beta.simulation.enqueue",
            "owner_beta_simulation_run",
            &result.run_id.to_string(),
            None,
            Some(json!({
                "run_id": result.run_id,
                "job_id": result.job_id,
                "strategy_config_id": strategy_config_id,
                "strategy_id": input.strategy().strategy_id(),
                "contract_version": SimulationContractVersion::V2.contract_version(),
                "start_date": input.evaluation_start().to_string(),
                "end_date": input.evaluation_end().to_string(),
            })),
            None,
        )
        .await;
        (
            StatusCode::ACCEPTED,
            Json(OwnerBetaSimulationEnqueuedDto {
                run_id: result.run_id.to_string(),
                job_id: result.job_id.to_string(),
                status: "PENDING",
            }),
        )
            .into_response()
    })
    .await;
    let mut result = result;
    if durable_replay.load(std::sync::atomic::Ordering::Acquire) {
        result.headers_mut().insert(
            "X-Idempotent-Replay",
            axum::http::HeaderValue::from_static("true"),
        );
    }
    result
}

enum V2AdmissionSource {
    Prepared(PreparedV2Admission),
    #[cfg(test)]
    Synthetic(V2SimulationJobInput),
}

impl V2AdmissionSource {
    fn input(&self) -> &V2SimulationJobInput {
        match self {
            Self::Prepared(admission) => admission.input(),
            #[cfg(test)]
            Self::Synthetic(input) => input,
        }
    }
}

enum V2AdmissionFailure {
    Approval,
    InvalidInput,
    Window,
    Strategy,
}

async fn prepare_v2_admission(
    state: &ApiState,
    strategy_config_id: Uuid,
    resolved: &ResolvedConfig,
    evaluation_start: Option<TradingDate>,
    evaluation_end: Option<TradingDate>,
    replay: Option<&V2SimulationJobInput>,
) -> Result<V2AdmissionSource, V2AdmissionFailure> {
    #[cfg(test)]
    if let Some(input) = state.take_owner_beta_simulation_v2_test_input() {
        let expected_strategy =
            job_queue::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(resolved)
                .map_err(|_| V2AdmissionFailure::Strategy)?;
        let matches_replay = replay.is_none_or(|stored| same_v2_intent(&input, stored));
        if input.strategy_config_id() != strategy_config_id
            || input.strategy() != &expected_strategy
            || evaluation_start.is_some_and(|date| date != input.evaluation_start())
            || evaluation_end.is_some_and(|date| date != input.evaluation_end())
            || !matches_replay
        {
            return Err(V2AdmissionFailure::InvalidInput);
        }
        return Ok(V2AdmissionSource::Synthetic(input));
    }

    let permit = state
        .owner_beta_approval
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| V2AdmissionFailure::Approval)?;
    let run_id = replay.map_or_else(Uuid::new_v4, V2SimulationJobInput::run_id);
    let root = state.cfg.artifact_root.join("historical-price-beta-root");
    let resolved = resolved.clone();
    tokio::task::spawn_blocking(move || {
        let result = job_queue::owner_beta::simulation::v2::prepare_v2_simulation_input(&root)
            .map_err(|_| V2AdmissionFailure::Approval)
            .and_then(|prepared| {
                PreparedV2Admission::from_prepared(
                    &prepared,
                    run_id,
                    strategy_config_id,
                    resolved,
                    evaluation_start,
                    evaluation_end,
                )
                .map(V2AdmissionSource::Prepared)
                .map_err(map_v2_admission_error)
            });
        drop(permit);
        result
    })
    .await
    .map_err(|_| V2AdmissionFailure::Approval)?
}

fn map_v2_admission_error(
    error: job_queue::owner_beta::simulation_execution::SimulationExecutionError,
) -> V2AdmissionFailure {
    match error {
        job_queue::owner_beta::simulation_execution::SimulationExecutionError::WindowInvalid => {
            V2AdmissionFailure::Window
        }
        job_queue::owner_beta::simulation_execution::SimulationExecutionError::StrategyInvalid => {
            V2AdmissionFailure::Strategy
        }
        _ => V2AdmissionFailure::InvalidInput,
    }
}

#[cfg(test)]
fn same_v2_intent(expected: &V2SimulationJobInput, stored: &V2SimulationJobInput) -> bool {
    expected.strategy_config_id() == stored.strategy_config_id()
        && expected.evaluation_start() == stored.evaluation_start()
        && expected.evaluation_end() == stored.evaluation_end()
        && expected.required_dates() == stored.required_dates()
        && expected.pins() == stored.pins()
        && expected.contract_document_sha256() == stored.contract_document_sha256()
        && expected.bars_sha256() == stored.bars_sha256()
        && expected.strategy() == stored.strategy()
}

pub async fn list_runs(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(cursor) => cursor,
        Err(response) => return response,
    };
    let entitlements = match fresh_entitlements(&state, &rid).await {
        Ok(service) => service,
        Err(response) => return response,
    };
    let result = state
        .owner_beta_simulations()
        .list(
            &session.actor(),
            cursor,
            params.limit_or(PageParams::DEFAULT_LIMIT),
            &state.cfg.cursor_secret,
            &entitlements,
            (state.cfg.seoul_today)(),
        )
        .await;
    match result {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(error) => map_repo_error(error, &rid),
    }
}

pub async fn get_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    let run_id = match parse_run_id(&run_id, &rid) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let repo = state.owner_beta_simulations();
    let header = match repo.get_header(&session.actor(), run_id).await {
        Ok(header) => header,
        Err(error) => return map_repo_error(error, &rid),
    };
    let entitlements = match fresh_entitlements(&state, &rid).await {
        Ok(service) => service,
        Err(response) => return response,
    };
    if let Err(error) = authorize_versioned_input(&entitlements, &session.actor(), &header.input) {
        return map_repo_error(error, &rid);
    }
    match repo.get(&session.actor(), run_id, &entitlements).await {
        Ok(run) => (StatusCode::OK, Json(run)).into_response(),
        Err(error) => map_repo_error(error, &rid),
    }
}

pub async fn get_pair(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    let run_id = match parse_run_id(&run_id, &rid) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let repo = state.owner_beta_simulations();
    let header = match repo.get_header(&session.actor(), run_id).await {
        Ok(header) => header,
        Err(error) => return map_repo_error(error, &rid),
    };
    let entitlements = match fresh_entitlements(&state, &rid).await {
        Ok(service) => service,
        Err(response) => return response,
    };
    if let Err(error) = authorize_versioned_input(&entitlements, &session.actor(), &header.input) {
        return map_repo_error(error, &rid);
    }
    match repo.get_pair(&session.actor(), run_id, &entitlements).await {
        Ok(pair) => (StatusCode::OK, Json(pair)).into_response(),
        Err(error) => map_repo_error(error, &rid),
    }
}

pub async fn cancel_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    JsonBody(_body): JsonBody<EmptyBody>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    if let Err(response) = require_csrf(&headers, &session.0) {
        return response;
    }
    if let Err(response) = client_key(&headers, &rid) {
        return response;
    }
    let run_id = match parse_run_id(&run_id, &rid) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let repo = state.owner_beta_simulations();
    let (owned_run_id, owned_job_id) = match repo.cancel_target(&session.actor(), run_id).await {
        Ok(target) => target,
        Err(error) => return map_repo_error(error, &rid),
    };
    let body_hash = crate::http::idempotency::body_hash(&json!({
        "method": "POST",
        "route": format!("{BASE_PATH}/runs/{run_id}/cancel"),
        "run_id": run_id,
        "body": {},
    }));
    let actor = session.actor();
    idempotent(&state, &session, &headers, &body_hash, async {
        let (current_run_id, job_id) = match repo.cancel_target(&actor, run_id).await {
            Ok(target) => target,
            Err(error) => return map_repo_error(error, &rid),
        };
        if current_run_id != owned_run_id || job_id != owned_job_id {
            return code_error(
                "RESULT_INTEGRITY_FAILED",
                "simulation binding is invalid",
                &rid,
            );
        }
        let queue = match state.queue_for(&actor).await {
            Ok(queue) => queue,
            Err(_) => return code_error("INTERNAL", "queue unavailable", &rid),
        };
        let actor_for_audit = job_queue::AuditActor {
            role: "owner".to_owned(),
            user_id: crate::actor_tx::actor_uuid(&actor).ok(),
            correlation_id: Some(rid.clone()),
        };
        if queue
            .request_cancel(job_id, &actor_for_audit)
            .await
            .is_err()
        {
            return code_error(
                "INTERNAL",
                "cancellation outcome is unavailable; check the run status",
                &rid,
            );
        }
        audit(
            &state,
            &session,
            &headers,
            "owner_beta.simulation.cancel",
            "owner_beta_simulation_run",
            &run_id.to_string(),
            Some(json!({ "job_id": job_id })),
            Some(json!({ "status": "CANCEL_REQUESTED" })),
            None,
        )
        .await;
        (
            StatusCode::OK,
            Json(json!({
                "run_id": run_id,
                "job_id": job_id,
                "status": "CANCEL_REQUESTED",
            })),
        )
            .into_response()
    })
    .await
}

fn owner_allowed(state: &ApiState, session: &Session) -> bool {
    state.cfg.owner_beta_access == OwnerBetaAccessMode::OwnerOnly && session.actor().is_owner()
}

#[allow(clippy::result_large_err)]
fn client_key(headers: &HeaderMap, rid: &str) -> Result<String, Response> {
    let Some(value) = headers.get(header::HeaderName::from_static("idempotency-key")) else {
        return Err(code_error(
            "IDEMPOTENCY_KEY_REQUIRED",
            "mutating routes require an Idempotency-Key header",
            rid,
        ));
    };
    let Ok(value) = value.to_str() else {
        return Err(code_error(
            "INVALID_PARAMETER",
            "Idempotency-Key is invalid",
            rid,
        ));
    };
    if value.is_empty()
        || value.len() > crate::http::idempotency::MAX_KEY_BYTES
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(code_error(
            "INVALID_PARAMETER",
            "Idempotency-Key is invalid",
            rid,
        ));
    }
    Ok(value.to_owned())
}

#[allow(clippy::result_large_err)]
fn parse_date_field(
    field: SimulationDateField,
    name: &str,
    rid: &str,
) -> Result<Option<TradingDate>, Response> {
    match field {
        SimulationDateField::Missing => Ok(None),
        SimulationDateField::Null => Err(api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            format!("{name} cannot be null"),
            rid,
            None,
        )),
        SimulationDateField::Value(value) => {
            let Some(date) = parse_strict_date(&value) else {
                return Err(code_error(
                    "INVALID_DATE",
                    "dates must be valid YYYY-MM-DD calendar dates",
                    rid,
                ));
            };
            TradingDate::new(date.year(), date.month(), date.day())
                .map(Some)
                .map_err(|_| {
                    code_error(
                        "INVALID_DATE",
                        "dates must be valid YYYY-MM-DD calendar dates",
                        rid,
                    )
                })
        }
    }
}

fn parse_strict_date(value: &str) -> Option<NaiveDate> {
    if value.len() != 10
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

async fn approve_artifact(state: &ApiState) -> Result<ApprovedHistoricalPriceOnlyArtifact, ()> {
    let permit = state
        .owner_beta_approval
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ())?;
    let root = state.cfg.artifact_root.join("historical-price-beta-root");
    tokio::task::spawn_blocking(move || {
        let result = market_data::approve_historical_price_only_artifact(&root);
        drop(permit);
        result.map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}

async fn fresh_entitlements(state: &ApiState, rid: &str) -> Result<EntitlementService, Response> {
    crate::repos::entitlements::EntitlementRepo::new(state.app_pool.clone())
        .load()
        .await
        .map(EntitlementService::new)
        .map_err(|_| code_error("INTERNAL", "internal error", rid))
}

fn authorize_schedule(
    service: &EntitlementService,
    session: &Session,
    schedule: &job_queue::owner_beta::simulation::SimulationSchedule,
) -> Result<(), crate::repos::owner_beta_simulations::OwnerBetaSimulationRepoError> {
    for date in schedule
        .warmup_sessions()
        .iter()
        .chain(schedule.evaluation_sessions())
    {
        let iso = date.to_string();
        let request = AccessRequest {
            actor: session.actor(),
            dataset: DatasetId::new(BACKTEST_DATASET),
            as_of: CalendarDate::parse(&iso).map_err(|_| {
                crate::repos::owner_beta_simulations::OwnerBetaSimulationRepoError::Integrity
            })?,
        };
        service
            .authorize_use(KrUse::Backtest, &request)
            .map_err(|_| {
                crate::repos::owner_beta_simulations::OwnerBetaSimulationRepoError::EntitlementDenied
            })?;
    }
    Ok(())
}

fn authorize_versioned_input(
    service: &EntitlementService,
    actor: &auth::entitlement::Actor,
    input: &VersionedSimulationJobInput,
) -> Result<(), OwnerBetaSimulationRepoError> {
    for date in input.required_dates() {
        let request = AccessRequest {
            actor: actor.clone(),
            dataset: DatasetId::new(BACKTEST_DATASET),
            as_of: CalendarDate::parse(&date.to_string())
                .map_err(|_| OwnerBetaSimulationRepoError::Integrity)?,
        };
        service
            .authorize_use(KrUse::Backtest, &request)
            .map_err(|_| OwnerBetaSimulationRepoError::EntitlementDenied)?;
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn parse_contract_version(
    field: SimulationContractVersionField,
    rid: &str,
) -> Result<SimulationContractVersion, Response> {
    match field {
        SimulationContractVersionField::Missing => Ok(SimulationContractVersion::V1),
        SimulationContractVersionField::Value(value)
            if value == SimulationContractVersion::V1.contract_version() =>
        {
            Ok(SimulationContractVersion::V1)
        }
        SimulationContractVersionField::Value(value)
            if value == SimulationContractVersion::V2.contract_version() =>
        {
            Ok(SimulationContractVersion::V2)
        }
        SimulationContractVersionField::Null | SimulationContractVersionField::Value(_) => Err(
            code_error("INVALID_PARAMETER", "contract_version is unsupported", rid),
        ),
    }
}

#[allow(clippy::result_large_err)]
fn decode_cursor(
    state: &ApiState,
    rid: &str,
    raw: Option<&str>,
) -> Result<Option<(DateTime<Utc>, Uuid)>, Response> {
    let Some(raw) = raw else { return Ok(None) };
    let cursor = Cursor::decode(raw, &state.cfg.cursor_secret)
        .map_err(|_| code_error("INVALID_CURSOR", "pagination cursor is invalid", rid))?;
    let created_at = DateTime::parse_from_rfc3339(&cursor.k)
        .map_err(|_| code_error("INVALID_CURSOR", "pagination cursor is invalid", rid))?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(&cursor.i)
        .map_err(|_| code_error("INVALID_CURSOR", "pagination cursor is invalid", rid))?;
    Ok(Some((created_at, id)))
}

#[allow(clippy::result_large_err)]
fn parse_run_id(value: &str, rid: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(value)
        .map_err(|_| code_error("INVALID_PARAMETER", "run id must be a uuid", rid))
}

fn map_repo_error(
    error: crate::repos::owner_beta_simulations::OwnerBetaSimulationRepoError,
    rid: &str,
) -> Response {
    use crate::repos::owner_beta_simulations::OwnerBetaSimulationRepoError as RepoError;
    match error {
        RepoError::NotFound => code_error("RESOURCE_NOT_FOUND", "resource not found", rid),
        RepoError::EntitlementDenied => code_error(
            "DATA_ENTITLEMENT_REQUIRED",
            "an active KRX entitlement covering Backtest use is required",
            rid,
        ),
        RepoError::CapacityExceeded => code_error(
            "BACKTEST_CAPACITY_EXCEEDED",
            "per-owner queued job capacity exceeded",
            rid,
        ),
        RepoError::IdempotencyMismatch => code_error(
            "IDEMPOTENCY_KEY_MISMATCH",
            "the same Idempotency-Key was already used with a different request intent",
            rid,
        ),
        RepoError::StrategyUnsupported => code_error(
            "OWNER_BETA_STRATEGY_UNSUPPORTED",
            "owner-beta strategy unsupported",
            rid,
        ),
        RepoError::WindowInvalid => code_error(
            "INVALID_DATE",
            "requested dates are outside the approved trading calendar",
            rid,
        ),
        RepoError::Integrity => code_error(
            "RESULT_INTEGRITY_FAILED",
            "simulation result integrity failed",
            rid,
        ),
        RepoError::Internal => code_error("INTERNAL", "internal error", rid),
        RepoError::NotReady => code_error(
            "SIMULATION_RESULT_NOT_READY",
            "simulation result pair is not ready",
            rid,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_status_test_common::{Harness, status};
    use auth::entitlement::Role;

    const START: &str = "2026-08-27";
    const END: &str = "2026-08-28";

    fn body(config_id: Uuid, start: &str, end: &str) -> serde_json::Value {
        json!({
            "strategy_config_id": config_id,
            "start_date": start,
            "end_date": end,
        })
    }

    fn test_input(config_id: Uuid, start: &str, end: &str) -> SimulationJobInput {
        Harness::simulation_test_input(config_id, Uuid::new_v4(), start, end)
    }

    #[tokio::test]
    async fn simulation_durable_replay_and_intent_binding() {
        let Some(mut harness) = Harness::new().await else {
            eprintln!("SKIP: DATABASE_URL not set");
            return;
        };
        harness.seed_simulation_backtest_rights().await;
        let owner_config = harness
            .seed_simulation_strategy_config(&harness.owner)
            .await;
        harness.restart_api_with_owner_beta_price_input().await;
        harness.restart_api_with_max_jobs_per_owner(1).await;

        let app_a = harness.app.clone();
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, START, END));
        harness.restart_api_with_owner_beta_price_input().await;
        let app_b = harness.app.clone();
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, START, END));
        let path = format!("{BASE_PATH}/runs");
        let first = Harness::send_to(
            app_a,
            "POST",
            &path,
            Some(&harness.owner),
            true,
            Some("simulation-concurrent-a"),
            Some("same-owner-intent"),
            Some(body(owner_config, START, END)),
        );
        let second = Harness::send_to(
            app_b,
            "POST",
            &path,
            Some(&harness.owner),
            true,
            Some("simulation-concurrent-b"),
            Some("same-owner-intent"),
            Some(body(owner_config, START, END)),
        );
        let (first, second) = tokio::join!(first, second);
        assert_eq!(status(&first), axum::http::StatusCode::ACCEPTED);
        assert_eq!(status(&second), axum::http::StatusCode::ACCEPTED);
        let first_was_replay = first
            .headers()
            .get("x-idempotent-replay")
            .and_then(|value| value.to_str().ok())
            == Some("true");
        let second_was_replay = second
            .headers()
            .get("x-idempotent-replay")
            .and_then(|value| value.to_str().ok())
            == Some("true");
        assert_ne!(first_was_replay, second_was_replay);
        let first_body = Harness::body_json(first).await;
        let second_body = Harness::body_json(second).await;
        assert_eq!(first_body["status"], "PENDING");
        assert_eq!(first_body["run_id"], second_body["run_id"]);
        assert_eq!(first_body["job_id"], second_body["job_id"]);
        assert_eq!(harness.simulation_job_count(&harness.owner).await, 1);

        // Rebuild ApiState to discard the process-local cache. The durable
        // actor-owned queue binding must still return the original envelope.
        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, START, END));
        let durable = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-durable-replay"),
                Some("same-owner-intent"),
                Some(body(owner_config, START, END)),
            )
            .await;
        assert_eq!(status(&durable), axum::http::StatusCode::ACCEPTED);
        assert_eq!(
            durable
                .headers()
                .get("x-idempotent-replay")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        let durable_body = Harness::body_json(durable).await;
        assert_eq!(durable_body["run_id"], first_body["run_id"]);
        assert_eq!(durable_body["job_id"], first_body["job_id"]);

        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, "2026-08-26", END));
        let changed = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-changed-intent"),
                Some("same-owner-intent"),
                Some(body(owner_config, "2026-08-26", END)),
            )
            .await;
        assert_eq!(status(&changed), axum::http::StatusCode::CONFLICT);
        assert_eq!(
            Harness::error_code(&Harness::body_json(changed).await),
            "IDEMPOTENCY_KEY_MISMATCH"
        );
        assert_eq!(harness.simulation_job_count(&harness.owner).await, 1);

        // A new public key exceeds this Owner's one-job limit, while another
        // Owner can independently use the same public key and frozen input.
        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, START, END));
        let capacity = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-capacity"),
                Some("new-owner-intent"),
                Some(body(owner_config, START, END)),
            )
            .await;
        assert_eq!(status(&capacity), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            Harness::error_code(&Harness::body_json(capacity).await),
            "BACKTEST_CAPACITY_EXCEEDED"
        );

        let terminal_fixture = crate::service_status_test_common::SimulationFixture {
            run_id: Uuid::parse_str(first_body["run_id"].as_str().expect("run id"))
                .expect("original run UUID"),
            job_id: Uuid::parse_str(first_body["job_id"].as_str().expect("job id"))
                .expect("original job UUID"),
            owner_user_id: harness.owner.user_id,
            strategy_config_id: owner_config,
        };
        harness
            .seed_simulation_success_pair(&terminal_fixture)
            .await;
        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, START, END));
        let terminal_replay = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-terminal-replay"),
                Some("same-owner-intent"),
                Some(body(owner_config, START, END)),
            )
            .await;
        assert_eq!(status(&terminal_replay), axum::http::StatusCode::ACCEPTED);
        let terminal_replay_body = Harness::body_json(terminal_replay).await;
        assert_eq!(terminal_replay_body["run_id"], first_body["run_id"]);
        assert_eq!(terminal_replay_body["job_id"], first_body["job_id"]);

        let owner_b = harness
            .seed_user(
                Role::Owner,
                "simulation-replay-owner-b@lagrange.test",
                "simulation-replay-owner-b-iss",
                "simulation-replay-owner-b-sub",
            )
            .await;
        let config_b = harness.seed_simulation_strategy_config(&owner_b).await;
        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(config_b, START, END));
        let independent = harness
            .send(
                "POST",
                &path,
                Some(&owner_b),
                true,
                Some("simulation-other-owner"),
                Some("same-owner-intent"),
                Some(body(config_b, START, END)),
            )
            .await;
        assert_eq!(status(&independent), axum::http::StatusCode::ACCEPTED);
        let independent_body = Harness::body_json(independent).await;
        assert_ne!(independent_body["run_id"], first_body["run_id"]);
        assert_eq!(harness.simulation_job_count(&owner_b).await, 1);

        harness
            .seed_shared(
                "UPDATE data_entitlements SET effective_from='2026-08-27' \
                   WHERE contract_reference='simulation-api-test'",
            )
            .await;
        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(owner_config, START, END));
        let warmup_denied = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-warmup-rights"),
                Some("warmup-denied-intent"),
                Some(body(owner_config, START, END)),
            )
            .await;
        assert_eq!(status(&warmup_denied), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(
            Harness::error_code(&Harness::body_json(warmup_denied).await),
            "DATA_ENTITLEMENT_REQUIRED"
        );
        assert_eq!(harness.simulation_job_count(&harness.owner).await, 1);
        harness.teardown().await;
    }

    #[tokio::test]
    async fn simulation_v1_omitted_and_explicit_version_share_the_original_intent() {
        let Some(mut harness) = Harness::new().await else {
            eprintln!("SKIP: DATABASE_URL not set");
            return;
        };
        harness.seed_simulation_backtest_rights().await;
        let config_id = harness
            .seed_simulation_strategy_config(&harness.owner)
            .await;
        harness.restart_api_with_owner_beta_price_input().await;
        let path = format!("{BASE_PATH}/runs");
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(config_id, START, END));
        let omitted = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v1-omitted"),
                Some("v1-equivalent-intent"),
                Some(body(config_id, START, END)),
            )
            .await;
        assert_eq!(status(&omitted), axum::http::StatusCode::ACCEPTED);
        let original = Harness::body_json(omitted).await;

        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(config_id, START, END));
        let mut explicit_body = body(config_id, START, END);
        explicit_body["contract_version"] = json!(SimulationContractVersion::V1.contract_version());
        let explicit = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v1-explicit"),
                Some("v1-equivalent-intent"),
                Some(explicit_body),
            )
            .await;
        assert_eq!(status(&explicit), axum::http::StatusCode::ACCEPTED);
        assert_eq!(
            explicit
                .headers()
                .get("x-idempotent-replay")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        let replay = Harness::body_json(explicit).await;
        assert_eq!(replay["run_id"], original["run_id"]);
        assert_eq!(replay["job_id"], original["job_id"]);
        assert_eq!(harness.simulation_job_count(&harness.owner).await, 1);
        harness.teardown().await;
    }

    #[tokio::test]
    async fn simulation_v2_replay_uses_frozen_snapshot_and_keeps_version_namespace_closed() {
        let Some(mut harness) = Harness::new().await else {
            eprintln!("SKIP: DATABASE_URL not set");
            return;
        };
        harness.seed_simulation_backtest_rights().await;
        let strategy_id = "relative_momentum";
        let config_id = harness
            .seed_simulation_v2_strategy_config(&harness.owner, strategy_id)
            .await;
        harness.restart_api_with_owner_beta_price_input().await;
        harness.restart_api_with_max_jobs_per_owner(1).await;
        let path = format!("{BASE_PATH}/runs");
        let run_id = Uuid::new_v4();
        let input = Harness::simulation_v2_test_input(strategy_id, config_id, run_id, START, END);
        harness
            .state()
            .set_owner_beta_simulation_v2_test_input(input.clone());
        let mut request_body = body(config_id, START, END);
        request_body["contract_version"] = json!(SimulationContractVersion::V2.contract_version());
        let original = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v2-first"),
                Some("v2-frozen-intent"),
                Some(request_body.clone()),
            )
            .await;
        assert_eq!(status(&original), axum::http::StatusCode::ACCEPTED);
        let original = Harness::body_json(original).await;
        assert_eq!(original["run_id"], run_id.to_string());

        harness
            .state()
            .strategy_configs()
            .update(
                &harness.owner.actor(),
                config_id,
                json!({"lookback_months": 1, "top_n": 1}),
                true,
            )
            .await
            .expect("test mutates current config after durable enqueue");
        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_v2_test_input(input);
        let replay = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v2-durable-replay"),
                Some("v2-frozen-intent"),
                Some(request_body.clone()),
            )
            .await;
        assert_eq!(status(&replay), axum::http::StatusCode::ACCEPTED);
        assert_eq!(
            replay
                .headers()
                .get("x-idempotent-replay")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        let replay = Harness::body_json(replay).await;
        assert_eq!(replay["run_id"], original["run_id"]);
        assert_eq!(replay["job_id"], original["job_id"]);
        assert_eq!(harness.simulation_job_count(&harness.owner).await, 1);

        harness.restart_api_with_owner_beta_price_input().await;
        let mut changed_intent = request_body.clone();
        changed_intent["start_date"] = json!("2026-08-26");
        let mismatch = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v2-changed-intent"),
                Some("v2-frozen-intent"),
                Some(changed_intent),
            )
            .await;
        assert_eq!(status(&mismatch), axum::http::StatusCode::CONFLICT);
        assert_eq!(
            Harness::error_code(&Harness::body_json(mismatch).await),
            "IDEMPOTENCY_KEY_MISMATCH"
        );

        harness.restart_api_with_owner_beta_price_input().await;
        harness
            .state()
            .set_owner_beta_simulation_test_input(test_input(config_id, START, END));
        let mut v1_body = body(config_id, START, END);
        v1_body["contract_version"] = json!(SimulationContractVersion::V1.contract_version());
        let cross_version = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v2-cross-version"),
                Some("v2-frozen-intent"),
                Some(v1_body),
            )
            .await;
        assert_eq!(status(&cross_version), axum::http::StatusCode::CONFLICT);
        assert_eq!(
            Harness::error_code(&Harness::body_json(cross_version).await),
            "IDEMPOTENCY_KEY_MISMATCH"
        );
        harness.teardown().await;
    }

    #[tokio::test]
    async fn simulation_v2_capacity_cancel_and_closed_version_selector() {
        let Some(mut harness) = Harness::new().await else {
            eprintln!("SKIP: DATABASE_URL not set");
            return;
        };
        harness.seed_simulation_backtest_rights().await;
        let strategy_id = "buy_and_hold";
        let config_id = harness
            .seed_simulation_v2_strategy_config(&harness.owner, strategy_id)
            .await;
        harness.restart_api_with_owner_beta_price_input().await;
        harness.restart_api_with_max_jobs_per_owner(1).await;
        let path = format!("{BASE_PATH}/runs");
        let mut v2_body = body(config_id, START, END);
        v2_body["contract_version"] = json!(SimulationContractVersion::V2.contract_version());
        let input =
            Harness::simulation_v2_test_input(strategy_id, config_id, Uuid::new_v4(), START, END);
        harness
            .state()
            .set_owner_beta_simulation_v2_test_input(input);
        let first = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v2-capacity-first"),
                Some("v2-capacity-first"),
                Some(v2_body.clone()),
            )
            .await;
        assert_eq!(status(&first), axum::http::StatusCode::ACCEPTED);
        let first = Harness::body_json(first).await;

        harness
            .state()
            .set_owner_beta_simulation_v2_test_input(Harness::simulation_v2_test_input(
                strategy_id,
                config_id,
                Uuid::new_v4(),
                START,
                END,
            ));
        let capacity = harness
            .send(
                "POST",
                &path,
                Some(&harness.owner),
                true,
                Some("simulation-v2-capacity-second"),
                Some("v2-capacity-second"),
                Some(v2_body),
            )
            .await;
        assert_eq!(status(&capacity), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            Harness::error_code(&Harness::body_json(capacity).await),
            "BACKTEST_CAPACITY_EXCEEDED"
        );

        let canceled = harness
            .send(
                "POST",
                &format!(
                    "{BASE_PATH}/runs/{}/cancel",
                    first["run_id"].as_str().unwrap()
                ),
                Some(&harness.owner),
                true,
                Some("simulation-v2-cancel"),
                Some("v2-cancel-first"),
                Some(json!({})),
            )
            .await;
        assert_eq!(status(&canceled), axum::http::StatusCode::OK);
        assert_eq!(
            Harness::body_json(canceled).await["status"],
            "CANCEL_REQUESTED"
        );

        for invalid in [
            json!({"strategy_config_id": config_id, "contract_version": null}),
            json!({"strategy_config_id": config_id, "contract_version": "v9"}),
            json!({"strategy_config_id": config_id, "contract_version": 2}),
            json!({"strategy_config_id": config_id, "unexpected": true}),
        ] {
            let rejected = harness
                .send(
                    "POST",
                    &path,
                    Some(&harness.owner),
                    true,
                    Some("simulation-v2-invalid-version"),
                    Some("v2-invalid-version"),
                    Some(invalid),
                )
                .await;
            assert_eq!(status(&rejected), axum::http::StatusCode::BAD_REQUEST);
        }
        harness.teardown().await;
    }
}
