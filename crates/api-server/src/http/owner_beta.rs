//! HTTP boundary for the sealed owner-beta historical price-only enqueue.
//!
//! The handler performs policy, CSRF, request, and date checks before the
//! filesystem approval read. Approval is re-run for every enqueue and replay;
//! the durable repository is the only idempotency authority.

use crate::http::dto::{OwnerBetaPriceOnlyRunBody, OwnerBetaPriceOnlyRunDto};
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::{Session, require_csrf};
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{Datelike, NaiveDate};
use domain::TradingDate;
use market_data::{ApprovedHistoricalPriceOnlyArtifact, KR_ETF_CORE_SYMBOLS};
use std::collections::BTreeSet;
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
    let expected = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<BTreeSet<_>>();
    let actual = artifact
        .bars()
        .iter()
        .filter(|bar| bar.session_date == as_of)
        .map(|bar| bar.instrument_id.to_string())
        .collect::<BTreeSet<_>>();
    actual == expected
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
}
