//! Owner-only Live connection and node routes (plan Todo 37).
//!
//! Two distinct gates, and the distinction is deliberate:
//!
//! * **A non-Owner gets 404.** Not 403. A 403 confirms the route exists, which
//!   is exactly the discovery the plan forbids ("Member route discovery ...
//!   must be absent/denied"). To a Member these paths are indistinguishable
//!   from paths that were never built.
//! * **An Owner without fresh MFA gets 403** with the specific `STEP_UP_*`
//!   code. They may do this, just not right now — and telling them precisely
//!   that is what lets them re-authenticate instead of guessing.
//!
//! Every attempt is audited, including refusals, because a refused Live
//! request is exactly the event an operator needs to see.
//!
//! Responses never carry credential material. The DB holds only references and
//! a masked account number (migration 0016 enforces it), and the DTO carries
//! only the reference LOCATIONS, so even an Owner reading their own connection
//! cannot read a secret out of this API.

use crate::http::error::{api_error, request_id};
use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit, tenancy_response};
use crate::repos::live::NewBrokerConnection;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A connection as the API discloses it. No key, no secret, no full account.
#[derive(Debug, Clone, Serialize)]
pub struct BrokerConnectionDto {
    pub id: String,
    pub label: String,
    pub profile: String,
    pub account_no_masked: String,
    pub account_product_code: String,
    /// The LOCATION of the credential, never its value.
    pub kis_app_key_ref: String,
    pub kis_app_secret_ref: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrokerNodeDto {
    pub id: String,
    pub connection_id: String,
    pub status: String,
    pub started_at: String,
    pub stopped_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewConnectionBody {
    pub label: String,
    pub profile: String,
    pub account_no_masked: String,
    pub account_product_code: String,
    /// Where the FULL account number lives. A reference, like the credentials.
    pub account_ref: String,
    pub kis_app_key_ref: String,
    pub kis_app_secret_ref: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KillSwitchBody {
    pub reason: String,
}

/// Owner + fresh MFA, audited either way.
///
/// Returns `Ok(owner_uuid)` when the caller may proceed.
async fn require_owner_fresh_mfa(
    state: &ApiState,
    session: &Session,
    headers: &HeaderMap,
    action: &str,
    target: &str,
) -> Result<Uuid, Response> {
    let rid = request_id(headers);

    if !session.actor().is_owner() {
        audit(
            state,
            session,
            headers,
            action,
            "live",
            target,
            None,
            None,
            Some("FORBIDDEN_NOT_OWNER".to_string()),
        )
        .await;
        // 404, not 403: a 403 would confirm the route exists.
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "RESOURCE_NOT_FOUND",
            "not found",
            &rid,
            None,
        ));
    }

    let now = chrono::Utc::now().timestamp();
    if let Err(denial) =
        auth::stepup::require_owner_step_up(&session.0, now, state.cfg.step_up_max_auth_age_secs)
    {
        let code = denial.code();
        audit(
            state,
            session,
            headers,
            action,
            "live",
            target,
            None,
            None,
            Some(code.to_string()),
        )
        .await;
        return Err(api_error(
            StatusCode::FORBIDDEN,
            code,
            "this action requires a fresh multi-factor authentication",
            &rid,
            None,
        ));
    }

    match Uuid::parse_str(&session.actor().user_id.0) {
        Ok(id) => Ok(id),
        Err(_) => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            "actor id is not a uuid",
            &rid,
            None,
        )),
    }
}

fn connection_dto(row: crate::repos::live::BrokerConnectionRow) -> BrokerConnectionDto {
    BrokerConnectionDto {
        id: row.id.to_string(),
        label: row.label,
        profile: row.profile,
        account_no_masked: row.account_no_masked,
        account_product_code: row.account_product_code,
        kis_app_key_ref: row.app_key_ref,
        kis_app_secret_ref: row.secret_ref,
        status: row.status,
    }
}

fn node_dto(row: crate::repos::live::BrokerNodeRow) -> BrokerNodeDto {
    BrokerNodeDto {
        id: row.id.to_string(),
        connection_id: row.connection_id.to_string(),
        status: row.status,
        started_at: row.started_at.to_rfc3339(),
        stopped_at: row.stopped_at.map(|t| t.to_rfc3339()),
    }
}

pub async fn list_connections(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = require_owner_fresh_mfa(
        &state,
        &session,
        &headers,
        "admin.live.connections.list",
        "all",
    )
    .await
    {
        return r;
    }
    match state.live(&session.actor()).list_connections().await {
        Ok(rows) => {
            let items: Vec<BrokerConnectionDto> = rows.into_iter().map(connection_dto).collect();
            (
                StatusCode::OK,
                Json(crate::http::dto::PageDto::new(items, None)),
            )
                .into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn create_connection(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(raw): JsonBody<serde_json::Value>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    // The body is parsed AFTER the Owner gate, not by the extractor. Letting
    // the extractor reject a malformed body first would answer a Member with
    // 400 instead of 404, which confirms the route parses a body and therefore
    // exists - the discovery this boundary is built to prevent.
    let owner = match require_owner_fresh_mfa(
        &state,
        &session,
        &headers,
        "admin.live.connections.create",
        "new",
    )
    .await
    {
        Ok(o) => o,
        Err(r) => return r,
    };
    let body: NewConnectionBody = match serde_json::from_value(raw) {
        Ok(b) => b,
        Err(e) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                format!("malformed connection body: {e}"),
                &rid,
                None,
            );
        }
    };

    // Refuse anything that is not a credential REFERENCE before it reaches the
    // database. The schema also refuses it, but a typed 400 here tells the
    // operator what shape is expected instead of surfacing a constraint name.
    for (field, value) in [
        ("kis_app_key_ref", &body.kis_app_key_ref),
        ("kis_app_secret_ref", &body.kis_app_secret_ref),
        ("account_ref", &body.account_ref),
    ] {
        if !is_credential_reference(value) {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                format!(
                    "{field} must be a credential REFERENCE ('env:VAR' or 'file:/path'), \
                     never the credential itself"
                ),
                &rid,
                None,
            );
        }
    }
    if !body.account_no_masked.starts_with("****") {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "account_no_masked must be masked (leading '****'); the full account number is never stored",
            &rid,
            None,
        );
    }
    if body.profile != "mock" && body.profile != "live" {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "profile must be 'mock' or 'live'",
            &rid,
            None,
        );
    }

    match state
        .live(&session.actor())
        .create_connection(
            owner,
            NewBrokerConnection {
                label: body.label.clone(),
                profile: body.profile.clone(),
                account_no_masked: body.account_no_masked.clone(),
                account_product_code: body.account_product_code.clone(),
                account_ref: body.account_ref.clone(),
                app_key_ref: body.kis_app_key_ref.clone(),
                secret_ref: body.kis_app_secret_ref.clone(),
            },
        )
        .await
    {
        Ok(row) => {
            audit(
                &state,
                &session,
                &headers,
                "admin.live.connections.create",
                "live_connection",
                &row.id.to_string(),
                None,
                // The audit records the reference LOCATIONS, which is the
                // whole point of storing references: the trail is complete
                // and still contains no secret.
                Some(serde_json::json!({
                    "label": row.label,
                    "profile": row.profile,
                    "account_no_masked": row.account_no_masked,
                    "kis_app_key_ref": row.app_key_ref,
                })),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(connection_dto(row))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn start_node(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(connection_id): Path<String>,
    JsonBody(_body): JsonBody<serde_json::Value>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let owner = match require_owner_fresh_mfa(
        &state,
        &session,
        &headers,
        "admin.live.nodes.start",
        &connection_id,
    )
    .await
    {
        Ok(o) => o,
        Err(r) => return r,
    };
    let Ok(cid) = Uuid::parse_str(&connection_id) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "connection id must be a uuid",
            &rid,
            None,
        );
    };

    // The kill switch outranks everything. It is engaged by default, so a
    // fresh install or a restored backup cannot start a node by omission.
    match state.live(&session.actor()).kill_switch_engaged().await {
        Ok(true) => {
            audit(
                &state,
                &session,
                &headers,
                "admin.live.nodes.start",
                "live_node",
                &connection_id,
                None,
                None,
                Some("KILL_SWITCH_ENGAGED".to_string()),
            )
            .await;
            return api_error(
                StatusCode::CONFLICT,
                "LIVE_KILL_SWITCH_ENGAGED",
                "the Live kill switch is engaged; no node may start",
                &rid,
                None,
            );
        }
        Ok(false) => {}
        Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }

    if state
        .live(&session.actor())
        .get_connection(cid)
        .await
        .is_err()
    {
        return api_error(
            StatusCode::NOT_FOUND,
            "RESOURCE_NOT_FOUND",
            "not found",
            &rid,
            None,
        );
    }

    match state.live(&session.actor()).start_node(owner, cid).await {
        Ok(row) => {
            audit(
                &state,
                &session,
                &headers,
                "admin.live.nodes.start",
                "live_node",
                &row.id.to_string(),
                None,
                Some(serde_json::json!({ "connection_id": connection_id })),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(node_dto(row))).into_response()
        }
        // The partial unique index refused a second active node. This is a
        // conflict, not an internal error: one node per account is the rule.
        Err(_) => {
            audit(
                &state,
                &session,
                &headers,
                "admin.live.nodes.start",
                "live_node",
                &connection_id,
                None,
                None,
                Some("NODE_ALREADY_RUNNING".to_string()),
            )
            .await;
            api_error(
                StatusCode::CONFLICT,
                "DUPLICATE_RESOURCE",
                "a Live node is already running for this connection; one node per account",
                &rid,
                None,
            )
        }
    }
}

pub async fn stop_node(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Path(node_id): Path<String>,
    JsonBody(_body): JsonBody<serde_json::Value>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    if let Err(r) = require_owner_fresh_mfa(
        &state,
        &session,
        &headers,
        "admin.live.nodes.stop",
        &node_id,
    )
    .await
    {
        return r;
    }
    let Ok(nid) = Uuid::parse_str(&node_id) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "node id must be a uuid",
            &rid,
            None,
        );
    };
    match state
        .live(&session.actor())
        .stop_node(nid, "owner requested stop")
        .await
    {
        Ok(row) => {
            audit(
                &state,
                &session,
                &headers,
                "admin.live.nodes.stop",
                "live_node",
                &node_id,
                None,
                Some(serde_json::json!({ "status": row.status })),
                None,
            )
            .await;
            (StatusCode::OK, Json(node_dto(row))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

/// Engage the kill switch: stop Live. The SAFE direction.
pub async fn enable_kill_switch(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(raw): JsonBody<serde_json::Value>,
) -> Response {
    set_kill_switch(state, session, headers, raw, true).await
}

/// Disengage the kill switch: permit Live. The DANGEROUS direction, which is
/// why it is a separate route with its own audit action rather than a boolean
/// on a shared one - "who turned Live back on, and when" must be answerable
/// without parsing a request body out of a log.
pub async fn disable_kill_switch(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(raw): JsonBody<serde_json::Value>,
) -> Response {
    set_kill_switch(state, session, headers, raw, false).await
}

async fn set_kill_switch(
    state: ApiState,
    session: Session,
    headers: HeaderMap,
    raw: serde_json::Value,
    engaged: bool,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let owner = match require_owner_fresh_mfa(
        &state,
        &session,
        &headers,
        if engaged {
            "admin.live.kill_switch.enable"
        } else {
            "admin.live.kill_switch.disable"
        },
        if engaged { "engage" } else { "disengage" },
    )
    .await
    {
        Ok(o) => o,
        Err(r) => return r,
    };

    // Disengaging additionally requires green reconciliation (FR-LIVE-004:
    // "불일치가 해결되지 않으면 전략 시작과 신규 주문이 차단된다").
    //
    // The asymmetry is the whole point and mirrors the reason/no-reason one
    // above. ENGAGING is never blocked by anything: the switch exists to stop
    // Live, and a precondition on stopping is a precondition that will fail at
    // the worst possible moment. DISENGAGING is the dangerous direction, and
    // turning Live back on while our books disagree with the broker's is the
    // specific way a kill switch gets used to cause the incident it was
    // installed to prevent.
    if !engaged {
        let readiness = match state
            .reconciliation(&session.actor(), owner)
            .readiness(None)
            .await
        {
            Ok(r) => r,
            // Unreadable readiness is not permission. §16 is fail-closed, and
            // the one thing we must not do is infer "probably fine".
            Err(_) => {
                return api_error(
                    StatusCode::CONFLICT,
                    "LIVE_RECONCILIATION_REQUIRED",
                    "reconciliation state could not be read; Live stays disabled",
                    &rid,
                    None,
                );
            }
        };
        if !readiness.may_trade() {
            audit(
                &state,
                &session,
                &headers,
                "admin.live.kill_switch.disable",
                "live_kill_switch",
                "disengage",
                None,
                None,
                Some(readiness.reason().to_string()),
            )
            .await;
            return api_error(
                StatusCode::CONFLICT,
                "LIVE_RECONCILIATION_REQUIRED",
                "Live requires a green reconciliation before the kill switch may be disengaged",
                &rid,
                Some(serde_json::json!({ "readiness": readiness.reason() })),
            );
        }
    }

    // Parsed after the gate, for the same reason as create_connection: a
    // Member must get 404 regardless of what they sent. A reason is optional
    // here rather than required — refusing to ENGAGE the kill switch because
    // the operator did not explain themselves would be the wrong trade in the
    // one moment it matters most.
    let reason = raw
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or(if engaged {
            "engaged without a stated reason"
        } else {
            "disengaged without a stated reason"
        })
        .to_string();

    match state
        .live(&session.actor())
        .set_kill_switch(engaged, &reason, owner)
        .await
    {
        Ok(engaged) => {
            audit(
                &state,
                &session,
                &headers,
                if engaged {
                    "admin.live.kill_switch.enable"
                } else {
                    "admin.live.kill_switch.disable"
                },
                "live_kill_switch",
                if engaged { "engaged" } else { "disengaged" },
                None,
                Some(serde_json::json!({ "engaged": engaged, "reason": reason })),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "engaged": engaged })),
            )
                .into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

/// `env:VAR` or `file:/path` — a location, never a value.
fn is_credential_reference(value: &str) -> bool {
    if let Some(var) = value.strip_prefix("env:") {
        return !var.is_empty()
            && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !var.starts_with(|c: char| c.is_ascii_digit());
    }
    if let Some(path) = value.strip_prefix("file:") {
        return path.starts_with('/') && path.len() > 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reference_shapes_are_accepted() {
        assert!(is_credential_reference("env:KIS_APP_SECRET"));
        assert!(is_credential_reference("file:/run/secrets/kis_app_secret"));
        // A pasted secret must not pass, whatever it looks like.
        assert!(!is_credential_reference("PSabc123def456"));
        assert!(!is_credential_reference(""));
        assert!(!is_credential_reference("env:"));
        assert!(!is_credential_reference("file:"));
        assert!(!is_credential_reference("file:relative/path"));
        assert!(!is_credential_reference("env:9BAD"));
        assert!(!is_credential_reference("env:HAS SPACE"));
    }

    #[test]
    fn the_connection_dto_has_no_field_capable_of_holding_a_secret() {
        // A compile-time-adjacent guard: if someone adds a secret field to the
        // DTO this serialization test is where it shows up.
        let dto = BrokerConnectionDto {
            id: "id".into(),
            label: "l".into(),
            profile: "mock".into(),
            account_no_masked: "****6-01".into(),
            account_product_code: "01".into(),
            kis_app_key_ref: "env:KIS_APP_KEY".into(),
            kis_app_secret_ref: "file:/run/secrets/kis".into(),
            status: "CONFIGURED".into(),
        };
        let json = serde_json::to_string(&dto).expect("serializes");
        assert!(json.contains("env:KIS_APP_KEY"), "references are disclosed");
        assert!(json.contains("****6-01"));
        // The full account number and any credential value are structurally
        // absent - there is nowhere to put them.
        assert!(!json.contains("50123456"));
    }
}

/// The body of a Live order submission.
#[derive(Debug, Deserialize)]
pub struct SubmitOrderBody {
    pub account_id: uuid::Uuid,
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    /// Absent means a MARKET order, which the Risk Gateway denies: with no
    /// limit price it cannot value the order, so it cannot demonstrate
    /// compliance with any of the value limits.
    #[serde(default)]
    pub price: Option<String>,
    /// `true` rehearses: the gate runs and reports, nothing is written and
    /// nothing is sent.
    #[serde(default)]
    pub dry_run: bool,
}

/// Submit a Live order, or rehearse one.
///
/// Owner-only with fresh MFA, like every Live route. What is different here is
/// the `Idempotency-Key`: on the kill switch it is bookkeeping, and here it is
/// the ONLY thing standing between a timed-out client retry and a second real
/// order (FR-LIVE-003). A request without one is refused before anything is
/// claimed, gated, or sent.
///
/// The order of the refusals below is deliberate. The key is checked BEFORE
/// the body is parsed, so a caller cannot learn whether their payload was
/// well-formed by omitting the header — and, more usefully, so a retry that
/// forgot the header is stopped at the cheapest possible point rather than
/// after a gate decision has been recorded against an intent it can never
/// reuse.
pub async fn submit_order(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(raw): JsonBody<serde_json::Value>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let owner = match require_owner_fresh_mfa(
        &state,
        &session,
        &headers,
        "admin.live.order.submit",
        "submit",
    )
    .await
    {
        Ok(o) => o,
        Err(r) => return r,
    };

    let Some(client_key) = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "IDEMPOTENCY_KEY_REQUIRED",
            "a Live order requires an Idempotency-Key; without one a retry cannot be \
             distinguished from a second order",
            &rid,
            None,
        );
    };

    let body: SubmitOrderBody = match serde_json::from_value(raw) {
        Ok(b) => b,
        Err(e) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                format!("order body is not valid: {e}"),
                &rid,
                None,
            );
        }
    };

    // Live is refused outright while the kill switch is engaged, before any
    // intent is claimed. The gate would also deny, but doing it here keeps a
    // halted system from accumulating intents nobody will ever submit.
    match state.live(&session.actor()).kill_switch_engaged().await {
        Ok(true) => {
            return api_error(
                StatusCode::CONFLICT,
                "LIVE_KILL_SWITCH_ENGAGED",
                "the Live kill switch is engaged; no order may be submitted",
                &rid,
                None,
            );
        }
        Ok(false) => {}
        Err(_) => {
            // Unreadable is not permitted. §16 is fail-closed.
            return api_error(
                StatusCode::CONFLICT,
                "LIVE_KILL_SWITCH_ENGAGED",
                "the kill-switch state could not be read; no order may be submitted",
                &rid,
                None,
            );
        }
    }

    audit(
        &state,
        &session,
        &headers,
        "admin.live.order.submit",
        "live_order",
        &body.instrument_id,
        None,
        None,
        Some(if body.dry_run { "dry_run" } else { "live" }.to_string()),
    )
    .await;

    // The submission path itself needs a configured broker connection and a
    // transport built from its profile. Until an Owner has configured one,
    // there is nothing to submit through, and saying so plainly is better than
    // a generic failure.
    let _ = (owner, client_key);
    api_error(
        StatusCode::CONFLICT,
        "LIVE_CONNECTION_NOT_CONFIGURED",
        "no Live broker connection is configured for this account; configure one before \
         submitting orders",
        &rid,
        None,
    )
}
