//! Todo 27 artifact authorization: downloads are DB-manifest-backed
//! (sha256/media/owner) and served via an INTERNAL `X-Accel-Redirect` that
//! Nginx alone can follow — the API issues the header ONLY after ownership,
//! entitlement, path-safety, and hash verification. Everything else fails
//! closed (403/404/422) with zero filesystem-path or payload leakage.
//!
//! All tests carry the `artifact_` prefix so the plan acceptance filter
//! `cargo test -p api-server artifact admin observability` selects them.

mod common;

use axum::http::StatusCode;
use common::{Harness, sha256_hex, status};

const ARTIFACT_BYTES: &[u8] = b"PAR1\x00\x00\x00\x00equity-curve-parquet-bytes\x00\x00";

/// Seed a member-owned SUCCEEDED run with one EQUITY_CURVE artifact whose
/// manifest sha256 matches the real file at `artifact_root/<rel_path>`.
/// Returns (run_id, artifact_id).
async fn seed_run_with_artifact(
    h: &Harness,
    actor: &common::UserCtx,
    rel_path: &str,
    bytes: &[u8],
) -> (String, String) {
    let sha = h.write_artifact(rel_path, bytes);
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO backtest_runs (id, owner_user_id, strategy_id, strategy_version, dataset_version, engine_version, config_sha256, code_commit, status, summary_json) VALUES \
             (gen_random_uuid(), '{owner}', 'buy_and_hold', '1.0.0', 'krx_eod_bars@2026-01-01', '1.231.0', repeat('1',64), 'PENDING', 'SUCCEEDED', '{{}}'::jsonb)",
            owner = actor.user_id
        ),
    )
    .await;
    let run_id: String = sqlx::query_scalar(
        "SELECT id::text FROM backtest_runs WHERE owner_user_id = $1::uuid ORDER BY created_at DESC LIMIT 1",
    )
    .bind(actor.user_id.to_string())
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    h.seed_tenant(
        actor,
        &format!(
            "INSERT INTO result_artifacts (id, backtest_run_id, owner_user_id, artifact_type, parquet_path, row_count, sha256, size_bytes, summary_json) VALUES \
             (gen_random_uuid(), '{run_id}', '{owner}', 'EQUITY_CURVE', '{rel}', 5, '{sha}', {size}, '{{\"points\":[{{\"date\":\"2026-01-05\",\"equity\":\"100000000\"}}]}}'::jsonb)",
            owner = actor.user_id,
            rel = rel_path,
            sha = sha,
            size = bytes.len(),
        ),
    )
    .await;
    let artifact_id: String = sqlx::query_scalar(
        "SELECT a.id::text FROM result_artifacts a WHERE a.backtest_run_id = $1::uuid LIMIT 1",
    )
    .bind(&run_id)
    .fetch_one(&h.member_pool().await)
    .await
    .unwrap();
    (run_id, artifact_id)
}

fn no_redirect(resp: &axum::http::Response<axum::body::Body>) -> bool {
    resp.headers().get("x-accel-redirect").is_none()
}

#[tokio::test]
async fn artifact_owner_download_issues_internal_redirect_with_matching_hash() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let (_run_id, artifact_id) = seed_run_with_artifact(
        &h,
        &h.member,
        "runs/owner-ok/equity.parquet",
        ARTIFACT_BYTES,
    )
    .await;
    let sha = sha256_hex(ARTIFACT_BYTES);

    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let redirect = resp
        .headers()
        .get("x-accel-redirect")
        .and_then(|v| v.to_str().ok())
        .expect("authorized download must carry X-Accel-Redirect");
    assert_eq!(
        redirect,
        "/internal-artifacts/runs/owner-ok/equity.parquet",
        "redirect targets the internal alias path, never a filesystem path"
    );
    assert_eq!(
        resp.headers()
            .get("x-lagrange-sha256")
            .and_then(|v| v.to_str().ok()),
        Some(sha.as_str()),
        "the manifest hash must match the stored file bytes"
    );
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/vnd.apache.parquet")
    );
    let body = Harness::body_text(resp).await;
    assert!(
        !body.contains("parquet_path") && !body.contains("data/artifacts"),
        "response body must not leak filesystem paths"
    );
    h.teardown().await;
}

#[tokio::test]
async fn artifact_cross_user_download_fails_closed_without_redirect() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let (_run_id, artifact_id) = seed_run_with_artifact(
        &h,
        &h.member,
        "runs/foreign/equity.parquet",
        ARTIFACT_BYTES,
    )
    .await;
    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND, "foreign owner -> 404");
    assert!(no_redirect(&resp), "denied download must not carry the redirect");
    let body = Harness::body_text(resp).await;
    assert!(
        !body.contains("parquet") && !body.contains("data/artifacts") && !body.contains("runs/"),
        "denial must not leak paths (body: {body})"
    );
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("error envelope");
    assert_eq!(Harness::error_code(&parsed), "RESOURCE_NOT_FOUND");
    h.teardown().await;
}

#[tokio::test]
async fn artifact_direct_internal_path_is_not_served() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // The internal alias path exists only inside Nginx; the API router must
    // answer 404 for a direct request (no route, no bytes).
    let resp = h
        .get(
            "/internal-artifacts/runs/xyz/equity.parquet",
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::NOT_FOUND);
    let body = Harness::body_text(resp).await;
    assert!(!body.contains("PAR1"), "no artifact bytes may be served");
    h.teardown().await;
}

#[tokio::test]
async fn artifact_hash_mismatch_fails_closed() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let (_run_id, artifact_id) = seed_run_with_artifact(
        &h,
        &h.member,
        "runs/tampered/equity.parquet",
        ARTIFACT_BYTES,
    )
    .await;
    // The manifest hash now disagrees with the stored bytes.
    let _ = h.write_artifact("runs/tampered/equity.parquet", b"PAR1 tampered-bytes");
    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&h.member),
        )
        .await;
    assert_eq!(
        status(&resp),
        StatusCode::UNPROCESSABLE_ENTITY,
        "tampered artifact -> RESULT_INTEGRITY_FAILED"
    );
    assert!(no_redirect(&resp), "integrity failure must not issue the redirect");
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "RESULT_INTEGRITY_FAILED");
    assert!(!body.to_string().contains("runs/tampered"));
    h.teardown().await;
}

#[tokio::test]
async fn artifact_traversal_path_rejected() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // A poisoned manifest path (path traversal / absolute / backslash) must
    // never translate into a redirect or a file read.
    for poison in [
        "../outside/secret.parquet",
        "runs/../../etc/passwd",
        "/etc/passwd",
        "C:\\windows\\system.ini",
        "runs/..%2f..%2fsecret",
    ] {
        let (_run_id, artifact_id) = seed_run_with_artifact(
            &h,
            &h.member,
            "runs/safe/equity.parquet",
            ARTIFACT_BYTES,
        )
        .await;
        h.seed_tenant(
            &h.member,
            &format!(
                "UPDATE result_artifacts SET parquet_path = '{poison}' WHERE id = '{artifact_id}'"
            ),
        )
        .await;
        let resp = h
            .get(
                &format!("/api/v1/artifacts/{artifact_id}/download"),
                Some(&h.member),
            )
            .await;
        assert!(
            no_redirect(&resp),
            "poisoned path {poison:?} must not produce a redirect"
        );
        assert_ne!(
            status(&resp),
            StatusCode::OK,
            "poisoned path {poison:?} must fail closed"
        );
        let body = Harness::body_text(resp).await;
        assert!(
            !body.contains("secret") && !body.contains("passwd") && !body.contains("system.ini"),
            "denial must not echo the poisoned path (body: {body})"
        );
    }
    h.teardown().await;
}

#[tokio::test]
async fn artifact_expired_entitlement_denied() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let (_run_id, artifact_id) = seed_run_with_artifact(
        &h,
        &h.member,
        "runs/entitled/equity.parquet",
        ARTIFACT_BYTES,
    )
    .await;
    // Fail closed: revoke the ACTIVE entitlement; only the EXPIRED (2020)
    // contract remains, so the download use is denied for the member.
    h.seed_shared(
        "UPDATE data_entitlements SET status = 'REVOKED' WHERE contract_reference = 'krx-2026-01'",
    )
    .await;
    let resp = h
        .get(
            &format!("/api/v1/artifacts/{artifact_id}/download"),
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);
    assert!(no_redirect(&resp), "expired entitlement must not issue the redirect");
    let body = Harness::body_json(resp).await;
    assert_eq!(Harness::error_code(&body), "DATA_ENTITLEMENT_REQUIRED");
    assert!(!body.to_string().contains("parquet"));
    h.teardown().await;
}

#[tokio::test]
async fn artifact_metadata_never_leaks_internal_path() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let (_run_id, artifact_id) = seed_run_with_artifact(
        &h,
        &h.member,
        "runs/meta/equity.parquet",
        ARTIFACT_BYTES,
    )
    .await;
    let resp = h
        .get(&format!("/api/v1/artifacts/{artifact_id}"), Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let text = body.to_string();
    assert!(
        !text.contains("parquet_path") && !text.contains("data/artifacts") && !text.contains("runs/meta"),
        "metadata must not expose the storage path: {text}"
    );
    assert_eq!(body["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(body["artifact_type"], "EQUITY_CURVE");
    h.teardown().await;
}
