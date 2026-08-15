use api_server_auth::{
    HttpOidcTransport,
    config::{AUTH0_CLIENT_SECRET_FILE, ClientSecret},
};
use auth::oidc::{OidcTransport, TokenRequest};
use axum::{
    Form, Router,
    http::{StatusCode, header},
    routing::{get, post},
};
use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex},
};

async fn token_server(
    status: StatusCode,
    response_body: &'static str,
) -> (String, Arc<Mutex<Option<HashMap<String, String>>>>) {
    let captured = Arc::new(Mutex::new(None));
    let handler_capture = Arc::clone(&captured);
    let app = Router::new().route(
        "/oauth/token",
        post(move |Form(form): Form<HashMap<String, String>>| {
            let handler_capture = Arc::clone(&handler_capture);
            async move {
                *handler_capture.lock().expect("capture token form") = Some(form);
                (status, response_body)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind token server");
    let address = listener.local_addr().expect("read token server address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve token endpoint");
    });

    (format!("http://{address}/oauth/token"), captured)
}

async fn redirecting_token_server(location: String) -> String {
    let app = Router::new().route(
        "/oauth/token",
        post(move || {
            let location = location.clone();
            async move {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, location)],
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirecting token server");
    let address = listener
        .local_addr()
        .expect("read redirecting token server address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve redirecting token endpoint");
    });

    format!("http://{address}/oauth/token")
}

async fn jwks_server(status: StatusCode, response_body: &'static str) -> String {
    let app = Router::new().route(
        "/.well-known/jwks.json",
        get(move || async move { (status, response_body) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind JWKS server");
    let address = listener.local_addr().expect("read JWKS server address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve JWKS endpoint");
    });

    format!("http://{address}/.well-known/jwks.json")
}

async fn hanging_oidc_server() -> (String, String) {
    let app = Router::new()
        .route(
            "/oauth/token",
            post(|| async {
                std::future::pending::<()>().await;
            }),
        )
        .route(
            "/.well-known/jwks.json",
            get(|| async {
                std::future::pending::<()>().await;
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind hanging OIDC server");
    let address = listener.local_addr().expect("hanging OIDC server address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve hanging OIDC endpoint");
    });
    (
        format!("http://{address}/oauth/token"),
        format!("http://{address}/.well-known/jwks.json"),
    )
}

fn token_request() -> TokenRequest {
    TokenRequest {
        code: "authorization-code".to_string(),
        redirect_uri: "https://app.lagrange.local/auth/callback".to_string(),
        client_id: "lagrange-client".to_string(),
        code_verifier: "pkce-verifier".to_string(),
    }
}

#[tokio::test]
async fn token_exchange_posts_client_secret_and_pkce_verifier() {
    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("auth0-client-secret");
    fs::write(&secret_path, "confidential-value").expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (token_url, captured) =
        token_server(StatusCode::OK, r#"{"id_token":"header.payload.signature"}"#).await;
    let transport = HttpOidcTransport::new(token_url, "https://unused.invalid/jwks", secret)
        .expect("construct OIDC transport");

    transport
        .exchange_code(&token_request())
        .await
        .expect("exchange authorization code");

    let captured = captured
        .lock()
        .expect("read captured token form")
        .clone()
        .expect("token form was captured");
    assert_eq!(
        captured.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(
        captured.get("client_secret").map(String::as_str),
        Some("confidential-value")
    );
    assert_eq!(
        captured.get("code_verifier").map(String::as_str),
        Some("pkce-verifier")
    );
    assert_eq!(
        captured.get("redirect_uri").map(String::as_str),
        Some("https://app.lagrange.local/auth/callback")
    );
    assert_eq!(
        captured.get("client_id").map(String::as_str),
        Some("lagrange-client")
    );
    assert_eq!(
        captured.get("code").map(String::as_str),
        Some("authorization-code")
    );
}

#[tokio::test]
async fn token_exchange_does_not_follow_redirects() {
    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("auth0-client-secret");
    fs::write(&secret_path, "redirect-secret").expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (redirect_target, target_capture) =
        token_server(StatusCode::OK, r#"{"id_token":"header.payload.signature"}"#).await;
    let token_url = redirecting_token_server(redirect_target).await;
    let transport = HttpOidcTransport::new(token_url, "https://unused.invalid/jwks", secret)
        .expect("construct OIDC transport");

    let result = transport.exchange_code(&token_request()).await;

    assert!(
        result.is_err(),
        "redirect response must fail token exchange"
    );
    assert!(
        target_capture
            .lock()
            .expect("read redirect target capture")
            .is_none(),
        "redirect target must not receive the credential-bearing form"
    );
}

#[tokio::test]
async fn token_exchange_error_never_renders_reflected_client_secret() {
    const SECRET_MARKER: &str = "reflected-secret-value";

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("auth0-client-secret");
    fs::write(&secret_path, SECRET_MARKER).expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (token_url, _) = token_server(StatusCode::UNAUTHORIZED, SECRET_MARKER).await;
    let transport = HttpOidcTransport::new(token_url, "https://unused.invalid/jwks", secret)
        .expect("construct OIDC transport");

    let error = match transport.exchange_code(&token_request()).await {
        Ok(_) => panic!("unauthorized token exchange must fail"),
        Err(error) => error,
    };
    let rendered = error.to_string();

    assert!(
        rendered.contains("401"),
        "rendered error must contain the HTTP status: {rendered}"
    );
    assert!(
        !rendered.contains(SECRET_MARKER),
        "rendered error must not contain the reflected client secret: {rendered}"
    );
}

#[tokio::test]
async fn jwks_error_never_renders_hostile_response_body() {
    const HOSTILE_MARKER: &str = "reflected-jwks-secret-marker";

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("auth0-client-secret");
    fs::write(&secret_path, "test-client-secret").expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let jwks_url = jwks_server(StatusCode::BAD_GATEWAY, HOSTILE_MARKER).await;
    let transport = HttpOidcTransport::new("https://unused.invalid/token", jwks_url, secret)
        .expect("construct OIDC transport");

    let error = match transport.fetch_jwks().await {
        Ok(_) => panic!("unsuccessful JWKS response must fail"),
        Err(error) => error,
    };
    let rendered = error.to_string();

    assert!(
        rendered.contains("502"),
        "rendered error must contain the HTTP status: {rendered}"
    );
    assert!(
        !rendered.contains(HOSTILE_MARKER),
        "rendered error must not contain the hostile response body: {rendered}"
    );
}

#[tokio::test]
async fn token_and_jwks_requests_have_a_total_deadline() {
    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("auth0-client-secret");
    fs::write(&secret_path, "timeout-secret").expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (token_url, jwks_url) = hanging_oidc_server().await;
    let transport = HttpOidcTransport::with_timeouts(
        token_url,
        jwks_url,
        secret,
        std::time::Duration::from_millis(25),
        std::time::Duration::from_millis(75),
    )
    .expect("construct timeout-limited OIDC transport");

    let token_result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        transport.exchange_code(&token_request()),
    )
    .await
    .expect("token exchange must be bounded")
    .expect_err("hanging token endpoint must fail by deadline");
    assert!(token_result.to_string().contains("token exchange"));

    let jwks_result =
        tokio::time::timeout(std::time::Duration::from_secs(1), transport.fetch_jwks())
            .await
            .expect("JWKS fetch must be bounded")
            .expect_err("hanging JWKS endpoint must fail by deadline");
    assert!(jwks_result.to_string().contains("jwks fetch"));
}

#[test]
fn client_secret_file_rejects_missing_empty_and_non_file_inputs_without_values() {
    const SECRET_MARKER: &str = "auth0-secret-must-never-render";

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let missing_path = temp_dir.path().join("missing-secret");
    let empty_path = temp_dir.path().join("empty-secret");
    fs::write(&empty_path, "").expect("write empty secret file");

    for (result, context) in [
        (ClientSecret::from_file(&missing_path), "missing path"),
        (ClientSecret::from_file(&empty_path), "empty file"),
        (ClientSecret::from_file(temp_dir.path()), "directory path"),
    ] {
        let error = match result {
            Ok(_) => panic!("{context} must fail"),
            Err(error) => error,
        };
        let rendered = error.to_string();
        assert!(
            rendered.contains(AUTH0_CLIENT_SECRET_FILE),
            "rendered error must name the public configuration key: {rendered}"
        );
        assert!(
            !rendered.contains(SECRET_MARKER),
            "rendered error must not contain the secret marker: {rendered}"
        );
    }
}

#[test]
fn client_secret_file_rejects_multiple_lines_without_values() {
    const FIRST_LINE: &str = "first-line";
    const SECOND_LINE: &str = "second-line";

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("multiline-secret");
    fs::write(&secret_path, format!("{FIRST_LINE}\n{SECOND_LINE}\n"))
        .expect("write multiline client secret file");

    let error = match ClientSecret::from_file(&secret_path) {
        Ok(_) => panic!("multiline client secret file must fail"),
        Err(error) => error,
    };
    let rendered = error.to_string();

    assert!(
        rendered.contains(AUTH0_CLIENT_SECRET_FILE),
        "rendered error must name the public configuration key: {rendered}"
    );
    assert!(
        !rendered.contains(FIRST_LINE),
        "rendered error must not contain the first line: {rendered}"
    );
    assert!(
        !rendered.contains(SECOND_LINE),
        "rendered error must not contain the second line: {rendered}"
    );
}

#[test]
fn client_secret_file_rejects_trailing_line_endings_before_trimming() {
    let temp_dir = tempfile::tempdir().expect("create temporary directory");

    for (suffix, contents) in [
        ("lf", "single-line-secret\n"),
        ("cr", "single-line-secret\r"),
        ("crlf", "single-line-secret\r\n"),
    ] {
        let secret_path = temp_dir.path().join(format!("trailing-{suffix}"));
        fs::write(&secret_path, contents).expect("write trailing-line-ending secret file");
        assert!(
            matches!(
                ClientSecret::from_file(&secret_path),
                Err(api_server_auth::config::ClientSecretError::MultipleLines { .. })
            ),
            "secret file with {suffix} must fail before trimming"
        );
    }
}
