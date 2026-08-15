use api_server_auth::config::{
    DEFAULT_AUTH0_REDIRECT_URI, ProductionAuthConfig, ProductionAuthConfigError,
};
use std::fs;

#[test]
fn production_config_requires_domain_and_client_id() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("client-secret");
    fs::write(&secret_path, "simulator-secret").expect("write secret fixture");
    let secret = api_server_auth::config::ClientSecret::from_file(&secret_path)
        .expect("load secret fixture");

    let missing_domain = match ProductionAuthConfig::from_values(
        None,
        Some("client-id".to_string()),
        None,
        None,
        None,
        secret,
    ) {
        Ok(_) => panic!("missing domain must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(
        missing_domain,
        ProductionAuthConfigError::Missing {
            key: "AUTH0_DOMAIN"
        }
    ));
}

#[test]
fn production_config_builds_exact_https_endpoints_without_credentials_in_urls() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("client-secret");
    fs::write(&secret_path, "simulator-secret").expect("write secret fixture");
    let secret = api_server_auth::config::ClientSecret::from_file(&secret_path)
        .expect("load secret fixture");
    let config = ProductionAuthConfig::from_values(
        Some("tenant.auth0.com".to_string()),
        Some("client-id".to_string()),
        None,
        None,
        None,
        secret,
    )
    .expect("valid production config");

    assert_eq!(config.provider.issuer, "https://tenant.auth0.com/");
    assert_eq!(config.provider.redirect_uri, DEFAULT_AUTH0_REDIRECT_URI);
    assert_eq!(
        config.provider.authorize_url,
        "https://tenant.auth0.com/authorize"
    );
    assert_eq!(
        config.provider.token_url,
        "https://tenant.auth0.com/oauth/token"
    );
    assert_eq!(
        config.provider.jwks_url,
        "https://tenant.auth0.com/.well-known/jwks.json"
    );
}

#[test]
fn production_config_rejects_insecure_or_ambiguous_endpoints() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("client-secret");
    fs::write(&secret_path, "simulator-secret").expect("write secret fixture");
    let secret = api_server_auth::config::ClientSecret::from_file(&secret_path)
        .expect("load secret fixture");

    assert!(matches!(
        ProductionAuthConfig::from_values(
            Some("http://tenant.auth0.com".to_string()),
            Some("client-id".to_string()),
            None,
            None,
            None,
            secret,
        ),
        Err(ProductionAuthConfigError::InvalidDomain)
    ));
}

#[test]
fn production_config_caps_clock_skew_at_five_minutes() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("client-secret");
    fs::write(&secret_path, "simulator-secret").expect("write secret fixture");
    let secret = api_server_auth::config::ClientSecret::from_file(&secret_path)
        .expect("load secret fixture");
    let config = ProductionAuthConfig::from_values(
        Some("tenant.auth0.com".to_string()),
        Some("client-id".to_string()),
        None,
        None,
        Some("300".to_string()),
        secret,
    )
    .expect("five-minute boundary is valid");
    assert_eq!(config.provider.clock_skew_secs, 300);

    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("client-secret");
    fs::write(&secret_path, "simulator-secret").expect("write secret fixture");
    let secret = api_server_auth::config::ClientSecret::from_file(&secret_path)
        .expect("load secret fixture");
    assert!(matches!(
        ProductionAuthConfig::from_values(
            Some("tenant.auth0.com".to_string()),
            Some("client-id".to_string()),
            None,
            None,
            Some("301".to_string()),
            secret,
        ),
        Err(ProductionAuthConfigError::InvalidClockSkew)
    ));
}

#[test]
fn client_secret_rejects_any_carriage_return_or_line_feed_before_trimming() {
    let root = tempfile::tempdir().expect("temporary secret root");

    for (suffix, contents) in [
        ("lf", "simulator-secret\n"),
        ("cr", "simulator-secret\r"),
        ("crlf", "simulator-secret\r\n"),
    ] {
        let path = root.path().join(format!("client-secret-{suffix}"));
        fs::write(&path, contents).expect("write invalid secret fixture");
        assert!(
            matches!(
                api_server_auth::config::ClientSecret::from_file(&path),
                Err(api_server_auth::config::ClientSecretError::MultipleLines { .. })
            ),
            "secret file with {suffix} must fail before trimming"
        );
    }
}
