//! The HTTP seam (plan Todo 36).
//!
//! Everything above this trait — token serialization, rate limiting, retry
//! classification, order idempotency — is broker logic that must be provable
//! without a network. Everything below it is bytes on a socket. Putting the
//! seam here is what lets the whole adapter be tested against a versioned
//! simulator, and it is also what keeps the mock and live profiles from
//! differing by anything except which [`Transport`] is installed.
//!
//! Request and response types carry no secrets of their own: headers hold
//! [`Secret`] values so a logged request cannot disclose a token, and bodies
//! reach an audit record only through [`crate::error::redact_payload`].

use std::collections::BTreeMap;
use std::future::Future;

use crate::error::KisError;
use crate::secret::Secret;

/// One outbound broker call.
/// The header carrying the client order id.
///
/// A constant because FOUR places read or write it -- the REST client, the
/// simulator, the live transport, and its tests -- and a typo in any of them
/// silently degrades an `Ambiguous` error's `client_order_id` to "unknown".
/// That is the correlation id used to find the order at the broker, so it is
/// lost at exactly the moment it matters most.
pub const CLIENT_ORDER_ID_HEADER: &str = "x-client-order-id";

pub struct HttpRequest {
    pub method: &'static str,
    pub path: String,
    /// KIS transaction id. Part of the rate-limit key, not a decoration.
    pub tr_id: String,
    /// Non-sensitive headers.
    pub headers: BTreeMap<String, String>,
    /// Headers whose values must never be rendered (authorization, appkey).
    pub secret_headers: BTreeMap<String, Secret<String>>,
    /// Non-sensitive URL query parameters (market, symbol, dates, ...).
    pub query: Vec<(String, String)>,
    /// Query parameters containing account or other private identifiers.
    pub secret_query: Vec<(String, Secret<String>)>,
    pub body: Option<String>,
}

impl HttpRequest {
    pub fn get(path: impl Into<String>, tr_id: impl Into<String>) -> Self {
        Self {
            method: "GET",
            path: path.into(),
            tr_id: tr_id.into(),
            headers: BTreeMap::new(),
            secret_headers: BTreeMap::new(),
            query: Vec::new(),
            secret_query: Vec::new(),
            body: None,
        }
    }

    pub fn post(
        path: impl Into<String>,
        tr_id: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            method: "POST",
            path: path.into(),
            tr_id: tr_id.into(),
            headers: BTreeMap::new(),
            secret_headers: BTreeMap::new(),
            query: Vec::new(),
            secret_query: Vec::new(),
            body: Some(body.into()),
        }
    }

    pub fn with_header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.headers.insert(k.into(), v.into());
        self
    }

    pub fn with_secret_header(mut self, k: impl Into<String>, v: Secret<String>) -> Self {
        self.secret_headers.insert(k.into(), v);
        self
    }

    pub fn with_query(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.query.push((k.into(), v.into()));
        self
    }

    pub fn with_secret_query(mut self, k: impl Into<String>, v: Secret<String>) -> Self {
        self.secret_query.push((k.into(), v));
        self
    }
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Secret header VALUES are unrenderable by type; their NAMES are shown
        // because knowing which headers were attached is useful and harmless.
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("tr_id", &self.tr_id)
            .field("headers", &self.headers)
            .field(
                "secret_headers",
                &self.secret_headers.keys().collect::<Vec<_>>(),
            )
            .field("query", &self.query)
            .field(
                "secret_query",
                &self
                    .secret_query
                    .iter()
                    .map(|(key, _)| key)
                    .collect::<Vec<_>>(),
            )
            .field(
                "body",
                &self.body.as_deref().map(crate::error::redact_payload),
            )
            .finish()
    }
}

/// One broker reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

impl HttpResponse {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: body.into(),
        }
    }

    pub fn status(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body: body.into(),
        }
    }

    pub fn with_header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.headers.insert(k.into(), v.into());
        self
    }
}

/// Sends a request and returns the reply.
///
/// A transport reports only what it can prove: [`KisError::Connect`] when the
/// request never left, and [`KisError::Ambiguous`] when it was sent and no
/// usable reply came back. It must NEVER convert a timeout into a failure —
/// that judgement belongs to nobody, because the information does not exist.
#[allow(async_fn_in_trait)]
pub trait Transport: Send + Sync {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, KisError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_debug_never_renders_a_secret_header_value() {
        let req = HttpRequest::get(
            "/uapi/domestic-stock/v1/quotations/inquire-price",
            "FHKST01010100",
        )
        .with_header("content-type", "application/json")
        .with_secret_header(
            "authorization",
            Secret::new("Bearer eyJhbGciOiJIUzI1NiJ9".to_string()),
        )
        .with_secret_header("appkey", Secret::new("PSabc123".to_string()));

        let rendered = format!("{req:?}");
        assert!(!rendered.contains("eyJhbGciOiJIUzI1NiJ9"), "{rendered}");
        assert!(!rendered.contains("PSabc123"), "{rendered}");
        // Knowing WHICH headers were attached is useful and harmless.
        assert!(rendered.contains("authorization"));
        assert!(rendered.contains("appkey"));
        assert!(rendered.contains("FHKST01010100"));
    }

    #[test]
    fn a_request_debug_keeps_private_query_values_redacted() {
        let req = HttpRequest::get("/balance", "TTTC8434R")
            .with_query("FID_INPUT_ISCD", "069500")
            .with_secret_query("CANO", Secret::new("50123456".to_string()));

        let rendered = format!("{req:?}");
        assert!(rendered.contains("069500"), "{rendered}");
        assert!(rendered.contains("CANO"), "{rendered}");
        assert!(!rendered.contains("50123456"), "{rendered}");
    }

    #[test]
    fn a_request_body_is_redacted_in_debug() {
        let req = HttpRequest::post(
            "/uapi/domestic-stock/v1/trading/order-cash",
            "TTTC0802U",
            r#"{"CANO":"50123456","ORD_QTY":"10"}"#,
        );
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("50123456"),
            "an account number reached a debug rendering: {rendered}"
        );
        assert!(rendered.contains("ORD_QTY"), "{rendered}");
    }
}
