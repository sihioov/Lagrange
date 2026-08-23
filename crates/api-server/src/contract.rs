//! The versioned `/api/v1` contract: stable error codes and the route
//! inventory that the router mounts and the OpenAPI document declares.
//!
//! The route inventory is the single source of truth shared by:
//! - the router assembly ([`crate::http::api_router`]),
//! - the OpenAPI drift test (`tests/openapi_contract.rs`), which asserts the
//!   authored `apps/api-server/openapi.json` matches this table exactly,
//! - the npm `openapi:check` gate (same metadata contract, Node side).
//!
//! Convention rules (design §12.1): `/api/v1` prefix, JSON bodies, cursor
//! pagination, `X-Request-Id` correlation in/out, the typed error envelope
//! `{error: {code, message, request_id, details?}}`, and idempotency keys on
//! every mutating route.

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

/// The finite candidate universe vocabulary exposed by the API.
///
/// This is deliberately an enum rather than an arbitrary string: accepting a
/// value here is part of the public contract, and unknown values must fail
/// closed before a repository query can accidentally select another feed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UniverseKey {
    #[default]
    Kospi200,
    Kosdaq150,
}

impl UniverseKey {
    pub const ALL: [Self; 2] = [Self::Kospi200, Self::Kosdaq150];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kospi200 => "kospi200",
            Self::Kosdaq150 => "kosdaq150",
        }
    }

    pub const fn sort_order(self) -> i32 {
        match self {
            Self::Kospi200 => 10,
            Self::Kosdaq150 => 20,
        }
    }

    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "kospi200" => Ok(Self::Kospi200),
            "kosdaq150" => Ok(Self::Kosdaq150),
            _ => Err("unrecognized candidate universe"),
        }
    }
}

/// Whether a route is part of the current product surface or a Phase 3
/// (Owner-only KIS Live) surface that must never be exposed to Members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Current,
    Phase3,
}

/// One stable error code of the contract.
#[derive(Debug, Clone, Copy)]
pub struct ErrorCodeDef {
    pub code: &'static str,
    pub status: StatusCode,
    pub description: &'static str,
}

impl ErrorCodeDef {
    const fn new(code: &'static str, status: StatusCode, description: &'static str) -> Self {
        Self {
            code,
            status,
            description,
        }
    }
}

/// The stable error-code table (OpenAPI `ErrorCode` enum + HTTP mapping).
/// Codes are unique; the plan acceptance names blocked dataset/entitlement,
/// duplicate key, invalid parameter, capacity, result-integrity, and
/// ownership responses explicitly.
pub const ERROR_CODES: &[ErrorCodeDef] = &[
    ErrorCodeDef::new(
        "SESSION_UNKNOWN",
        StatusCode::UNAUTHORIZED,
        "no valid session",
    ),
    ErrorCodeDef::new(
        "SESSION_EXPIRED",
        StatusCode::UNAUTHORIZED,
        "session expired",
    ),
    ErrorCodeDef::new(
        "FORBIDDEN",
        StatusCode::FORBIDDEN,
        "actor not permitted for this resource",
    ),
    ErrorCodeDef::new(
        "DATA_ENTITLEMENT_REQUIRED",
        StatusCode::FORBIDDEN,
        "an ACTIVE KRX data entitlement is required for this surface",
    ),
    ErrorCodeDef::new(
        "OWNER_ONLY_DEVELOPMENT_PATH",
        StatusCode::FORBIDDEN,
        "Owner-only development path",
    ),
    ErrorCodeDef::new(
        "CSRF_DENIED",
        StatusCode::FORBIDDEN,
        "missing or invalid CSRF token",
    ),
    ErrorCodeDef::new(
        "STEP_UP_NOT_OWNER",
        StatusCode::FORBIDDEN,
        "step-up requires the Owner role",
    ),
    ErrorCodeDef::new(
        "STEP_UP_MFA_REQUIRED",
        StatusCode::FORBIDDEN,
        "step-up requires an MFA authentication method",
    ),
    ErrorCodeDef::new(
        "STEP_UP_AUTH_TIME_ABSENT",
        StatusCode::FORBIDDEN,
        "step-up requires an authentication timestamp",
    ),
    ErrorCodeDef::new(
        "STEP_UP_AUTH_TIME_STALE",
        StatusCode::FORBIDDEN,
        "authentication is too old - re-authenticate",
    ),
    ErrorCodeDef::new(
        "RESOURCE_NOT_FOUND",
        StatusCode::NOT_FOUND,
        "resource does not exist or is not owned",
    ),
    ErrorCodeDef::new(
        "INVALID_PARAMETER",
        StatusCode::BAD_REQUEST,
        "request parameter is invalid",
    ),
    ErrorCodeDef::new(
        "INVALID_DATE",
        StatusCode::BAD_REQUEST,
        "date is not a valid calendar date",
    ),
    ErrorCodeDef::new(
        "INVALID_DECIMAL",
        StatusCode::BAD_REQUEST,
        "decimal string is malformed or non-finite",
    ),
    ErrorCodeDef::new(
        "INVALID_CURSOR",
        StatusCode::BAD_REQUEST,
        "pagination cursor is invalid",
    ),
    ErrorCodeDef::new(
        "IDEMPOTENCY_KEY_REQUIRED",
        StatusCode::BAD_REQUEST,
        "mutating routes require an Idempotency-Key header",
    ),
    ErrorCodeDef::new(
        "IDEMPOTENCY_KEY_MISMATCH",
        StatusCode::CONFLICT,
        "same Idempotency-Key with a different request body",
    ),
    ErrorCodeDef::new(
        "DUPLICATE_RESOURCE",
        StatusCode::CONFLICT,
        "a resource with the same identity already exists",
    ),
    ErrorCodeDef::new(
        "PAYLOAD_TOO_LARGE",
        StatusCode::PAYLOAD_TOO_LARGE,
        "request body exceeds the size limit",
    ),
    ErrorCodeDef::new(
        "DATASET_BLOCKED",
        StatusCode::UNPROCESSABLE_ENTITY,
        "dataset is quality-blocked; a NEW dataset version is required",
    ),
    ErrorCodeDef::new(
        "DATA_STALE",
        StatusCode::UNPROCESSABLE_ENTITY,
        "dataset is stale for the requested as-of",
    ),
    ErrorCodeDef::new(
        "INVALID_STRATEGY_PARAMETER",
        StatusCode::UNPROCESSABLE_ENTITY,
        "strategy parameters violate the published schema",
    ),
    ErrorCodeDef::new(
        "UNSUPPORTED_MARKET_CURRENCY",
        StatusCode::UNPROCESSABLE_ENTITY,
        "market/currency combination is not supported (KRW first)",
    ),
    ErrorCodeDef::new(
        "BACKTEST_CAPACITY_EXCEEDED",
        StatusCode::TOO_MANY_REQUESTS,
        "per-owner queued job capacity exceeded",
    ),
    ErrorCodeDef::new(
        "ROBUSTNESS_CAPACITY_EXCEEDED",
        StatusCode::TOO_MANY_REQUESTS,
        "robustness fan-out would exceed per-owner queued job capacity",
    ),
    ErrorCodeDef::new(
        "RECOMMENDATION_CAPACITY_EXCEEDED",
        StatusCode::TOO_MANY_REQUESTS,
        "per-owner queued recommendation capacity exceeded",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_CAPACITY_EXCEEDED",
        StatusCode::TOO_MANY_REQUESTS,
        "per-owner queued Paper preview capacity exceeded",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_BINDING_REQUIRED",
        StatusCode::CONFLICT,
        "an active matching Paper binding is required",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_NOT_READY",
        StatusCode::CONFLICT,
        "the recommendation or preview is not ready",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_DATA_BLOCKED",
        StatusCode::UNPROCESSABLE_ENTITY,
        "the recommendation dataset or calendar is blocked",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_ENTITLEMENT_REQUIRED",
        StatusCode::FORBIDDEN,
        "an active recommendation entitlement is required",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_STALE",
        StatusCode::CONFLICT,
        "the preview no longer matches current Paper inputs",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_FAILED",
        StatusCode::UNPROCESSABLE_ENTITY,
        "the preview calculation failed",
    ),
    ErrorCodeDef::new(
        "REBALANCE_PREVIEW_CONFLICT",
        StatusCode::CONFLICT,
        "the preview conflicts with an existing Paper target",
    ),
    ErrorCodeDef::new(
        "RESULT_INTEGRITY_FAILED",
        StatusCode::UNPROCESSABLE_ENTITY,
        "result artifacts failed integrity verification",
    ),
    ErrorCodeDef::new(
        "LIVE_RECONCILIATION_REQUIRED",
        StatusCode::CONFLICT,
        "Live requires reconciliation before new orders",
    ),
    ErrorCodeDef::new(
        "LIVE_KILL_SWITCH_ENGAGED",
        StatusCode::CONFLICT,
        "the Live kill switch is engaged; no node may start",
    ),
    ErrorCodeDef::new(
        "LIVE_CONNECTION_NOT_CONFIGURED",
        StatusCode::CONFLICT,
        "no Live broker connection is configured for this account",
    ),
    ErrorCodeDef::new(
        "RISK_LIMIT_EXCEEDED",
        StatusCode::UNPROCESSABLE_ENTITY,
        "order risk limit violated",
    ),
    ErrorCodeDef::new(
        "ORDER_STATE_UNKNOWN",
        StatusCode::CONFLICT,
        "order submission outcome is unknown; reconcile before retry",
    ),
    ErrorCodeDef::new(
        "NOT_IMPLEMENTED",
        StatusCode::NOT_IMPLEMENTED,
        "the operation is not available in this phase",
    ),
    ErrorCodeDef::new(
        "INTERNAL",
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error",
    ),
];

/// Look up the HTTP status for a stable code (404 default for unknown).
pub fn status_for_code(code: &str) -> StatusCode {
    ERROR_CODES
        .iter()
        .find(|c| c.code == code)
        .map(|c| c.status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// One mounted route with its full contract semantics.
#[derive(Debug, Clone, Copy)]
pub struct RouteSpec {
    pub method: &'static str,
    pub path: &'static str,
    pub phase: Phase,
    /// Mutating routes require CSRF + audit + (usually) an Idempotency-Key.
    pub mutating: bool,
    /// Requires the `Idempotency-Key` header and stores key->result.
    pub idempotent: bool,
    /// Idempotent by nature (logout/csrf rotation): no key required.
    pub naturally_idempotent: bool,
    pub owner_only: bool,
    /// The KR-derived surface this route serves (fail-closed gate), if any.
    pub entitlement_use: Option<&'static str>,
    pub audit: bool,
}

/// The temporary owner-beta boundary is conditional deployment policy, not
/// an unconditional property of the normal API contract.  Keep this list
/// adjacent to the route inventory so tests can prove every matching route is
/// covered without incorrectly advertising the routes as always Owner-only.
pub const OWNER_BETA_PRODUCT_PREFIXES: &[&str] = &[
    "/api/v1/recommendations",
    "/api/v1/backtests",
    "/api/v1/paper",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerBetaProduct {
    Recommendations,
    Backtests,
    Paper,
}

pub(crate) fn owner_beta_product(path: &str) -> Option<OwnerBetaProduct> {
    let groups = [
        (
            OWNER_BETA_PRODUCT_PREFIXES[0],
            OwnerBetaProduct::Recommendations,
        ),
        (OWNER_BETA_PRODUCT_PREFIXES[1], OwnerBetaProduct::Backtests),
        (OWNER_BETA_PRODUCT_PREFIXES[2], OwnerBetaProduct::Paper),
    ];
    groups.into_iter().find_map(|(prefix, product)| {
        (path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/')))
        .then_some(product)
    })
}

pub fn owner_beta_applies(path: &str) -> bool {
    owner_beta_product(path).is_some()
}

#[allow(clippy::too_many_arguments)]
const fn route(
    method: &'static str,
    path: &'static str,
    phase: Phase,
    mutating: bool,
    idempotent: bool,
    naturally_idempotent: bool,
    owner_only: bool,
    entitlement_use: Option<&'static str>,
    audit: bool,
) -> RouteSpec {
    RouteSpec {
        method,
        path,
        phase,
        mutating,
        idempotent,
        naturally_idempotent,
        owner_only,
        entitlement_use,
        audit,
    }
}

/// Every route the router mounts, with its contract semantics. This table is
/// mirrored 1:1 by `apps/api-server/openapi.json` (asserted by tests).
pub const CONTRACT_ROUTES: &[RouteSpec] = &[
    // --- auth / session ---------------------------------------------------
    route(
        "GET",
        "/api/v1/auth/session",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "POST",
        "/api/v1/auth/logout",
        Phase::Current,
        true,
        false,
        true,
        false,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/auth/csrf",
        Phase::Current,
        true,
        false,
        true,
        false,
        None,
        false,
    ),
    route(
        "GET",
        "/api/v1/auth/step-up-check",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        true,
    ),
    // --- strategies ---------------------------------------------------------
    route(
        "GET",
        "/api/v1/strategies",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "GET",
        "/api/v1/strategies/{strategy_id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "POST",
        "/api/v1/strategies/{strategy_id}/configs",
        Phase::Current,
        true,
        true,
        false,
        false,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/strategy-configs",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "GET",
        "/api/v1/strategy-configs/{config_id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    // --- recommendations -----------------------------------------------------
    route(
        "POST",
        "/api/v1/recommendations/runs",
        Phase::Current,
        true,
        true,
        false,
        false,
        Some("recommendation"),
        true,
    ),
    route(
        "GET",
        "/api/v1/recommendations/runs/{run_id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("recommendation"),
        false,
    ),
    route(
        "GET",
        "/api/v1/recommendations/runs",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("recommendation"),
        false,
    ),
    route(
        "GET",
        "/api/v1/recommendations/latest",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("recommendation"),
        false,
    ),
    // --- common individual-stock research -----------------------------------
    route(
        "GET",
        "/api/v1/candidates/feed/latest",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("candidate"),
        false,
    ),
    route(
        "GET",
        "/api/v1/candidates/feed/{date}",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("candidate"),
        false,
    ),
    route(
        "GET",
        "/api/v1/stocks/{instrument_id}/analysis",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("candidate"),
        false,
    ),
    route(
        "POST",
        "/api/v1/screener/query",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("candidate"),
        false,
    ),
    route(
        "GET",
        "/api/v1/screener/screens",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "POST",
        "/api/v1/screener/screens",
        Phase::Current,
        true,
        true,
        false,
        false,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/screener/screens/{id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "PUT",
        "/api/v1/screener/screens/{id}",
        Phase::Current,
        true,
        true,
        false,
        false,
        None,
        true,
    ),
    route(
        "DELETE",
        "/api/v1/screener/screens/{id}",
        Phase::Current,
        true,
        true,
        false,
        false,
        None,
        true,
    ),
    // --- backtests -----------------------------------------------------------
    route(
        "POST",
        "/api/v1/backtests",
        Phase::Current,
        true,
        true,
        false,
        false,
        Some("backtest"),
        true,
    ),
    route(
        "GET",
        "/api/v1/backtests",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("backtest"),
        false,
    ),
    route(
        "GET",
        "/api/v1/backtests/{run_id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("backtest"),
        false,
    ),
    route(
        "POST",
        "/api/v1/backtests/{run_id}/cancel",
        Phase::Current,
        true,
        true,
        false,
        false,
        Some("backtest"),
        true,
    ),
    route(
        "GET",
        "/api/v1/backtests/{run_id}/metrics",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("backtest"),
        false,
    ),
    route(
        "GET",
        "/api/v1/backtests/{run_id}/equity",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("backtest"),
        false,
    ),
    route(
        "GET",
        "/api/v1/backtests/{run_id}/trades",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("backtest"),
        false,
    ),
    route(
        "POST",
        "/api/v1/backtests/{run_id}/robustness",
        Phase::Current,
        true,
        true,
        false,
        false,
        Some("backtest"),
        true,
    ),
    route(
        "POST",
        "/api/v1/backtests/compare",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("backtest"),
        false,
    ),
    // --- paper ----------------------------------------------------------------
    route(
        "GET",
        "/api/v1/paper/accounts",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "POST",
        "/api/v1/paper/accounts",
        Phase::Current,
        true,
        true,
        false,
        false,
        Some("paper_view"),
        true,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "POST",
        "/api/v1/paper/accounts/{account_id}/bind-strategy",
        Phase::Current,
        true,
        true,
        false,
        false,
        Some("paper_view"),
        true,
    ),
    route(
        "POST",
        "/api/v1/paper/accounts/{account_id}/recommendation-previews",
        Phase::Current,
        true,
        true,
        false,
        true,
        Some("recommendation"),
        true,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}",
        Phase::Current,
        false,
        false,
        false,
        true,
        Some("paper_view"),
        false,
    ),
    route(
        "POST",
        "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply",
        Phase::Current,
        true,
        true,
        false,
        true,
        Some("recommendation"),
        true,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/orders",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/positions",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/equity",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/performance",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/lineage",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    route(
        "GET",
        "/api/v1/paper/accounts/{account_id}/parity",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("paper_view"),
        false,
    ),
    // --- admin (Owner-only, audited) -------------------------------------------
    route(
        "GET",
        "/api/v1/admin/datasets",
        Phase::Current,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/datasets/{dataset_id}/approve",
        Phase::Current,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/datasets/{dataset_id}/block",
        Phase::Current,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/admin/jobs",
        Phase::Current,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/jobs/{job_id}/retry",
        Phase::Current,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/admin/workers",
        Phase::Current,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/admin/users",
        Phase::Current,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/admin/audit-logs",
        Phase::Current,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/admin/notifications/deliveries",
        Phase::Current,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    // --- notifications -----------------------------------------------------
    route(
        "GET",
        "/api/v1/notifications",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "GET",
        "/api/v1/notifications/subscriptions",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "PUT",
        "/api/v1/notifications/subscriptions",
        Phase::Current,
        true,
        true,
        false,
        false,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/notifications/test",
        Phase::Current,
        true,
        true,
        false,
        false,
        None,
        true,
    ),
    // --- Live (Phase 3: Owner-only; not available) ------------------------------
    route(
        "GET",
        "/api/v1/admin/live/connections",
        Phase::Phase3,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/live/connections",
        Phase::Phase3,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    // Submitting an order. Owner-only, mutating, and IDEMPOTENT: the
    // Idempotency-Key is not decoration here, it is the identity a
    // retransmission repeats and therefore the only thing that can stop a
    // timed-out retry from placing a second real order (FR-LIVE-003).
    route(
        "POST",
        "/api/v1/admin/live/orders",
        Phase::Phase3,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/live/connections/{connection_id}/start",
        Phase::Phase3,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/live/nodes/{node_id}/stop",
        Phase::Phase3,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/live/kill-switch/enable",
        Phase::Phase3,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "POST",
        "/api/v1/admin/live/kill-switch/disable",
        Phase::Phase3,
        true,
        true,
        false,
        true,
        None,
        true,
    ),
    route(
        "GET",
        "/api/v1/admin/live/reconciliation",
        Phase::Phase3,
        false,
        false,
        false,
        true,
        None,
        true,
    ),
    // --- licensing / artifacts --------------------------------------------------
    route(
        "GET",
        "/api/v1/licensing-status",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "GET",
        "/api/v1/metrics",
        Phase::Current,
        false,
        false,
        false,
        false,
        None,
        false,
    ),
    route(
        "GET",
        "/api/v1/artifacts/{artifact_id}",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("download"),
        false,
    ),
    route(
        "GET",
        "/api/v1/artifacts/{artifact_id}/download",
        Phase::Current,
        false,
        false,
        false,
        false,
        Some("download"),
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_beta_prefixes_cover_exactly_the_temporary_product_groups() {
        let expected = CONTRACT_ROUTES
            .iter()
            .filter(|route| {
                matches!(
                    route.path,
                    path if path.starts_with("/api/v1/recommendations/")
                        || path.starts_with("/api/v1/backtests")
                        || path.starts_with("/api/v1/paper/")
                )
            })
            .map(|route| route.path)
            .collect::<Vec<_>>();
        let covered = CONTRACT_ROUTES
            .iter()
            .filter(|route| owner_beta_applies(route.path))
            .map(|route| route.path)
            .collect::<Vec<_>>();
        assert_eq!(covered, expected);

        for path in [
            "/api/v1/recommendations",
            "/api/v1/recommendations/runs",
            "/api/v1/backtests",
            "/api/v1/backtests/anything",
            "/api/v1/paper",
            "/api/v1/paper/accounts",
        ] {
            assert!(owner_beta_applies(path), "{path} must be gated");
        }
        for path in [
            "/api/v1/auth/session",
            "/api/v1/strategies",
            "/api/v1/candidates/feed/latest",
            "/api/v1/admin/datasets",
            "/api/v1/live",
            "/api/v1/papers/accounts",
        ] {
            assert!(!owner_beta_applies(path), "{path} must not be beta-gated");
        }
    }
}
