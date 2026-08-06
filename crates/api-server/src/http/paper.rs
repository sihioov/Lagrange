//! Paper account routes: create (PAPER only), get, bind-strategy, and the
//! ledger views (orders/positions/equity). Writes to the ledger are the
//! Paper scheduler's (Todos 30-32); the API validates, audits, and reads.

use crate::http::dto::{
    AccountDto, BindStrategyBody, BindStrategyDto, EquityPointDto, NewAccountBody, OrderDto,
    PageDto, PositionDto,
};
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{PageParams, Session};
use crate::http::state::ApiState;
use crate::http::validation::is_supported_currency;
use crate::http::{JsonBody, audit, idempotent, tenancy_response};
use crate::repos::accounts::NewAccount;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

pub async fn create_account(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<NewAccountBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    if body.name.trim().is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "name must not be empty",
            &rid,
            None,
        );
    }
    if !is_supported_currency(&body.currency) {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNSUPPORTED_MARKET_CURRENCY",
            format!("currency {} is not supported (KRW first)", body.currency),
            &rid,
            None,
        );
    }
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    let body_value = serde_json::to_value(&body).unwrap_or_default();
    let body_hash = crate::http::idempotency::body_hash(&body_value);
    idempotent(&state, &session, &headers, &body_hash, async {
        let created = match state
            .accounts()
            .create(
                &actor,
                NewAccount {
                    account_type: "PAPER".to_string(),
                    name: body.name.clone(),
                    currency: body.currency.clone(),
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
            "paper.account.create",
            "account",
            &created.id.to_string(),
            None,
            Some(serde_json::json!({
                "name": created.name,
                "currency": created.currency,
            })),
            None,
        )
        .await;
        (StatusCode::CREATED, Json(account_dto(created))).into_response()
    })
    .await
}

pub async fn get_account(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "account id", &account_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    match state.accounts().get(&actor, id).await {
        Ok(row) => (StatusCode::OK, Json(account_dto(row))).into_response(),
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn bind_strategy(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    JsonBody(body): JsonBody<BindStrategyBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let actor = session.actor();
    let id = match parse_uuid(&rid, "account id", &account_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
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
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    let body_value = serde_json::to_value(&body).unwrap_or_default();
    let body_hash = crate::http::idempotency::body_hash(&body_value);
    idempotent(&state, &session, &headers, &body_hash, async {
        // Ownership of BOTH the account and the config (404 on foreign).
        if let Err(e) = state.accounts().get(&actor, id).await {
            return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
        }
        if let Err(e) = state.strategy_configs().get(&actor, cfg_id).await {
            return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
        }
        // The durable binding is applied by the Paper scheduler (T30) from
        // the audited event; this route validates and records it.
        let bound_at = chrono::Utc::now();
        audit(
            &state,
            &session,
            &headers,
            "paper.account.bind_strategy",
            "account",
            &id.to_string(),
            None,
            Some(serde_json::json!({
                "strategy_config_id": cfg_id,
                "bound_at": bound_at,
            })),
            None,
        )
        .await;
        (
            StatusCode::OK,
            Json(BindStrategyDto {
                account_id: id.to_string(),
                strategy_config_id: cfg_id.to_string(),
                bound_at,
            }),
        )
            .into_response()
    })
    .await
}

pub async fn orders(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "account id", &account_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if let Err(e) = state.accounts().get(&actor, id).await {
        return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
    }
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .paper()
        .orders(&actor, id, cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let items = rows
                .into_iter()
                .map(|o| OrderDto {
                    id: o.id.to_string(),
                    order_ref: o.order_ref,
                    instrument_id: o.instrument_id,
                    side: o.side,
                    quantity: o.quantity,
                    price: o.price,
                    status: o.status,
                    submitted_at: o.submitted_at,
                    created_at: o.created_at,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn positions(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(account_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "account id", &account_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if let Err(e) = state.accounts().get(&actor, id).await {
        return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
    }
    match state.paper().positions(&actor, id).await {
        Ok(rows) => {
            let items = rows
                .into_iter()
                .map(|p| PositionDto {
                    instrument_id: p.instrument_id,
                    quantity: p.quantity,
                    avg_price: p.avg_price,
                    updated_at: p.updated_at,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn equity(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    let id = match parse_uuid(&rid, "account id", &account_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if let Err(r) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
    {
        return r;
    }
    if let Err(e) = state.accounts().get(&actor, id).await {
        return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND");
    }
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .paper()
        .equity(&actor, id, cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let items = rows
                .into_iter()
                .map(|p| EquityPointDto {
                    trading_date: p.trading_date,
                    equity: p.equity,
                    cash: p.cash,
                    positions_value: p.positions_value,
                    currency: p.currency,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

fn account_dto(a: crate::repos::accounts::AccountRow) -> AccountDto {
    AccountDto {
        id: a.id.to_string(),
        account_type: a.account_type,
        name: a.name,
        currency: a.currency,
        status: a.status,
        created_at: a.created_at,
        updated_at: a.updated_at,
    }
}

#[allow(clippy::result_large_err)]
fn parse_uuid(rid: &str, what: &str, raw: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(raw).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            format!("{what} must be a uuid"),
            rid,
            None,
        )
    })
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
