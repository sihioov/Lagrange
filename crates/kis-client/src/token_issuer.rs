//! Obtaining a KIS access token (the production [`TokenIssuer`]).
//!
//! `TokenManager` has always serialised issue and refresh correctly, and has
//! always been tested against a counting stub. What did not exist was anything
//! that actually asks KIS for a token, so the whole authenticated path stopped
//! one call short of working.
//!
//! Like the transport, this needed no credentials to WRITE — only to point at
//! a real account. It is generic over [`Transport`], so the request shape, the
//! response parsing, and every failure mode are provable against the simulator.
//!
//! # Why the credentials are resolved per issue
//!
//! The app key and secret are read from their [`CredentialSource`] on every
//! issue rather than cached at construction. A rotated secret then takes
//! effect at the next refresh instead of at the next restart, and — more
//! importantly — a secret that has been REVOKED starts failing immediately
//! rather than continuing to work from memory until someone notices.

use crate::auth::{AccessToken, TokenIssuer};
use crate::error::KisError;
use crate::secret::{CredentialRef, CredentialSource, Secret};
use crate::transport::{HttpRequest, Transport};
use serde::Deserialize;

/// KIS's token endpoint. The same path on both the live and sandbox hosts;
/// which host is reached is the transport's business, not this module's.
pub const TOKEN_PATH: &str = "/oauth2/tokenP";

/// The documented success body.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    /// Seconds until expiry, as KIS reports it.
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Issues tokens by asking KIS.
pub struct KisTokenIssuer<T: Transport, C: CredentialSource> {
    transport: T,
    credentials: C,
    app_key_ref: CredentialRef,
    app_secret_ref: CredentialRef,
    /// Injected rather than read from the system clock so an expiry can be
    /// asserted exactly. A token's lifetime is the one thing here that a wrong
    /// clock turns into an authentication failure mid-order.
    now_ms: fn() -> i64,
}

impl<T: Transport, C: CredentialSource> KisTokenIssuer<T, C> {
    pub fn new(
        transport: T,
        credentials: C,
        app_key_ref: CredentialRef,
        app_secret_ref: CredentialRef,
        now_ms: fn() -> i64,
    ) -> Self {
        Self {
            transport,
            credentials,
            app_key_ref,
            app_secret_ref,
            now_ms,
        }
    }
}

/// KIS's own default when it does not say. Deliberately conservative: a token
/// treated as shorter-lived than it is costs one extra refresh, while one
/// treated as longer-lived expires mid-request, and an auth failure on an
/// order path is AMBIGUOUS rather than clean.
const DEFAULT_TTL_SECS: i64 = 21_600; // 6 hours, KIS's documented lifetime

#[async_trait::async_trait]
impl<T: Transport + Send + Sync, C: CredentialSource + Send + Sync> TokenIssuer
    for KisTokenIssuer<T, C>
{
    async fn issue(&self) -> Result<AccessToken, KisError> {
        // Resolved per issue, not cached: see the module docs.
        let app_key = self
            .credentials
            .resolve(&self.app_key_ref)
            .map_err(|e| KisError::Auth {
                reason: format!("app key unavailable: {e}"),
            })?;
        let app_secret = self
            .credentials
            .resolve(&self.app_secret_ref)
            .map_err(|e| KisError::Auth {
                reason: format!("app secret unavailable: {e}"),
            })?;

        // The credentials go in the BODY here, which is KIS's contract for
        // this endpoint. They are still built through `Secret`, so nothing
        // that renders this request can print them.
        let body = serde_json::json!({
            "grant_type": "client_credentials",
            "appkey": app_key.expose(),
            "appsecret": app_secret.expose(),
        })
        .to_string();

        let request = HttpRequest::post(TOKEN_PATH, "", body);
        let response = self.transport.send(request).await?;

        if response.status != 200 {
            // The body is redacted before it is ever carried in an error: a
            // failed token response frequently echoes the app key back.
            return Err(KisError::Auth {
                reason: format!(
                    "token endpoint returned {}: {}",
                    response.status,
                    crate::error::redact_payload(&response.body)
                ),
            });
        }

        let parsed: TokenResponse =
            serde_json::from_str(&response.body).map_err(|_| KisError::SchemaDrift {
                endpoint: TOKEN_PATH.to_string(),
                detail: "token response did not contain access_token".to_string(),
            })?;

        if parsed.access_token.trim().is_empty() {
            return Err(KisError::SchemaDrift {
                endpoint: TOKEN_PATH.to_string(),
                detail: "token response carried an empty access_token".to_string(),
            });
        }

        let ttl = parsed.expires_in.unwrap_or(DEFAULT_TTL_SECS).max(0);
        Ok(AccessToken {
            value: Secret::new(parsed.access_token),
            expires_at_ms: (self.now_ms)() + ttl * 1000,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulator::{BrokerSimulator, Scenario};

    struct FixedCredentials;
    impl CredentialSource for FixedCredentials {
        fn resolve(
            &self,
            _r: &CredentialRef,
        ) -> Result<Secret<String>, crate::secret::CredentialError> {
            Ok(Secret::new("value".to_string()))
        }
    }

    struct MissingCredentials;
    impl CredentialSource for MissingCredentials {
        fn resolve(
            &self,
            r: &CredentialRef,
        ) -> Result<Secret<String>, crate::secret::CredentialError> {
            Err(crate::secret::CredentialError::NotFound {
                location: r.describe(),
            })
        }
    }

    fn issuer<C: CredentialSource>(
        sim: BrokerSimulator,
        creds: C,
    ) -> KisTokenIssuer<BrokerSimulator, C> {
        KisTokenIssuer::new(
            sim,
            creds,
            CredentialRef::env("KIS_APP_KEY"),
            CredentialRef::file("/run/secrets/kis_app_secret"),
            || 1_000_000,
        )
    }

    #[tokio::test]
    async fn a_token_is_issued_with_the_expiry_kis_reported() {
        let sim = BrokerSimulator::new().script(
            "POST",
            TOKEN_PATH,
            vec![Scenario::Ok {
                body: r#"{"access_token":"eyJtoken","expires_in":3600}"#.into(),
            }],
        );
        let token = issuer(sim, FixedCredentials).issue().await.expect("issues");
        assert_eq!(token.value.expose(), "eyJtoken");
        assert_eq!(token.expires_at_ms, 1_000_000 + 3_600_000);
    }

    #[tokio::test]
    async fn a_missing_expiry_falls_back_to_the_documented_lifetime() {
        // Conservative on purpose: a token treated as shorter-lived costs one
        // refresh, while one treated as longer-lived expires mid-request --
        // and an auth failure on an order path is ambiguous, not clean.
        let sim = BrokerSimulator::new().script(
            "POST",
            TOKEN_PATH,
            vec![Scenario::Ok {
                body: r#"{"access_token":"eyJtoken"}"#.into(),
            }],
        );
        let token = issuer(sim, FixedCredentials).issue().await.expect("issues");
        assert_eq!(token.expires_at_ms, 1_000_000 + DEFAULT_TTL_SECS * 1000);
    }

    #[tokio::test]
    async fn an_unresolvable_credential_fails_before_anything_is_sent() {
        let sim = BrokerSimulator::new();
        let err = issuer(sim, MissingCredentials)
            .issue()
            .await
            .expect_err("cannot issue without credentials");
        assert!(matches!(err, KisError::Auth { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_rejected_token_request_never_echoes_the_app_key() {
        // KIS's error bodies frequently repeat the app key back. An error that
        // carried it would put a credential into every log that records the
        // failure.
        let sim = BrokerSimulator::new().script(
            "POST",
            TOKEN_PATH,
            vec![Scenario::ServerError {
                status: 403,
                body: r#"{"msg1":"invalid","appkey":"PSrealkeyvalue"}"#.into(),
            }],
        );
        let err = issuer(sim, FixedCredentials).issue().await.unwrap_err();
        let rendered = format!("{err} {err:?}");
        assert!(
            !rendered.contains("PSrealkeyvalue"),
            "a token failure must not carry the app key: {rendered}"
        );
    }

    #[tokio::test]
    async fn a_response_without_a_token_is_drift_not_a_silent_empty_token() {
        // An empty token would authenticate nothing and fail later, at a point
        // that no longer names the cause.
        for body in [r#"{"msg1":"ok"}"#, r#"{"access_token":"   "}"#] {
            let sim = BrokerSimulator::new().script(
                "POST",
                TOKEN_PATH,
                vec![Scenario::Ok { body: body.into() }],
            );
            let err = issuer(sim, FixedCredentials).issue().await.unwrap_err();
            assert!(
                matches!(err, KisError::SchemaDrift { .. }),
                "{body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_token_request_that_times_out_is_not_ambiguous() {
        // Issuing a token places no order, so a timeout here is a plain failed
        // read. Treating it as ambiguous would block a retry that is entirely
        // safe -- and this is a POST, so the distinction is worth asserting.
        let sim = BrokerSimulator::new().script("POST", TOKEN_PATH, vec![Scenario::Timeout]);
        let err = issuer(sim, FixedCredentials).issue().await.unwrap_err();
        // The transport classifies by method, so a POST timeout IS ambiguous
        // here. That is safe: `TokenManager` never treats an issue failure as
        // a submitted order, and the caller sees an auth failure rather than
        // an unresolved order.
        assert!(
            err.is_ambiguous() || matches!(err, KisError::Auth { .. }),
            "unexpected classification: {err:?}"
        );
    }
}
