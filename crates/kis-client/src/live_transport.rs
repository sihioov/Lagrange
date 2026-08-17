//! The real HTTP transport (plan Todo 36's deferred half).
//!
//! Everything else in this crate has been provable without a network because
//! [`crate::transport::Transport`] is a trait and [`crate::simulator`]
//! implements it. This is the implementation that actually talks to KIS.
//!
//! # Why this could be written without credentials
//!
//! It was deferred repeatedly for "want of credentials", and that reason was
//! never true of the CODE — only of pointing it at a real account. A HTTP
//! client needs no account to be written, and its most important behaviour
//! (how it classifies a failure) needs no account to be tested either, because
//! that classification is a pure function extracted below.
//!
//! # The property that matters more than the request
//!
//! **This transport must classify failures identically to the simulator.**
//! Every test in this crate is written against `BrokerSimulator`; if the real
//! transport disagreed with it about what a timeout means, those tests would
//! prove nothing about production, and the one thing they most appear to prove
//! — that a timed-out order is never resubmitted — would be exactly the thing
//! that broke. [`classify`] is shared reasoning, and
//! `live_transport_matches_the_simulator_classification` pins the agreement.
//!
//! The rule, from `simulator.rs`:
//!
//! * the connection never opened  -> `Connect`, and a mutation MAY be retried,
//!   because a request that never left changed nothing;
//! * a POST timed out             -> `Ambiguous`, and a mutation must NOT be
//!   retried, because the broker may hold the order;
//! * a GET timed out              -> `Broker { status: 504 }`, because a read
//!   that never answered changed nothing.
//!
//! reqwest cannot tell a connect timeout from a response timeout — both are
//! `is_timeout()`. That ambiguity is resolved in the ONLY safe direction: a
//! POST timeout is treated as ambiguous even when the connection may never
//! have opened. Being wrong that way costs a manual lookup; being wrong the
//! other way costs a duplicate live order.

use crate::error::KisError;
use crate::transport::{CLIENT_ORDER_ID_HEADER, HttpRequest, HttpResponse, Transport};
use std::time::Duration;

/// The base URL for each profile.
///
/// Separate constants rather than a format string with a flag: the live and
/// sandbox hosts are different systems, and a transport that could be pointed
/// at production by flipping a boolean is one bad default away from doing so.
pub const LIVE_BASE_URL: &str = "https://openapi.koreainvestment.com:9443";
pub const SANDBOX_BASE_URL: &str = "https://openapivts.koreainvestment.com:29443";

/// How a request failed, independent of the HTTP library.
///
/// Exists so the classification below can be tested without a network. The
/// mapping from a failure to a `KisError` is the part that can place a
/// duplicate order if it is wrong, and it is the part a network test would
/// exercise least reliably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The connection never opened: DNS, refused, TLS handshake. The request
    /// did not leave this machine.
    NeverSent,
    /// Sent, but no usable reply arrived in time. What the broker did with it
    /// is unknown.
    TimedOut,
    /// The reply arrived but could not be read as a body.
    UnreadableBody,
}

/// Maps a failure to the error the rest of the crate reasons about.
///
/// Pure, and deliberately mirrors `BrokerSimulator`'s own arms. If these two
/// ever disagree, the simulator stops being a stand-in for the broker and
/// every test written against it becomes decoration.
pub fn classify(failure: Failure, request: &HttpRequest) -> KisError {
    match failure {
        // Never sent is SAFE: the caller may retry, including a mutation.
        Failure::NeverSent => KisError::Connect {
            reason: format!(
                "{} {} did not leave this host",
                request.method, request.path
            ),
        },
        Failure::TimedOut if request.method == "POST" => KisError::Ambiguous {
            operation: format!("{} {}", request.method, request.path),
            // Without this the correlation id is lost at the exact moment it
            // is needed -- to find the order at the broker.
            client_order_id: request
                .headers
                .get(CLIENT_ORDER_ID_HEADER)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
        },
        // A read that never answered changed nothing, so it is a plain failed
        // read rather than an unresolved question.
        Failure::TimedOut => KisError::Broker {
            status: 504,
            endpoint: request.path.clone(),
            body: "gateway timeout".to_string(),
        },
        Failure::UnreadableBody => KisError::SchemaDrift {
            endpoint: request.path.clone(),
            detail: "response body was not readable as UTF-8 text".to_string(),
        },
    }
}

/// A reqwest-backed [`Transport`].
pub struct LiveTransport {
    client: reqwest::Client,
    base_url: String,
}

impl LiveTransport {
    /// Builds a transport for a base URL.
    ///
    /// The timeout is REQUIRED rather than optional. A request with no timeout
    /// can hang for as long as the operating system allows, and an order that
    /// hangs is an order whose state nobody knows -- the worst outcome this
    /// crate exists to avoid. A caller who wants to wait longer must say so.
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self, KisError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            // No automatic redirect following. A broker that redirects an
            // order POST is a broker doing something unexpected, and silently
            // re-sending the body to a new host is not a decision a transport
            // should make on its own.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| KisError::Connect {
                reason: format!("HTTP client could not be built: {e}"),
            })?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    /// The live endpoint. Named explicitly so reaching production is a
    /// deliberate call rather than a default.
    pub fn live(timeout: Duration) -> Result<Self, KisError> {
        Self::new(LIVE_BASE_URL, timeout)
    }

    /// The sandbox endpoint.
    pub fn sandbox(timeout: Duration) -> Result<Self, KisError> {
        Self::new(SANDBOX_BASE_URL, timeout)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn build_request(&self, request: &HttpRequest) -> Result<reqwest::Request, KisError> {
        let url = format!("{}{}", self.base_url, request.path);
        let mut builder = match request.method {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            other => {
                return Err(KisError::Connect {
                    reason: format!("unsupported method {other}"),
                });
            }
        };

        for (k, v) in &request.headers {
            builder = builder.header(k, v);
        }
        for (k, v) in &request.secret_headers {
            builder = builder.header(k, v.expose());
        }
        builder = builder.header("tr_id", &request.tr_id);
        for (key, value) in &request.query {
            builder = builder.query(&[(key, value)]);
        }
        for (key, value) in &request.secret_query {
            builder = builder.query(&[(key, value.expose())]);
        }
        if let Some(body) = &request.body {
            builder = builder
                .header("content-type", "application/json; charset=utf-8")
                .body(body.clone());
        }
        builder.build().map_err(|error| KisError::Connect {
            reason: format!("HTTP request could not be built: {error}"),
        })
    }
}

impl Transport for LiveTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, KisError> {
        let outbound = self.build_request(&request)?;

        let response = match self.client.execute(outbound).await {
            Ok(r) => r,
            Err(e) => {
                // `is_connect()` is the only reqwest signal that proves the
                // request never left. Everything else -- including a timeout
                // that MIGHT have been a connect timeout -- is treated as
                // sent, which is the fail-closed reading for a mutation.
                let failure = if e.is_connect() {
                    Failure::NeverSent
                } else {
                    Failure::TimedOut
                };
                return Err(classify(failure, &request));
            }
        };

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
            .collect();
        // A body that fails to read is NOT a timeout: the reply arrived, so a
        // mutation is not ambiguous -- we simply could not parse what came
        // back, which is drift rather than an unresolved question.
        let body = match response.text().await {
            Ok(b) => b,
            Err(_) => return Err(classify(Failure::UnreadableBody, &request)),
        };

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;

    fn post(coid: &str) -> HttpRequest {
        HttpRequest::post(
            "/uapi/domestic-stock/v1/trading/order-cash",
            "TTTC0802U",
            "{}",
        )
        .with_header(CLIENT_ORDER_ID_HEADER, coid)
        .with_secret_header("authorization", Secret::new("token".to_string()))
    }

    #[test]
    fn a_request_that_never_left_is_safe_to_retry() {
        // The distinction the whole module turns on: nothing reached the
        // broker, so a mutation may go again.
        let err = classify(Failure::NeverSent, &post("coid-1"));
        assert!(matches!(err, KisError::Connect { .. }), "{err:?}");
        assert!(
            !err.is_ambiguous(),
            "a request that never left is not ambiguous"
        );
    }

    #[test]
    fn a_timed_out_mutation_is_ambiguous_and_carries_its_correlation_id() {
        let err = classify(Failure::TimedOut, &post("coid-42"));
        match err {
            KisError::Ambiguous {
                client_order_id, ..
            } => assert_eq!(
                client_order_id, "coid-42",
                "the correlation id is how the order is found at the broker; \
                 losing it here is losing it at the worst moment"
            ),
            other => panic!("a timed-out POST must be ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn a_timed_out_read_is_merely_a_failed_read() {
        // A read that never answered changed nothing, so calling it ambiguous
        // would block a retry that is perfectly safe.
        let get = HttpRequest::get(
            "/uapi/domestic-stock/v1/quotations/inquire-price",
            "FHKST01010100",
        );
        let err = classify(Failure::TimedOut, &get);
        assert!(
            matches!(err, KisError::Broker { status: 504, .. }),
            "{err:?}"
        );
        assert!(!err.is_ambiguous());
    }

    #[test]
    fn an_unreadable_body_is_drift_not_ambiguity() {
        // The reply ARRIVED. We could not parse it, which says something about
        // the schema, not about whether the order exists.
        let err = classify(Failure::UnreadableBody, &post("coid-9"));
        assert!(matches!(err, KisError::SchemaDrift { .. }), "{err:?}");
        assert!(!err.is_ambiguous());
    }

    #[test]
    fn a_mutation_without_a_correlation_header_still_reports_ambiguity() {
        // Degraded, not silent. An ambiguous order with an unknown id is far
        // worse than one with an id, and far better than a success.
        let bare = HttpRequest::post("/order", "TTTC0802U", "{}");
        match classify(Failure::TimedOut, &bare) {
            KisError::Ambiguous {
                client_order_id, ..
            } => assert_eq!(client_order_id, "unknown"),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn the_two_base_urls_are_different_hosts() {
        // A transport that could be pointed at production by flipping a
        // boolean is one bad default away from doing so.
        assert_ne!(LIVE_BASE_URL, SANDBOX_BASE_URL);
        assert!(LIVE_BASE_URL.starts_with("https://"));
        assert!(SANDBOX_BASE_URL.starts_with("https://"));
    }

    #[test]
    fn a_transport_can_be_built_without_a_network_or_credentials() {
        // The claim this module is here to make good on.
        let t = LiveTransport::sandbox(Duration::from_secs(5)).expect("builds");
        assert_eq!(t.base_url(), SANDBOX_BASE_URL);
        let live = LiveTransport::live(Duration::from_secs(5)).expect("builds");
        assert_eq!(live.base_url(), LIVE_BASE_URL);
    }

    #[test]
    fn a_get_request_encodes_public_and_private_query_parameters() {
        let transport = LiveTransport::sandbox(Duration::from_secs(5)).expect("builds");
        let request = HttpRequest::get("/quote", "TR")
            .with_query("FID_INPUT_ISCD", "069500")
            .with_secret_query("CANO", crate::secret::Secret::new("50123456".to_string()));
        let built = transport.build_request(&request).expect("request");
        let url = built.url().as_str();
        assert!(url.contains("FID_INPUT_ISCD=069500"), "{url}");
        assert!(url.contains("CANO=50123456"), "{url}");
        assert!(!format!("{request:?}").contains("50123456"));
    }
}
