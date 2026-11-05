//! Owner-only HTTP handlers for the separate Owner-beta Paper v1 ledger.

use crate::http::dto::{
    OwnerBetaPaperApplyBody, OwnerBetaPaperConsentBody, OwnerBetaPaperPreviewBody,
    OwnerBetaPaperStateDto, OwnerBetaPaperStateKind,
};
use crate::http::entitlement::require_use;
use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::{Session, require_csrf};
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit};
use crate::repos::rebalance_previews::OwnerBetaPaperRepoError;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

const IDEMPOTENCY_KEY_MAX_BYTES: usize = 96;
pub(crate) async fn get_state(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = require_owner_beta_paper_owner(&state, &session, &rid) {
        return response;
    }
    if !state.cfg.owner_beta_paper.is_enabled() {
        return Json(disabled_state()).into_response();
    }
    if let Err(response) = require_paper_view(&state, &session, &headers).await {
        return response;
    }
    match state
        .rebalance_previews()
        .owner_beta_paper_state(&session.actor())
        .await
    {
        Ok(state) => Json(state).into_response(),
        Err(error) => paper_repo_error(error, &rid),
    }
}

pub(crate) async fn consent(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<OwnerBetaPaperConsentBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = require_owner_beta_paper_owner(&state, &session, &rid) {
        return response;
    }
    if let Some(response) = disabled_response(&state, &rid) {
        return response;
    }
    if let Err(response) = require_csrf(&headers, &session.0) {
        return response;
    }
    let key = match idempotency_key(&headers, &rid) {
        Ok(key) => key,
        Err(response) => return response,
    };
    if body.expected_epoch < 0 {
        return invalid_parameter("expected_epoch must be a non-negative integer", &rid);
    }
    // Revocation remains available after entitlement withdrawal. A grant is a
    // new consent event, so it rechecks the current Paper and recommendation
    // rights on every request.
    if body.opt_in {
        if let Err(response) = require_paper_view(&state, &session, &headers).await {
            return response;
        }
        if let Err(response) = require_recommendation(&state, &session, &headers).await {
            return response;
        }
    }
    let consent = match state
        .rebalance_previews()
        .owner_beta_paper_consent(&session.actor(), body.opt_in, body.expected_epoch, &key)
        .await
    {
        Ok(consent) => consent,
        Err(error) => return paper_repo_error(error, &rid),
    };
    if !consent.replayed {
        audit(
            &state,
            &session,
            &headers,
            "paper.owner_beta.consent",
            "owner_beta_paper_account",
            &consent.account_id.to_string(),
            None,
            Some(serde_json::json!({
                "event_id": consent.event_id,
                "control_epoch": consent.control_epoch,
                "opt_in": consent.opt_in,
            })),
            None,
        )
        .await;
    }
    (StatusCode::OK, Json(consent)).into_response()
}

pub(crate) async fn create_preview(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<OwnerBetaPaperPreviewBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = require_owner_beta_paper_owner(&state, &session, &rid) {
        return response;
    }
    if let Some(response) = disabled_response(&state, &rid) {
        return response;
    }
    if let Err(response) = require_csrf(&headers, &session.0) {
        return response;
    }
    let key = match idempotency_key(&headers, &rid) {
        Ok(key) => key,
        Err(response) => return response,
    };
    if body.expected_epoch < 0 {
        return invalid_parameter("expected_epoch must be a non-negative integer", &rid);
    }
    if body.recommendation_run_id.is_nil() {
        return invalid_parameter(
            "recommendation_run_id must be a canonical non-nil uuid",
            &rid,
        );
    }
    if let Err(response) = require_paper_view(&state, &session, &headers).await {
        return response;
    }
    if let Err(response) = require_recommendation(&state, &session, &headers).await {
        return response;
    }
    let submitted = match state
        .rebalance_previews()
        .owner_beta_paper_submit(
            &session.actor(),
            body.recommendation_run_id,
            body.expected_epoch,
            key,
            state.cfg.max_jobs_per_owner,
        )
        .await
    {
        Ok(submitted) => submitted,
        Err(error) => return paper_repo_error(error, &rid),
    };
    let preview = match state
        .rebalance_previews()
        .owner_beta_paper_preview(&session.actor(), submitted.preview_id)
        .await
    {
        Ok(preview) => preview,
        Err(error) => return paper_repo_error(error, &rid),
    };
    if !submitted.replayed {
        audit(
            &state,
            &session,
            &headers,
            "paper.owner_beta.preview.create",
            "owner_beta_paper_preview",
            &submitted.preview_id.to_string(),
            None,
            Some(serde_json::json!({
                "job_id": submitted.job_id,
                "recommendation_run_id": body.recommendation_run_id,
                "signal_date": preview.signal_date,
                "effective_session": preview.effective_session,
            })),
            None,
        )
        .await;
    }
    let status = if submitted.replayed {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    (status, Json(preview)).into_response()
}

pub(crate) async fn get_preview(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(preview_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = require_owner_beta_paper_owner(&state, &session, &rid) {
        return response;
    }
    if let Some(response) = disabled_response(&state, &rid) {
        return response;
    }
    let preview_id = match canonical_path_uuid(&preview_id) {
        Some(id) => id,
        None => return invalid_parameter("preview_id must be a canonical non-nil uuid", &rid),
    };
    if let Err(response) = require_paper_view(&state, &session, &headers).await {
        return response;
    }
    match state
        .rebalance_previews()
        .owner_beta_paper_preview(&session.actor(), preview_id)
        .await
    {
        Ok(preview) => Json(preview).into_response(),
        Err(error) => paper_repo_error(error, &rid),
    }
}

pub(crate) async fn apply_preview(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(preview_id): Path<String>,
    JsonBody(body): JsonBody<OwnerBetaPaperApplyBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = require_owner_beta_paper_owner(&state, &session, &rid) {
        return response;
    }
    if let Some(response) = disabled_response(&state, &rid) {
        return response;
    }
    if let Err(response) = require_csrf(&headers, &session.0) {
        return response;
    }
    let _key = match idempotency_key(&headers, &rid) {
        Ok(key) => key,
        Err(response) => return response,
    };
    let preview_id = match canonical_path_uuid(&preview_id) {
        Some(id) => id,
        None => return invalid_parameter("preview_id must be a canonical non-nil uuid", &rid),
    };
    if body.expected_epoch < 0 {
        return invalid_parameter("expected_epoch must be a non-negative integer", &rid);
    }
    if !is_bare_lower_hex(&body.preview_token, 64) {
        return invalid_parameter(
            "preview_token must be 64 lowercase hexadecimal characters",
            &rid,
        );
    }
    if let Err(response) = require_paper_view(&state, &session, &headers).await {
        return response;
    }
    if let Err(response) = require_recommendation(&state, &session, &headers).await {
        return response;
    }
    // The current SQL ABI identifies a replay by the preview ID, token,
    // account and epoch. It has no apply request-key parameter; the header is
    // still validated here and the unresolved durable key/body seam is listed
    // in the API handoff report.
    let applied = match state
        .rebalance_previews()
        .owner_beta_paper_apply(
            &session.actor(),
            preview_id,
            body.preview_token,
            body.expected_epoch,
        )
        .await
    {
        Ok(applied) => applied,
        Err(error) => return paper_repo_error(error, &rid),
    };
    if !applied.replayed {
        audit(
            &state,
            &session,
            &headers,
            "paper.owner_beta.preview.apply",
            "owner_beta_paper_preview",
            &preview_id.to_string(),
            Some(serde_json::json!({ "status": "READY" })),
            Some(serde_json::json!({
                "status": applied.status,
                "pending_target_id": applied.pending_target_id,
                "effective_session": applied.effective_session,
            })),
            None,
        )
        .await;
    }
    (StatusCode::OK, Json(applied)).into_response()
}

pub(crate) async fn get_session(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(pending_target_id): Path<String>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = require_owner_beta_paper_owner(&state, &session, &rid) {
        return response;
    }
    if let Some(response) = disabled_response(&state, &rid) {
        return response;
    }
    let pending_target_id = match canonical_path_uuid(&pending_target_id) {
        Some(id) => id,
        None => {
            return invalid_parameter("pending_target_id must be a canonical non-nil uuid", &rid);
        }
    };
    if let Err(response) = require_paper_view(&state, &session, &headers).await {
        return response;
    }
    match state
        .rebalance_previews()
        .owner_beta_paper_session(&session.actor(), pending_target_id)
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(error) => paper_repo_error(error, &rid),
    }
}

#[allow(
    clippy::result_large_err,
    reason = "Route guards preserve the complete typed Axum response at the HTTP boundary."
)]
fn require_owner_beta_paper_owner(
    state: &ApiState,
    session: &Session,
    rid: &str,
) -> Result<(), Response> {
    if !state.cfg.owner_beta_access.requires_owner() || !session.actor().is_owner() {
        return Err(code_error(
            "FORBIDDEN",
            "Owner-beta Paper requires an Owner session",
            rid,
        ));
    }
    Ok(())
}

fn disabled_response(state: &ApiState, rid: &str) -> Option<Response> {
    (!state.cfg.owner_beta_paper.is_enabled())
        .then(|| code_error("PAPER_DISABLED", "Owner-beta Paper is disabled", rid))
}

fn disabled_state() -> OwnerBetaPaperStateDto {
    OwnerBetaPaperStateDto {
        schema: "owner-beta-paper-state-v1".to_owned(),
        state: OwnerBetaPaperStateKind::Disabled,
        account_id: None,
        strategy_config_id: None,
        strategy_id: None,
        strategy_version: None,
        control_epoch: None,
        consent_present: false,
        daily_cohort_sha256: None,
        process_binding_sha256: None,
        sessions: Vec::new(),
    }
}

async fn require_paper_view(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
) -> Result<(), Response> {
    require_use(
        state,
        session,
        headers,
        auth::entitlement::KrUse::PaperView,
        &crate::http::entitlement::today_iso(),
    )
    .await
}

async fn require_recommendation(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
) -> Result<(), Response> {
    require_use(
        state,
        session,
        headers,
        auth::entitlement::KrUse::Recommendation,
        &crate::http::entitlement::today_iso(),
    )
    .await
}

#[allow(
    clippy::result_large_err,
    reason = "Route validation preserves the complete typed Axum response at the HTTP boundary."
)]
fn idempotency_key(headers: &HeaderMap, rid: &str) -> Result<String, Response> {
    let Some(header) = headers.get(crate::http::idempotency::HEADER) else {
        return Err(code_error(
            "IDEMPOTENCY_KEY_REQUIRED",
            "mutating routes require an Idempotency-Key header",
            rid,
        ));
    };
    if header.to_str().is_err() {
        return Err(invalid_parameter(
            "Idempotency-Key must contain 1-96 printable ASCII bytes",
            rid,
        ));
    }
    // Reuse the shared helper's canonical trim behavior, then constrain the
    // key before it is used in the durable Paper namespace.
    let Some(key) = crate::http::idempotency::key_from(headers) else {
        return Err(code_error(
            "IDEMPOTENCY_KEY_REQUIRED",
            "mutating routes require a non-empty Idempotency-Key header",
            rid,
        ));
    };
    if !(1..=IDEMPOTENCY_KEY_MAX_BYTES).contains(&key.len())
        || !key.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(invalid_parameter(
            "Idempotency-Key must contain 1-96 printable ASCII bytes",
            rid,
        ));
    }
    Ok(key)
}

fn canonical_path_uuid(value: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(value).ok()?;
    (!id.is_nil() && id.to_string() == value).then_some(id)
}

fn is_bare_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_parameter(message: &'static str, rid: &str) -> Response {
    api_error(
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMETER",
        message,
        rid,
        None,
    )
}

fn paper_repo_error(error: OwnerBetaPaperRepoError, rid: &str) -> Response {
    match error {
        OwnerBetaPaperRepoError::Unbound => code_error(
            "PAPER_UNBOUND",
            "Owner-beta Paper has no usable bound runtime",
            rid,
        ),
        OwnerBetaPaperRepoError::NotFound => {
            code_error("RESOURCE_NOT_FOUND", "resource not found", rid)
        }
        OwnerBetaPaperRepoError::Forbidden => code_error("FORBIDDEN", "forbidden", rid),
        OwnerBetaPaperRepoError::Stale => {
            code_error("PAPER_STALE", "Paper state or current inputs changed", rid)
        }
        OwnerBetaPaperRepoError::Conflict => code_error(
            "PAPER_CONFLICT",
            "Paper action conflicts with current state",
            rid,
        ),
        OwnerBetaPaperRepoError::NotReady => code_error(
            "PAPER_NOT_READY",
            "Paper preview is waiting for current inputs or capacity",
            rid,
        ),
        OwnerBetaPaperRepoError::RightsBlocked => code_error(
            "PAPER_RIGHTS_BLOCKED",
            "current Paper or recommendation rights are unavailable",
            rid,
        ),
        OwnerBetaPaperRepoError::SourceBlocked => code_error(
            "PAPER_SOURCE_BLOCKED",
            "current Paper source or runtime verification is blocked",
            rid,
        ),
        OwnerBetaPaperRepoError::IdempotencyMismatch => code_error(
            "IDEMPOTENCY_KEY_MISMATCH",
            "the Idempotency-Key was already used with a different request",
            rid,
        ),
        OwnerBetaPaperRepoError::CommitUnknown => code_error(
            "PAPER_COMMIT_UNKNOWN",
            "Paper action completion is unknown; read current Paper state before retrying",
            rid,
        ),
        OwnerBetaPaperRepoError::InvalidParameter => {
            code_error("INVALID_PARAMETER", "request parameter is invalid", rid)
        }
        OwnerBetaPaperRepoError::Integrity | OwnerBetaPaperRepoError::Internal => {
            code_error("INTERNAL", "internal error", rid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::state::{OwnerBetaAccessMode, OwnerBetaPaperMode};
    use auth::entitlement::{Role, UserId};
    use auth::sessions::SessionInfo;
    use axum::body::to_bytes;
    use axum::http::{HeaderMap, HeaderValue};
    use serde_json::Value;

    fn session(role: Role) -> Session {
        Session(SessionInfo {
            user_id: UserId::new("00000000-0000-0000-0000-000000000001"),
            role,
            auth_time_secs: 1,
            amr: vec!["pwd".into()],
            expires_at_secs: i64::MAX,
            csrf_token_hash: auth::sessions::cookie::hash("paper-csrf"),
        })
    }

    fn mutation_headers(with_key: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("paper-test"));
        headers.insert("x-csrf-token", HeaderValue::from_static("paper-csrf"));
        if with_key {
            headers.insert(
                crate::http::idempotency::HEADER,
                HeaderValue::from_static("paper-test-key"),
            );
        }
        headers
    }

    async fn error_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("bounded response body");
        serde_json::from_slice(&bytes).expect("error JSON")
    }

    #[tokio::test]
    async fn disabled_state_is_typed_and_does_not_open_paper_pools() {
        let state = ApiState::test_without_database_with_policy(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
        );
        let response =
            get_state_handler(State(state.clone()), session(Role::Owner), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("body");
        let body: Value = serde_json::from_slice(&bytes).expect("state JSON");
        assert_eq!(body["schema"], "owner-beta-paper-state-v1");
        assert_eq!(body["state"], "DISABLED");
        assert_eq!(state.app_pool.size(), 0);
        assert_eq!(state.admin_pool.size(), 0);
    }

    async fn get_state_handler(
        state: State<ApiState>,
        session: Session,
        headers: HeaderMap,
    ) -> Response {
        get_state(state, session, headers).await
    }

    #[tokio::test]
    async fn direct_handler_rejects_non_owner_even_when_feature_is_disabled() {
        let state = ApiState::test_without_database_with_policy(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
        );
        let member = session(Role::Member);
        let responses = [
            get_state(State(state.clone()), member.clone(), HeaderMap::new()).await,
            consent(
                State(state.clone()),
                member.clone(),
                HeaderMap::new(),
                JsonBody(OwnerBetaPaperConsentBody {
                    opt_in: true,
                    expected_epoch: 0,
                }),
            )
            .await,
            create_preview(
                State(state.clone()),
                member.clone(),
                HeaderMap::new(),
                JsonBody(OwnerBetaPaperPreviewBody {
                    recommendation_run_id: Uuid::from_u128(1),
                    expected_epoch: 0,
                }),
            )
            .await,
            get_preview(
                State(state.clone()),
                member.clone(),
                HeaderMap::new(),
                Path(Uuid::from_u128(2).to_string()),
            )
            .await,
            apply_preview(
                State(state.clone()),
                member.clone(),
                HeaderMap::new(),
                Path(Uuid::from_u128(2).to_string()),
                JsonBody(OwnerBetaPaperApplyBody {
                    preview_token: "a".repeat(64),
                    expected_epoch: 0,
                }),
            )
            .await,
            get_session(
                State(state.clone()),
                member,
                HeaderMap::new(),
                Path(Uuid::from_u128(3).to_string()),
            )
            .await,
        ];
        for response in responses {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(error_body(response).await["error"]["code"], "FORBIDDEN");
        }
        assert_eq!(state.app_pool.size(), 0);
    }

    #[tokio::test]
    async fn disabled_mutations_return_paper_disabled_before_csrf_or_database() {
        let state = ApiState::test_without_database_with_policy(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Disabled,
        );
        let owner = session(Role::Owner);
        let response = consent(
            State(state.clone()),
            owner.clone(),
            HeaderMap::new(),
            JsonBody(OwnerBetaPaperConsentBody {
                opt_in: true,
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = error_body(response).await;
        assert_eq!(body["error"]["code"], "PAPER_DISABLED");
        let disabled_responses = [
            create_preview(
                State(state.clone()),
                owner.clone(),
                HeaderMap::new(),
                JsonBody(OwnerBetaPaperPreviewBody {
                    recommendation_run_id: Uuid::from_u128(1),
                    expected_epoch: 0,
                }),
            )
            .await,
            get_preview(
                State(state.clone()),
                owner.clone(),
                HeaderMap::new(),
                Path(Uuid::from_u128(2).to_string()),
            )
            .await,
            apply_preview(
                State(state.clone()),
                owner.clone(),
                HeaderMap::new(),
                Path(Uuid::from_u128(2).to_string()),
                JsonBody(OwnerBetaPaperApplyBody {
                    preview_token: "a".repeat(64),
                    expected_epoch: 0,
                }),
            )
            .await,
            get_session(
                State(state.clone()),
                owner,
                HeaderMap::new(),
                Path(Uuid::from_u128(3).to_string()),
            )
            .await,
        ];
        for response in disabled_responses {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                error_body(response).await["error"]["code"],
                "PAPER_DISABLED"
            );
        }
        assert_eq!(state.app_pool.size(), 0);
    }

    #[tokio::test]
    async fn enabled_mutation_requires_csrf_key_and_valid_epoch_before_database() {
        let state = ApiState::test_without_database_with_policy(
            OwnerBetaAccessMode::OwnerOnly,
            OwnerBetaPaperMode::Enabled,
        );
        let missing_csrf = consent(
            State(state.clone()),
            session(Role::Owner),
            HeaderMap::new(),
            JsonBody(OwnerBetaPaperConsentBody {
                opt_in: true,
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            error_body(missing_csrf).await["error"]["code"],
            "CSRF_DENIED"
        );

        let missing_key = consent(
            State(state.clone()),
            session(Role::Owner),
            mutation_headers(false),
            JsonBody(OwnerBetaPaperConsentBody {
                opt_in: true,
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(missing_key.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_body(missing_key).await["error"]["code"],
            "IDEMPOTENCY_KEY_REQUIRED"
        );

        let negative_epoch = consent(
            State(state.clone()),
            session(Role::Owner),
            mutation_headers(true),
            JsonBody(OwnerBetaPaperConsentBody {
                opt_in: true,
                expected_epoch: -1,
            }),
        )
        .await;
        assert_eq!(negative_epoch.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_body(negative_epoch).await["error"]["code"],
            "INVALID_PARAMETER"
        );

        let preview_missing_csrf = create_preview(
            State(state.clone()),
            session(Role::Owner),
            HeaderMap::new(),
            JsonBody(OwnerBetaPaperPreviewBody {
                recommendation_run_id: Uuid::from_u128(1),
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(preview_missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            error_body(preview_missing_csrf).await["error"]["code"],
            "CSRF_DENIED"
        );

        let apply_missing_csrf = apply_preview(
            State(state.clone()),
            session(Role::Owner),
            HeaderMap::new(),
            Path(Uuid::from_u128(2).to_string()),
            JsonBody(OwnerBetaPaperApplyBody {
                preview_token: "a".repeat(64),
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(apply_missing_csrf.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            error_body(apply_missing_csrf).await["error"]["code"],
            "CSRF_DENIED"
        );

        let preview_missing_key = create_preview(
            State(state.clone()),
            session(Role::Owner),
            mutation_headers(false),
            JsonBody(OwnerBetaPaperPreviewBody {
                recommendation_run_id: Uuid::from_u128(1),
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(preview_missing_key.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_body(preview_missing_key).await["error"]["code"],
            "IDEMPOTENCY_KEY_REQUIRED"
        );

        let apply_missing_key = apply_preview(
            State(state.clone()),
            session(Role::Owner),
            mutation_headers(false),
            Path(Uuid::from_u128(2).to_string()),
            JsonBody(OwnerBetaPaperApplyBody {
                preview_token: "a".repeat(64),
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(apply_missing_key.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_body(apply_missing_key).await["error"]["code"],
            "IDEMPOTENCY_KEY_REQUIRED"
        );

        let preview_negative_epoch = create_preview(
            State(state.clone()),
            session(Role::Owner),
            mutation_headers(true),
            JsonBody(OwnerBetaPaperPreviewBody {
                recommendation_run_id: Uuid::from_u128(1),
                expected_epoch: -1,
            }),
        )
        .await;
        assert_eq!(preview_negative_epoch.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_body(preview_negative_epoch).await["error"]["code"],
            "INVALID_PARAMETER"
        );

        let apply_bad_token = apply_preview(
            State(state.clone()),
            session(Role::Owner),
            mutation_headers(true),
            Path(Uuid::from_u128(2).to_string()),
            JsonBody(OwnerBetaPaperApplyBody {
                preview_token: "A".repeat(64),
                expected_epoch: 0,
            }),
        )
        .await;
        assert_eq!(apply_bad_token.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_body(apply_bad_token).await["error"]["code"],
            "INVALID_PARAMETER"
        );
        assert_eq!(state.app_pool.size(), 0);
    }

    #[test]
    fn beta_request_bodies_reject_unknown_fields_and_noncanonical_uuid() {
        assert!(
            serde_json::from_str::<OwnerBetaPaperConsentBody>(
                r#"{"opt_in":true,"expected_epoch":0,"enable":true}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<OwnerBetaPaperPreviewBody>(
            r#"{"recommendation_run_id":"AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA","expected_epoch":0}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<OwnerBetaPaperApplyBody>(
                r#"{"preview_token":"xyz","expected_epoch":0,"enabled":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn canonical_route_uuid_and_preview_token_checks_are_closed() {
        let id = "00000000-0000-0000-0000-000000000001";
        assert!(canonical_path_uuid(id).is_some());
        assert!(canonical_path_uuid("00000000-0000-0000-0000-000000000000").is_none());
        assert!(canonical_path_uuid("00000000000000000000000000000001").is_none());
        assert!(is_bare_lower_hex(&"a".repeat(64), 64));
        assert!(!is_bare_lower_hex(
            &format!("sha256:{}", "a".repeat(64)),
            64
        ));
        assert!(!is_bare_lower_hex(&"A".repeat(64), 64));
    }

    #[test]
    fn idempotency_key_uses_shared_trim_and_96_printable_byte_limit() {
        let mut headers = HeaderMap::new();
        headers.insert(
            crate::http::idempotency::HEADER,
            HeaderValue::from_str(&format!(" {} ", "k".repeat(96))).unwrap(),
        );
        assert_eq!(idempotency_key(&headers, "rid").unwrap().len(), 96);

        headers.insert(
            crate::http::idempotency::HEADER,
            HeaderValue::from_str(&"k".repeat(97)).unwrap(),
        );
        let error = idempotency_key(&headers, "rid").unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}
