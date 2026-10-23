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
use serde_json::Value;
use uuid::Uuid;

fn baseline_defaults(strategy_id: &str, version: &str, published_schema: &Value) -> Option<Value> {
    selector::baseline::baseline_packages()
        .into_iter()
        .find(|package| {
            package.strategy_id == strategy_id
                && package.version.to_string() == version
                && package.parameter_schema == *published_schema
        })
        .map(|package| package.default_parameters)
}

pub async fn list(State(state): State<ApiState>, session: Session, headers: HeaderMap) -> Response {
    let actor = session.actor();
    let catalog = state.strategy_catalog();
    let mut items = Vec::new();
    match catalog.list(&actor).await {
        Ok(rows) => {
            for row in rows {
                let latest = match catalog.latest_version(&actor, &row.id).await {
                    Ok(version) => version,
                    Err(e) => {
                        return tenancy_response(e, &request_id(&headers), "RESOURCE_NOT_FOUND");
                    }
                };
                let (parameter_schema, default_parameters) = match latest.as_deref() {
                    Some(version) => {
                        let schema = match catalog.param_schema(&actor, &row.id, version).await {
                            Ok(schema) => schema,
                            Err(e) => {
                                return tenancy_response(
                                    e,
                                    &request_id(&headers),
                                    "RESOURCE_NOT_FOUND",
                                );
                            }
                        };
                        let defaults = baseline_defaults(&row.id, version, &schema);
                        (Some(schema), defaults)
                    }
                    None => (None, None),
                };
                items.push(StrategyDto {
                    id: row.id,
                    display_name: row.display_name,
                    description: row.description,
                    risk_description: row.risk_description,
                    state: row.state,
                    latest_version: latest,
                    parameter_schema,
                    default_parameters,
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
    let latest = match catalog.latest_version(&actor, &strategy_id).await {
        Ok(version) => version,
        Err(e) => return tenancy_response(e, &request_id(&headers), "RESOURCE_NOT_FOUND"),
    };
    let (parameter_schema, default_parameters) = match latest.as_deref() {
        Some(version) => {
            let schema = match catalog.param_schema(&actor, &strategy_id, version).await {
                Ok(schema) => schema,
                Err(e) => {
                    return tenancy_response(e, &request_id(&headers), "RESOURCE_NOT_FOUND");
                }
            };
            let defaults = baseline_defaults(&strategy_id, version, &schema);
            (Some(schema), defaults)
        }
        None => (None, None),
    };
    (
        StatusCode::OK,
        Json(StrategyDto {
            id: row.id,
            display_name: row.display_name,
            description: row.description,
            risk_description: row.risk_description,
            state: row.state,
            latest_version: latest,
            parameter_schema,
            default_parameters,
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
    if body.owner_beta_daily_enabled && !actor.is_owner() {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
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
            .create_with_daily(
                &actor,
                NewStrategyConfig {
                    strategy_id: strategy_id.clone(),
                    strategy_version: body.strategy_version.clone(),
                    config_json: body.config.clone(),
                    is_active: body.is_active,
                },
                body.owner_beta_daily_enabled,
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
                "owner_beta_daily_enabled":created.daily_enabled,
                "owner_beta_daily_epoch":created.daily_epoch,
            })),
            None,
        )
        .await;
        (StatusCode::CREATED, Json(StrategyConfigDto::from(created))).into_response()
    })
    .await
}

/// The actor's own saved strategy configurations.
///
/// Binding a Paper account needs a config id, so the branching UI has to be
/// able to enumerate what the actor may bind. RLS scopes the rows, so this
/// can never surface another member's configuration.
pub async fn list_configs(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let actor = session.actor();
    match state.strategy_configs().list(&actor).await {
        Ok(rows) => {
            let items: Vec<StrategyConfigDto> =
                rows.into_iter().map(StrategyConfigDto::from).collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &request_id(&headers), "RESOURCE_NOT_FOUND"),
    }
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
        Ok(row) => (StatusCode::OK, Json(StrategyConfigDto::from(row))).into_response(),
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

/// Update only this actor's configuration. Missing fields preserve stored values;
/// SQL owns epoch advancement and rejects stale role/Owner authority at commit.
pub async fn update_config(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(config_id): Path<String>,
    JsonBody(body): JsonBody<crate::http::dto::UpdateStrategyConfigBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = crate::http::session::require_csrf(&headers, &session.0) {
        return response;
    }
    let actor = session.actor();
    if body.owner_beta_daily_enabled == Some(true) && !actor.is_owner() {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    let id = match Uuid::parse_str(&config_id) {
        Ok(value) if !value.is_nil() => value,
        _ => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "config id must be a uuid",
                &rid,
                None,
            );
        }
    };
    if body.config.is_none() && body.is_active.is_none() && body.owner_beta_daily_enabled.is_none()
    {
        return code_error(
            "INVALID_PARAMETER",
            "at least one configuration field is required",
            &rid,
        );
    }
    let body_hash = crate::http::idempotency::body_hash(
        &serde_json::json!({"method":"PATCH","resource":"strategy_config","id":id,"body":body}),
    );
    idempotent(&state,&session,&headers,&body_hash,async {
        let old=match state.strategy_configs().get(&actor,id).await {
            Ok(row)=>row,Err(error)=>return tenancy_response(error,&rid,"RESOURCE_NOT_FOUND"),
        };
        if let Some(config)=&body.config {
            let schema=match state.strategy_catalog().param_schema(&actor,&old.strategy_id,&old.strategy_version).await {
                Ok(value)=>value,Err(error)=>return tenancy_response(error,&rid,"RESOURCE_NOT_FOUND"),
            };
            if let Err(message)=validate_params(&schema,config){
                return api_error(StatusCode::UNPROCESSABLE_ENTITY,"INVALID_STRATEGY_PARAMETER",message,&rid,None);
            }
        }
        let changed=match state.strategy_configs().update_with_daily(
            &actor,id,body.config.clone(),body.is_active,body.owner_beta_daily_enabled).await {
            Ok(row)=>row,Err(error)=>return tenancy_response(error,&rid,"RESOURCE_NOT_FOUND"),
        };
        audit(&state,&session,&headers,"strategy_config.update","strategy_config",&id.to_string(),
            Some(serde_json::json!({"is_active":old.is_active,"owner_beta_daily_enabled":old.daily_enabled,"owner_beta_daily_epoch":old.daily_epoch})),
            Some(serde_json::json!({"is_active":changed.is_active,"owner_beta_daily_enabled":changed.daily_enabled,"owner_beta_daily_epoch":changed.daily_epoch})),None).await;
        (StatusCode::OK,Json(StrategyConfigDto::from(changed))).into_response()
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_exposed_only_for_the_exact_embedded_schema() {
        let package = selector::baseline::baseline_packages()
            .into_iter()
            .find(|package| package.strategy_id == "buy_and_hold")
            .expect("baseline package");
        let version = package.version.to_string();
        assert_eq!(
            baseline_defaults(&package.strategy_id, &version, &package.parameter_schema),
            Some(package.default_parameters.clone())
        );

        let mut mismatched_schema = package.parameter_schema.clone();
        mismatched_schema["additionalProperties"] = serde_json::json!(true);
        assert_eq!(
            baseline_defaults(&package.strategy_id, &version, &mismatched_schema),
            None
        );
        assert_eq!(
            baseline_defaults(&package.strategy_id, "2.0.0", &package.parameter_schema),
            None
        );
    }
}
