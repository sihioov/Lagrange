//! Correlation-id middleware: normalizes the `X-Request-Id` header (generate
//! a uuid when absent) and echoes it on every response. All authenticated
//! responses also carry `Cache-Control: no-store` (no shared caching of
//! authenticated data - the plan's Must-NOT list).

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

pub const HEADER: &str = "x-request-id";

/// The middleware fn mounted around the whole `/api/v1` router.
pub async fn correlation(req: Request, next: Next) -> Response {
    let rid = match req.headers().get(HEADER) {
        Some(v) => v.clone(),
        None => HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
            .expect("uuid is a valid header value"),
    };
    let mut req = req;
    req.headers_mut().insert(HEADER, rid.clone());
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(HEADER, rid);
    resp.headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-store"));
    resp
}
