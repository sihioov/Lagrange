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

#[test]
fn nginx_market_stream_is_unbuffered_and_has_bounded_stall_timeouts() {
    let location = "= /api/v1/research/owner-beta/equity-universe-v2/market-stream";
    let block = location_block(NGINX_CONF, location);
    for directive in [
        "proxy_pass http://api-server:8080;",
        "proxy_http_version 1.1;",
        "proxy_set_header Host $http_host;",
        "proxy_set_header Connection \"\";",
        "proxy_buffering off;",
        "proxy_cache off;",
        "gzip off;",
        "proxy_read_timeout 60s;",
        "send_timeout 5s;",
    ] {
        assert!(
            block.contains(directive),
            "missing SSE directive: {directive}"
        );
    }
    assert!(!block.contains("Access-Control-Allow-Origin"));
    assert!(!block.contains("proxy_hide_header"));
    assert!(location_block(NGINX_CONF, "/api/").contains("proxy_read_timeout 300s;"));
}

fn location_block<'a>(conf: &'a str, location: &str) -> &'a str {
    let start = conf
        .find(&format!("location {location} {{"))
        .unwrap_or_else(|| panic!("location {location} must exist"));
    let body_start = conf[start..].find('{').expect("location block opens") + start;
    let body_end = conf[body_start..].find('}').expect("location block closes") + body_start;
    &conf[body_start + 1..body_end]
}

#[test]
fn nginx_lease_mutations_preserve_the_same_authority_as_sse() {
    let lease = location_block(
        NGINX_CONF,
        "/api/v1/research/owner-beta/equity-universe-v2/stream-leases",
    );
    let stream = location_block(
        NGINX_CONF,
        "= /api/v1/research/owner-beta/equity-universe-v2/market-stream",
    );
    for block in [lease, stream] {
        assert!(block.contains("proxy_pass http://api-server:8080;"));
        assert!(block.contains("proxy_set_header Host $http_host;"));
        assert!(block.contains("proxy_set_header X-Forwarded-Host $http_host;"));
        assert!(!block.contains("Access-Control-Allow-Origin"));
    }
    // The stream-specific fix must not replace the existing general API route.
    let general = location_block(NGINX_CONF, "/api/");
    assert!(general.contains("proxy_set_header Host $host;"));
    assert!(general.contains("proxy_read_timeout 300s;"));
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
