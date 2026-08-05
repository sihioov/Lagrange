//! VENDOR suite: real Auth0 tenant verification (tagged `vendor`).
//!
//! BLOCKED_EXTERNAL on this host: no Auth0 tenant/credentials exist here, so
//! this suite is `#[ignore]`d by default and NEVER silently skipped - running
//! it explicitly (e.g. `cargo test -p auth vendor_auth0 -- --ignored --nocapture`)
//! FAILS LOUDLY unless the tenant variables are set, and the plan requires it
//! before any production release gate (credential-split rule). The full
//! protocol contract is proven meanwhile by the Auth0 SIMULATOR suite
//! (tests/auth0_simulator.rs).

const TENANT_ENV: [&str; 3] = [
    "LAGRANGE_AUTH0_DOMAIN",
    "LAGRANGE_AUTH0_CLIENT_ID",
    "LAGRANGE_AUTH0_CLIENT_SECRET",
];

fn tenant_config() -> (String, String, String) {
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
    (values[0].clone(), values[1].clone(), values[2].clone())
}

fn curl_get(url: &str) -> String {
    let out = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "15", "-f", url])
        .output()
        .expect("curl available on vendor hosts");
    assert!(
        out.status.success(),
        "GET {url} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8 body")
}

#[test]
#[ignore = "vendor: real Auth0 tenant required (BLOCKED_EXTERNAL on this host); must run before production release"]
fn vendor_tenant_jwks_issuer_audience_endpoints() {
    let (domain, _client_id, _secret) = tenant_config();
    let issuer = format!("https://{domain}");
    let jwks_url = format!("{issuer}/.well-known/jwks.json");
    let body = curl_get(&jwks_url);
    let jwks: auth::oidc::jwks::Jwks =
        auth::oidc::jwks::Jwks::parse(&body).expect("tenant JWKS parses");
    assert!(
        !jwks.keys.is_empty(),
        "tenant publishes at least one signing key"
    );
    assert!(
        jwks.keys.iter().any(|k| k.alg.as_deref() == Some("RS256")),
        "tenant keys are RS256"
    );
}

#[test]
#[ignore = "vendor: real Auth0 tenant required (BLOCKED_EXTERNAL on this host); must run before production release"]
fn vendor_authorize_endpoint_engages_oidc() {
    let (domain, client_id, _secret) = tenant_config();
    let authorize_url = format!(
        "https://{domain}/authorize?response_type=code&client_id={client_id}&redirect_uri=https%3A%2F%2Fapp.lagrange.local%2Fauth%2Fcallback&scope=openid%20email%20profile&code_challenge_method=S256"
    );
    let body = curl_get(&authorize_url);
    assert!(
        body.contains("code_challenge") || body.contains("state") || body.contains("login"),
        "tenant authorize endpoint engages the OIDC flow"
    );
}
