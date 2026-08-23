//! Correlation-id middleware: normalizes the `X-Request-Id` header (generate
//! a uuid when absent), echoes it on every response, records the request in
//! the Prometheus counters (status class only — no PII labels), and emits a
//! structured JSON log line with the correlation id (design §15.1). All
//! authenticated responses also carry `Cache-Control: no-store` (no shared
//! caching of authenticated data - the plan's Must-NOT list).

use crate::contract::owner_beta_applies;
use crate::http::error::{code_error, request_id};
use crate::http::session::session_from_headers;
use crate::http::state::ApiState;
use crate::observability::{log, metrics};
use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;

pub const HEADER: &str = "x-request-id";

/// The middleware fn mounted around the whole `/api/v1` router.
pub async fn correlation(req: Request, next: Next) -> Response {
    let started = Instant::now();
    let rid = match req.headers().get(HEADER) {
        Some(v) => v.clone(),
        None => HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
            .expect("uuid is a valid header value"),
    };
    let rid_str = rid.to_str().unwrap_or_default().to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let mut req = req;
    req.headers_mut().insert(HEADER, rid.clone());
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(HEADER, rid);
    resp.headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-store"));
    let status = resp.status();
    let status_class = status.as_u16() / 100;
    let class = format!("{status_class}");
    metrics::record_response(&class);
    if status_class == 4 || status_class == 5 {
        metrics::record_error(&class);
    }
    metrics::record_latency(started.elapsed().as_secs_f64());
    log::LogEvent::info("http.request")
        .correlation(&rid_str)
        .message(format!("{method} {path} -> {status}"))
        .emit();
    resp
}

/// One central admission boundary for the explicitly configured owner-only
/// beta.  It deliberately precedes product handlers, so direct API callers
/// cannot bypass a Web navigation restriction.  It has no effect unless the
/// non-secret runtime mode is exactly `owner_only`.
pub async fn owner_beta_admission(
    State(state): State<ApiState>,
    req: Request,
    next: Next,
) -> Response {
    if !state.cfg.owner_beta_access.requires_owner() || !owner_beta_applies(req.uri().path()) {
        return next.run(req).await;
    }

    let rid = request_id(req.headers());
    let session = match session_from_headers(&state, req.headers()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if !session.actor().is_owner() {
        return code_error("FORBIDDEN", "forbidden", &rid);
    }
    next.run(req).await
}
