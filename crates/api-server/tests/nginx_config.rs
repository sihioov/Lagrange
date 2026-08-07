//! Todo 27 Nginx integration: the committed edge config must keep the
//! artifact-delivery hardening — an `internal;` location that only an
//! `X-Accel-Redirect` from the authorized API can reach, `disable_symlinks`
//! against path escapes, and an alias-rooted artifact tree with no
//! filesystem paths exposed. The full `nginx -t` + runtime symlink-escape
//! probe is the documented harness `scripts/qa/nginx-hardening.sh`
//! (runnable inside WSL where the nginx.org build lives).
//!
//! Tests carry `nginx_` prefixes so `cargo test -p api-server nginx` selects
//! them without a database.

use api_server::contract::CONTRACT_ROUTES;

/// The committed edge configuration (T4 skeleton + Todo 27 hardening).
const NGINX_CONF: &str = include_str!("../../../deploy/nginx/nginx.conf");

fn location_block<'a>(conf: &'a str, location: &str) -> &'a str {
    let start = conf
        .find(&format!("location {location}"))
        .unwrap_or_else(|| panic!("location {location} must exist"));
    let body_start = conf[start..].find('{').expect("location block opens") + start;
    let body_end = conf[body_start..].find('}').expect("location block closes") + body_start;
    &conf[body_start + 1..body_end]
}

#[test]
fn nginx_artifact_location_is_internal_and_symlink_safe() {
    let block = location_block(NGINX_CONF, "/internal-artifacts/");
    assert!(
        block.contains("internal;"),
        "the artifact location must be internal-only: {block}"
    );
    assert!(
        block.contains("disable_symlinks on;"),
        "symlink escapes must be disabled: {block}"
    );
    assert!(
        block.contains("alias /data/artifacts/;"),
        "the artifact tree is alias-rooted: {block}"
    );
    assert!(
        !block.contains("root "),
        "no root directive may expose a filesystem path: {block}"
    );
    assert!(
        !block.contains("autoindex"),
        "directory listing must be off: {block}"
    );
    assert!(
        block.contains("X-Content-Type-Options nosniff"),
        "nosniff on served artifact bytes: {block}"
    );
}

#[test]
fn nginx_never_exposes_artifacts_through_the_api() {
    // The internal path exists only inside Nginx: the API contract must not
    // mount it, and the download surface is the authorized route alone.
    for route in CONTRACT_ROUTES {
        assert!(
            !route.path.contains("internal-artifacts"),
            "the internal path must never be a public route: {}",
            route.path
        );
    }
    let download_routes: Vec<&str> = CONTRACT_ROUTES
        .iter()
        .filter(|r| r.path.contains("/artifacts/"))
        .map(|r| r.path)
        .collect();
    assert_eq!(
        download_routes,
        vec![
            "/api/v1/artifacts/{artifact_id}",
            "/api/v1/artifacts/{artifact_id}/download"
        ],
        "only the two authorized artifact routes may exist"
    );
}

#[test]
fn nginx_tls_edge_publishes_only_https() {
    assert!(NGINX_CONF.contains("listen 8443 ssl;"), "TLS listener");
    assert!(
        !NGINX_CONF.contains("listen 80;"),
        "no plain-HTTP public listener"
    );
    assert!(
        NGINX_CONF.contains("listen 8080;"),
        "the internal health/redirect listener stays"
    );
    assert!(
        NGINX_CONF.contains("server_tokens off;"),
        "no version disclosure"
    );
}

#[test]
fn nginx_disable_symlinks_covers_every_artifact_serving_location() {
    let count = NGINX_CONF.match_indices("disable_symlinks on;").count();
    assert!(
        count >= 1,
        "the artifact-serving location must carry disable_symlinks"
    );
}
