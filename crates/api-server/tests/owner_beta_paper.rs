//! Owner-beta Paper's disabled boundary and authenticated router surface.

mod common;

use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::{Value, json};

const PREVIEW_ID: &str = "00000000-0000-0000-0000-000000000003";

fn paper_routes() -> [(&'static str, &'static str, Option<Value>); 6] {
    [
        ("GET", "/api/v1/paper/owner-beta/state", None),
        (
            "POST",
            "/api/v1/paper/owner-beta/consent",
            Some(json!({"opt_in": true, "expected_epoch": 0})),
        ),
        (
            "POST",
            "/api/v1/paper/owner-beta/previews",
            Some(json!({
                "recommendation_run_id": "00000000-0000-0000-0000-000000000001",
                "expected_epoch": 0
            })),
        ),
        (
            "GET",
            "/api/v1/paper/owner-beta/previews/00000000-0000-0000-0000-000000000003",
            None,
        ),
        (
            "POST",
            "/api/v1/paper/owner-beta/previews/00000000-0000-0000-0000-000000000003/apply",
            Some(
                json!({"preview_token": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "expected_epoch": 0}),
            ),
        ),
        (
            "GET",
            "/api/v1/paper/owner-beta/sessions/00000000-0000-0000-0000-000000000004",
            None,
        ),
    ]
}

#[tokio::test]
async fn owner_beta_paper_routes_keep_owner_gate_and_typed_disabled_boundary() {
    let Some(mut harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    harness.restart_api_with_owner_beta_access().await;

    for (method, path, body) in paper_routes() {
        let member = harness
            .send(
                method,
                path,
                Some(&harness.member),
                false,
                Some("paper-member"),
                None,
                body.clone(),
            )
            .await;
        assert_eq!(status(&member), StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(
            Harness::error_code(&Harness::body_json(member).await),
            "FORBIDDEN",
            "{method} {path}"
        );

        let owner = harness
            .send(
                method,
                path,
                Some(&harness.owner),
                false,
                Some("paper-owner-disabled"),
                None,
                body,
            )
            .await;
        if path.ends_with("/state") {
            assert_eq!(status(&owner), StatusCode::OK);
            let response_id = owner
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .expect("request correlation is echoed");
            assert_eq!(response_id, "paper-owner-disabled");
            assert_eq!(
                owner
                    .headers()
                    .get("cache-control")
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            let body = Harness::body_json(owner).await;
            assert_eq!(body["schema"], "owner-beta-paper-state-v1");
            assert_eq!(body["state"], "DISABLED");
            assert_eq!(body["sessions"], json!([]));
        } else {
            assert_eq!(status(&owner), StatusCode::FORBIDDEN, "{method} {path}");
            assert_eq!(
                Harness::error_code(&Harness::body_json(owner).await),
                "PAPER_DISABLED",
                "{method} {path}"
            );
        }
    }

    let generic_paper = harness
        .get("/api/v1/paper/accounts", Some(&harness.owner))
        .await;
    assert_eq!(status(&generic_paper), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(generic_paper).await),
        "FORBIDDEN",
        "generic Paper keeps its existing disabled behavior"
    );

    harness.teardown().await;
}

#[tokio::test]
async fn owner_beta_paper_enabled_without_matching_protected_runtime_cannot_queue_or_apply() {
    let Some(mut harness) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    harness.restart_api_with_owner_beta_paper().await;

    let submit = harness
        .send(
            "POST",
            "/api/v1/paper/owner-beta/previews",
            Some(&harness.owner),
            true,
            Some("paper-runtime-required-submit"),
            Some("runtime-required-submit"),
            Some(json!({
                "recommendation_run_id": "00000000-0000-0000-0000-000000000001",
                "expected_epoch": 0
            })),
        )
        .await;
    assert_ne!(status(&submit), StatusCode::ACCEPTED);
    assert_ne!(status(&submit), StatusCode::OK);
    let submit_code = Harness::error_code(&Harness::body_json(submit).await);
    assert!(submit_code == "PAPER_UNBOUND" || submit_code == "PAPER_SOURCE_BLOCKED");

    let apply = harness
        .send(
            "POST",
            &format!("/api/v1/paper/owner-beta/previews/{PREVIEW_ID}/apply"),
            Some(&harness.owner),
            true,
            Some("paper-runtime-required-apply"),
            Some("runtime-required-apply"),
            Some(json!({
                "preview_token": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "expected_epoch": 0
            })),
        )
        .await;
    assert_ne!(status(&apply), StatusCode::OK);
    let apply_code = Harness::error_code(&Harness::body_json(apply).await);
    assert!(apply_code == "PAPER_UNBOUND" || apply_code == "PAPER_SOURCE_BLOCKED");

    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE owner_user_id=$1 AND job_type='owner_beta_paper_preview'",
    )
    .bind(harness.owner.user_id)
    .fetch_one(&harness.owner_pool)
    .await
    .expect("scratch database queue count");
    let targets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pending_targets WHERE owner_user_id=$1 AND source_kind IN ('OWNER_BETA_DAILY_MANUAL','OWNER_BETA_DAILY_AUTO')",
    )
    .bind(harness.owner.user_id)
    .fetch_one(&harness.owner_pool)
    .await
    .expect("scratch database target count");
    assert_eq!(queued, 0, "runtime refusal must not create a Paper job");
    assert_eq!(
        targets, 0,
        "runtime refusal must not create an economic target"
    );

    harness.teardown().await;
}
