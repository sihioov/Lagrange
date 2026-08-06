//! Recommendation routes: create (entitlement-gated, queued), get, list,
//! latest. The research worker settles run status + items; the API reads.

use crate::http::dto::{
    PageDto, RecommendationItemDto, RecommendationRunBody, RecommendationRunDto,
};
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{PageParams, Session};
use crate::http::state::ApiState;
use crate::http::validation::parse_date;
use crate::http::{JsonBody, audit, idempotent, tenancy_response};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

pub async fn create_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<RecommendationRunBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let cfg_id = match Uuid::parse_str(&body.strategy_config_id) {
        Ok(i) => i,
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
    let Some(as_of) = parse_date(&body.as_of) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            "as_of must be a valid YYYY-MM-DD calendar date",
            &rid,
            None,
        );
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &body.as_of,
    )
    .await
    {
        return r;
    }
    let body_value = serde_json::to_value(&body).unwrap_or_default();
    let body_hash = crate::http::idempotency::body_hash(&body_value);
    let key = crate::http::idempotency::key_from(&headers);
    idempotent(&state, &session, &headers, &body_hash, async {
        // The strategy config must be the actor's own (ownership check).
        if let Err(e) = state.strategy_configs().get(&actor, cfg_id).await {
            return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
        }
        let run = match state
            .recommendations()
            .create_run(&actor, cfg_id, as_of)
            .await
        {
            Ok(r) => r,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        // Enqueue the research job (queue-level idempotency key rides along).
        let queue = match state.queue_for(&actor).await {
            Ok(q) => q,
            Err(_) => return code_error("INTERNAL", "queue unavailable", &rid),
        };
        let job = match queue
            .submit(job_queue::SubmitJob {
                owner_user_id: crate::actor_tx::actor_uuid(&actor).unwrap_or_default(),
                job_type: "recommendation".to_string(),
                payload: serde_json::json!({
                    "run_id": run.id,
                    "strategy_config_id": cfg_id,
                    "as_of": body.as_of,
                }),
                priority: 10,
                idempotency_key: key,
                max_attempts: 3,
                available_at: None,
            })
            .await
        {
            Ok(j) => j,
            Err(e) => return code_error("INTERNAL", format!("enqueue failed: {e}"), &rid),
        };
        audit(
            &state,
            &session,
            &headers,
            "recommendation.run.create",
            "recommendation_run",
            &run.id.to_string(),
            None,
            Some(serde_json::json!({
                "strategy_config_id": cfg_id,
                "as_of": body.as_of,
                "job_id": job.id,
            })),
            None,
        )
        .await;
        (
            StatusCode::CREATED,
            Json(RecommendationRunDto {
                id: run.id.to_string(),
                strategy_config_id: Some(cfg_id.to_string()),
                as_of,
                status: run.status,
                summary: run.summary_json,
                created_at: run.created_at,
                job_id: Some(job.id.to_string()),
                items: None,
            }),
        )
            .into_response()
    })
    .await
}

pub async fn get_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match Uuid::parse_str(&run_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "run id must be a uuid",
                &rid,
                None,
            );
        }
    };
    let run = match state.recommendations().get_run(&actor, id).await {
        Ok(r) => r,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &run.as_of.format("%Y-%m-%d").to_string(),
    )
    .await
    {
        return r;
    }
    let items = match state.recommendations().items(&actor, id).await {
        Ok(i) => i.into_iter().map(item_dto).collect(),
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    (
        StatusCode::OK,
        Json(RecommendationRunDto {
            id: run.id.to_string(),
            strategy_config_id: run.strategy_config_id.map(|c| c.to_string()),
            as_of: run.as_of,
            status: run.status,
            summary: run.summary_json,
            created_at: run.created_at,
            job_id: None,
            items: Some(items),
        }),
    )
        .into_response()
}

pub async fn list_runs(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .recommendations()
        .list_runs(&actor, cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = encode_cursor(&state, next);
            let items = rows
                .into_iter()
                .map(|r| RecommendationRunDto {
                    id: r.id.to_string(),
                    strategy_config_id: r.strategy_config_id.map(|c| c.to_string()),
                    as_of: r.as_of,
                    status: r.status,
                    summary: r.summary_json,
                    created_at: r.created_at,
                    job_id: None,
                    items: None,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn latest(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<LatestParams>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let cfg_id = match params.strategy_config_id {
        Some(raw) => match Uuid::parse_str(&raw) {
            Ok(i) => Some(i),
            Err(_) => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "INVALID_PARAMETER",
                    "strategy_config_id must be a uuid",
                    &rid,
                    None,
                );
            }
        },
        None => None,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    let run = match state.recommendations().latest_run(&actor, cfg_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return code_error("RESOURCE_NOT_FOUND", "no recommendation run yet", &rid),
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    let items = match state.recommendations().items(&actor, run.id).await {
        Ok(i) => i.into_iter().map(item_dto).collect(),
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "run": RecommendationRunDto {
                id: run.id.to_string(),
                strategy_config_id: run.strategy_config_id.map(|c| c.to_string()),
                as_of: run.as_of,
                status: run.status,
                summary: run.summary_json,
                created_at: run.created_at,
                job_id: None,
                items: Some(items),
            }
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct LatestParams {
    pub strategy_config_id: Option<String>,
}

fn item_dto(i: crate::repos::recommendations::RecommendationItemRow) -> RecommendationItemDto {
    RecommendationItemDto {
        instrument_id: i.instrument_id,
        rank: i.rank,
        target_weight: i.target_weight,
        excluded: i.excluded,
        exclusion_reason: i.exclusion_reason,
        reason_codes: i
            .reason_codes
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        factors: i.factors_json,
    }
}

#[allow(clippy::result_large_err)]
fn decode_cursor(
    state: &ApiState,
    rid: &str,
    raw: Option<&str>,
) -> Result<Option<Cursor>, Response> {
    match raw {
        None => Ok(None),
        Some(r) => match Cursor::decode(r, &state.cfg.cursor_secret) {
            Ok(c) => Ok(Some(c)),
            Err(_) => Err(code_error(
                "INVALID_CURSOR",
                "pagination cursor is invalid",
                rid,
            )),
        },
    }
}

fn encode_cursor(state: &ApiState, next: Option<Cursor>) -> Option<String> {
    next.map(|c| c.encode(&state.cfg.cursor_secret))
}
