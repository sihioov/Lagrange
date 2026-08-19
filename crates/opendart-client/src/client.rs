//! The public client: path allowlist enforcement, credential attachment,
//! single-flight + 1-req/sec pacing, and bounded retries. Owns no response
//! parsing -- callers get raw bytes back.
//!
//! [`OpenDartClient`] is deliberately concrete (not generic over a
//! transport): the pluggable-transport machinery ([`Transport`],
//! `ClientCore`) stays crate-private so this crate's public interface is
//! exactly the shape the task calls for -- a client built from a credential
//! source plus a config, and one `get` method -- with nothing about the
//! internal HTTP seam leaking into it.

use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::allowlist;
use crate::credential::{CredentialSource, SystemCredentialSource};
use crate::error::OpenDartTransportError;
use crate::rate::{RateState, wait_duration};
use crate::status::{StatusClass, classify_status, is_retryable_failure, is_retryable_status};
use crate::transport::{HttpRequest, LiveTransport, OPENDART_BASE_URL, Transport, classify};

/// Retries are capped low and apply only to failures that plausibly mean
/// "try again": a connect failure that never reached the server, or a
/// `5xx`. Redirects, `4xx`, and OpenDART's own `020`/`021` application
/// statuses are always terminal -- seek `error::ApplicationStatus` for why
/// `020`/`021` specifically must never be probed or backed off into.
const MAX_RETRIES: u32 = 2;

/// Timeouts and pacing. All fields are required -- there is no "default
/// timeout" path, matching the house rule against relying on library
/// defaults for network calls.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// The 1-request-per-second ceiling. Pass `Duration::from_secs(1)` for
    /// the house convention; tests may pass `Duration::ZERO` to skip
    /// pacing entirely.
    pub min_request_interval: Duration,
}

/// The literal minimum body inspection this crate permits: a byte-level
/// scan for OpenDART's own `020`/`021` application status strings, so the
/// retry loop can fail closed on them without ever parsing or retaining the
/// body. Runs on every response regardless of path -- OpenDART returns the
/// same small JSON error envelope for all three surfaces on failure, so
/// `corpCode.xml`'s successful ZIP body is the only shape that could ever
/// spuriously contain one of these byte strings, which is astronomically
/// unlikely for compressed binary data and, if it ever happened, would only
/// misclassify a success as a terminal error -- never the reverse, and
/// never a leak.
fn detect_terminal_application_status(body: &[u8]) -> Option<crate::error::ApplicationStatus> {
    use crate::error::ApplicationStatus;

    const REQUEST_LIMIT_PATTERNS: [&[u8]; 2] = [br#""status":"020""#, br#""status": "020""#];
    const COMPANY_COUNT_PATTERNS: [&[u8]; 2] = [br#""status":"021""#, br#""status": "021""#];

    if REQUEST_LIMIT_PATTERNS.iter().any(|pat| contains(body, pat)) {
        return Some(ApplicationStatus::RequestLimitExceeded);
    }
    if COMPANY_COUNT_PATTERNS.iter().any(|pat| contains(body, pat)) {
        return Some(ApplicationStatus::CompanyCountExceeded);
    }
    None
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The generic engine behind [`OpenDartClient`]. Crate-private: real
/// callers only ever see `ClientCore<LiveTransport>` through the concrete
/// wrapper below, and tests instantiate `ClientCore<SomeFakeTransport>`
/// directly since they live in this same module tree.
struct ClientCore<T: Transport> {
    transport: T,
    credentials: Box<dyn CredentialSource + Send + Sync>,
    // Single-flight AND the rate gate share one lock: it is held for the
    // entire lifetime of a `get` call (including retries), so at most one
    // request is ever in flight, and it carries the last-sent timestamp
    // the gate needs.
    state: Mutex<RateState>,
    config: ClientConfig,
}

impl<T: Transport> ClientCore<T> {
    fn new(
        transport: T,
        credentials: Box<dyn CredentialSource + Send + Sync>,
        config: ClientConfig,
    ) -> Self {
        Self {
            transport,
            credentials,
            state: Mutex::new(RateState::default()),
            config,
        }
    }

    async fn get(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Vec<u8>, OpenDartTransportError> {
        let allowed_path =
            allowlist::resolve(path).ok_or(OpenDartTransportError::PathNotAllowed)?;

        // Loaded before the single-flight lock: a local file read, not a
        // network request, so it does not need to be paced or serialized
        // against other in-flight HTTP calls.
        let key = self.credentials.load()?;

        let mut full_query: Vec<(String, String)> = query.to_vec();
        full_query.push(("crtfc_key".to_string(), key.expose().clone()));
        let request = HttpRequest {
            path: allowed_path,
            query: full_query,
        };

        let mut state = self.state.lock().await;
        let mut attempt: u32 = 0;
        loop {
            let wait = wait_duration(
                state.last_sent,
                Instant::now(),
                self.config.min_request_interval,
            );
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }

            let outcome = self.transport.send(&request).await;
            state.last_sent = Some(Instant::now());

            match outcome {
                Err(failure) => {
                    if attempt < MAX_RETRIES && is_retryable_failure(failure) {
                        attempt += 1;
                        continue;
                    }
                    return Err(classify(failure));
                }
                Ok(response) => {
                    let class = classify_status(response.status);
                    if class == StatusClass::Redirect {
                        return Err(OpenDartTransportError::Redirected {
                            status: response.status,
                        });
                    }
                    if is_retryable_status(class) {
                        if attempt < MAX_RETRIES {
                            attempt += 1;
                            continue;
                        }
                        return Err(OpenDartTransportError::UnexpectedStatus {
                            status: response.status,
                        });
                    }
                    if class == StatusClass::ClientError {
                        return Err(OpenDartTransportError::UnexpectedStatus {
                            status: response.status,
                        });
                    }
                    // class == StatusClass::Success
                    if let Some(app_status) = detect_terminal_application_status(&response.body) {
                        return Err(OpenDartTransportError::ApplicationStatus(app_status));
                    }
                    return Ok(response.body);
                }
            }
        }
    }
}

/// The public OpenDART client. Construct with [`OpenDartClient::new`],
/// fetch bytes with [`OpenDartClient::get`]. That is the entire surface --
/// there is no way to bypass the path allowlist, no way to supply
/// `crtfc_key` yourself, and no way to reach the transport layer directly.
pub struct OpenDartClient {
    core: ClientCore<LiveTransport>,
}

impl OpenDartClient {
    /// Builds a client against the real OpenDART host. `credentials` is
    /// typically `Box::new(SystemCredentialSource::default())`, which reads
    /// the key from the file named by `OPENDART_CRTFC_KEY_FILE`; a caller
    /// may supply any other [`CredentialSource`] (e.g. one built with a
    /// non-default env var name via `SystemCredentialSource::new`).
    pub fn new(
        credentials: Box<dyn CredentialSource + Send + Sync>,
        config: ClientConfig,
    ) -> Result<Self, OpenDartTransportError> {
        let transport = LiveTransport::new(
            OPENDART_BASE_URL,
            config.connect_timeout,
            config.read_timeout,
        )?;
        Ok(Self {
            core: ClientCore::new(transport, credentials, config),
        })
    }

    /// Convenience constructor using [`SystemCredentialSource::default`]
    /// (the `OPENDART_CRTFC_KEY_FILE` environment variable).
    pub fn with_default_credentials(config: ClientConfig) -> Result<Self, OpenDartTransportError> {
        Self::new(Box::new(SystemCredentialSource::default()), config)
    }

    /// Fetches `path` (must be one of [`crate::allowlist::ALLOWED_PATHS`])
    /// with `query`, plus the `crtfc_key` this client appends itself --
    /// callers never pass it. Returns raw, uninterpreted bytes; this crate
    /// does not validate the OpenDART response schema.
    pub async fn get(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Vec<u8>, OpenDartTransportError> {
        self.core.get(path, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::CredentialError;
    use crate::transport::{Failure, HttpResponse};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn test_config() -> ClientConfig {
        ClientConfig {
            connect_timeout: Duration::from_millis(50),
            read_timeout: Duration::from_millis(50),
            min_request_interval: Duration::ZERO,
        }
    }

    struct FixedCredential;
    impl CredentialSource for FixedCredential {
        fn load(&self) -> Result<crate::credential::Secret<String>, CredentialError> {
            Ok(crate::credential::Secret::new(
                "test-key-never-real".to_string(),
            ))
        }
    }

    struct AlwaysNeverSent {
        calls: AtomicU32,
    }
    impl Transport for AlwaysNeverSent {
        async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(Failure::NeverSent)
        }
    }

    struct AlwaysServerError {
        calls: AtomicU32,
    }
    impl Transport for AlwaysServerError {
        async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HttpResponse {
                status: 503,
                body: Vec::new(),
            })
        }
    }

    struct AlwaysRedirect {
        calls: AtomicU32,
    }
    impl Transport for AlwaysRedirect {
        async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HttpResponse {
                status: 301,
                body: Vec::new(),
            })
        }
    }

    struct AlwaysRequestLimit {
        calls: AtomicU32,
    }
    impl Transport for AlwaysRequestLimit {
        async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HttpResponse {
                status: 200,
                body: br#"{"status":"020","message":"limit"}"#.to_vec(),
            })
        }
    }

    struct PanicTransport;
    impl Transport for PanicTransport {
        async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
            panic!("transport must never be called for a disallowed path");
        }
    }

    struct RecordingTransport {
        seen_query: std::sync::Mutex<Option<Vec<(String, String)>>>,
    }
    impl Transport for RecordingTransport {
        async fn send(&self, request: &HttpRequest) -> Result<HttpResponse, Failure> {
            *self.seen_query.lock().unwrap() = Some(request.query.clone());
            Ok(HttpResponse {
                status: 200,
                body: br#"{"status":"000"}"#.to_vec(),
            })
        }
    }

    struct ConcurrencyTrackingTransport {
        in_flight: AtomicU32,
        max_in_flight: AtomicU32,
    }
    impl Transport for ConcurrencyTrackingTransport {
        async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
            let now_in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight
                .fetch_max(now_in_flight, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(HttpResponse {
                status: 200,
                body: br#"{"status":"000"}"#.to_vec(),
            })
        }
    }

    #[tokio::test]
    async fn disallowed_path_is_rejected_before_any_request_is_built() {
        let core = ClientCore::new(PanicTransport, Box::new(FixedCredential), test_config());
        let result = core.get("/api/not-on-the-allowlist.json", &[]).await;
        assert_eq!(result.unwrap_err(), OpenDartTransportError::PathNotAllowed);
    }

    #[tokio::test]
    async fn never_sent_retries_up_to_the_bound_then_fails_closed() {
        let core = ClientCore::new(
            AlwaysNeverSent {
                calls: AtomicU32::new(0),
            },
            Box::new(FixedCredential),
            test_config(),
        );
        let result = core.get("/api/list.json", &[]).await;
        assert_eq!(result.unwrap_err(), OpenDartTransportError::NeverSent);
        assert_eq!(core.transport.calls.load(Ordering::SeqCst), 1 + MAX_RETRIES);
    }

    #[tokio::test]
    async fn server_error_retries_up_to_the_bound_then_fails_closed() {
        let core = ClientCore::new(
            AlwaysServerError {
                calls: AtomicU32::new(0),
            },
            Box::new(FixedCredential),
            test_config(),
        );
        let result = core.get("/api/company.json", &[]).await;
        assert_eq!(
            result.unwrap_err(),
            OpenDartTransportError::UnexpectedStatus { status: 503 }
        );
        assert_eq!(core.transport.calls.load(Ordering::SeqCst), 1 + MAX_RETRIES);
    }

    #[tokio::test]
    async fn redirect_is_terminal_with_no_retry() {
        let core = ClientCore::new(
            AlwaysRedirect {
                calls: AtomicU32::new(0),
            },
            Box::new(FixedCredential),
            test_config(),
        );
        let result = core.get("/api/corpCode.xml", &[]).await;
        assert_eq!(
            result.unwrap_err(),
            OpenDartTransportError::Redirected { status: 301 }
        );
        assert_eq!(core.transport.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn application_status_020_is_terminal_with_no_retry() {
        let core = ClientCore::new(
            AlwaysRequestLimit {
                calls: AtomicU32::new(0),
            },
            Box::new(FixedCredential),
            test_config(),
        );
        let result = core.get("/api/list.json", &[]).await;
        assert_eq!(
            result.unwrap_err(),
            OpenDartTransportError::ApplicationStatus(
                crate::error::ApplicationStatus::RequestLimitExceeded
            )
        );
        assert_eq!(core.transport.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn client_appends_crtfc_key_itself_and_preserves_caller_query() {
        let core = ClientCore::new(
            RecordingTransport {
                seen_query: std::sync::Mutex::new(None),
            },
            Box::new(FixedCredential),
            test_config(),
        );
        let caller_query = vec![("corp_code".to_string(), "00126380".to_string())];
        let result = core.get("/api/company.json", &caller_query).await;
        assert!(result.is_ok());

        let seen = core.transport.seen_query.lock().unwrap().clone().unwrap();
        assert!(
            seen.iter()
                .any(|(k, v)| k == "crtfc_key" && v == "test-key-never-real")
        );
        assert!(
            seen.iter()
                .any(|(k, v)| k == "corp_code" && v == "00126380")
        );
    }

    #[tokio::test]
    async fn credential_failure_surfaces_as_a_credential_error() {
        struct AlwaysMissingCredential;
        impl CredentialSource for AlwaysMissingCredential {
            fn load(&self) -> Result<crate::credential::Secret<String>, CredentialError> {
                Err(CredentialError::EnvVarMissing)
            }
        }
        let core = ClientCore::new(
            PanicTransport,
            Box::new(AlwaysMissingCredential),
            test_config(),
        );
        let result = core.get("/api/list.json", &[]).await;
        assert_eq!(
            result.unwrap_err(),
            OpenDartTransportError::Credential(CredentialError::EnvVarMissing)
        );
    }

    #[tokio::test]
    async fn single_flight_serializes_concurrent_callers() {
        let core = Arc::new(ClientCore::new(
            ConcurrencyTrackingTransport {
                in_flight: AtomicU32::new(0),
                max_in_flight: AtomicU32::new(0),
            },
            Box::new(FixedCredential),
            test_config(),
        ));

        let a = {
            let core = core.clone();
            tokio::spawn(async move { core.get("/api/list.json", &[]).await })
        };
        let b = {
            let core = core.clone();
            tokio::spawn(async move { core.get("/api/list.json", &[]).await })
        };
        let (result_a, result_b) = tokio::join!(a, b);
        result_a.unwrap().unwrap();
        result_b.unwrap().unwrap();

        assert_eq!(core.transport.max_in_flight.load(Ordering::SeqCst), 1);
    }
}
