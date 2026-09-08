//! Authenticated, rate-limited KIS market-data reads.
//!
//! This is the common transport edge used by the licensed data collector. It
//! deliberately returns the response body unchanged: parsing and normalization
//! belong to the market-data provider adapter, while immutable Raw can retain
//! the exact KIS JSON response and continuation headers.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::auth::{AccessToken, TokenIssuer, TokenManager};
use crate::clock::Clock;
use crate::error::{KisError, RequestKind, redact_payload};
use crate::rate_limit::{BucketKey, Permit, RateLimiter};
use crate::read_coordination::{
    COORDINATED_EOD_RETRY_DELAY, COORDINATED_INTRADAY_RETRY_DELAY, LockAcquisition,
    ReadCallbackResult, ReadChannel, ReadCoordinationConfig, ReadCoordinationError,
    ReadCoordinator, ReadCredentialSnapshot, ReadDispatchGuard, ReadFailureKind,
};
use crate::retry::{RetryPolicy, Sleeper};
use crate::secret::{CredentialRef, CredentialSource, Secret};
use crate::transport::{HttpRequest, Transport};

const INTRADAY_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const SHARED_EOD_LOCK_WAIT: Duration = Duration::from_secs(30);

fn coordination_error(path: &str, error: ReadCoordinationError) -> KisError {
    let retry_after_ms = match error {
        ReadCoordinationError::ReservationActive { retry_after_ms }
        | ReadCoordinationError::TokenIssueCooldown { retry_after_ms }
        | ReadCoordinationError::BrokerCooldown { retry_after_ms }
        | ReadCoordinationError::GlobalSpacing { retry_after_ms }
        | ReadCoordinationError::ChannelSpacing { retry_after_ms }
        | ReadCoordinationError::IntradaySpacing { retry_after_ms }
        | ReadCoordinationError::CallbackRateLimited { retry_after_ms } => Some(retry_after_ms),
        ReadCoordinationError::LockBusy => Some(0),
        _ => None,
    };
    if let Some(retry_after_ms) = retry_after_ms {
        KisError::RateLimited {
            endpoint: path.to_owned(),
            retry_after_ms,
        }
    } else {
        KisError::Auth {
            reason: error.code().to_owned(),
        }
    }
}

/// One successful KIS read, before provider-specific parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketDataReply {
    pub body: Vec<u8>,
    pub continuation: Option<String>,
}

/// Opaque shared authentication bundle. The coordinator verifier and token
/// issuer are constructed from the same resolved credential snapshot that is
/// later used for the GET headers.
pub struct CoordinatedReadAuth {
    coordinator: Arc<ReadCoordinator>,
    issuer: Arc<dyn TokenIssuer>,
    credentials: ReadCredentialSnapshot,
}

impl CoordinatedReadAuth {
    pub fn new(
        config: ReadCoordinationConfig,
        credentials: ReadCredentialSnapshot,
        clock: Arc<dyn Clock>,
        issuer: Arc<dyn TokenIssuer>,
    ) -> Result<Self, ReadCoordinationError> {
        let coordinator = Arc::new(ReadCoordinator::new(
            config,
            credentials.coordinator_credentials()?,
            clock,
        ));
        Ok(Self {
            coordinator,
            issuer,
            credentials,
        })
    }
}

impl std::fmt::Debug for CoordinatedReadAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoordinatedReadAuth")
            .field("coordinator", &self.coordinator)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
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
    coordinated: Option<CoordinatedReadAuth>,
    intraday_transport: Option<T>,
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
            coordinated: None,
            intraday_transport: None,
        }
    }

    /// Opt this client into the process-shared read boundary. The separate
    /// intraday transport is constructed with its own three-second HTTP
    /// timeout by production callers.
    pub fn with_shared_read_coordination(
        mut self,
        coordinated: CoordinatedReadAuth,
        intraday_transport: T,
    ) -> Self {
        self.coordinated = Some(coordinated);
        self.intraday_transport = Some(intraday_transport);
        self
    }

    /// Perform one documented KIS GET request.
    ///
    /// Legacy mode resolves app credentials on every attempt. Shared mode uses
    /// its validated generation snapshot until the process is restarted for a
    /// higher generation. Values live only in `Secret` headers and therefore
    /// cannot be rendered by request debug output.
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
        ReadChannel::from_pair(path, tr_id).map_err(|_| KisError::UnsupportedEndpoint {
            endpoint: path.to_owned(),
            tr_id: tr_id.to_owned(),
        })?;
        if self.coordinated.is_some() {
            return self
                .get_shared(path, tr_id, query, continuation, false)
                .await;
        }
        self.get_legacy(path, tr_id, query, continuation).await
    }

    /// Perform one exact current-price GET as an intraday-class read.
    ///
    /// Legacy clients fail before token lookup, credential resolution, or
    /// transport. No other endpoint/TR pair can consume the intraday seam.
    pub async fn get_intraday(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
    ) -> Result<MarketDataReply, KisError> {
        if ReadChannel::from_pair(path, tr_id).ok() != Some(ReadChannel::InquirePrice) {
            return Err(KisError::UnsupportedEndpoint {
                endpoint: path.to_owned(),
                tr_id: tr_id.to_owned(),
            });
        }
        if self.coordinated.is_none() {
            return Err(KisError::Auth {
                reason: "KIS_INTRADAY_REQUIRES_SHARED".to_owned(),
            });
        }
        self.get_shared(path, tr_id, query, None, true).await
    }

    async fn get_legacy(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
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
                self.send_attempt(
                    &self.transport,
                    &self.credentials,
                    token,
                    path,
                    tr_id,
                    query,
                    continuation,
                )
                .await
            },
        )
        .await
    }

    async fn get_shared(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
        intraday: bool,
    ) -> Result<MarketDataReply, KisError> {
        let policy = if intraday {
            RetryPolicy {
                max_attempts: 3,
                initial_backoff_ms: 100,
                max_backoff_ms: 200,
            }
        } else {
            RetryPolicy::reads()
        };
        let mut attempts = 0_u32;
        let mut auth_refresh_available = true;
        loop {
            if attempts > 0 {
                let spacing = if intraday {
                    COORDINATED_INTRADAY_RETRY_DELAY
                } else {
                    COORDINATED_EOD_RETRY_DELAY
                };
                let backoff = Duration::from_millis(policy.backoff_ms(attempts + 1));
                self.sleeper
                    .sleep_ms(spacing.max(backoff).as_millis() as u64)
                    .await;
            }

            let started = Arc::new(AtomicBool::new(false));
            let result = self
                .coordinated_attempt(
                    path,
                    tr_id,
                    query,
                    continuation,
                    intraday,
                    Arc::clone(&started),
                )
                .await;
            if !started.load(Ordering::SeqCst) {
                return result;
            }
            attempts += 1;
            match result {
                Ok(reply) => return Ok(reply),
                // Shared 429 state is already durable. Returning it prevents
                // an early nested retry; a later job/cycle must honor it.
                Err(error @ KisError::RateLimited { .. }) => return Err(error),
                Err(KisError::Broker { status: 401, .. })
                    if auth_refresh_available && attempts < policy.max_attempts =>
                {
                    // The coordinator already invalidated the persisted token.
                    // Permit exactly one local reissue opportunity; the next
                    // gate still enforces the durable 60-second issue debt and
                    // this GET still counts against the attempt bound.
                    auth_refresh_available = false;
                }
                Err(error)
                    if attempts < policy.max_attempts && error.is_retryable(RequestKind::Read) => {}
                Err(error) => return Err(error),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn coordinated_attempt(
        &self,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
        intraday: bool,
        started: Arc<AtomicBool>,
    ) -> Result<MarketDataReply, KisError> {
        let coordinated = self.coordinated.as_ref().expect("shared path checked");
        let transport = if intraday {
            self.intraday_transport
                .as_ref()
                .expect("shared clients carry an intraday transport")
        } else {
            &self.transport
        };
        let captured = Arc::new(Mutex::new(None));
        let captured_by_callback = Arc::clone(&captured);
        let credentials = coordinated.credentials.clone();
        let execute = |token, dispatch: ReadDispatchGuard| async move {
            let request =
                match self.build_request(&credentials, token, path, tr_id, query, continuation) {
                    Ok(request) => request,
                    Err(error) => {
                        *captured_by_callback.lock().expect("captured error mutex") = Some(error);
                        return ReadCallbackResult::CompletedFailure(ReadFailureKind::Transport);
                    }
                };
            // This is the cooperative final send boundary. The clock check and
            // transport call are intentionally adjacent; arbitrary OS
            // preemption between them cannot be physically excluded.
            if dispatch.begin_dispatch().is_err() {
                return ReadCallbackResult::CompletedFailure(ReadFailureKind::Transport);
            }
            started.store(true, Ordering::SeqCst);
            let result = self.send_request(transport, request, path).await;
            match result {
                Ok(reply) => ReadCallbackResult::Success(reply),
                Err(error) => {
                    let outcome = match &error {
                        KisError::Broker { status: 401, .. } => ReadCallbackResult::Unauthorized,
                        KisError::RateLimited { retry_after_ms, .. } => {
                            ReadCallbackResult::RateLimited {
                                retry_after: Duration::from_millis(*retry_after_ms),
                            }
                        }
                        KisError::Connect { .. } => {
                            ReadCallbackResult::CompletedFailure(ReadFailureKind::Transport)
                        }
                        KisError::Broker {
                            status: 500..=599, ..
                        } => ReadCallbackResult::CompletedFailure(
                            ReadFailureKind::ProviderUnavailable,
                        ),
                        KisError::SchemaDrift { .. } | KisError::Broker { .. } => {
                            ReadCallbackResult::CompletedFailure(ReadFailureKind::ResponseInvalid)
                        }
                        _ => ReadCallbackResult::CompletedFailure(ReadFailureKind::Transport),
                    };
                    *captured_by_callback.lock().expect("captured error mutex") = Some(error);
                    outcome
                }
            }
        };
        let acquisition = if intraday {
            LockAcquisition::NonBlocking
        } else {
            LockAcquisition::Bounded(SHARED_EOD_LOCK_WAIT)
        };
        let outcome = if intraday {
            coordinated
                .coordinator
                .execute_intraday(
                    path,
                    tr_id,
                    coordinated.issuer.as_ref(),
                    acquisition,
                    INTRADAY_REQUEST_TIMEOUT,
                    execute,
                )
                .await
        } else {
            coordinated
                .coordinator
                .execute(
                    path,
                    tr_id,
                    coordinated.issuer.as_ref(),
                    acquisition,
                    SHARED_EOD_LOCK_WAIT,
                    execute,
                )
                .await
        };
        match outcome {
            Ok(reply) => Ok(reply),
            Err(ReadCoordinationError::CallbackTimedOut) => Err(KisError::Broker {
                status: 504,
                endpoint: path.to_owned(),
                body: "local read deadline reached".to_owned(),
            }),
            Err(
                ReadCoordinationError::Unauthorized
                | ReadCoordinationError::CallbackRateLimited { .. }
                | ReadCoordinationError::CallbackFailed { .. },
            ) => Err(captured
                .lock()
                .expect("captured error mutex")
                .take()
                .unwrap_or_else(|| coordination_error(path, ReadCoordinationError::CorruptState))),
            Err(error) => Err(coordination_error(path, error)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_attempt<D: CredentialSource>(
        &self,
        transport: &T,
        credentials: &D,
        token: AccessToken,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<MarketDataReply, KisError> {
        let request = self.build_request(credentials, token, path, tr_id, query, continuation)?;
        self.send_request(transport, request, path).await
    }

    #[allow(clippy::too_many_arguments)]
    fn build_request<D: CredentialSource>(
        &self,
        credentials: &D,
        token: AccessToken,
        path: &str,
        tr_id: &str,
        query: &[(String, String)],
        continuation: Option<&str>,
    ) -> Result<HttpRequest, KisError> {
        let app_key = credentials.resolve(&self.app_key_ref)?;
        let app_secret = credentials.resolve(&self.app_secret_ref)?;
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
        Ok(request)
    }

    async fn send_request(
        &self,
        transport: &T,
        request: HttpRequest,
        path: &str,
    ) -> Result<MarketDataReply, KisError> {
        let response = transport.send(request).await?;
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
    }
}

impl<T: Transport, S: Sleeper, C: CredentialSource> std::fmt::Debug
    for KisMarketDataClient<T, S, C>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KisMarketDataClient")
            .field("app_key_ref", &self.app_key_ref)
            .field("app_secret_ref", &self.app_secret_ref)
            .field("coordinated", &self.coordinated)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::auth::{AccessToken, TokenIssuer};
    use crate::clock::{Clock, TestClock};
    use crate::rate_limit::Quota;
    use crate::read_coordination::ReadCredentialSnapshot;
    use crate::retry::Sleeper;
    use crate::secret::{CredentialError, Secret};
    use crate::token_issuer::KisTokenIssuer;
    use crate::transport::HttpResponse;

    use super::*;

    #[derive(Clone, Default)]
    struct CapturingTransport {
        requests: Arc<Mutex<Vec<RequestSnapshot>>>,
        responses: Arc<Mutex<Vec<HttpResponse>>>,
    }

    #[derive(Debug)]
    struct RequestSnapshot {
        path: String,
        query: Vec<(String, String)>,
        headers: BTreeMap<String, String>,
        secret_headers: BTreeMap<String, String>,
        body: Option<String>,
    }

    impl CapturingTransport {
        fn with_responses(responses: Vec<HttpResponse>) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    impl Transport for CapturingTransport {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, KisError> {
            self.requests.lock().unwrap().push(RequestSnapshot {
                path: request.path,
                query: request.query,
                headers: request.headers,
                secret_headers: request
                    .secret_headers
                    .into_iter()
                    .map(|(key, value)| (key, value.into_inner()))
                    .collect(),
                body: request.body,
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

    #[derive(Clone)]
    struct ClockSleeper(TestClock);

    impl Sleeper for ClockSleeper {
        fn sleep_ms(&self, ms: u64) -> impl std::future::Future<Output = ()> + Send {
            self.0
                .advance_ms(i64::try_from(ms).expect("bounded test sleep"));
            std::future::ready(())
        }
    }

    struct CountingCredentials {
        calls: AtomicUsize,
    }

    struct PrivateRoot(PathBuf);

    impl PrivateRoot {
        fn new(label: &str) -> Self {
            static NONCE: AtomicUsize = AtomicUsize::new(1);
            let path = std::env::temp_dir().join(format!(
                "kis-market-data-{label}-{}-{}",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create private test root");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for PrivateRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl CredentialSource for CountingCredentials {
        fn resolve(&self, reference: &CredentialRef) -> Result<Secret<String>, CredentialError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let value = match reference {
                CredentialRef::Env { .. } => "snapshot-key",
                CredentialRef::File { .. } => "snapshot-secret",
            };
            Ok(Secret::new(value.to_owned()))
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

    fn protect_test_root(root: &Path) -> u32 {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .expect("protect shared test root");
        std::fs::metadata(root).expect("shared root metadata").uid()
    }

    fn shared_client(
        root: &Path,
        clock: TestClock,
        read_transport: CapturingTransport,
        intraday_transport: CapturingTransport,
        token_transport: CapturingTransport,
        snapshot: ReadCredentialSnapshot,
    ) -> KisMarketDataClient<CapturingTransport, ClockSleeper, FixedCredentials> {
        let app_key_ref = CredentialRef::env("KIS_APP_KEY");
        let app_secret_ref = CredentialRef::file("/run/secrets/kis_app_secret");
        let issuer: Arc<dyn TokenIssuer> = Arc::new(KisTokenIssuer::new(
            token_transport,
            snapshot.clone(),
            app_key_ref.clone(),
            app_secret_ref.clone(),
            || 1_000_000,
        ));
        let auth = CoordinatedReadAuth::new(
            ReadCoordinationConfig::new(root, protect_test_root(root)),
            snapshot,
            Arc::new(clock.clone()),
            Arc::clone(&issuer),
        )
        .expect("coordinated test auth");
        KisMarketDataClient::new(
            read_transport,
            ClockSleeper(clock.clone()),
            Arc::new(TokenManager::new(Arc::new(clock.clone()), issuer)),
            Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(100, 100))),
            FixedCredentials,
            app_key_ref,
            app_secret_ref,
        )
        .with_shared_read_coordination(auth, intraday_transport)
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

    #[tokio::test]
    async fn legacy_keeps_eod_reads_but_rejects_intraday_before_auth_or_transport() {
        let transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output":{}}"#,
        )]);
        let client = client(transport);
        let error = client
            .get_intraday(
                ReadChannel::InquirePrice.pair().0,
                ReadChannel::InquirePrice.pair().1,
                &[],
            )
            .await
            .expect_err("legacy intraday is disabled");
        assert!(matches!(
            error,
            KisError::Auth { ref reason }
                if reason == "KIS_INTRADAY_REQUIRES_SHARED"
        ));
        assert!(client.transport.requests.lock().unwrap().is_empty());

        client
            .get(
                ReadChannel::InquirePrice.pair().0,
                ReadChannel::InquirePrice.pair().1,
                &[],
                None,
            )
            .await
            .expect("legacy EOD reference remains compatible");
        assert_eq!(client.transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn shared_token_verifier_and_headers_use_one_resolved_snapshot() {
        let root = PrivateRoot::new("snapshot");
        let source = CountingCredentials {
            calls: AtomicUsize::new(0),
        };
        let app_key_ref = CredentialRef::env("KIS_APP_KEY");
        let app_secret_ref = CredentialRef::file("/run/secrets/kis_app_secret");
        let snapshot =
            ReadCredentialSnapshot::resolve(&source, app_key_ref, app_secret_ref, 1).unwrap();
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        let token_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"access_token":"shared-token","expires_in":3600}"#,
        )]);
        let token_probe = token_transport.clone();
        let read_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output":{}}"#,
        )]);
        let read_probe = read_transport.clone();
        let client = shared_client(
            root.path(),
            TestClock::at(1_000_000),
            read_transport,
            CapturingTransport::default(),
            token_transport,
            snapshot,
        );
        client
            .get(
                ReadChannel::InquirePrice.pair().0,
                ReadChannel::InquirePrice.pair().1,
                &[],
                None,
            )
            .await
            .expect("coordinated EOD read");

        assert_eq!(source.calls.load(Ordering::SeqCst), 2, "no re-resolution");
        let token_requests = token_probe.requests.lock().unwrap();
        assert_eq!(token_requests.len(), 1);
        assert_eq!(token_requests[0].path, crate::token_issuer::TOKEN_PATH);
        let body = token_requests[0].body.as_deref().unwrap();
        assert!(body.contains("snapshot-key"));
        assert!(body.contains("snapshot-secret"));
        let reads = read_probe.requests.lock().unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(
            reads[0].secret_headers.get("appkey").map(String::as_str),
            Some("snapshot-key")
        );
        assert_eq!(
            reads[0].secret_headers.get("appsecret").map(String::as_str),
            Some("snapshot-secret")
        );
    }

    #[tokio::test]
    async fn eod_and_intraday_on_inquire_price_consume_distinct_class_budgets() {
        let root = PrivateRoot::new("classes");
        let clock = TestClock::at(2_000_000);
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let read_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output":{}}"#,
        )]);
        let read_probe = read_transport.clone();
        let intraday_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output":{}}"#,
        )]);
        let intraday_probe = intraday_transport.clone();
        let client = shared_client(
            root.path(),
            clock.clone(),
            read_transport,
            intraday_transport,
            CapturingTransport::with_responses(vec![HttpResponse::ok(
                r#"{"access_token":"shared-token","expires_in":3600}"#,
            )]),
            snapshot,
        );
        let (path, tr_id) = ReadChannel::InquirePrice.pair();
        client
            .get(path, tr_id, &[], None)
            .await
            .expect("EOD reference");
        clock.advance_ms(COORDINATED_EOD_RETRY_DELAY.as_millis() as i64);
        client
            .get_intraday(path, tr_id, &[])
            .await
            .expect("first intraday read is not charged by EOD");
        clock.advance_ms(COORDINATED_EOD_RETRY_DELAY.as_millis() as i64);
        client
            .get(path, tr_id, &[], None)
            .await
            .expect("EOD read does not wait for intraday five-second spacing");
        assert_eq!(read_probe.requests.lock().unwrap().len(), 2);
        assert_eq!(intraday_probe.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn shared_429_is_persisted_and_not_retried_early() {
        let root = PrivateRoot::new("rate-limit");
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let intraday_transport = CapturingTransport::with_responses(vec![
            HttpResponse::status(429, "limited").with_header("retry-after", "9"),
        ]);
        let probe = intraday_transport.clone();
        let client = shared_client(
            root.path(),
            TestClock::at(3_000_000),
            CapturingTransport::default(),
            intraday_transport,
            CapturingTransport::with_responses(vec![HttpResponse::ok(
                r#"{"access_token":"shared-token","expires_in":3600}"#,
            )]),
            snapshot,
        );
        let error = client
            .get_intraday(
                ReadChannel::InquirePrice.pair().0,
                ReadChannel::InquirePrice.pair().1,
                &[],
            )
            .await
            .expect_err("429 remains typed");
        assert!(matches!(
            error,
            KisError::RateLimited {
                retry_after_ms: 9_000,
                ..
            }
        ));
        assert_eq!(probe.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fresh_shared_401_uses_one_refresh_opportunity_but_issue_debt_blocks_another_get() {
        let root = PrivateRoot::new("unauthorized");
        let clock = TestClock::at(3_500_000);
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let token_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"access_token":"shared-token","expires_in":3600}"#,
        )]);
        let token_probe = token_transport.clone();
        let intraday_transport =
            CapturingTransport::with_responses(vec![HttpResponse::status(401, "unauthorized")]);
        let intraday_probe = intraday_transport.clone();
        let client = shared_client(
            root.path(),
            clock.clone(),
            CapturingTransport::default(),
            intraday_transport,
            token_transport,
            snapshot,
        );
        let (path, tr_id) = ReadChannel::InquirePrice.pair();
        assert!(matches!(
            client.get_intraday(path, tr_id, &[]).await,
            Err(KisError::RateLimited { .. })
        ));
        assert_eq!(token_probe.requests.lock().unwrap().len(), 1);
        assert_eq!(intraday_probe.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn aged_shared_token_gets_one_401_reissue_and_then_succeeds() {
        let root = PrivateRoot::new("aged-unauthorized");
        let clock = TestClock::at(1_000_000);
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let token_transport = CapturingTransport::with_responses(vec![
            HttpResponse::ok(r#"{"access_token":"first-token","expires_in":3600}"#),
            HttpResponse::ok(r#"{"access_token":"renewed-token","expires_in":3600}"#),
        ]);
        let token_probe = token_transport.clone();
        let read_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output":{}}"#,
        )]);
        let intraday_transport = CapturingTransport::with_responses(vec![
            HttpResponse::status(401, "unauthorized"),
            HttpResponse::ok(r#"{"rt_cd":"0","output":{}}"#),
        ]);
        let intraday_probe = intraday_transport.clone();
        let client = shared_client(
            root.path(),
            clock.clone(),
            read_transport,
            intraday_transport,
            token_transport,
            snapshot,
        );
        let (path, tr_id) = ReadChannel::InquirePrice.pair();
        client
            .get(path, tr_id, &[], None)
            .await
            .expect("prime an existing shared token");
        clock.advance_ms(60_000);

        client
            .get_intraday(path, tr_id, &[])
            .await
            .expect("one aged-token invalidation may reissue and retry");
        assert_eq!(token_probe.requests.lock().unwrap().len(), 2);
        assert_eq!(intraday_probe.requests.lock().unwrap().len(), 2);
        assert!(matches!(
            client.get_intraday(path, tr_id, &[]).await,
            Err(KisError::RateLimited { .. })
        ));
        assert_eq!(
            intraday_probe.requests.lock().unwrap().len(),
            2,
            "successful retry still leaves its shared spacing debt"
        );
    }

    #[tokio::test]
    async fn a_second_shared_401_stops_without_a_refresh_loop_or_third_get() {
        let root = PrivateRoot::new("repeated-unauthorized");
        let clock = TestClock::at(1_500_000);
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let token_transport = CapturingTransport::with_responses(vec![
            HttpResponse::ok(r#"{"access_token":"first-token","expires_in":3600}"#),
            HttpResponse::ok(r#"{"access_token":"renewed-token","expires_in":3600}"#),
        ]);
        let token_probe = token_transport.clone();
        let intraday_transport = CapturingTransport::with_responses(vec![
            HttpResponse::status(401, "unauthorized"),
            HttpResponse::status(401, "unauthorized-again"),
            HttpResponse::ok(r#"{"rt_cd":"0","output":{}}"#),
        ]);
        let intraday_probe = intraday_transport.clone();
        let client = shared_client(
            root.path(),
            clock.clone(),
            CapturingTransport::with_responses(vec![HttpResponse::ok(
                r#"{"rt_cd":"0","output":{}}"#,
            )]),
            intraday_transport,
            token_transport,
            snapshot,
        );
        let (path, tr_id) = ReadChannel::InquirePrice.pair();
        client
            .get(path, tr_id, &[], None)
            .await
            .expect("prime an existing shared token");
        clock.advance_ms(60_000);

        assert!(matches!(
            client.get_intraday(path, tr_id, &[]).await,
            Err(KisError::Broker { status: 401, .. })
        ));
        assert_eq!(token_probe.requests.lock().unwrap().len(), 2);
        assert_eq!(intraday_probe.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn unsafe_shared_root_denies_before_token_or_get_callbacks() {
        let root = PrivateRoot::new("unsafe-root");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let clock = TestClock::at(3_750_000);
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let token_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"access_token":"shared-token","expires_in":3600}"#,
        )]);
        let token_probe = token_transport.clone();
        let read_transport = CapturingTransport::with_responses(vec![HttpResponse::ok(
            r#"{"rt_cd":"0","output":{}}"#,
        )]);
        let read_probe = read_transport.clone();
        let app_key_ref = CredentialRef::env("KIS_APP_KEY");
        let app_secret_ref = CredentialRef::file("/run/secrets/kis_app_secret");
        let issuer: Arc<dyn TokenIssuer> = Arc::new(KisTokenIssuer::new(
            token_transport,
            snapshot.clone(),
            app_key_ref.clone(),
            app_secret_ref.clone(),
            || 3_750_000,
        ));
        let auth = CoordinatedReadAuth::new(
            ReadCoordinationConfig::new(root.path(), protect_test_root(root.path())),
            snapshot,
            Arc::new(clock.clone()),
            Arc::clone(&issuer),
        )
        .unwrap();
        // Make the root unsafe only after capturing its real uid.
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let client = KisMarketDataClient::new(
            read_transport,
            ClockSleeper(clock.clone()),
            Arc::new(TokenManager::new(Arc::new(clock.clone()), issuer)),
            Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(1, 1))),
            FixedCredentials,
            app_key_ref,
            app_secret_ref,
        )
        .with_shared_read_coordination(auth, CapturingTransport::default());
        assert!(matches!(
            client
                .get(
                    ReadChannel::InquirePrice.pair().0,
                    ReadChannel::InquirePrice.pair().1,
                    &[],
                    None,
                )
                .await,
            Err(KisError::Auth { ref reason })
                if reason == "KIS_READ_COORDINATION_STORAGE_UNSAFE"
        ));
        assert!(token_probe.requests.lock().unwrap().is_empty());
        assert!(read_probe.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shared_intraday_retries_at_most_three_actual_get_attempts() {
        let root = PrivateRoot::new("retry-bound");
        let clock = TestClock::at(4_000_000);
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            1,
        )
        .unwrap();
        let intraday_transport = CapturingTransport::with_responses(vec![
            HttpResponse::status(503, "unavailable"),
            HttpResponse::status(503, "unavailable"),
            HttpResponse::status(503, "unavailable"),
        ]);
        let probe = intraday_transport.clone();
        let client = shared_client(
            root.path(),
            clock,
            CapturingTransport::default(),
            intraday_transport,
            CapturingTransport::with_responses(vec![HttpResponse::ok(
                r#"{"access_token":"shared-token","expires_in":3600}"#,
            )]),
            snapshot,
        );
        let error = client
            .get_intraday(
                ReadChannel::InquirePrice.pair().0,
                ReadChannel::InquirePrice.pair().1,
                &[],
            )
            .await
            .expect_err("three provider failures exhaust intraday cycle");
        assert!(matches!(error, KisError::Broker { status: 503, .. }));
        assert_eq!(probe.requests.lock().unwrap().len(), 3);
    }
}
