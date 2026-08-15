use api_server_auth::{HttpOidcTransport, config::ClientSecret};
use std::fs;

#[test]
fn constructor_rejects_non_loopback_plain_http_without_exposing_secret() {
    const SECRET_MARKER: &str = "destination-secret-marker";

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let secret_path = temp_dir.path().join("auth0-client-secret");
    fs::write(&secret_path, SECRET_MARKER).expect("write client secret file");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");

    let error = match HttpOidcTransport::new(
        "http://auth.example.com/oauth/token",
        "https://auth.example.com/.well-known/jwks.json",
        secret,
    ) {
        Ok(_) => panic!("non-loopback plain HTTP token URL must be rejected"),
        Err(error) => error,
    };

    assert!(
        !error.to_string().contains(SECRET_MARKER),
        "constructor error must not expose the client secret: {error}"
    );
}
