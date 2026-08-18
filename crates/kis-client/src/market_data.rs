//! Authenticated, rate-limited KIS market-data reads.
//!
//! This is the common transport edge used by the licensed data collector. It
//! deliberately returns the response body unchanged: parsing and normalization
//! belong to the market-data provider adapter, while immutable Raw can retain
//! the exact KIS JSON response and continuation headers.

use std::sync::Arc;

use serde_json::Value;

use crate::auth::TokenManager;
use crate::error::{KisError, RequestKind, redact_payload};
use crate::rate_limit::{BucketKey, Permit, RateLimiter};
use crate::retry::{RetryPolicy, Sleeper};
use crate::secret::{CredentialRef, CredentialSource, Secret};
use crate::transport::{HttpRequest, Transport};

/// Exact read-only KIS channels approved for the market-data client.
///
/// Keep this deny-by-default list at the HTTP client boundary as well as in
/// the provider adapter: a future caller must not be able to turn a generic
/// `get(path, tr_id, ...)` seam into an account, order, or undocumented API
/// request by typo or copy/paste.  Expanding it requires a reviewed contract
/// and focused tests.
const READ_ONLY_CHANNELS: &[(&str, &str)] = &[
    (
        "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice",
        "FHKST03010100",
    ),
    (
        "/uapi/domestic-stock/v1/quotations/inquire-price",
        "FHKST01010100",
    ),
    (
        "/uapi/domestic-stock/v1/quotations/chk-holiday",
        "CTCA0903R",
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        "HHKDB669100C0",
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
        "HHKDB669101C0",
    ),
    ("/uapi/domestic-stock/v1/ksdinfo/dividend", "HHKDB669102C0"),
    (
        "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        "HHKDB669104C0",
    ),
    ("/uapi/domestic-stock/v1/ksdinfo/rev-split", "HHKDB669105C0"),
    ("/uapi/domestic-stock/v1/ksdinfo/cap-dcrs", "HHKDB669106C0"),
];

fn is_allowed_read_channel(path: &str, tr_id: &str) -> bool {
    READ_ONLY_CHANNELS
        .iter()
        .any(|(allowed_path, allowed_tr_id)| *allowed_path == path && *allowed_tr_id == tr_id)
}

/// One successful KIS read, before provider-specific parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketDataReply {
    pub body: Vec<u8>,
    pub continuation: Option<String>,
}

/// Shared KIS client for non-mutating market-data endpoints.
pub struct KisMarketDataClient<T: Transport, S: Sleeper, C: CredentialSource> {
    transport: T,
    sleeper: S,
    tokens: Arc<TokenManager>,
    limiter: Arc<RateLimiter>,
    credentials: C,
    app_key_ref: CredentialRef,
    app_secret_ref: CredentialRef,
}

impl<T: Transport, S: Sleeper, C: CredentialSource> KisMarketDataClient<T, S, C> {
    pub fn new(
        transport: T,
        sleeper: S,
        tokens: Arc<TokenManager>,
        limiter: Arc<RateLimiter>,
        credentials: C,
        app_key_ref: CredentialRef,
        app_secret_ref: CredentialRef,
    ) -> Self {
        Self {
            transport,
            sleeper,
            tokens,
            limiter,
            credentials,
            app_key_ref,
            app_secret_ref,
        }
    }

    /// Perform one documented KIS GET request.
    ///
    /// The app credentials are resolved on every attempt so a rotated secret
    /// takes effect without a process restart. Values live only in `Secret`
    /// headers and therefore cannot be rendered by request debug output.
    pub async fn get(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        // Reject before rate limiting, token lookup, credential resolution, or
        // transport construction.  An invalid endpoint must be unable to
        // cause even an authenticated network attempt.
        if !is_allowed_read_channel(path, tr_id) {
            return Err(KisError::UnsupportedEndpoint {
                endpoint: path.to_owned(),
                tr_id: tr_id.to_owned(),
            });
        }
        crate::retry::execute(
            RetryPolicy::reads(),
            RequestKind::Read,
            &self.sleeper,
            |_attempt| async move {
                match self.limiter.acquire(&BucketKey::new(path, tr_id)) {
                    Permit::Granted => {}
                    Permit::Throttled { retry_after_ms } => {
                        return Err(KisError::RateLimited {
                            endpoint: path.to_owned(),
                            retry_after_ms,
                        });
                    }
                }

                let token = self.tokens.token().await?;
                let app_key = self.credentials.resolve(&self.app_key_ref)?;
                let app_secret = self.credentials.resolve(&self.app_secret_ref)?;
                let mut request = HttpRequest::get(path, tr_id)
                    .with_header("custtype", "P")
                    .with_secret_header(
                        "authorization",
                        Secret::new(format!("Bearer {}", token.value.expose())),
                    )
                    .with_secret_header("appkey", app_key)
                    .with_secret_header("appsecret", app_secret);
                if let Some(value) = continuation {
                    request = request.with_header("tr_cont", value);
                }
                for (key, value) in query {
                    request = request.with_query(key, value);
                }

                let response = self.transport.send(request).await?;
                if response.status == 429 {
                    let retry_after_ms = response
                        .headers
                        .get("retry-after")
                        .and_then(|value| value.parse::<u64>().ok())
                        .map(|seconds| seconds.saturating_mul(1_000))
                        .unwrap_or(1_000);
                    return Err(KisError::RateLimited {
                        endpoint: path.to_owned(),
                        retry_after_ms,
                    });
                }
                if !(200..300).contains(&response.status) {
                    return Err(KisError::Broker {
                        status: response.status,
                        endpoint: path.to_owned(),
                        body: redact_payload(&response.body),
                    });
                }

                let document: Value =
                    serde_json::from_str(&response.body).map_err(|_| KisError::SchemaDrift {
                        endpoint: path.to_owned(),
                        detail: "KIS response was not a JSON object".to_owned(),
                    })?;
                let rt_cd = document
                    .get("rt_cd")
                    .and_then(Value::as_str)
                    .ok_or_else(|| KisError::SchemaDrift {
                        endpoint: path.to_owned(),
                        detail: "KIS response did not contain string rt_cd".to_owned(),
                    })?;
                if rt_cd != "0" {
                    return Err(KisError::Broker {
                        status: 400,
                        endpoint: path.to_owned(),
                        body: redact_payload(&response.body),
                    });
                }

                Ok(MarketDataReply {
                    body: response.body.into_bytes(),
                    continuation: response.headers.get("tr_cont").cloned(),
                })
            },
        )
        .await
    }
}

impl<T: Transport, S: Sleeper, C: CredentialSource> std::fmt::Debug
    for KisMarketDataClient<T, S, C>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KisMarketDataClient")
            .field("app_key_ref", &self.app_key_ref)
            .field("app_secret_ref", &self.app_secret_ref)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::auth::{AccessToken, TokenIssuer};
    use crate::clock::{Clock, TestClock};
    use crate::rate_limit::Quota;
    use crate::retry::Sleeper;
    use crate::secret::{CredentialError, Secret};
    use crate::transport::HttpResponse;

    use super::*;

    #[derive(Default)]
    struct CapturingTransport {
        requests: Mutex<Vec<RequestSnapshot>>,
        responses: Mutex<Vec<HttpResponse>>,
    }

    #[derive(Debug)]
    struct RequestSnapshot {
        query: Vec<(String, String)>,
        headers: BTreeMap<String, String>,
        secret_headers: BTreeMap<String, String>,
    }

    impl CapturingTransport {
        fn with_responses(responses: Vec<HttpResponse>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            }
        }
    }

    impl Transport for CapturingTransport {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, KisError> {
            self.requests.lock().unwrap().push(RequestSnapshot {
                query: request.query,
                headers: request.headers,
                secret_headers: request
                    .secret_headers
                    .into_iter()
                    .map(|(key, value)| (key, value.into_inner()))
                    .collect(),
            });
            let mut responses = self.responses.lock().unwrap();
            Ok(if responses.len() > 1 {
                responses.remove(0)
            } else {
                responses[0].clone()
            })
        }
    }

    struct FixedIssuer(TestClock);

    #[async_trait::async_trait]
    impl TokenIssuer for FixedIssuer {
        async fn issue(&self) -> Result<AccessToken, KisError> {
            Ok(AccessToken {
                value: Secret::new("access-token".to_owned()),
                expires_at_ms: self.0.now_ms() + 3_600_000,
            })
        }
    }

    #[derive(Clone, Copy)]
    struct FixedCredentials;

    impl CredentialSource for FixedCredentials {
        fn resolve(&self, reference: &CredentialRef) -> Result<Secret<String>, CredentialError> {
            let value = match reference {
                CredentialRef::Env { var } if var == "KIS_APP_KEY" => "app-key",
                CredentialRef::File { .. } => "app-secret",
                _ => {
                    return Err(CredentialError::NotFound {
                        location: reference.describe(),
                    });
                }
            };
            Ok(Secret::new(value.to_owned()))
        }
    }

    #[derive(Clone, Copy)]
    struct NoSleep;

    impl Sleeper for NoSleep {
        fn sleep_ms(&self, _ms: u64) -> impl std::future::Future<Output = ()> + Send {
            std::future::ready(())
        }
    }

    fn client(
        transport: CapturingTransport,
    ) -> KisMarketDataClient<CapturingTransport, NoSleep, FixedCredentials> {
        let clock = TestClock::at(0);
        KisMarketDataClient::new(
            transport,
            NoSleep,
            Arc::new(TokenManager::new(
                Arc::new(clock.clone()),
                Arc::new(FixedIssuer(clock.clone())),
            )),
            Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(100, 100))),
            FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
        )
    }

    #[tokio::test]
    async fn a_market_read_attaches_all_kis_auth_and_query_fields() {
        let transport = CapturingTransport::with_responses(vec![
            HttpResponse::ok(r#"{"rt_cd":"0","output2":[]}"#).with_header("tr_cont", "F"),
        ]);
        let client = client(transport);
        let reply = client
            .get(
                "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice",
                "FHKST03010100",
                &[("FID_INPUT_ISCD".to_owned(), "069500".to_owned())],
                None,
            )
            .await
            .expect("KIS read");
        assert_eq!(reply.continuation.as_deref(), Some("F"));

        let requests = client.transport.requests.lock().unwrap();
        let request = &requests[0];
        assert_eq!(
            request.query[0],
            ("FID_INPUT_ISCD".to_owned(), "069500".to_owned())
        );
        assert_eq!(
            request.headers.get("custtype").map(String::as_str),
            Some("P")
        );
        assert_eq!(
            request
                .secret_headers
                .get("authorization")
                .map(String::as_str),
            Some("Bearer access-token")
        );
        assert_eq!(
            request.secret_headers.get("appkey").map(String::as_str),
            Some("app-key")
        );
        assert_eq!(
            request.secret_headers.get("appsecret").map(String::as_str),
            Some("app-secret")
        );
    }

    #[tokio::test]
    async fn a_kis_business_error_is_typed_and_redacted() {
        let transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"1","msg1":"bad","appkey":"app-key"}"#,
        )]);
        let error = client(transport)
            .get(
                "/uapi/domestic-stock/v1/quotations/inquire-price",
                "FHKST01010100",
                &[],
                None,
            )
            .await
            .expect_err("business error");
        let rendered = error.to_string();
        assert!(matches!(error, KisError::Broker { status: 400, .. }));
        assert!(!rendered.contains("app-key"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[tokio::test]
    async fn malformed_success_is_schema_drift_and_is_not_retried() {
        let transport = CapturingTransport::with_responses(vec![HttpResponse::ok("not-json")]);
        let client = client(transport);
        let error = client
            .get(
                "/uapi/domestic-stock/v1/quotations/inquire-price",
                "FHKST01010100",
                &[],
                None,
            )
            .await
            .expect_err("schema drift");
        assert!(matches!(error, KisError::SchemaDrift { .. }));
        assert_eq!(client.transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_order_or_account_endpoint_is_rejected_before_auth_or_transport() {
        let transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output1":[]}"#,
        )]);
        let client = client(transport);
        let error = client
            .get(
                "/uapi/domestic-stock/v1/trading/inquire-balance",
                "TTTC8434R",
                &[("CANO".to_owned(), "must-not-be-sent".to_owned())],
                None,
            )
            .await
            .expect_err("account endpoint must be outside read-only client");
        assert!(matches!(error, KisError::UnsupportedEndpoint { .. }));
        assert_eq!(client.transport.requests.lock().unwrap().len(), 0);
        let rendered = error.to_string();
        assert!(!rendered.contains("must-not-be-sent"), "{rendered}");
    }
}
