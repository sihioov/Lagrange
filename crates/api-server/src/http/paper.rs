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
use domain::{Currency, Money};
use portfolio_model::cost::CostProfile;
use portfolio_model::error::PortfolioError;
use portfolio_model::paper_account::NewPaperAccount;
use uuid::Uuid;

/// Resolves the requested cost profile id to a real, versioned [`CostProfile`].
///
/// The match itself lives in `portfolio-model` beside `CostProfileId`, which
/// owns the serde names; this only supplies the route's default for an absent
/// field. Two hand-written copies of that match is how one profile ended up
/// with two spellings.
fn resolve_cost_profile(id: Option<&str>) -> Result<CostProfile, String> {
    CostProfile::resolve(id.unwrap_or("KRX_ETF_DEFAULT")).map_err(|e| e.to_string())
}

fn cost_profile_id_str(profile: &CostProfile) -> &'static str {
    profile.id_str()
}

fn portfolio_error_response(rid: &str, err: PortfolioError) -> Response {
    api_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "INVALID_PARAMETER",
        err.to_string(),
        rid,
        None,
    )
}

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
    let Ok(currency) = Currency::from_code(&body.currency) else {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNSUPPORTED_MARKET_CURRENCY",
            format!("currency {} is not a valid ISO 4217 code", body.currency),
            &rid,
            None,
        );
    };
    let initial_cash = match Money::parse(&body.initial_cash, currency) {
        Ok(m) => m,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_DECIMAL",
                "initial_cash must be a finite, non-negative fixed-point decimal string",
                &rid,
                None,
            );
        }
    };
    let cost_profile = match resolve_cost_profile(body.cost_profile_id.as_deref()) {
        Ok(p) => p,
        Err(detail) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                detail,
                &rid,
                None,
            );
        }
    };
    let spec = match NewPaperAccount::new(initial_cash, cost_profile.clone()) {
        Ok(s) => s,
        Err(e) => return portfolio_error_response(&rid, e),
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
        let created = match state
            .accounts()
            .create(
                &actor,
                NewAccount {
                    account_type: "PAPER".to_string(),
                    name: body.name.clone(),
                    currency: body.currency.clone(),
                    initial_cash: Some(spec.initial_cash.as_decimal_string()),
                    cost_profile_id: cost_profile_id_str(&cost_profile).to_string(),
                    cost_profile_version: cost_profile.version as i32,
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

/// The actor's own paper accounts.
///
/// Discovery has to be a route rather than a client-side guess: RLS is what
/// makes the list the actor's own, so two users hitting the same path can
/// never see each other's accounts. LIVE accounts are filtered out here —
/// this is the Paper surface, and the Owner-only Live routes are separate.
pub async fn list_accounts(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
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
    match state.accounts().list(&actor).await {
        Ok(rows) => {
            let items: Vec<AccountDto> = rows
                .into_iter()
                .filter(|a| a.account_type == "PAPER")
                .map(account_dto)
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
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
        let account = match state.accounts().get(&actor, id).await {
            Ok(a) => a,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        if account.account_type != "PAPER" {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "strategy binding applies to PAPER accounts only",
                &rid,
                None,
            );
        }
        let config = match state.strategy_configs().get(&actor, cfg_id).await {
            Ok(c) => c,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        // A retired strategy can never gain a new binding; existing
        // bindings/history for it are untouched.
        let strategy = match state
            .strategy_catalog()
            .get(&actor, &config.strategy_id)
            .await
        {
            Ok(s) => s,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
        if strategy.state == "Retired" {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_PARAMETER",
                format!("strategy {} is Retired and cannot be bound", strategy.id),
                &rid,
                None,
            );
        }
        // Branch-on-change (FR-PAPER-004): closes any existing active
        // binding and opens a new one atomically, so execution history
        // never mixes strategy versions and the account is never observed
        // with zero or two active bindings.
        let binding = match state
            .accounts()
            .bind_strategy(
                &actor,
                id,
                cfg_id,
                &config.strategy_id,
                &config.strategy_version,
            )
            .await
        {
            Ok(b) => b,
            Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        };
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
                "strategy_id": binding.strategy_id,
                "strategy_version": binding.strategy_version,
                "bound_at": binding.bound_at,
            })),
            None,
        )
        .await;
        (
            StatusCode::OK,
            Json(BindStrategyDto {
                account_id: id.to_string(),
                strategy_config_id: cfg_id.to_string(),
                strategy_id: binding.strategy_id,
                strategy_version: binding.strategy_version,
                bound_at: binding.bound_at,
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
                    cash_reconciled: p.cash_reconciled,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

/// Paper results are simulated. The UI renders this verbatim so a reader
/// never mistakes a Paper curve for a promise (design §10.2).
const PAPER_DISCLAIMER: &str = "Simulated results from a paper account. Fills are modeled, not \
     executed in a real market, and past simulated performance is not a guarantee of future \
     returns.";

/// Day-over-day performance derived from the ledger's own daily equity.
///
/// The returns are computed HERE on read rather than stored: `daily_equity`
/// is the single source of truth the runner writes from the shared ledger,
/// and a cached return column could only ever drift from it.
pub async fn performance(
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
    let rows = match state
        .paper()
        .equity(&actor, id, cursor.as_ref(), limit)
        .await
    {
        Ok((rows, _)) => rows,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };

    let mut points: Vec<crate::http::dto::PerformancePointDto> = Vec::with_capacity(rows.len());
    let mut previous: Option<f64> = None;
    for row in rows {
        let current = row.equity.parse::<f64>().ok();
        let return_pct = match (previous, current) {
            (Some(prev), Some(cur)) if prev != 0.0 => Some(format!("{:.6}", cur / prev - 1.0)),
            _ => None,
        };
        previous = current.or(previous);
        points.push(crate::http::dto::PerformancePointDto {
            trading_date: row.trading_date,
            equity: row.equity,
            cash: row.cash,
            positions_value: row.positions_value,
            currency: row.currency,
            return_pct,
            cash_reconciled: row.cash_reconciled,
        });
    }
    (
        StatusCode::OK,
        Json(crate::http::dto::PerformanceDto {
            account_id: id.to_string(),
            points,
            disclaimer: PAPER_DISCLAIMER,
        }),
    )
        .into_response()
}

/// The account's full lineage: its immutable strategy-binding history
/// (Todo 30's branching record) and the targets each session queued and
/// executed (Todo 31).
pub async fn lineage(
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
    let bindings = match state.accounts().binding_history(&actor, id).await {
        Ok(b) => b,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    let targets = match state.pending_targets().history(&actor, id).await {
        Ok(t) => t,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    (
        StatusCode::OK,
        Json(crate::http::dto::LineageDto {
            account_id: id.to_string(),
            bindings: bindings
                .into_iter()
                .map(|b| crate::http::dto::BindingHistoryDto {
                    strategy_config_id: b.strategy_config_id.to_string(),
                    strategy_id: b.strategy_id,
                    strategy_version: b.strategy_version,
                    bound_at: b.bound_at,
                    unbound_at: b.unbound_at,
                    active: b.unbound_at.is_none(),
                })
                .collect(),
            targets: targets
                .into_iter()
                .map(|t| crate::http::dto::TargetLineageDto {
                    id: t.id.to_string(),
                    computed_on: t.computed_on,
                    effective_date: t.effective_date,
                    status: t.status,
                    executed_at: t.executed_at,
                })
                .collect(),
        }),
    )
        .into_response()
}

/// The backtest-vs-Paper signal parity report for one session.
///
/// Computed on read from the two sides' persisted signals — never stored,
/// so it can never go stale against the lineage it describes. A divergence
/// or an incomparable lineage is WARNING-grade (design §15.3); the caller
/// sees `warrants_alert` rather than having to re-derive the grade.
pub async fn parity(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(account_id): Path<String>,
    Query(params): Query<ParityParams>,
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
    let Some(as_of) = params.as_of.clone() else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "as_of is required (the session both sides must share)",
            &rid,
            None,
        );
    };
    if crate::http::validation::parse_date(&as_of).is_none() {
        return code_error("INVALID_DATE", "as_of must be a valid calendar date", &rid);
    }

    let report = match state.parity_report(&actor, id, &as_of).await {
        Ok(r) => r,
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    };
    (
        StatusCode::OK,
        Json(crate::http::dto::ParityDto {
            account_id: id.to_string(),
            as_of,
            status: report.status.as_str().to_owned(),
            lineage: serde_json::to_value(&report.lineage).unwrap_or(serde_json::Value::Null),
            divergences: serde_json::to_value(&report.divergences)
                .unwrap_or(serde_json::Value::Null),
            warrants_alert: report.warrants_alert(),
            fill_model_difference: report.fill_model_difference,
        }),
    )
        .into_response()
}

/// Query parameters of the parity route.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ParityParams {
    pub as_of: Option<String>,
}

fn account_dto(a: crate::repos::accounts::AccountRow) -> AccountDto {
    AccountDto {
        id: a.id.to_string(),
        account_type: a.account_type,
        name: a.name,
        currency: a.currency,
        status: a.status,
        initial_cash: a.initial_cash,
        cost_profile_id: a.cost_profile_id,
        cost_profile_version: a.cost_profile_version,
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
