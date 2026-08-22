//! VENDOR suite: real Auth0 tenant verification (tagged `vendor`).
//!
//! This suite is `#[ignore]`d by default and NEVER silently skipped: running it
//! explicitly FAILS LOUDLY unless the tenant variables are set. The full
//! protocol contract is proven meanwhile by the Auth0 SIMULATOR suite
//! (`tests/auth0_simulator.rs`).

use reqwest::{Client, StatusCode, redirect::Policy};
use serde::Deserialize;
use std::time::Duration;
use url::Url;
use zeroize::Zeroizing;

/// Fallback only. `https://app.lagrange.local` is the repository's documented
/// PLACEHOLDER host — `scripts/ops/validate-production-config.sh` rejects a
/// production config that still contains it. The real tenant only redirects to
/// callbacks registered on the app, and that value is deployment-specific
/// (currently a Tailscale hostname), so it is supplied by the operator through
/// [`REDIRECT_URI_ENV`] exactly as the domain and client id already are.
/// Hard-coding one host's address here would pin a machine into the suite.
const DEFAULT_CALLBACK_URI: &str = "https://app.lagrange.local/auth/callback";
const INVALID_AUTHORIZATION_CODE: &str = "deliberately-invalid-authorization-code";
const INVALID_CLIENT_SECRET: &str = "deliberately-invalid-vendor-test-secret";
const PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const PKCE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

const DOMAIN_ENV: &str = "LAGRANGE_AUTH0_DOMAIN";
const CLIENT_ID_ENV: &str = "LAGRANGE_AUTH0_CLIENT_ID";
const CLIENT_SECRET_ENV: &str = "LAGRANGE_AUTH0_CLIENT_SECRET";
const REDIRECT_URI_ENV: &str = "LAGRANGE_AUTH0_REDIRECT_URI";

pub const EXPECTED_AUTH0_DOMAIN: &str = "lagrange-station.jp.auth0.com";
pub const EXPECTED_AUTH0_CLIENT_ID: &str = "YZ4T7g575IohtS1HsltlFAiU7AlyUUuI";

struct TenantConfig {
    domain: String,
    client_id: String,
}

fn tenant_config() -> TenantConfig {
    let domain = required_env(DOMAIN_ENV);
    let client_id = required_env(CLIENT_ID_ENV);
    validate_tenant_identity(&domain, &client_id).unwrap_or_else(|message| panic!("{message}"));
    TenantConfig { domain, client_id }
}

fn client_secret() -> Zeroizing<String> {
    Zeroizing::new(required_env(CLIENT_SECRET_ENV))
}

/// The callback the tenant is expected to accept.
///
/// Unset falls back to the placeholder, which the tenant will refuse with 403 —
/// a loud, correct failure rather than a silent skip, matching how this suite
/// treats every other missing input.
fn callback_uri() -> String {
    match std::env::var(REDIRECT_URI_ENV) {
        Ok(value) if !value.is_empty() => value,
        _ => DEFAULT_CALLBACK_URI.to_owned(),
    }
}

fn required_env(key: &'static str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("BLOCKED_EXTERNAL: vendor Auth0 suite requires env {key}"),
    }
}

fn validate_tenant_identity(domain: &str, client_id: &str) -> Result<(), &'static str> {
    if domain == EXPECTED_AUTH0_DOMAIN && client_id == EXPECTED_AUTH0_CLIENT_ID {
        Ok(())
    } else {
        Err("vendor Auth0 tenant identity does not match selected app")
    }
}

#[test]
fn tenant_identity_is_pinned_without_echoing_rejected_values() {
    const HOSTILE_MARKER: &str = "attacker-controlled-tenant-marker";
    const FIXED_DIAGNOSTIC: &str = "vendor Auth0 tenant identity does not match selected app";

    assert!(validate_tenant_identity(EXPECTED_AUTH0_DOMAIN, EXPECTED_AUTH0_CLIENT_ID).is_ok());

    for (domain, client_id) in [
        (HOSTILE_MARKER, EXPECTED_AUTH0_CLIENT_ID),
        (EXPECTED_AUTH0_DOMAIN, HOSTILE_MARKER),
        (HOSTILE_MARKER, HOSTILE_MARKER),
    ] {
        let diagnostic = validate_tenant_identity(domain, client_id)
            .expect_err("unselected tenant identity must fail");
        assert_eq!(diagnostic, FIXED_DIAGNOSTIC);
        assert!(!diagnostic.contains(HOSTILE_MARKER));
    }
}

fn http_client() -> Client {
    Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| panic!("vendor HTTP client construction failed"))
}

fn issuer(domain: &str) -> Url {
    Url::parse(&format!("https://{domain}/"))
        .unwrap_or_else(|_| panic!("vendor Auth0 domain must form a valid HTTPS issuer"))
}

#[derive(Deserialize)]
struct TokenErrorResponse {
    error: TokenErrorCode,
}

#[derive(Clone, Copy, Deserialize)]
enum TokenErrorCode {
    #[serde(rename = "invalid_grant")]
    InvalidGrant,
    #[serde(rename = "access_denied")]
    AccessDenied,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy)]
enum ExpectedTokenError {
    InvalidGrant,
    AccessDenied,
}

fn parse_token_error(body: &[u8]) -> Result<TokenErrorCode, &'static str> {
    serde_json::from_slice::<TokenErrorResponse>(body)
        .map(|response| response.error)
        .map_err(|_| "vendor token endpoint returned invalid OAuth error JSON")
}

fn validate_token_error(
    actual: TokenErrorCode,
    expected: ExpectedTokenError,
) -> Result<(), &'static str> {
    let matches = matches!(
        (actual, expected),
        (
            TokenErrorCode::InvalidGrant,
            ExpectedTokenError::InvalidGrant
        ) | (
            TokenErrorCode::AccessDenied,
            ExpectedTokenError::AccessDenied
        )
    );
    if matches {
        Ok(())
    } else {
        Err("vendor token endpoint returned an unexpected OAuth error")
    }
}

async fn token_probe(
    client: &Client,
    issuer: &Url,
    client_id: &str,
    client_secret: &str,
) -> (StatusCode, TokenErrorCode) {
    let token_url = issuer
        .join("oauth/token")
        .unwrap_or_else(|_| panic!("vendor token endpoint URL construction failed"));
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", INVALID_AUTHORIZATION_CODE),
            ("redirect_uri", callback_uri().as_str()),
            ("code_verifier", PKCE_VERIFIER),
        ])
        .send()
        .await
        .unwrap_or_else(|_| panic!("vendor token endpoint request failed"));
    let status = response.status();
    let body = response
        .bytes()
        .await
        .unwrap_or_else(|_| panic!("vendor token endpoint response read failed"));
    let error = parse_token_error(&body).unwrap_or_else(|message| panic!("{message}"));
    (status, error)
}

#[tokio::test]
#[ignore = "vendor: real Auth0 tenant required (BLOCKED_EXTERNAL on this host); must run before production release"]
async fn vendor_tenant_jwks_issuer_audience_endpoints() {
    let config = tenant_config();
    let issuer = issuer(&config.domain);
    let jwks_url = issuer
        .join(".well-known/jwks.json")
        .unwrap_or_else(|_| panic!("vendor JWKS URL construction failed"));
    let response = http_client()
        .get(jwks_url)
        .send()
        .await
        .unwrap_or_else(|_| panic!("vendor JWKS request failed"));
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "vendor JWKS endpoint returned an unexpected status"
    );
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| panic!("vendor JWKS response decoding failed"));
    let jwks: auth::oidc::jwks::Jwks =
        auth::oidc::jwks::Jwks::parse(&body).unwrap_or_else(|_| panic!("tenant JWKS is invalid"));
    assert!(
        !jwks.keys.is_empty(),
        "tenant publishes at least one signing key"
    );
    assert!(
        jwks.keys.iter().any(|k| k.alg.as_deref() == Some("RS256")),
        "tenant keys are RS256"
    );
}

#[tokio::test]
#[ignore = "vendor: real Auth0 tenant required (BLOCKED_EXTERNAL on this host); must run before production release"]
async fn vendor_authorize_endpoint_engages_oidc() {
    let config = tenant_config();
    let issuer = issuer(&config.domain);
    let callback = callback_uri();
    let mut authorize_url = issuer
        .join("authorize")
        .unwrap_or_else(|_| panic!("vendor authorize URL construction failed"));
    authorize_url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", callback.as_str()),
        ("scope", "openid email profile"),
        ("state", "vendor-auth0-state"),
        ("nonce", "vendor-auth0-nonce"),
        ("code_challenge", PKCE_CHALLENGE),
        ("code_challenge_method", "S256"),
    ]);

    let response = http_client()
        .get(authorize_url)
        .send()
        .await
        .unwrap_or_else(|_| panic!("vendor authorize request failed"));
    assert_eq!(
        response.status(),
        StatusCode::FOUND,
        "vendor authorize endpoint must return the initial redirect"
    );
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("vendor authorize redirect must include a valid Location"));
    let redirect = issuer
        .join(location)
        .unwrap_or_else(|_| panic!("vendor authorize Location must resolve as a URL"));
    assert!(
        redirect.scheme() == "https"
            && redirect.host_str() == issuer.host_str()
            && redirect.port_or_known_default() == issuer.port_or_known_default(),
        "vendor authorize redirect must remain on the HTTPS Auth0 tenant"
    );
}

#[tokio::test]
#[ignore = "vendor: real Auth0 tenant required (BLOCKED_EXTERNAL on this host); must run before production release"]
async fn vendor_confidential_client_credential_is_accepted() {
    let config = tenant_config();
    let issuer = issuer(&config.domain);
    let client = http_client();
    let secret = client_secret();

    let (valid_status, valid_error) =
        token_probe(&client, &issuer, &config.client_id, secret.as_str()).await;
    drop(secret);
    assert_eq!(
        valid_status,
        StatusCode::FORBIDDEN,
        "configured credential must authenticate before the deliberately invalid grant is rejected"
    );
    validate_token_error(valid_error, ExpectedTokenError::InvalidGrant)
        .unwrap_or_else(|message| panic!("{message}"));

    let (invalid_status, invalid_error) =
        token_probe(&client, &issuer, &config.client_id, INVALID_CLIENT_SECRET).await;
    assert_eq!(
        invalid_status,
        StatusCode::UNAUTHORIZED,
        "negative-control credential must fail client authentication"
    );
    validate_token_error(invalid_error, ExpectedTokenError::AccessDenied)
        .unwrap_or_else(|message| panic!("{message}"));
}

#[test]
fn hostile_token_error_is_classified_without_retaining_payload() {
    const HOSTILE_MARKER: &str = "reflected-vendor-secret-marker";
    let body = format!(r#"{{"error":"{HOSTILE_MARKER}"}}"#);

    let error = match parse_token_error(body.as_bytes()) {
        Ok(error) => error,
        Err(message) => panic!("{message}"),
    };
    assert!(matches!(error, TokenErrorCode::Unknown));

    let diagnostic = match validate_token_error(error, ExpectedTokenError::InvalidGrant) {
        Ok(()) => panic!("unknown OAuth error must fail validation"),
        Err(message) => message,
    };
    assert_eq!(
        diagnostic,
        "vendor token endpoint returned an unexpected OAuth error"
    );
    assert!(!diagnostic.contains(HOSTILE_MARKER));
}
