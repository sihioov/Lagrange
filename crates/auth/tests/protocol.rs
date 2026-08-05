//! Todo 22 RED suite: OIDC protocol core.
//!
//! PKCE S256 (RFC 7636), exact redirect URI, state+nonce binding, and
//! RS256 JWT/JWKS validation with issuer/audience/expiry enforcement.
//! Run: `cargo test -p auth protocol -- --nocapture`

use auth::oidc::{
    self, AuthorizeRequest, OidcClient, OidcError, OidcProviderConfig, OidcTransport, PendingAuth,
    PendingAuthStore, TokenRequest, TokenResponse, TransportError,
};
use auth::simulator::Simulator;
use serde_json::json;
use std::sync::{Arc, Mutex};
use url::Url;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn config(issuer: &str, redirect_uri: &str) -> OidcProviderConfig {
    OidcProviderConfig {
        issuer: issuer.to_string(),
        client_id: "lagrange-app".to_string(),
        redirect_uri: redirect_uri.to_string(),
        authorize_url: format!("{issuer}/authorize"),
        token_url: format!("{issuer}/oauth/token"),
        jwks_url: format!("{issuer}/.well-known/jwks.json"),
        audience: Some("https://api.lagrange.local".to_string()),
        clock_skew_secs: 60,
    }
}

fn pending(state: &str, nonce: &str, verifier: &str) -> PendingAuth {
    PendingAuth {
        state: state.to_string(),
        nonce: nonce.to_string(),
        code_verifier: verifier.to_string(),
        created_at_secs: now(),
        ttl_secs: 300,
    }
}

struct RecordingTransport {
    seen: Mutex<Vec<TokenRequest>>,
}

impl OidcTransport for RecordingTransport {
    async fn exchange_code(&self, request: &TokenRequest) -> Result<TokenResponse, TransportError> {
        self.seen.lock().unwrap().push(request.clone());
        Ok(TokenResponse {
            id_token: "irrelevant".to_string(),
        })
    }

    async fn fetch_jwks(&self) -> Result<oidc::jwks::Jwks, TransportError> {
        Err(TransportError("not used".to_string()))
    }
}

#[test]
fn pkce_s256_rfc7636_vector() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        oidc::pkce::s256_challenge(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn pkce_verifier_is_well_formed() {
    for _ in 0..32 {
        let pair = oidc::pkce::generate();
        assert!(
            (43..=128).contains(&pair.verifier.len()),
            "verifier length {} outside 43..=128",
            pair.verifier.len()
        );
        assert!(
            pair.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')),
            "verifier contains a char outside the PKCE alphabet"
        );
        assert_eq!(pair.challenge, oidc::pkce::s256_challenge(&pair.verifier));
    }
}

fn authorize(cfg: &OidcProviderConfig) -> AuthorizeRequest {
    let transport = Arc::new(RecordingTransport {
        seen: Mutex::new(Vec::new()),
    });
    let client = OidcClient {
        config: cfg.clone(),
        transport,
    };
    client.begin_authorize().expect("authorize URL builds")
}

#[test]
fn authorize_url_carries_exact_redirect_and_pkce() {
    let cfg = config("https://lagrange-test.auth0.com", "https://app.lagrange.local/auth/callback");
    let req = authorize(&cfg);
    let url = Url::parse(&req.url.to_string()).expect("parses");

    assert_eq!(url.as_str(), req.url.as_str());
    let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    assert_eq!(get("client_id").as_deref(), Some("lagrange-app"));
    assert_eq!(get("response_type").as_deref(), Some("code"));
    assert_eq!(get("redirect_uri").as_deref(), Some("https://app.lagrange.local/auth/callback"));
    assert_eq!(get("code_challenge_method").as_deref(), Some("S256"));
    let challenge = get("code_challenge").expect("code_challenge present");
    assert_eq!(challenge, oidc::pkce::s256_challenge(&req.pkce.verifier));
    assert_eq!(get("scope").as_deref(), Some("openid email profile"));
    assert!(get("audience").is_some());
    assert!(get("state").is_some());
    assert!(get("nonce").is_some());
}

#[test]
fn state_nonce_and_verifier_are_unique_per_request() {
    let cfg = config("https://lagrange-test.auth0.com", "https://app.lagrange.local/auth/callback");
    let a = authorize(&cfg);
    let b = authorize(&cfg);
    assert_ne!(a.state, b.state, "state must be unique per request");
    assert_ne!(a.nonce, b.nonce, "nonce must be unique per request");
    assert_ne!(a.pkce.verifier, b.pkce.verifier, "verifier must be unique per request");
    assert_ne!(a.url.to_string(), b.url.to_string());
}

#[test]
fn token_request_carries_exact_redirect_verifier_and_client() {
    let cfg = config("https://lagrange-test.auth0.com", "https://app.lagrange.local/auth/callback");
    let transport = Arc::new(RecordingTransport {
        seen: Mutex::new(Vec::new()),
    });
    let client = OidcClient {
        config: cfg.clone(),
        transport: transport.clone(),
    };
    let req = authorize(&cfg);
    let pen = pending(&req.state, &req.nonce, &req.pkce.verifier);

    tokio::runtime::Runtime::new().unwrap().block_on(async { client.validate_callback("auth-code-1", &req.state, &pen, now()).await });

    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let token_req = &seen[0];
    assert_eq!(token_req.code, "auth-code-1");
    assert_eq!(token_req.redirect_uri, "https://app.lagrange.local/auth/callback");
    assert_eq!(token_req.client_id, "lagrange-app");
    assert_eq!(token_req.code_verifier, req.pkce.verifier);
}

#[test]
fn callback_with_wrong_state_is_denied() {
    let cfg = config("https://lagrange-test.auth0.com", "https://app.lagrange.local/auth/callback");
    let sim = Simulator::new(&cfg.issuer, &cfg.client_id, &cfg.redirect_uri);
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let req = authorize(&cfg);
    let pen = pending("expected-state", "nonce-x", "verifier-x");
    let code = sim.issue_code(&req, json!({}), &req.pkce.verifier);
    let err = tokio::runtime::Runtime::new().unwrap().block_on(async {
        client.validate_callback(&code, "attacker-state", &pen, now()).await
    }).expect_err("wrong state must be denied");
    assert!(matches!(err, OidcError::StateMismatch), "got {err:?}");
}

#[test]
fn callback_with_expired_pending_auth_is_denied() {
    let cfg = config("https://lagrange-test.auth0.com", "https://app.lagrange.local/auth/callback");
    let sim = Simulator::new(&cfg.issuer, &cfg.client_id, &cfg.redirect_uri);
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let req = authorize(&cfg);
    let code = sim.issue_code(&req, json!({}), &req.pkce.verifier);
    let stale = PendingAuth {
        state: req.state.clone(),
        nonce: req.nonce.clone(),
        code_verifier: req.pkce.verifier.clone(),
        created_at_secs: now() - 600,
        ttl_secs: 300,
    };
    let err = tokio::runtime::Runtime::new().unwrap().block_on(async {
        client.validate_callback(&code, &req.state, &stale, now()).await
    }).expect_err("expired pending auth must be denied");
    assert!(matches!(err, OidcError::PendingExpired), "got {err:?}");
}

#[test]
fn valid_id_token_validates() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() + 3600,
        "iat": now(),
        "nonce": "nonce-abc",
        "email": "user@example.com",
        "email_verified": true,
        "auth_time": now() - 60,
        "amr": ["pwd", "mfa"]
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let claims = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect("valid token passes");
    assert_eq!(claims.sub, "auth0|usr-1");
    assert_eq!(claims.iss, sim.issuer);
    assert_eq!(claims.email.as_deref(), Some("user@example.com"));
    assert_eq!(claims.amr, vec!["pwd".to_string(), "mfa".to_string()]);
}

#[test]
fn wrong_issuer_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": "https://evil.example.com",
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() + 3600,
        "nonce": "nonce-abc"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("wrong issuer must be denied");
    assert!(matches!(err, OidcError::IssuerMismatch { .. }), "got {err:?}");
}

#[test]
fn wrong_audience_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://other-api.example.com"],
        "exp": now() + 3600,
        "nonce": "nonce-abc"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("wrong audience must be denied");
    assert!(matches!(err, OidcError::AudienceMismatch { .. }), "got {err:?}");
}

#[test]
fn expired_token_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() - 10,
        "nonce": "nonce-abc"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("expired token must be denied");
    assert!(matches!(err, OidcError::TokenExpired { .. }), "got {err:?}");
}

#[test]
fn wrong_nonce_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() + 3600,
        "nonce": "attacker-nonce"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("wrong nonce must be denied");
    assert!(matches!(err, OidcError::NonceMismatch), "got {err:?}");
}

#[test]
fn missing_nonce_when_expected_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() + 3600
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("missing nonce must be denied");
    assert!(matches!(err, OidcError::NonceMismatch), "got {err:?}");
}

#[test]
fn tampered_signature_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() + 3600,
        "nonce": "nonce-abc"
    }));
    let (head, _, sig) = token.split('.').collect::<Vec<_>>().into_iter().collect::<(_, _, _)>();
    let tampered = format!("{head}.eyJzdWIiOiJhdXRoMHxldmlsIn0.{sig}");
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&tampered, Some("nonce-abc"), now())
        .expect_err("tampered payload must be denied");
    assert!(matches!(err, OidcError::SignatureInvalid), "got {err:?}");
}

#[test]
fn wrong_algorithm_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_raw(
        json!({"alg": "HS256", "typ": "JWT", "kid": sim.kid}),
        json!({
            "iss": sim.issuer,
            "sub": "auth0|usr-1",
            "aud": ["https://api.lagrange.local"],
            "exp": now() + 3600,
            "nonce": "nonce-abc"
        }),
    );
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("HS256 must be denied");
    assert!(matches!(err, OidcError::AlgorithmUnsupported(_)), "got {err:?}");
}

#[test]
fn unknown_kid_is_denied() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token_with_kid("other-kid", &json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() + 3600,
        "nonce": "nonce-abc"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let err = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect_err("unknown kid must be denied");
    assert!(matches!(err, OidcError::KeyNotFound(_)), "got {err:?}");
}

#[test]
fn audience_as_single_string_is_accepted() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": "https://api.lagrange.local",
        "exp": now() + 3600,
        "nonce": "nonce-abc"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    let claims = client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect("string audience accepted");
    assert_eq!(claims.aud, vec!["https://api.lagrange.local".to_string()]);
}

#[test]
fn clock_skew_permits_recently_expired_token() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let token = sim.sign_id_token(&json!({
        "iss": sim.issuer,
        "sub": "auth0|usr-1",
        "aud": ["https://api.lagrange.local"],
        "exp": now() - 30,
        "nonce": "nonce-abc"
    }));
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    client
        .validate_id_token(&token, Some("nonce-abc"), now())
        .expect("within 60s skew is allowed");
}

#[test]
fn token_response_never_exposes_provider_tokens() {
    let response = TokenResponse::from_json(
        r#"{"id_token":"header.payload.sig","access_token":"at-123","refresh_token":"rt-456","token_type":"Bearer"}"#,
    )
    .expect("parses");
    assert_eq!(response.id_token, "header.payload.sig");
}

#[test]
fn pending_auth_store_consumes_state_single_use() {
    let store = oidc::InMemoryPendingAuthStore::default();
    let pen = pending("state-1", "nonce-1", "verifier-1");
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        store.insert("state-1".to_string(), pen).await.unwrap();
        let first = store.take("state-1").await.unwrap().expect("first take");
        assert_eq!(first.state, "state-1");
        let second = store.take("state-1").await.unwrap();
        assert!(second.is_none(), "state must be single-use");
    });
}

#[test]
fn malformed_jwt_is_typed_error_not_panic() {
    let sim = Simulator::new(
        "https://lagrange-test.auth0.com",
        "lagrange-app",
        "https://app.lagrange.local/auth/callback",
    );
    let cfg = config(&sim.issuer, &sim.redirect_uri);
    let client = OidcClient {
        config: cfg.clone(),
        transport: Arc::new(sim.clone()),
    };
    for garbage in ["", "abc", "a.b", "a.b.c.d", "!!!.!!!.!!!"] {
        assert!(
            client.validate_id_token(garbage, &sim.jwks(), Some("nonce-abc"), now()).is_err(),
            "garbage {garbage:?} must be a typed error"
        );
    }
}
