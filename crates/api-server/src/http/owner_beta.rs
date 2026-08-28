//! HTTP boundary for the sealed owner-beta historical price-only enqueue.
//!
//! The handler performs policy, CSRF, request, and date checks before the
//! filesystem approval read. Approval is re-run for every enqueue and replay;
//! the durable repository is the only idempotency authority.

use crate::http::dto::{
    OwnerBetaInstrumentDto, OwnerBetaPriceOnlyReadItemDto, OwnerBetaPriceOnlyReadListItemDto,
    OwnerBetaPriceOnlyReadRunDto, OwnerBetaPriceOnlyRunBody, OwnerBetaPriceOnlyRunDto,
    OwnerBetaPriceOnlySupportedAsOfDto, PageDto,
};
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{Session, require_csrf};
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit, tenancy_response};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{Datelike, NaiveDate};
use domain::TradingDate;
use market_data::{ApprovedHistoricalPriceOnlyArtifact, KR_ETF_CORE_SYMBOLS};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// POST `/api/v1/recommendations/owner-beta/price-only/runs`.
pub async fn create_price_only_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<OwnerBetaPriceOnlyRunBody>,
) -> Response {
    let rid = request_id(&headers);

    // The central recommendation-prefix middleware is intentionally repeated
    // here. This handler must remain closed if it is mounted independently or
    // if a future router refactor changes middleware nesting.
    if state.cfg.owner_beta_access != crate::http::state::OwnerBetaAccessMode::OwnerOnly
        || !session.actor().is_owner()
    {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    if !state.cfg.owner_beta_price_input.is_enabled() {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
            "owner-beta price input unavailable",
            &rid,
            None,
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
    let Some(as_of) = parse_strict_date(&body.as_of) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            "as_of must be a valid YYYY-MM-DD calendar date",
            &rid,
            None,
        );
    };
    if as_of > (state.cfg.seoul_today)() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_DATE",
            "as_of must not be after the Seoul civil date",
            &rid,
            None,
        );
    }
    if let Err(response) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &body.as_of,
    )
    .await
    {
        return response;
    }

    let artifact = match approve_artifact(&state).await {
        Ok(artifact) => artifact,
        Err(()) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
                "owner-beta price input unavailable",
                &rid,
                None,
            );
        }
    };
    let as_of_trading = match TradingDate::new(as_of.year(), as_of.month(), as_of.day()) {
        Ok(date) => date,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_DATE",
                "as_of must be a valid YYYY-MM-DD calendar date",
                &rid,
                None,
            );
        }
    };
    if !has_exact_session(&artifact, as_of_trading) {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
            "owner-beta price input unavailable",
            &rid,
            None,
        );
    }

    let result = state
        .owner_beta_recommendations()
        .submit(
            &session.actor(),
            strategy_config_id,
            as_of,
            &client_key,
            &artifact,
            state.cfg.max_jobs_per_owner,
        )
        .await;
    let result = match result {
        Ok(result) => result,
        Err(crate::repos::owner_beta::OwnerBetaPriceRecommendationError::NotFound) => {
            return code_error("RESOURCE_NOT_FOUND", "resource not found", &rid);
        }
        Err(crate::repos::owner_beta::OwnerBetaPriceRecommendationError::Forbidden) => {
            return code_error("FORBIDDEN", "forbidden", &rid);
        }
        Err(crate::repos::owner_beta::OwnerBetaPriceRecommendationError::CapacityExceeded) => {
            return code_error(
                "RECOMMENDATION_CAPACITY_EXCEEDED",
                "owner-beta recommendation capacity exceeded",
                &rid,
            );
        }
        Err(crate::repos::owner_beta::OwnerBetaPriceRecommendationError::StrategyUnsupported) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "OWNER_BETA_STRATEGY_UNSUPPORTED",
                "owner-beta strategy unsupported",
                &rid,
                None,
            );
        }
        Err(crate::repos::owner_beta::OwnerBetaPriceRecommendationError::IdempotencyMismatch) => {
            return code_error(
                "IDEMPOTENCY_KEY_MISMATCH",
                "the same Idempotency-Key was already used with a different request body",
                &rid,
            );
        }
        Err(crate::repos::owner_beta::OwnerBetaPriceRecommendationError::Internal) => {
            return code_error("INTERNAL", "internal error", &rid);
        }
    };

    audit(
        &state,
        &session,
        &headers,
        "owner_beta.price_only.run.enqueue",
        "owner_beta_recommendation_run",
        &result.run_id.to_string(),
        None,
        Some(serde_json::json!({
            "run_id": result.run_id,
            "job_id": result.job_id,
            "strategy_config_id": strategy_config_id,
            "as_of": as_of,
        })),
        None,
    )
    .await;

    let mut response = (
        StatusCode::ACCEPTED,
        Json(OwnerBetaPriceOnlyRunDto {
            run_id: result.run_id.to_string(),
            job_id: result.job_id.to_string(),
            status: result.status,
        }),
    )
        .into_response();
    if result.replay {
        response.headers_mut().insert(
            "X-Idempotent-Replay",
            axum::http::HeaderValue::from_static("true"),
        );
    }
    response
}

/// GET `/api/v1/recommendations/owner-beta/price-only/runs`.
///
/// This is a read-only route: it repeats the owner-beta policy check and
/// entitlement gate but never checks the sealed input mode, reads an
/// artifact, re-approves a pin, writes an audit event, or accepts an
/// idempotency key.
pub async fn list_price_only_runs(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<crate::http::session::PageParams>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_beta_read_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
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
    let cursor = match decode_read_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(cursor) => cursor,
        Err(response) => return response,
    };
    let limit = params.limit_or(crate::http::session::PageParams::DEFAULT_LIMIT);
    match state
        .owner_beta_recommendations()
        .list_price_only_runs(&session.actor(), cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|cursor| cursor.encode(&state.cfg.cursor_secret));
            let items = rows.into_iter().map(read_list_item_dto).collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(error) => tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    }
}

/// GET `/api/v1/recommendations/owner-beta/price-only/supported-as-of`.
pub async fn get_supported_as_of(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    if !owner_beta_read_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
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

    let artifact = match approve_artifact(&state).await {
        Ok(artifact) => artifact,
        Err(()) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
                "owner-beta price input unavailable",
                &rid,
                None,
            );
        }
    };
    let Some(supported_as_of) = supported_as_of_response(artifact.bars()) else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
            "owner-beta price input unavailable",
            &rid,
            None,
        );
    };

    (StatusCode::OK, Json(supported_as_of)).into_response()
}

/// GET `/api/v1/recommendations/owner-beta/price-only/runs/{run_id}`.
pub async fn get_price_only_run(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if !owner_beta_read_allowed(&state, &session) {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    let run_id = match Uuid::parse_str(&run_id) {
        Ok(run_id) => run_id,
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
    let header = match state
        .owner_beta_recommendations()
        .get_price_only_run_header(&session.actor(), run_id)
        .await
    {
        Ok(header) => header,
        Err(error) => return tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    };
    if let Err(response) = require_use(
        &state,
        &session,
        &headers,
        auth::entitlement::KrUse::Recommendation,
        &header.as_of.format("%Y-%m-%d").to_string(),
    )
    .await
    {
        return response;
    }
    match state
        .owner_beta_recommendations()
        .get_price_only_run(&session.actor(), run_id)
        .await
    {
        Ok((row, items)) => (StatusCode::OK, Json(read_run_dto(row, items))).into_response(),
        Err(error) => tenancy_response(error, &rid, "RESOURCE_NOT_FOUND"),
    }
}

fn owner_beta_read_allowed(state: &ApiState, session: &Session) -> bool {
    state.cfg.owner_beta_access == crate::http::state::OwnerBetaAccessMode::OwnerOnly
        && session.actor().is_owner()
}

#[allow(clippy::result_large_err)]
fn decode_read_cursor(
    state: &ApiState,
    rid: &str,
    raw: Option<&str>,
) -> Result<Option<Cursor>, Response> {
    match raw {
        None => Ok(None),
        Some(raw) => Cursor::decode(raw, &state.cfg.cursor_secret)
            .map(Some)
            .map_err(|_| code_error("INVALID_CURSOR", "pagination cursor is invalid", rid)),
    }
}

fn read_run_dto(
    row: crate::repos::owner_beta::OwnerBetaPriceOnlyReadRunRow,
    items: Vec<crate::repos::owner_beta::OwnerBetaPriceOnlyReadItemRow>,
) -> OwnerBetaPriceOnlyReadRunDto {
    OwnerBetaPriceOnlyReadRunDto {
        id: row.id.to_string(),
        job_id: row.job_id.to_string(),
        strategy_config_id: row.strategy_config_id.to_string(),
        strategy_id: row.strategy_id,
        strategy_version: row.strategy_version,
        as_of: row.as_of,
        status: row.status,
        input_kind: row.input_kind,
        capability: row.capability,
        audience: row.audience,
        vendor_snapshot: row.vendor_snapshot,
        strict_pit: row.strict_pit,
        strategy_config_sha256: row.strategy_config_sha256,
        candidate_content_sha256: row.candidate_content_sha256,
        artifact_manifest_sha256: row.artifact_manifest_sha256,
        stage5_manifest_sha256: row.stage5_manifest_sha256,
        action_manifest_sha256: row.action_manifest_sha256,
        approval_registry_sha256: row.approval_registry_sha256,
        factor_snapshot_sha256: row.factor_snapshot_sha256,
        target_snapshot_sha256: row.target_snapshot_sha256,
        cash_weight: row.cash_weight,
        error_code: row.error_code,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
        items: items.into_iter().map(read_item_dto).collect(),
    }
}

fn read_list_item_dto(
    row: crate::repos::owner_beta::OwnerBetaPriceOnlyReadRunRow,
) -> OwnerBetaPriceOnlyReadListItemDto {
    OwnerBetaPriceOnlyReadListItemDto {
        id: row.id.to_string(),
        job_id: row.job_id.to_string(),
        strategy_config_id: row.strategy_config_id.to_string(),
        strategy_id: row.strategy_id,
        strategy_version: row.strategy_version,
        as_of: row.as_of,
        status: row.status,
        input_kind: row.input_kind,
        capability: row.capability,
        audience: row.audience,
        vendor_snapshot: row.vendor_snapshot,
        strict_pit: row.strict_pit,
        strategy_config_sha256: row.strategy_config_sha256,
        candidate_content_sha256: row.candidate_content_sha256,
        artifact_manifest_sha256: row.artifact_manifest_sha256,
        stage5_manifest_sha256: row.stage5_manifest_sha256,
        action_manifest_sha256: row.action_manifest_sha256,
        approval_registry_sha256: row.approval_registry_sha256,
        factor_snapshot_sha256: row.factor_snapshot_sha256,
        target_snapshot_sha256: row.target_snapshot_sha256,
        cash_weight: row.cash_weight,
        error_code: row.error_code,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
    }
}

fn read_item_dto(
    item: crate::repos::owner_beta::OwnerBetaPriceOnlyReadItemRow,
) -> OwnerBetaPriceOnlyReadItemDto {
    let reason_codes = item
        .reason_codes
        .as_array()
        .map(|codes| {
            codes
                .iter()
                .filter_map(|code| code.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    OwnerBetaPriceOnlyReadItemDto {
        instrument_id: item.instrument_id.clone(),
        instrument: OwnerBetaInstrumentDto {
            id: item.instrument_id,
            name: item.instrument_name,
            asset_class: item.instrument_asset_class,
            tracking_index: None,
            exposure_group: None,
        },
        rank: item.rank,
        target_weight: item.target_weight,
        excluded: item.excluded,
        exclusion_reason: item.exclusion_reason,
        reason_codes,
        factors: item.factors_json,
    }
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
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "Idempotency-Key is invalid",
            rid,
            None,
        ));
    };
    if value.is_empty()
        || value.len() > crate::http::idempotency::MAX_KEY_BYTES
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "Idempotency-Key is invalid",
            rid,
            None,
        ));
    }
    Ok(value.to_owned())
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

fn has_exact_session(artifact: &ApprovedHistoricalPriceOnlyArtifact, as_of: TradingDate) -> bool {
    let actual = artifact
        .bars()
        .iter()
        .filter(|bar| bar.session_date == as_of)
        .map(|bar| bar.instrument_id.to_string())
        .collect::<BTreeSet<_>>();
    has_exact_etf11_set(&actual)
}

fn has_exact_etf11_set(actual: &BTreeSet<String>) -> bool {
    let expected = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    actual == &expected
}

fn supported_as_of_dates(bars: &[market_data::HistoricalPriceOnlyBar]) -> Vec<NaiveDate> {
    let mut instruments_by_date = BTreeMap::<NaiveDate, BTreeSet<String>>::new();
    for bar in bars {
        instruments_by_date
            .entry(bar.session_date.as_naive_date())
            .or_default()
            .insert(bar.instrument_id.to_string());
    }
    instruments_by_date
        .into_iter()
        .filter_map(|(date, instruments)| has_exact_etf11_set(&instruments).then_some(date))
        .collect()
}

fn supported_as_of_response(
    bars: &[market_data::HistoricalPriceOnlyBar],
) -> Option<OwnerBetaPriceOnlySupportedAsOfDto> {
    let supported_as_of = supported_as_of_dates(bars);
    let default_as_of = supported_as_of.last().copied()?;
    Some(OwnerBetaPriceOnlySupportedAsOfDto {
        default_as_of,
        supported_as_of,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::session::Session;
    use crate::http::state::{
        ApiState, OwnerBetaAccessMode, OwnerBetaPaperMode, OwnerBetaPriceInputMode,
    };
    use auth::entitlement::{Role, UserId};
    use auth::sessions::SessionInfo;
    use axum::body::to_bytes;
    use domain::{FixedPoint, InstrumentId, Venue};
    use serde_json::json;
    use std::collections::BTreeSet;

    fn owner_session() -> Session {
        Session(SessionInfo {
            user_id: UserId("00000000-0000-4000-8000-000000000001".to_owned()),
            role: Role::Owner,
            auth_time_secs: 1,
            amr: Vec::new(),
            expires_at_secs: 2,
            csrf_token_hash: "not-secret-test-hash".to_owned(),
        })
    }

    #[tokio::test]
    async fn access_and_price_mode_rejections_precede_approval_and_database() {
        for (access, price_mode, expected_status, expected_code) in [
            (
                OwnerBetaAccessMode::Disabled,
                OwnerBetaPriceInputMode::SealedV1,
                StatusCode::FORBIDDEN,
                "FORBIDDEN",
            ),
            (
                OwnerBetaAccessMode::OwnerOnly,
                OwnerBetaPriceInputMode::Disabled,
                StatusCode::SERVICE_UNAVAILABLE,
                "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
            ),
        ] {
            let state = ApiState::test_without_database_with_all_policy(
                access,
                OwnerBetaPaperMode::Disabled,
                price_mode,
            );
            let response = create_price_only_run(
                State(state.clone()),
                owner_session(),
                HeaderMap::from_iter([(
                    "x-request-id".parse().unwrap(),
                    "mode-test".parse().unwrap(),
                )]),
                JsonBody(OwnerBetaPriceOnlyRunBody {
                    strategy_config_id: "00000000-0000-0000-0000-000000000001".to_owned(),
                    as_of: "2026-08-24".to_owned(),
                }),
            )
            .await;
            assert_eq!(response.status(), expected_status);
            let body = to_bytes(response.into_body(), 16 * 1024)
                .await
                .expect("bounded mode rejection body");
            let body: serde_json::Value = serde_json::from_slice(&body).expect("error JSON");
            assert_eq!(body["error"]["code"], expected_code);
            assert_eq!(state.app_pool.size(), 0);
            assert_eq!(state.admin_pool.size(), 0);
            assert_eq!(state.audit_pool.size(), 0);
        }
    }

    #[tokio::test]
    async fn csrf_rejection_precedes_entitlement_approval_and_database() {
        let state = ApiState::test_without_database_with_all_policy(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
            OwnerBetaPriceInputMode::SealedV1,
        );
        let response = create_price_only_run(
            State(state.clone()),
            owner_session(),
            HeaderMap::from_iter([(
                "x-request-id".parse().unwrap(),
                "csrf-test".parse().unwrap(),
            )]),
            JsonBody(OwnerBetaPriceOnlyRunBody {
                strategy_config_id: "00000000-0000-0000-0000-000000000001".to_owned(),
                as_of: "2026-08-24".to_owned(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("bounded CSRF rejection body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("error JSON");
        assert_eq!(body["error"]["code"], "CSRF_DENIED");
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);
    }

    #[tokio::test]
    async fn supported_as_of_policy_rejection_precedes_entitlement_and_approval() {
        let state = ApiState::test_without_database(OwnerBetaAccessMode::Disabled);
        let response = get_supported_as_of(
            State(state.clone()),
            owner_session(),
            HeaderMap::from_iter([(
                "x-request-id".parse().unwrap(),
                "policy-test".parse().unwrap(),
            )]),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("bounded policy rejection body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("error JSON");
        assert_eq!(body["error"]["code"], "FORBIDDEN");
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
        assert_eq!(state.audit_pool.size(), 0);
    }

    #[test]
    fn request_dto_has_only_strategy_config_and_as_of() {
        let value = json!({
            "strategy_config_id": "00000000-0000-0000-0000-000000000001",
            "as_of": "2026-08-19"
        });
        let body: OwnerBetaPriceOnlyRunBody =
            serde_json::from_value(value.clone()).expect("valid owner-beta body");
        let fields = serde_json::to_value(body)
            .expect("body serializes")
            .as_object()
            .expect("body object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            fields,
            BTreeSet::from(["as_of".to_owned(), "strategy_config_id".to_owned()])
        );

        for forbidden in [
            "pins",
            "artifact_root",
            "artifact_path",
            "path",
            "dataset",
            "dataset_id",
            "paper",
            "paper_mode",
            "worker",
            "worker_id",
            "root",
            "candidate_content_sha256",
        ] {
            let mut unknown = value.clone();
            unknown[forbidden] = json!("forbidden");
            assert!(
                serde_json::from_value::<OwnerBetaPriceOnlyRunBody>(unknown).is_err(),
                "unknown field {forbidden} must be rejected"
            );
        }
    }

    #[test]
    fn idempotency_key_contract_rejects_missing_whitespace_and_non_ascii() {
        let mut headers = HeaderMap::new();
        assert!(client_key(&headers, "rid").is_err());
        headers.insert("idempotency-key", "client-key".parse().unwrap());
        assert_eq!(client_key(&headers, "rid").unwrap(), "client-key");
        for invalid in [" client-key", "client-key ", "client key"] {
            headers.insert("idempotency-key", invalid.parse().unwrap());
            assert!(client_key(&headers, "rid").is_err(), "{invalid:?}");
        }
        headers.insert(
            "idempotency-key",
            axum::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
        );
        assert!(client_key(&headers, "rid").is_err());
        headers.insert("idempotency-key", "x".repeat(201).parse().unwrap());
        assert!(client_key(&headers, "rid").is_err());
    }

    #[test]
    fn date_contract_rejects_future_shape_and_accepts_canonical_date() {
        assert_eq!(
            parse_strict_date("2026-08-19"),
            NaiveDate::from_ymd_opt(2026, 8, 19)
        );
        for invalid in ["2026-8-19", "2026-08-19 ", "2026/08/19", "2026-02-30"] {
            assert!(parse_strict_date(invalid).is_none(), "{invalid:?}");
        }
    }

    fn test_bar(symbol: &str, date: &str) -> market_data::HistoricalPriceOnlyBar {
        let value = || FixedPoint::parse("1").expect("valid fixed point");
        market_data::HistoricalPriceOnlyBar {
            instrument_id: InstrumentId::from_parts(symbol, Venue::Krx).expect("valid instrument"),
            session_date: TradingDate::parse(date).expect("valid date"),
            raw_open: value(),
            raw_high: value(),
            raw_low: value(),
            raw_close: value(),
            raw_volume: 1,
            raw_trading_value: Some(value()),
            adjusted_open: value(),
            adjusted_high: value(),
            adjusted_low: value(),
            adjusted_close: value(),
        }
    }

    #[test]
    fn supported_as_of_dates_are_sorted_and_require_every_exact_etf11_id() {
        let mut bars = Vec::new();
        for date in ["2026-08-19", "2026-08-16", "2026-08-17", "2026-08-18"] {
            for (index, symbol) in KR_ETF_CORE_SYMBOLS.iter().enumerate() {
                if date == "2026-08-16" && index == KR_ETF_CORE_SYMBOLS.len() - 1 {
                    continue;
                }
                bars.push(test_bar(symbol, date));
            }
        }
        // A complete ETF11 session with an unrelated instrument is not an
        // exact session and must not be advertised as supported.
        bars.push(test_bar("SPY", "2026-08-17"));

        assert_eq!(
            supported_as_of_dates(&bars),
            vec![
                NaiveDate::from_ymd_opt(2026, 8, 18).unwrap(),
                NaiveDate::from_ymd_opt(2026, 8, 19).unwrap()
            ]
        );
        assert!(supported_as_of_dates(&[]).is_empty());
        let response = supported_as_of_response(&bars).expect("supported response");
        assert_eq!(
            response.default_as_of,
            NaiveDate::from_ymd_opt(2026, 8, 19).unwrap()
        );
        assert_eq!(
            serde_json::to_value(response).expect("response JSON"),
            json!({
                "default_as_of": "2026-08-19",
                "supported_as_of": ["2026-08-18", "2026-08-19"]
            })
        );
        assert!(supported_as_of_response(&[]).is_none());
    }

    fn read_item_fixture(
        name: Option<&str>,
        asset_class: Option<&str>,
    ) -> crate::repos::owner_beta::OwnerBetaPriceOnlyReadItemRow {
        crate::repos::owner_beta::OwnerBetaPriceOnlyReadItemRow {
            recommendation_run_id: Uuid::new_v4(),
            instrument_id: "069500.KRX".to_owned(),
            instrument_name: name.map(str::to_owned),
            instrument_asset_class: asset_class.map(str::to_owned),
            rank: Some(1),
            target_weight: Some("0.500000".to_owned()),
            reason_codes: json!(["SELECTED_TOP_N"]),
            factors_json: json!({"close": "1"}),
            excluded: false,
            exclusion_reason: None,
        }
    }

    #[test]
    fn item_projection_preserves_id_and_allows_present_or_null_metadata() {
        let missing = serde_json::to_value(read_item_dto(read_item_fixture(None, None)))
            .expect("missing metadata serializes");
        let present = serde_json::to_value(read_item_dto(read_item_fixture(
            Some("KODEX 200"),
            Some("ETF"),
        )))
        .expect("present metadata serializes");

        for key in [
            "instrument_id",
            "rank",
            "target_weight",
            "excluded",
            "exclusion_reason",
            "reason_codes",
            "factors",
        ] {
            assert_eq!(missing[key], present[key], "metadata changed {key}");
        }
        assert_eq!(
            missing["instrument"],
            json!({
                "id": "069500.KRX",
                "name": null,
                "asset_class": null,
                "tracking_index": null,
                "exposure_group": null
            })
        );
        assert_eq!(
            present["instrument"],
            json!({
                "id": "069500.KRX",
                "name": "KODEX 200",
                "asset_class": "ETF",
                "tracking_index": null,
                "exposure_group": null
            })
        );
        assert_eq!(missing["instrument_id"], missing["instrument"]["id"]);
    }
}
