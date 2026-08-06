//! The typed error envelope: `{error: {code, message, request_id, details?}}`
//! (design §12.1). Every failure path in the router funnels through
//! [`ApiError::into_response`] so clients can rely on a single shape.

use crate::contract::status_for_code;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;

/// One error envelope payload.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Build a typed error response with the stable code and envelope.
pub fn api_error(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
    request_id: &str,
    details: Option<Value>,
) -> Response {
    let body = ErrorEnvelope {
        error: ErrorDetail {
            code,
            message: message.into(),
            request_id: request_id.to_string(),
            details,
        },
    };
    let mut resp = (status, axum::Json(body)).into_response();
    resp.headers_mut().insert(
        "X-Request-Id",
        HeaderValue::from_str(request_id).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    resp
}

/// Build a typed error response for a code from the stable table.
pub fn code_error(code: &'static str, message: impl Into<String>, request_id: &str) -> Response {
    let status = status_for_code(code);
    api_error(status, code, message, request_id, None)
}

/// Shortcut for the current-handler request id header value.
pub fn request_id(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}
