//! Strategy catalog + user strategy-config routes.

use crate::http::dto::{NewStrategyConfigBody, PageDto, StrategyConfigDto, StrategyDto};
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit, idempotent, tenancy_response};
use crate::repos::strategy_configs::NewStrategyConfig;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

pub async fn list(State(state): State<ApiState>, session: Session, headers: HeaderMap) -> Response {
    let actor = session.actor();
    let catalog = state.strategy_catalog();
    let mut items = Vec::new();
    match catalog.list(&actor).await {
        Ok(rows) => {
            for row in rows {
                let latest = catalog.latest_version(&actor, &row.id).await.ok().flatten();
                items.push(StrategyDto {
                    id: row.id,
                    display_name: row.display_name,
                    description: row.description,
                    risk_description: row.risk_description,
                    state: row.state,
                    latest_version: latest,
                });
            }
        }
        Err(e) => return tenancy_response(e, &request_id(&headers), "RESOURCE_NOT_FOUND"),
    }
    (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
}

pub async fn get(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(strategy_id): Path<String>,
) -> Response {
    let actor = session.actor();
    let catalog = state.strategy_catalog();
    let row = match catalog.get(&actor, &strategy_id).await {
        Ok(r) => r,
        Err(e) => return tenancy_response(e, &request_id(&headers), "RESOURCE_NOT_FOUND"),
    };
    let latest = catalog
        .latest_version(&actor, &strategy_id)
        .await
        .ok()
        .flatten();
    (
        StatusCode::OK,
        Json(StrategyDto {
            id: row.id,
            display_name: row.display_name,
            description: row.description,
            risk_description: row.risk_description,
            state: row.state,
            latest_version: latest,
        }),
    )
        .into_response()
}

pub async fn create_config(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(strategy_id): Path<String>,
    JsonBody(body): JsonBody<NewStrategyConfigBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let body_value = serde_json::to_value(&body).unwrap_or_default();
    let body_hash = crate::http::idempotency::body_hash(&body_value);
    idempotent(&state, &session, &headers, &body_hash, async {
        let catalog = state.strategy_catalog();
        // The strategy must exist and be non-retired for new member configs.
        let strategy = match catalog.get(&actor, &strategy_id).await {
            Ok(s) => s,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        if strategy.state == "Retired" {
            return code_error(
                "INVALID_PARAMETER",
                format!("strategy {strategy_id} is retired"),
                &rid,
            );
        }
        // The version must be published and its parameter schema fetched.
        if catalog
            .get_version(&actor, &strategy_id, &body.strategy_version)
            .await
            .is_err()
        {
            return code_error(
                "INVALID_PARAMETER",
                format!(
                    "unknown strategy version {} for {strategy_id}",
                    body.strategy_version
                ),
                &rid,
            );
        }
        let schema = match catalog
            .param_schema(&actor, &strategy_id, &body.strategy_version)
            .await
        {
            Ok(s) => s,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        // Schema-bound parameters only; arbitrary code is never accepted.
        if let Err(message) = validate_params(&schema, &body.config) {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_STRATEGY_PARAMETER",
                message,
                &rid,
                None,
            );
        }
        let created = match state
            .strategy_configs()
            .create(
                &actor,
                NewStrategyConfig {
                    strategy_id: strategy_id.clone(),
                    strategy_version: body.strategy_version.clone(),
                    config_json: body.config.clone(),
                    is_active: body.is_active,
                },
            )
            .await
        {
            Ok(c) => c,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        audit(
            &state,
            &session,
            &headers,
            "strategy_config.create",
            "strategy_config",
            &created.id.to_string(),
            None,
            Some(serde_json::json!({
                "strategy_id": created.strategy_id,
                "strategy_version": created.strategy_version,
                "is_active": created.is_active,
            })),
            None,
        )
        .await;
        (
            StatusCode::CREATED,
            Json(StrategyConfigDto {
                id: created.id.to_string(),
                strategy_id: created.strategy_id,
                strategy_version: created.strategy_version,
                config: created.config_json,
                is_active: created.is_active,
                created_at: created.created_at,
                updated_at: created.updated_at,
            }),
        )
            .into_response()
    })
    .await
}

pub async fn get_config(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(config_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match Uuid::parse_str(&config_id) {
        Ok(i) => i,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "config id must be a uuid",
                &rid,
                None,
            );
        }
    };
    match state.strategy_configs().get(&actor, id).await {
        Ok(row) => (
            StatusCode::OK,
            Json(StrategyConfigDto {
                id: row.id.to_string(),
                strategy_id: row.strategy_id,
                strategy_version: row.strategy_version,
                config: row.config_json,
                is_active: row.is_active,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }),
        )
            .into_response(),
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

fn validate_params(schema: &serde_json::Value, config: &serde_json::Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| format!("published schema is invalid: {e}"))?;
    let result = validator.validate(config);
    match result {
        Ok(()) => Ok(()),
        Err(errors) => {
            let first = errors.to_string();
            Err(format!(
                "strategy parameters violate the published schema: {first}"
            ))
        }
    }
}
