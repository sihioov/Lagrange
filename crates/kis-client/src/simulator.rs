//! A versioned KIS broker simulator (plan Todo 36).
//!
//! The plan requires "a versioned broker simulator" and acceptance against
//! "recorded simulator contracts". Versioning is the part that earns its keep:
//! the simulator declares which KIS API contract it reproduces, and the client
//! records that version alongside its results. When KIS changes a field, the
//! simulator version changes with it, and evidence recorded against the old
//! contract is visibly stale rather than silently wrong.
//!
//! Scenarios are scripted per endpoint so a test can stage the exact fault it
//! is asserting: a 429, a 500 that then recovers, a submit that times out
//! (ambiguous, NOT failed), a drifted schema, or a payload carrying a secret
//! the redaction layer must strip.
//!
//! The simulator never reaches the network and holds no real credentials.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::KisError;
use crate::transport::{HttpRequest, HttpResponse, Transport};

/// The KIS API contract this simulator reproduces.
///
/// Bumped whenever a reproduced payload changes shape. Recorded in evidence so
/// a result proven against an old contract cannot be mistaken for a current
/// one.
pub const SIMULATOR_CONTRACT_VERSION: &str = "kis-openapi-2026-08";

/// What the simulator should do for the next call to an endpoint.
#[derive(Debug, Clone)]
pub enum Scenario {
    /// Normal reply.
    Ok { body: String },
    /// Throttled, with the broker's own advice.
    RateLimited { retry_after_ms: u64 },
    /// Server error.
    ServerError { status: u16, body: String },
    /// Sent, but no usable reply. For a mutation this is AMBIGUOUS: the
    /// simulator deliberately offers no way to say "the order definitely did
    /// not happen", because a real broker cannot say that either.
    Timeout,
    /// The request never left. Safe for a mutation to repeat.
    Unreachable { reason: String },
    /// A reply whose shape no longer matches what the client parses.
    DriftedSchema { body: String },
}

/// A scripted KIS broker.
pub struct BrokerSimulator {
    /// Scenario queue per `(method, path)`; the last one repeats once drained,
    /// so a test can stage "fail twice then succeed" without scripting every
    /// subsequent call.
    scripts: Mutex<HashMap<String, Vec<Scenario>>>,
    calls: Mutex<Vec<String>>,
}

impl Default for BrokerSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl BrokerSimulator {
    pub fn new() -> Self {
        Self {
            scripts: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn key(method: &str, path: &str) -> String {
        format!("{method} {path}")
    }

    /// Script the scenarios an endpoint will return, in order.
    pub fn script(self, method: &str, path: &str, scenarios: Vec<Scenario>) -> Self {
        self.scripts
            .lock()
            .expect("simulator mutex")
            .insert(Self::key(method, path), scenarios);
        self
    }

    /// Every `(method, path)` the client actually called, in order. Lets a
    /// test assert that a mutation was attempted exactly once.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("simulator mutex").clone()
    }

    pub fn call_count(&self, method: &str, path: &str) -> usize {
        let want = Self::key(method, path);
        self.calls().iter().filter(|c| **c == want).count()
    }

    /// A realistic successful order acknowledgement.
    pub fn order_ack(broker_order_no: &str) -> String {
        format!(
            r#"{{"rt_cd":"0","msg1":"정상처리 되었습니다.","output":{{"KRX_FWDG_ORD_ORGNO":"00950","ODNO":"{broker_order_no}","ORD_TMD":"090512"}}}}"#
        )
    }
}

impl Transport for BrokerSimulator {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, KisError> {
        let key = Self::key(request.method, &request.path);
        self.calls
            .lock()
            .expect("simulator mutex")
            .push(key.clone());

        let scenario = {
            let mut scripts = self.scripts.lock().expect("simulator mutex");
            match scripts.get_mut(&key) {
                // The last scenario repeats once drained.
                Some(queue) if queue.len() > 1 => queue.remove(0),
                Some(queue) if queue.len() == 1 => queue[0].clone(),
                _ => Scenario::Ok {
                    body: r#"{"rt_cd":"0","output":{}}"#.to_string(),
                },
            }
        };

        match scenario {
            Scenario::Ok { body } => Ok(HttpResponse::ok(body)),
            Scenario::RateLimited { retry_after_ms } => Err(KisError::RateLimited {
                endpoint: request.path.clone(),
                retry_after_ms,
            }),
            Scenario::ServerError { status, body } => Err(KisError::Broker {
                status,
                endpoint: request.path.clone(),
                body: crate::error::redact_payload(&body),
            }),
            Scenario::Unreachable { reason } => Err(KisError::Connect { reason }),
            Scenario::DriftedSchema { body } => Ok(HttpResponse::ok(body)),
            Scenario::Timeout => {
                // A timeout on a MUTATION is ambiguous; on a read it is merely
                // a failed read, because a read that never answered changed
                // nothing.
                if request.method == "POST" {
                    Err(KisError::Ambiguous {
                        operation: format!("{} {}", request.method, request.path),
                        client_order_id: request
                            .headers
                            .get(crate::transport::CLIENT_ORDER_ID_HEADER)
                            .cloned()
                            .unwrap_or_else(|| "unknown".to_string()),
                    })
                } else {
                    Err(KisError::Broker {
                        status: 504,
                        endpoint: request.path.clone(),
                        body: "gateway timeout".to_string(),
                    })
                }
            }
        }
    }
}

impl std::fmt::Debug for BrokerSimulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerSimulator")
            .field("contract_version", &SIMULATOR_CONTRACT_VERSION)
            .field("calls", &self.calls().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unscripted_endpoint_answers_successfully() {
        let sim = BrokerSimulator::new();
        let resp = sim
            .send(HttpRequest::get("/quote", "FHKST01010100"))
            .await
            .expect("default ok");
        assert_eq!(resp.status, 200);
        assert_eq!(sim.call_count("GET", "/quote"), 1);
    }

    #[tokio::test]
    async fn a_scripted_queue_is_consumed_then_its_last_entry_repeats() {
        // "fail twice then succeed forever" without scripting every call.
        let sim = BrokerSimulator::new().script(
            "GET",
            "/quote",
            vec![
                Scenario::RateLimited { retry_after_ms: 10 },
                Scenario::Ok {
                    body: r#"{"rt_cd":"0"}"#.to_string(),
                },
            ],
        );
        assert!(matches!(
            sim.send(HttpRequest::get("/quote", "TR")).await,
            Err(KisError::RateLimited { .. })
        ));
        for _ in 0..3 {
            assert!(sim.send(HttpRequest::get("/quote", "TR")).await.is_ok());
        }
    }

    #[tokio::test]
    async fn a_post_timeout_is_ambiguous_and_a_get_timeout_is_not() {
        // The asymmetry the whole crate turns on: a read that never answered
        // changed nothing; a write that never answered may have.
        let sim = BrokerSimulator::new()
            .script("POST", "/order", vec![Scenario::Timeout])
            .script("GET", "/quote", vec![Scenario::Timeout]);

        let post = sim
            .send(HttpRequest::post("/order", "TTTC0802U", "{}"))
            .await
            .expect_err("ambiguous");
        assert!(post.is_ambiguous(), "a POST timeout must be ambiguous");

        let get = sim
            .send(HttpRequest::get("/quote", "TR"))
            .await
            .expect_err("failed read");
        assert!(!get.is_ambiguous(), "a GET timeout is just a failed read");
    }

    #[tokio::test]
    async fn an_ambiguous_timeout_carries_the_client_order_id() {
        // Without it the operator cannot correlate the unknown order with the
        // intent that produced it.
        let sim = BrokerSimulator::new().script("POST", "/order", vec![Scenario::Timeout]);
        let err = sim
            .send(
                HttpRequest::post("/order", "TTTC0802U", "{}")
                    .with_header("x-client-order-id", "coid-77"),
            )
            .await
            .expect_err("ambiguous");
        assert!(err.to_string().contains("UNKNOWN"));
        match err {
            KisError::Ambiguous {
                client_order_id, ..
            } => assert_eq!(client_order_id, "coid-77"),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_server_error_body_is_redacted_before_it_becomes_an_error() {
        let sim = BrokerSimulator::new().script(
            "POST",
            "/order",
            vec![Scenario::ServerError {
                status: 500,
                body: r#"{"appsecret":"leak-me","CANO":"50123456"}"#.to_string(),
            }],
        );
        let err = sim
            .send(HttpRequest::post("/order", "TR", "{}"))
            .await
            .expect_err("server error");
        let rendered = err.to_string();
        assert!(!rendered.contains("leak-me"), "{rendered}");
        assert!(!rendered.contains("50123456"), "{rendered}");
    }

    #[tokio::test]
    async fn the_simulator_records_every_call_for_exactly_once_assertions() {
        let sim = BrokerSimulator::new();
        sim.send(HttpRequest::post("/order", "TR", "{}")).await.ok();
        assert_eq!(sim.call_count("POST", "/order"), 1);
        assert_eq!(sim.call_count("POST", "/other"), 0);
        assert_eq!(sim.calls(), vec!["POST /order".to_string()]);
    }

    #[test]
    fn the_contract_version_is_recorded() {
        // Evidence proven against an old contract must be visibly stale.
        assert!(SIMULATOR_CONTRACT_VERSION.starts_with("kis-openapi-"));
        let sim = BrokerSimulator::new();
        assert!(format!("{sim:?}").contains(SIMULATOR_CONTRACT_VERSION));
    }

    #[test]
    fn the_order_ack_fixture_looks_like_a_real_kis_reply() {
        let ack = BrokerSimulator::order_ack("0000117057");
        assert!(ack.contains("\"ODNO\":\"0000117057\""));
        assert!(ack.contains("\"rt_cd\":\"0\""));
    }
}
