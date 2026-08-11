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

const CALLBACK_URI: &str = "https://app.lagrange.local/auth/callback";
const INVALID_AUTHORIZATION_CODE: &str = "deliberately-invalid-authorization-code";
const INVALID_CLIENT_SECRET: &str = "deliberately-invalid-vendor-test-secret";
const PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const PKCE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

const TENANT_ENV: [&str; 3] = [
    "LAGRANGE_AUTH0_DOMAIN",
    "LAGRANGE_AUTH0_CLIENT_ID",
    "LAGRANGE_AUTH0_CLIENT_SECRET",
];

struct TenantConfig {
    domain: String,
    client_id: String,
    client_secret: String,
}

fn tenant_config() -> TenantConfig {
    let values: Vec<String> = TENANT_ENV
        .iter()
        .map(|k| std::env::var(k).unwrap_or_default())
        .collect();
    if values.iter().any(|v| v.is_empty()) {
        panic!(
            "BLOCKED_EXTERNAL: vendor Auth0 suite requires env {}; no tenant/credentials exist on this host. \
             The simulator suite (tests/auth0_simulator.rs) proves the contract until a tenant is provisioned.",
            TENANT_ENV.join(", ")
        );
    }
    TenantConfig {
        domain: values[0].clone(),
        client_id: values[1].clone(),
        client_secret: values[2].clone(),
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
struct TokenError {
    error: String,
}

async fn token_probe(
    client: &Client,
    issuer: &Url,
    client_id: &str,
    client_secret: &str,
) -> (StatusCode, String) {
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
            ("redirect_uri", CALLBACK_URI),
            ("code_verifier", PKCE_VERIFIER),
        ])
        .send()
        .await
        .unwrap_or_else(|_| panic!("vendor token endpoint request failed"));
    let status = response.status();
    let error = response.json::<TokenError>().await.unwrap_or_else(|_| {
        panic!("vendor token endpoint returned non-JSON error status {status}")
    });
    (status, error.error)
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
    let mut authorize_url = issuer
        .join("authorize")
        .unwrap_or_else(|_| panic!("vendor authorize URL construction failed"));
    authorize_url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", CALLBACK_URI),
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

    let (valid_status, valid_error) =
        token_probe(&client, &issuer, &config.client_id, &config.client_secret).await;
    assert_eq!(
        valid_status,
        StatusCode::FORBIDDEN,
        "configured credential must authenticate before the deliberately invalid grant is rejected"
    );
    assert_eq!(
        valid_error, "invalid_grant",
        "configured credential must reach Auth0 grant validation"
    );

    let (invalid_status, invalid_error) =
        token_probe(&client, &issuer, &config.client_id, INVALID_CLIENT_SECRET).await;
    assert_eq!(
        invalid_status,
        StatusCode::UNAUTHORIZED,
        "negative-control credential must fail client authentication"
    );
    assert_eq!(
        invalid_error, "access_denied",
        "negative-control credential must be denied by Auth0"
    );
}
