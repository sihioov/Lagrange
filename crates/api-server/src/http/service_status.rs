//! Authenticated, read-only Owner diagnostics surface.

use crate::http::error::{api_error, code_error, request_id};
use crate::http::session::session_from_headers;
use crate::http::state::ApiState;
use crate::repos::audit::AuditEntry;
use auth::entitlement::Actor;
use auth::sessions::cookie;
use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::time::timeout;

const AUDIT_BUDGET: Duration = Duration::from_millis(500);
const HANDLER_BUDGET: Duration = Duration::from_secs(3);
const AUDIT_ACTION: &str = "admin.service_status.read";
const AUDIT_TARGET_TYPE: &str = "service_status";
const AUDIT_TARGET_ID: &str = "all";

/// Apply the endpoint browser/cache policy after correlation middleware.
pub async fn private_headers(request: Request, next: Next) -> Response {
    let status_path = request.uri().path() == "/api/v1/admin/service-status";
    let mut response = next.run(request).await;
    if status_path {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Cookie"));
    }
    response
}

pub async fn get(
    State(state): State<ApiState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    match timeout(HANDLER_BUDGET, authenticated_get(state, uri, headers)).await {
        Ok(response) => response,
        Err(_) => unavailable(&rid),
    }
}

async fn authenticated_get(state: ApiState, uri: axum::http::Uri, headers: HeaderMap) -> Response {
    let rid = request_id(&headers);
    let session_info = match session_from_headers(&state, &headers).await {
        Ok(session) => session,
        Err(response) if response.status().is_server_error() => return unavailable(&rid),
        Err(response) => return response,
    };
    let actor = session_info.actor();
    let session_hash = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie::parse(value, cookie::NAME))
        .map(|value| cookie::hash(&value));
    let Some(session_hash) = session_hash else {
        return code_error("SESSION_UNKNOWN", "session required", &rid);
    };

    let admitted_at = Instant::now();
    if state
        .service_status_runtime
        .admit(&actor.user_id.0, &session_hash, admitted_at)
        .is_err()
    {
        return rate_limited(&rid);
    }

    if !actor.is_owner() {
        if audit(&state, &actor, &headers, Some("DENIED"))
            .await
            .is_err()
        {
            return unavailable(&rid);
        }
        return code_error("FORBIDDEN", "forbidden", &rid);
    }

    if uri.query().is_some() {
        if audit(&state, &actor, &headers, None).await.is_err() {
            return unavailable(&rid);
        }
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            "query parameters are not supported",
            &rid,
            None,
        );
    }

    // The refresh has a two second evidence deadline and the mandatory audit
    // has a separate 500ms deadline, keeping the admitted path below 3s.
    serve_owner(&state, &actor, &session_hash, &headers).await
}

async fn serve_owner(
    state: &ApiState,
    actor: &Actor,
    session_hash: &str,
    headers: &HeaderMap,
) -> Response {
    let rid = request_id(headers);
    let evidence_started = Instant::now();
    let now = chrono::Utc::now();
    let runtime = state.service_status_runtime.clone();
    let release = state.cfg.code_commit.clone();
    let host = match timeout(
        Duration::from_secs(2),
        tokio::task::spawn_blocking(move || runtime.observe_host(now, &release)),
    )
    .await
    {
        Ok(Ok(host)) => host,
        _ => {
            let _ = audit(state, actor, headers, Some("SOURCE_UNAVAILABLE")).await;
            return unavailable(&rid);
        }
    };
    serve_owner_with_observation(
        state,
        actor,
        session_hash,
        headers,
        now,
        host,
        evidence_started,
    )
    .await
}

// Both production serving and isolated file-identity tests enter the same
// cache/read/audit/body path. Only serve_owner chooses the fixed production
// root/UID; there is no route, environment or configuration override.
async fn serve_owner_with_observation(
    state: &ApiState,
    actor: &Actor,
    session_hash: &str,
    headers: &HeaderMap,
    now: chrono::DateTime<chrono::Utc>,
    host: crate::service_status::HostEvidence,
    evidence_started: Instant,
) -> Response {
    let rid = request_id(headers);
    let cached = state.service_status_runtime.cached_owner_evidence(
        &actor.user_id.0,
        session_hash,
        &host,
        Instant::now(),
        now,
    );
    let input = if let Some(input) = cached {
        input
    } else {
        let Some(_permit) = state.service_status_runtime.try_refresh() else {
            if audit(state, actor, headers, Some("SOURCE_UNAVAILABLE"))
                .await
                .is_err()
            {
                return unavailable(&rid);
            }
            return unavailable(&rid);
        };
        if let Some(input) = state.service_status_runtime.cached_owner_evidence(
            &actor.user_id.0,
            session_hash,
            &host,
            Instant::now(),
            chrono::Utc::now(),
        ) {
            input
        } else {
            let shared = state
                .service_status_runtime
                .cached_shared_for_host(&host, Instant::now());
            let _shared_permit = if shared.is_none() {
                let Some(permit) = state.service_status_runtime.try_shared_refresh() else {
                    if audit(state, actor, headers, Some("SOURCE_UNAVAILABLE"))
                        .await
                        .is_err()
                    {
                        return unavailable(&rid);
                    }
                    return unavailable(&rid);
                };
                Some(permit)
            } else {
                None
            };
            let refreshed = timeout(
                Duration::from_secs(2).saturating_sub(evidence_started.elapsed()),
                crate::repos::service_status::refresh_with_host(
                    state,
                    actor,
                    session_hash,
                    now,
                    shared.as_ref(),
                    &host,
                ),
            )
            .await;
            match refreshed {
                Ok(input) => {
                    if shared.is_none() {
                        state.service_status_runtime.cache_shared_for_host(
                            &host,
                            crate::repos::service_status::extract_shared_evidence(&input),
                            Instant::now(),
                        );
                    }
                    state.service_status_runtime.cache_owner_evidence(
                        &actor.user_id.0,
                        session_hash,
                        &host,
                        input.clone(),
                        Instant::now(),
                    );
                    input
                }
                Err(_) => {
                    if audit(state, actor, headers, Some("SOURCE_UNAVAILABLE"))
                        .await
                        .is_err()
                    {
                        return unavailable(&rid);
                    }
                    return unavailable(&rid);
                }
            }
        }
    };

    let response_now = chrono::Utc::now();
    let host_error = input.get("host_error").and_then(Value::as_str);
    let payload =
        match crate::service_status::project_with_host_error(&input, response_now, host_error) {
            Ok(payload) => payload,
            Err(_) => {
                if audit(state, actor, headers, Some("SOURCE_UNAVAILABLE"))
                    .await
                    .is_err()
                {
                    return unavailable(&rid);
                }
                return unavailable(&rid);
            }
        };

    if audit(state, actor, headers, audit_reason(&payload))
        .await
        .is_err()
    {
        return unavailable(&rid);
    }
    (StatusCode::OK, axum::Json(payload)).into_response()
}

fn audit_reason(payload: &Value) -> Option<&'static str> {
    let services = payload.get("services").and_then(Value::as_array)?;
    let status_issue = services.iter().any(|service| {
        matches!(
            service.get("status").and_then(Value::as_str),
            Some("failed" | "degraded" | "unknown")
        )
    });
    if !status_issue {
        return None;
    }
    let mut any_source = false;
    let mut any_usable = false;
    for service in services {
        for source in ["host", "database", "window"] {
            if let Some(result) = service
                .pointer(&format!("/sources/{source}/result"))
                .and_then(Value::as_str)
            {
                any_source = true;
                any_usable |= result == "ok";
            }
        }
    }
    Some(if any_source && !any_usable {
        "SOURCE_UNAVAILABLE"
    } else {
        "PARTIAL"
    })
}

async fn audit(
    state: &ApiState,
    actor: &Actor,
    headers: &HeaderMap,
    reason: Option<&str>,
) -> Result<(), ()> {
    let entry = AuditEntry {
        action: AUDIT_ACTION.to_owned(),
        target_type: AUDIT_TARGET_TYPE.to_owned(),
        target_id: AUDIT_TARGET_ID.to_owned(),
        before_json: None,
        after_json: None,
        reason: reason.map(str::to_owned),
        correlation_id: Some(request_id(headers)),
    };
    timeout(AUDIT_BUDGET, state.audit_writer().record(actor, &entry))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

fn rate_limited(rid: &str) -> Response {
    let mut response = api_error(
        StatusCode::TOO_MANY_REQUESTS,
        "SERVICE_STATUS_RATE_LIMITED",
        "service status request limit reached",
        rid,
        None,
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    response
}

fn unavailable(rid: &str) -> Response {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "SERVICE_STATUS_UNAVAILABLE",
        "service status is temporarily unavailable",
        rid,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn audit_reason_uses_only_fixed_vocabulary() {
        assert_eq!(
            audit_reason(
                &json!({"services":[{"status":"healthy","sources":{"host":{"result":"ok"}}}]})
            ),
            None
        );
        assert_eq!(
            audit_reason(
                &json!({"services":[{"status":"unknown","sources":{"host":{"result":"missing"}}}]})
            ),
            Some("SOURCE_UNAVAILABLE")
        );
        assert_eq!(
            audit_reason(
                &json!({"services":[{"status":"degraded","sources":{"host":{"result":"ok"}}}]})
            ),
            Some("PARTIAL")
        );
    }

    struct SyntheticHost(std::path::PathBuf);
    impl Drop for SyntheticHost {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn shifted_snapshot(now: chrono::DateTime<chrono::Utc>, release: &str) -> Value {
        fn shift(value: &mut Value, delta: chrono::Duration) {
            match value {
                Value::String(text) => {
                    if let Ok(at) = chrono::DateTime::parse_from_rfc3339(text) {
                        *text = (at.with_timezone(&chrono::Utc) + delta)
                            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        shift(value, delta);
                    }
                }
                Value::Object(values) => {
                    for value in values.values_mut() {
                        shift(value, delta);
                    }
                }
                _ => {}
            }
        }
        let input: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/service-status/base-input.json"
        ))
        .unwrap();
        let clock = chrono::DateTime::parse_from_rfc3339(input["clock"].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc);
        let now = chrono::DateTime::from_timestamp_micros(now.timestamp_micros()).unwrap();
        let mut snapshot = input["host_read"]["snapshot"].clone();
        shift(&mut snapshot, now - clock);
        snapshot["policy"]["release_commit"] = json!(release);
        snapshot
    }

    fn replace_snapshot(root: &std::path::Path, snapshot: &Value) {
        use std::os::unix::fs::PermissionsExt;
        let next = root.join("snapshot.next");
        std::fs::write(&next, serde_json::to_vec(snapshot).unwrap()).unwrap();
        std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::fs::rename(next, root.join("snapshot.json")).unwrap();
    }

    fn service<'a>(body: &'a Value, id: &str) -> &'a Value {
        body["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap()
    }

    fn assert_wire_clocks(value: &Value) {
        match value {
            Value::String(text) if text.ends_with('Z') && text.contains('T') => {
                chrono::DateTime::parse_from_rfc3339(text).expect("real response UTC instant");
                if let Some((_, fraction)) = text.strip_suffix('Z').unwrap().split_once('.') {
                    assert!(
                        fraction.len() <= 6,
                        "accepted wire cannot carry a nanosecond instant: {text}"
                    );
                }
            }
            Value::Array(values) => {
                for value in values {
                    assert_wire_clocks(value);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    assert_wire_clocks(value);
                }
            }
            _ => {}
        }
    }

    async fn serve_synthetic_host(
        state: &ApiState,
        actor: &Actor,
        session: &str,
        root: &std::path::Path,
        headers: &HeaderMap,
    ) -> Value {
        let start = Instant::now();
        let now = chrono::Utc::now();
        let owner = (unsafe { libc::geteuid() }, unsafe { libc::getegid() });
        let host = state.service_status_runtime.observe_host_for_test(
            root,
            owner,
            now,
            &state.cfg.code_commit,
        );
        let response =
            serve_owner_with_observation(state, actor, session, headers, now, host, start).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = crate::service_status_test_common::Harness::body_json(response).await;
        assert_wire_clocks(&body);
        body
    }

    #[tokio::test]
    async fn actual_serving_path_reopens_same_commit_policy_and_revalidates_auth_before_cache() {
        use crate::service_status_test_common::Harness;
        use std::os::unix::fs::PermissionsExt;
        assert!(
            std::env::var("DATABASE_URL").is_ok_and(|url| !url.trim().is_empty()),
            "this actual serving test requires the disposable DATABASE_URL; never skip"
        );
        let h = Harness::new().await.expect("required actual DB harness");
        let state = h.state();
        let root = SyntheticHost(
            std::env::temp_dir().join(format!("service-status-serving-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir(&root.0).unwrap();
        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o750)).unwrap();
        let mut snapshot = shifted_snapshot(chrono::Utc::now(), &state.cfg.code_commit);
        replace_snapshot(&root.0, &snapshot);
        let actor = h.owner.actor();
        let session = cookie::hash(&h.owner.cookie_value);
        // Supply the preceding production admission for the private serving
        // seam. Its monotonic clock is fixed independently of real SQL clocks;
        // the later full GET can be admitted while the new evidence is cached.
        state
            .service_status_runtime
            .admit(
                &actor.user_id.0,
                &session,
                Instant::now() - Duration::from_secs(5),
            )
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{}={}", cookie::NAME, h.owner.cookie_value)).unwrap(),
        );
        headers.insert(
            "x-request-id",
            HeaderValue::from_static("service-status-serving-test"),
        );
        let mut tx = h.owner_pool.begin().await.unwrap();
        crate::actor_tx::set_actor_guc(&mut tx, &actor)
            .await
            .unwrap();
        sqlx::query("INSERT INTO jobs(owner_user_id,job_type,status,available_at,locked_at) VALUES($1,'recommendation','RUNNING',clock_timestamp()-interval '100 seconds',clock_timestamp()-interval '90 seconds')").bind(h.owner.user_id).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        sqlx::query(
            "UPDATE candidate_scheduler_control SET active=false WHERE control_key='scheduler'",
        )
        .execute(&h.owner_pool)
        .await
        .unwrap();
        let first = serve_synthetic_host(&state, &actor, &session, &root.0, &headers).await;
        assert_eq!(
            service(&first, "recommendation-runner")["metrics"]["expired_claims"],
            1
        );
        let cached = serve_synthetic_host(&state, &actor, &session, &root.0, &headers).await;
        assert_eq!(
            service(&cached, "recommendation-runner")["sources"]["database"],
            service(&first, "recommendation-runner")["sources"]["database"]
        );
        sqlx::query(
            "UPDATE candidate_scheduler_control SET active=true WHERE control_key='scheduler'",
        )
        .execute(&h.owner_pool)
        .await
        .unwrap();
        snapshot["policy"]["job_lease_seconds"]["recommendation"] = json!(120);
        replace_snapshot(&root.0, &snapshot);
        let changed = serve_synthetic_host(&state, &actor, &session, &root.0, &headers).await;
        assert_eq!(
            service(&changed, "recommendation-runner")["metrics"]["expired_claims"],
            0,
            "same-commit installed lease invalidates actor evidence"
        );
        assert_eq!(
            service(&changed, "recommendation-runner")["work_status"],
            "active"
        );
        assert!(
            service(&changed, "candidate-runner")["reason_codes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|code| code != "SCHEDULER_DISABLED"),
            "same identity/policy invalidates shared source evidence too"
        );
        assert_ne!(
            service(&changed, "recommendation-runner")["sources"]["database"]["observed_at"],
            service(&first, "recommendation-runner")["sources"]["database"]["observed_at"]
        );
        snapshot["policy"]["research_mode"] = json!("daemon");
        snapshot["policy"]["profile"] = json!("owner_daemon");
        snapshot["observations"][10]["process"]["state"] = json!("running");
        snapshot["observations"][10]["process"]["health"] = json!("healthy");
        snapshot["observations"][11]["timer"]["active"] = json!("inactive");
        snapshot["observations"][11]["timer"]["enabled"] = json!("disabled");
        replace_snapshot(&root.0, &snapshot);
        let daemon = serve_synthetic_host(&state, &actor, &session, &root.0, &headers).await;
        assert_eq!(service(&daemon, "kis-eod")["expected_mode"], "daemon");
        assert!(service(&daemon, "kis-eod")["metrics"]["next_run_at"].is_null());
        let installed = chrono::DateTime::parse_from_rfc3339(
            snapshot["policy"]["policy_installed_at"].as_str().unwrap(),
        )
        .unwrap()
            + chrono::Duration::seconds(1);
        snapshot["policy"]["policy_installed_at"] =
            json!(installed.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
        replace_snapshot(&root.0, &snapshot);
        let receipt = serve_synthetic_host(&state, &actor, &session, &root.0, &headers).await;
        assert_ne!(
            service(&receipt, "recommendation-runner")["sources"]["database"]["observed_at"],
            service(&daemon, "recommendation-runner")["sources"]["database"]["observed_at"]
        );
        std::fs::set_permissions(
            root.0.join("snapshot.json"),
            std::fs::Permissions::from_mode(0o660),
        )
        .unwrap();
        let unsafe_file = serve_synthetic_host(&state, &actor, &session, &root.0, &headers).await;
        assert_eq!(
            service(&unsafe_file, "recommendation-runner")["sources"]["host"]["result"],
            "invalid"
        );
        assert_eq!(
            service(&unsafe_file, "recommendation-runner")["process_status"],
            "unknown"
        );
        assert_eq!(unsafe_file["policy_state"], "unavailable");
        let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_user_id=$1 AND action='admin.service_status.read'").bind(h.owner.user_id).fetch_one(&h.admin_pool).await.unwrap();
        assert_eq!(
            audit_count, 6,
            "every success/cache/replacement request is audited"
        );
        // The real GET must re-read role/session even with actor evidence cached.
        sqlx::query("UPDATE user_roles SET role_id='member' WHERE user_id=$1")
            .bind(h.owner.user_id)
            .execute(&h.owner_pool)
            .await
            .unwrap();
        let denied = get(
            State(state.clone()),
            OriginalUri("/api/v1/admin/service-status".parse().unwrap()),
            headers.clone(),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let denied = Harness::body_json(denied).await;
        assert!(denied.get("services").is_none());
        assert!(denied["error"]["details"].is_null());
        let mut tx = h.owner_pool.begin().await.unwrap();
        crate::actor_tx::set_actor_guc(&mut tx, &actor)
            .await
            .unwrap();
        sqlx::query("UPDATE web_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1")
            .bind(h.owner.user_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let revoked = get(
            State(state.clone()),
            OriginalUri("/api/v1/admin/service-status".parse().unwrap()),
            headers,
        )
        .await;
        assert_eq!(
            revoked.status(),
            StatusCode::UNAUTHORIZED,
            "session validation precedes both cached evidence and rate admission"
        );
        let revoked = Harness::body_json(revoked).await;
        assert!(revoked.get("services").is_none());
        assert!(revoked["error"]["details"].is_null());
        let reasons: Vec<Option<String>> = sqlx::query_scalar("SELECT reason FROM audit_logs WHERE actor_user_id=$1 AND action='admin.service_status.read' ORDER BY created_at").bind(h.owner.user_id).fetch_all(&h.admin_pool).await.unwrap();
        assert_eq!(reasons.len(), 7);
        assert_eq!(reasons.last().unwrap().as_deref(), Some("DENIED"));
        drop(state);
        h.teardown().await;
    }
}
