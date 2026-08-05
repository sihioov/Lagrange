//! Confidential OIDC Authorization Code + PKCE S256 client core.
//!
//! `crates/auth` is the protocol/session authority: the Axum router in
//! `apps/api-server/auth` delegates every login decision to this module. The
//! contract (docs/Lagrange_Station_System_Design_v1.1.md §14.1):
//!
//! - Authorization Code flow with PKCE S256; the verifier is generated
//!   server-side, never seen by the browser.
//! - Exact redirect URI: the configured `redirect_uri` is emitted verbatim in
//!   the authorize request and the token exchange.
//! - `state` (CSRF for the callback) and `nonce` (ID-token replay binding) are
//!   unique per request and consumed single-use server-side.
//! - ID-token validation: RS256 signature against the tenant JWKS (keyed by
//!   `kid`), `iss`/`aud`/`exp` enforcement with bounded clock skew, nonce
//!   match. `auth_time`/`amr` are carried for the Owner step-up gate.
//! - Provider tokens never leave this crate: only the validated claims and the
//!   opaque first-party session cookie cross the boundary to the router.

pub mod claims;
pub mod jwks;
pub mod jwt;
pub mod pkce;

use self::claims::IdTokenClaims;
use self::jwks::Jwk;
use self::jwt::JwtHeader;
use std::sync::{Arc, RwLock};
use url::Url;

pub const DEFAULT_PENDING_TTL_SECS: i64 = 300;

#[derive(Debug, Clone)]
pub struct OidcProviderConfig {
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub audience: Option<String>,
    pub clock_skew_secs: i64,
}

/// Server-side record of an in-flight authorize request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAuth {
    pub state: String,
    pub nonce: String,
    pub code_verifier: String,
    pub created_at_secs: i64,
    pub ttl_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeRequest {
    pub url: Url,
    pub state: String,
    pub nonce: String,
    pub pkce: pkce::PkcePair,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRequest {
    pub code: String,
    pub redirect_uri: String,
    pub client_id: String,
    pub code_verifier: String,
}

/// Only the ID token is captured; `access_token`/`refresh_token` from the
/// provider response are dropped at this boundary by construction (the struct
/// has no fields for them) - browser-held provider tokens are excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenResponse {
    pub id_token: String,
}

impl TokenResponse {
    pub fn from_json(raw: &str) -> Result<Self, OidcError> {
        #[derive(serde::Deserialize)]
        struct Wire {
            id_token: String,
        }
        let wire: Wire = serde_json::from_str(raw)
            .map_err(|e| OidcError::InvalidJwt(format!("bad token response: {e}")))?;
        Ok(Self {
            id_token: wire.id_token,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TransportError(pub String);

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OIDC transport: {}", self.0)
    }
}

#[async_trait::async_trait]
pub trait OidcTransport: Send + Sync {
    async fn exchange_code(&self, request: &TokenRequest) -> Result<TokenResponse, TransportError>;
    async fn fetch_jwks(&self) -> Result<jwks::Jwks, TransportError>;
}

pub struct OidcClient {
    pub config: OidcProviderConfig,
    pub transport: Arc<dyn OidcTransport>,
}

impl OidcClient {
    /// Builds the authorize URL with exact redirect, PKCE S256, state, nonce.
    pub fn begin_authorize(&self) -> Result<AuthorizeRequest, OidcError> {
        let mut url = Url::parse(&self.config.authorize_url).map_err(OidcError::UrlBuild)?;
        let pkce = pkce::generate();
        let state = pkce::random_hex();
        let nonce = pkce::random_hex();
        let mut query = vec![
            ("client_id", self.config.client_id.clone()),
            ("response_type", "code".to_string()),
            ("redirect_uri", self.config.redirect_uri.clone()),
            ("scope", "openid email profile".to_string()),
            ("state", state.clone()),
            ("nonce", nonce.clone()),
            ("code_challenge", pkce.challenge.clone()),
            ("code_challenge_method", "S256".to_string()),
        ];
        if let Some(aud) = &self.config.audience {
            query.push(("audience", aud.clone()));
        }
        url.query_pairs_mut().extend_pairs(query);
        Ok(AuthorizeRequest {
            url,
            state,
            nonce,
            pkce,
        })
    }

    /// Full callback validation: state, pending freshness, code exchange with
    /// the exact redirect URI + verifier, JWKS fetch, ID-token validation.
    pub async fn validate_callback(
        &self,
        code: &str,
        state: &str,
        pending: &PendingAuth,
        now_secs: i64,
    ) -> Result<IdTokenClaims, OidcError> {
        if state != pending.state {
            return Err(OidcError::StateMismatch);
        }
        if now_secs > pending.created_at_secs + pending.ttl_secs {
            return Err(OidcError::PendingExpired);
        }
        let request = TokenRequest {
            code: code.to_string(),
            redirect_uri: self.config.redirect_uri.clone(),
            client_id: self.config.client_id.clone(),
            code_verifier: pending.code_verifier.clone(),
        };
        let response = self
            .transport
            .exchange_code(&request)
            .await
            .map_err(|e| OidcError::Transport(e.to_string()))?;
        let jwks = self
            .transport
            .fetch_jwks()
            .await
            .map_err(|e| OidcError::Transport(e.to_string()))?;
        self.validate_id_token(&response.id_token, &jwks, Some(&pending.nonce), now_secs)
    }

    /// Validates a raw ID token: alg, signature (JWKS kid), iss/aud/exp,
    /// nonce binding. Synchronous for testability.
    pub fn validate_id_token(
        &self,
        raw: &str,
        jwks: &jwks::Jwks,
        expected_nonce: Option<&str>,
        now_secs: i64,
    ) -> Result<IdTokenClaims, OidcError> {
        let (header, payload_b64, _) = jwt::split(raw)?;
        if header.alg != jwt::SUPPORTED_ALG {
            return Err(OidcError::AlgorithmUnsupported(header.alg));
        }
        let jwk = self.select_key(&header, jwks)?;
        jwt::verify_rs256(raw, jwk)?;
        let payload = jwt::decode_payload(&payload_b64)?;
        let claims: IdTokenClaims = serde_json::from_value(payload)
            .map_err(|e| OidcError::InvalidJwt(format!("unparseable claims: {e}")))?;
        self.enforce_claims(&claims, expected_nonce, now_secs)?;
        Ok(claims)
    }

    fn select_key<'a>(
        &self,
        header: &JwtHeader,
        jwks: &'a jwks::Jwks,
    ) -> Result<&'a Jwk, OidcError> {
        match &header.kid {
            Some(kid) => jwks
                .key_for_kid(kid)
                .ok_or_else(|| OidcError::KeyNotFound(kid.clone())),
            None => jwks
                .sole_key()
                .ok_or_else(|| OidcError::KeyNotFound("<no kid and multiple keys>".into())),
        }
    }

    fn enforce_claims(
        &self,
        claims: &IdTokenClaims,
        expected_nonce: Option<&str>,
        now: i64,
    ) -> Result<(), OidcError> {
        if claims.iss != self.config.issuer {
            return Err(OidcError::IssuerMismatch {
                expected: self.config.issuer.clone(),
                got: claims.iss.clone(),
            });
        }
        let expected_aud = self
            .config
            .audience
            .clone()
            .unwrap_or_else(|| self.config.client_id.clone());
        if !claims.aud.iter().any(|a| a == &expected_aud) {
            return Err(OidcError::AudienceMismatch {
                expected: expected_aud,
                got: claims.aud.clone(),
            });
        }
        let skew = self.config.clock_skew_secs;
        if claims.exp + skew < now {
            return Err(OidcError::TokenExpired {
                exp: claims.exp,
                now,
            });
        }
        match (expected_nonce, &claims.nonce) {
            (Some(expected), Some(got)) if expected == got => {}
            (Some(_), _) => return Err(OidcError::NonceMismatch),
            _ => {}
        }
        Ok(())
    }
}

/// Single-use server-side store for in-flight authorize state.
#[async_trait::async_trait]
pub trait PendingAuthStore: Send + Sync {
    async fn insert(&self, state: String, pending: PendingAuth) -> Result<(), OidcError>;
    async fn take(&self, state: &str) -> Result<Option<PendingAuth>, OidcError>;
}

/// In-memory pending-auth store (Todo 22). Bounded by a TTL sweep on insert;
/// the shared PostgreSQL store lands with Todo 3/23 alongside `web_sessions`.
#[derive(Debug, Default)]
pub struct InMemoryPendingAuthStore {
    inner: RwLock<std::collections::HashMap<String, PendingAuth>>,
}

#[async_trait::async_trait]
impl PendingAuthStore for InMemoryPendingAuthStore {
    async fn insert(&self, state: String, pending: PendingAuth) -> Result<(), OidcError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| OidcError::Store("lock".into()))?;
        let cutoff = pending.created_at_secs;
        map.retain(|_, p| p.created_at_secs + p.ttl_secs >= cutoff);
        map.insert(state, pending);
        Ok(())
    }

    async fn take(&self, state: &str) -> Result<Option<PendingAuth>, OidcError> {
        let mut map = self
            .inner
            .write()
            .map_err(|_| OidcError::Store("lock".into()))?;
        Ok(map.remove(state))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("PKCE verifier is invalid")]
    PkceInvalid,
    #[error("cannot build provider URL: {0}")]
    UrlBuild(url::ParseError),
    #[error("callback state does not match the pending login")]
    StateMismatch,
    #[error("pending login expired")]
    PendingExpired,
    #[error("provider transport failure: {0}")]
    Transport(String),
    #[error("JWT is malformed: {0}")]
    InvalidJwt(String),
    #[error("JWT algorithm {0} is not supported (only RS256)")]
    AlgorithmUnsupported(String),
    #[error("JWT signature verification failed")]
    SignatureInvalid,
    #[error("no JWKS key for kid {0}")]
    KeyNotFound(String),
    #[error("JWKS key type {0} is not RSA")]
    UnsupportedKeyType(String),
    #[error("JWKS key use {0} is not sig")]
    KeyNotForSigning(String),
    #[error("JWKS key is missing {0} material")]
    MissingKeyMaterial(String),
    #[error("issuer mismatch: expected {expected}, got {got}")]
    IssuerMismatch { expected: String, got: String },
    #[error("audience mismatch: expected {expected}, got {got:?}")]
    AudienceMismatch { expected: String, got: Vec<String> },
    #[error("token expired at {exp}, now {now}")]
    TokenExpired { exp: i64, now: i64 },
    #[error("nonce mismatch")]
    NonceMismatch,
    #[error("invalid JWKS document: {0}")]
    InvalidJwks(String),
    #[error("base64url decode failed")]
    Base64Decode,
    #[error("pending-auth store failure: {0}")]
    Store(String),
}

impl OidcError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PkceInvalid => "PKCE_INVALID",
            Self::UrlBuild(_) => "URL_BUILD_FAILED",
            Self::StateMismatch => "STATE_MISMATCH",
            Self::PendingExpired => "PENDING_EXPIRED",
            Self::Transport(_) => "PROVIDER_TRANSPORT",
            Self::InvalidJwt(_) => "INVALID_JWT",
            Self::AlgorithmUnsupported(_) => "ALGORITHM_UNSUPPORTED",
            Self::SignatureInvalid => "SIGNATURE_INVALID",
            Self::KeyNotFound(_) => "KEY_NOT_FOUND",
            Self::UnsupportedKeyType(_) => "KEY_TYPE_UNSUPPORTED",
            Self::KeyNotForSigning(_) => "KEY_NOT_FOR_SIGNING",
            Self::MissingKeyMaterial(_) => "KEY_MATERIAL_MISSING",
            Self::IssuerMismatch { .. } => "ISSUER_MISMATCH",
            Self::AudienceMismatch { .. } => "AUDIENCE_MISMATCH",
            Self::TokenExpired { .. } => "TOKEN_EXPIRED",
            Self::NonceMismatch => "NONCE_MISMATCH",
            Self::InvalidJwks(_) => "INVALID_JWKS",
            Self::Base64Decode => "BASE64_DECODE",
            Self::Store(_) => "PENDING_STORE",
        }
    }
}
