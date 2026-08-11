use api_server_auth::config::{AUTH0_CLIENT_SECRET_FILE, ClientSecret};
use std::fs;

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
