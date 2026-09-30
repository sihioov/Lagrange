//! KIS market WebSocket Approval-key acquisition.
//!
//! Approval state is intentionally not shared with the REST token manager.
//! The only durable secret-bearing value in this module is written through the
//! protected WS state store; every public error and debug representation is
//! fixed-code/redacted.

use std::fmt;
use std::sync::Arc;

use reqwest::redirect::Policy;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::clock::{Clock, SystemClock};
use crate::market_stream_state::{DurableWsState, MarketStreamDomain, StateError, WsStateStore};
use crate::secret::Secret;

pub const APPROVAL_PATH: &str = "/oauth2/Approval";
pub const APPROVAL_VALIDITY_MS: i64 = 24 * 60 * 60 * 1_000;
pub const MIN_CONNECTION_REMAINING_MS: i64 = 60_000;
pub const APPROVAL_RESERVATION_SPACING_MS: i64 = 60_000;
pub const APPROVAL_ROLLING_WINDOW_MS: i64 = APPROVAL_VALIDITY_MS;
pub const APPROVAL_ROLLING_LIMIT: usize = 2;
pub const MAX_APPROVAL_RESPONSE_BYTES: usize = 8 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ApprovalKey(String);

impl ApprovalKey {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn new(raw: String) -> Result<Self, ApprovalError> {
        if raw.is_empty()
            || raw.len() > MAX_APPROVAL_RESPONSE_BYTES
            || raw.bytes().any(|b| b < 0x20)
        {
            return Err(ApprovalError::ResponseInvalid);
        }
        Ok(Self(raw))
    }
}

impl fmt::Debug for ApprovalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApprovalKey(<redacted>)")
    }
}

impl fmt::Display for ApprovalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalStateSnapshot {
    pub credential_generation: String,
    pub key_present: bool,
    pub issued_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub reservations_in_window: usize,
    pub ambiguous_outcome: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    #[error("market approval endpoint is not allowed")]
    EndpointNotAllowed,
    #[error("market approval request was rejected with HTTP status {status}")]
    HttpRejected { status: u16 },
    #[error("market approval response is invalid")]
    ResponseInvalid,
    #[error("market approval outcome is ambiguous; automatic reissue is disabled")]
    OutcomeAmbiguous,
    #[error("market approval attempt spacing is not available")]
    AttemptTooSoon,
    #[error("market approval attempt budget is exhausted")]
    AttemptBudgetExhausted,
    #[error("market approval credential generation does not match durable state")]
    CredentialGenerationMismatch,
    #[error("market approval key has expired or has less than the connection safety margin")]
    Expired,
    #[error("market approval transport failed after dispatch")]
    TransportAmbiguous,
    #[error("market approval protected state is unavailable")]
    State,
}

impl From<StateError> for ApprovalError {
    fn from(_: StateError) -> Self {
        Self::State
    }
}

#[derive(Clone)]
pub struct ApprovalClient {
    http: reqwest::Client,
    endpoint: String,
    appkey: Secret<String>,
    secretkey: Secret<String>,
    state: WsStateStore,
    domain: MarketStreamDomain,
    credential_generation: String,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for ApprovalClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApprovalClient")
            .field("endpoint", &self.endpoint)
            .field("credential_generation", &self.credential_generation)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl ApprovalClient {
    pub(crate) fn domain(&self) -> MarketStreamDomain {
        self.domain.clone()
    }

    /// Build the fixed production endpoint.  The caller still needs the later
    /// owner activation/rights gates to invoke `key_for_connection`.
    pub fn production(
        appkey: Secret<String>,
        secretkey: Secret<String>,
        domain: MarketStreamDomain,
        credential_generation: impl Into<String>,
    ) -> Result<Self, ApprovalError> {
        Self::new(
            "https://openapi.koreainvestment.com:9443".to_owned(),
            appkey,
            secretkey,
            domain,
            credential_generation.into(),
            Arc::new(SystemClock),
        )
    }

    /// Test-only endpoint injection is deliberately limited to a loopback
    /// origin and cannot be used to redirect the production client.
    #[cfg(any(feature = "test-support", test))]
    pub fn for_loopback(
        base_url: &str,
        appkey: Secret<String>,
        secretkey: Secret<String>,
        domain: MarketStreamDomain,
        credential_generation: impl Into<String>,
    ) -> Result<Self, ApprovalError> {
        let parsed =
            reqwest::Url::parse(base_url).map_err(|_| ApprovalError::EndpointNotAllowed)?;
        if parsed.scheme() != "http"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.username() != ""
            || parsed.password().is_some()
            || !matches!(
                parsed.host_str(),
                Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
            )
            || parsed.port().is_none()
            || (parsed.path() != "/" && !parsed.path().is_empty())
        {
            return Err(ApprovalError::EndpointNotAllowed);
        }
        Self::new(
            base_url.trim_end_matches('/').to_owned(),
            appkey,
            secretkey,
            domain,
            credential_generation.into(),
            Arc::new(SystemClock),
        )
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    fn new(
        base: String,
        appkey: Secret<String>,
        secretkey: Secret<String>,
        domain: MarketStreamDomain,
        credential_generation: String,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ApprovalError> {
        let endpoint = format!("{base}{APPROVAL_PATH}");
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| ApprovalError::EndpointNotAllowed)?;
        let state = domain.state().clone();
        Ok(Self {
            http,
            endpoint,
            appkey,
            secretkey,
            state,
            domain,
            credential_generation,
            clock,
        })
    }

    /// Reuse the durable key when it has the full connection safety margin;
    /// otherwise reserve and issue exactly one new key.
    pub(crate) async fn key_for_connection(&self) -> Result<ApprovalKey, ApprovalError> {
        let now = self.clock.now_ms();
        if let Some(key) = self.cached_key(now)? {
            return Ok(key);
        }
        self.reserve_attempt(now)?;
        let body = ApprovalBody {
            grant_type: "client_credentials",
            appkey: self.appkey.expose(),
            secretkey: self.secretkey.expose(),
        };
        let response = self
            .http
            .post(&self.endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )
            .json(&body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                self.mark_ambiguous()?;
                return Err(ApprovalError::TransportAmbiguous);
            }
        };
        let status = response.status().as_u16();
        if status != 200 {
            self.mark_explicit_failure()?;
            return Err(ApprovalError::HttpRejected { status });
        }
        // Consume DATA frames incrementally.  `Response::bytes()` collects the
        // entire body before returning, which would let a chunked peer grow an
        // unbounded allocation past the adopted 8 KiB reply limit.
        if response
            .content_length()
            .is_some_and(|length| length > MAX_APPROVAL_RESPONSE_BYTES as u64)
        {
            self.mark_ambiguous()?;
            return Err(ApprovalError::ResponseInvalid);
        }
        let mut bytes = Vec::with_capacity(MAX_APPROVAL_RESPONSE_BYTES.min(1024));
        let mut response = response;
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_) => {
                    self.mark_ambiguous()?;
                    return Err(ApprovalError::TransportAmbiguous);
                }
            };
            if bytes.len().saturating_add(chunk.len()) > MAX_APPROVAL_RESPONSE_BYTES {
                self.mark_ambiguous()?;
                return Err(ApprovalError::ResponseInvalid);
            }
            bytes.extend_from_slice(&chunk);
        }
        let parsed: ApprovalResponse = match serde_json::from_slice(&bytes) {
            Ok(parsed) => parsed,
            Err(_) => {
                self.mark_ambiguous()?;
                return Err(ApprovalError::ResponseInvalid);
            }
        };
        let key = match ApprovalKey::new(parsed.approval_key) {
            Ok(key) => key,
            Err(error) => {
                self.mark_ambiguous()?;
                return Err(error);
            }
        };
        self.store_success(now, &key)?;
        Ok(key)
    }

    #[cfg(feature = "test-support")]
    pub async fn test_key_for_connection(&self) -> Result<(), ApprovalError> {
        self.key_for_connection().await.map(|_| ())
    }

    pub fn snapshot(&self) -> Result<ApprovalStateSnapshot, ApprovalError> {
        let state = self.state.load()?;
        let now = self.clock.now_ms();
        Ok(snapshot_from_state(
            &state,
            &self.credential_generation,
            now,
        ))
    }

    fn cached_key(&self, now: i64) -> Result<Option<ApprovalKey>, ApprovalError> {
        let state = self.state.load()?;
        if !state.credential_generation.is_empty()
            && state.credential_generation != self.credential_generation
        {
            return Err(ApprovalError::CredentialGenerationMismatch);
        }
        if state.approval_ambiguous || state.approval_dispatch_in_flight {
            return Err(ApprovalError::OutcomeAmbiguous);
        }
        let Some(key) = state.approval_key else {
            return Ok(None);
        };
        let Some(expires) = state.approval_expires_at_ms else {
            return Err(ApprovalError::ResponseInvalid);
        };
        if expires.saturating_sub(now) < MIN_CONNECTION_REMAINING_MS {
            return Ok(None);
        }
        Ok(Some(ApprovalKey::new(key)?))
    }

    fn reserve_attempt(&self, now: i64) -> Result<(), ApprovalError> {
        self.state
            .with_locked_state(|state| {
                if !state.credential_generation.is_empty()
                    && state.credential_generation != self.credential_generation
                {
                    return Err(StateError::InvalidState);
                }
                if state.approval_ambiguous || state.approval_dispatch_in_flight {
                    return Err(StateError::InvalidState);
                }
                state.approval_reservations_ms.retain(|timestamp| {
                    now.saturating_sub(*timestamp) < APPROVAL_ROLLING_WINDOW_MS
                });
                if state
                    .approval_reservations_ms
                    .last()
                    .is_some_and(|last| now.saturating_sub(*last) < APPROVAL_RESERVATION_SPACING_MS)
                {
                    return Err(StateError::Io);
                }
                if state.approval_reservations_ms.len() >= APPROVAL_ROLLING_LIMIT {
                    return Err(StateError::UnsafePath);
                }
                state.schema = crate::market_stream_state::STATE_SCHEMA.to_owned();
                state.credential_generation = self.credential_generation.clone();
                state.approval_reservations_ms.push(now);
                state.approval_dispatch_in_flight = true;
                Ok(())
            })
            .map_err(|error| match error {
                StateError::Io => ApprovalError::AttemptTooSoon,
                StateError::UnsafePath => ApprovalError::AttemptBudgetExhausted,
                StateError::InvalidState => ApprovalError::OutcomeAmbiguous,
                other => ApprovalError::from(other),
            })
    }

    fn mark_ambiguous(&self) -> Result<(), ApprovalError> {
        self.state.with_locked_state(|state| {
            state.approval_dispatch_in_flight = false;
            state.approval_ambiguous = true;
            Ok(())
        })?;
        Ok(())
    }

    fn mark_explicit_failure(&self) -> Result<(), ApprovalError> {
        self.state.with_locked_state(|state| {
            state.approval_dispatch_in_flight = false;
            Ok(())
        })?;
        Ok(())
    }

    fn store_success(&self, now: i64, key: &ApprovalKey) -> Result<(), ApprovalError> {
        self.state.with_locked_state(|state| {
            state.schema = crate::market_stream_state::STATE_SCHEMA.to_owned();
            state.credential_generation = self.credential_generation.clone();
            state.approval_key = Some(key.0.clone());
            state.approval_issued_at_ms = Some(now);
            state.approval_expires_at_ms = Some(now.saturating_add(APPROVAL_VALIDITY_MS));
            state.approval_ambiguous = false;
            state.approval_dispatch_in_flight = false;
            Ok(())
        })?;
        Ok(())
    }
}

fn snapshot_from_state(
    state: &DurableWsState,
    generation: &str,
    now: i64,
) -> ApprovalStateSnapshot {
    let reservations_in_window = state
        .approval_reservations_ms
        .iter()
        .filter(|timestamp| now.saturating_sub(**timestamp) < APPROVAL_ROLLING_WINDOW_MS)
        .count();
    ApprovalStateSnapshot {
        credential_generation: if state.credential_generation.is_empty() {
            generation.to_owned()
        } else {
            state.credential_generation.clone()
        },
        key_present: state.approval_key.is_some(),
        issued_at_ms: state.approval_issued_at_ms,
        expires_at_ms: state.approval_expires_at_ms,
        reservations_in_window,
        ambiguous_outcome: state.approval_ambiguous || state.approval_dispatch_in_flight,
    }
}

#[derive(Serialize)]
struct ApprovalBody<'a> {
    grant_type: &'static str,
    appkey: &'a str,
    secretkey: &'a str,
}

struct ApprovalResponse {
    approval_key: String,
}

impl<'de> Deserialize<'de> for ApprovalResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ResponseVisitor;
        impl<'de> Visitor<'de> for ResponseVisitor {
            type Value = ApprovalResponse;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an approval response object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut approval_key = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key != "approval_key" || approval_key.is_some() {
                        return Err(de::Error::custom("unexpected or duplicate approval field"));
                    }
                    approval_key = Some(map.next_value::<String>()?);
                }
                Ok(ApprovalResponse {
                    approval_key: approval_key
                        .ok_or_else(|| de::Error::custom("missing approval_key"))?,
                })
            }
        }
        deserializer.deserialize_map(ResponseVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn approval_key_debug_and_error_are_redacted() {
        let key = ApprovalKey::new("SENTINEL_APPROVAL_SECRET".into()).unwrap();
        assert!(!format!("{key:?}").contains("SENTINEL"));
        assert!(!format!("{key}").contains("SENTINEL"));
        let error = ApprovalError::TransportAmbiguous;
        assert!(!error.to_string().contains("SENTINEL"));
    }

    #[test]
    fn loopback_endpoint_is_the_only_injectable_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarketStreamDomain::for_test(dir.path(), Uuid::new_v4()).unwrap();
        assert!(
            ApprovalClient::for_loopback(
                "https://example.invalid:443",
                Secret::new("app".into()),
                Secret::new("secret".into()),
                store.clone(),
                "1"
            )
            .is_err()
        );
        let client = ApprovalClient::for_loopback(
            "http://127.0.0.1:34567",
            Secret::new("app".into()),
            Secret::new("secret".into()),
            store,
            "1",
        )
        .unwrap()
        .with_clock(Arc::new(TestClock::at(0)));
        assert_eq!(format!("{client:?}").contains("secret"), false);
        assert!(!fs::metadata(dir.path()).unwrap().permissions().readonly());
    }

    #[test]
    fn response_rejects_duplicate_or_unknown_fields() {
        assert!(
            serde_json::from_str::<ApprovalResponse>(
                r#"{"approval_key":"one","approval_key":"two"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<ApprovalResponse>(r#"{"message":"no"}"#).is_err());
    }
}
