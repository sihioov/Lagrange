//! Dedicated metadata read. It never approves files or starts a daily run.
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::Session;
use crate::http::state::{ApiState, OwnerBetaAccessMode};
use crate::http::tenancy_response;
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyStateQuery {
    strategy_config_id: Uuid,
}
pub async fn get_daily_state(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    query: Result<Query<DailyStateQuery>, QueryRejection>,
) -> Response {
    let rid = request_id(&headers);
    if state.cfg.owner_beta_access != OwnerBetaAccessMode::OwnerOnly || !session.actor().is_owner()
    {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    let config = match query {
        Ok(Query(value)) if !value.strategy_config_id.is_nil() => value.strategy_config_id,
        _ => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "strategy_config_id must be a uuid",
                &rid,
                None,
            );
        }
    };
    if let Err(response) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return response;
    }
    match state
        .owner_beta_recommendations()
        .read_daily_state(&session.actor(), config, state.cfg.owner_beta_daily)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    }
}
