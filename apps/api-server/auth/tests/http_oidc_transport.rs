use api_server_auth::{
    HttpOidcTransport,
    config::{AUTH0_CLIENT_SECRET_FILE, ClientSecret},
};
use auth::oidc::{OidcTransport, TokenRequest};
use axum::{Form, Router, http::StatusCode, routing::post};
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
    fs::write(&secret_path, "confidential-value\r\n").expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (token_url, captured) =
        token_server(StatusCode::OK, r#"{"id_token":"header.payload.signature"}"#).await;
    let transport = HttpOidcTransport::new(token_url, "https://unused.invalid/jwks", secret);

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

#[test]
fn client_secret_file_rejects_missing_empty_and_non_file_inputs_without_values() {
    const SECRET_MARKER: &str = "auth0-secret-must-never-render";

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let missing_path = temp_dir.path().join("missing-secret");
    let empty_path = temp_dir.path().join("empty-secret");
    fs::write(&empty_path, "\r\n").expect("write empty CRLF secret file");

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
