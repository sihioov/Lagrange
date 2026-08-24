//! The sealed price-only input is fail-closed when the approved artifact root
//! is unavailable. The handler must reject before the durable repository can
//! create either queue or run state.

mod common;

use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;

#[tokio::test]
async fn owner_beta_price_only_missing_approved_artifact_root_is_static_503_without_enqueue_rows() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    h.restart_api_with_owner_beta_price_input().await;

    let before_runs: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM owner_beta_recommendation_runs
          WHERE owner_user_id = $1",
    )
    .bind(h.owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner-beta run count before request");
    let before_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM jobs
          WHERE owner_user_id = $1",
    )
    .bind(h.owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner-beta job count before request");

    let response = h
        .send(
            "POST",
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&h.owner),
            true,
            Some("owner-beta-missing-approved-artifact-root"),
            Some("owner-beta-missing-approved-artifact-root-key"),
            Some(json!({
                "strategy_config_id": "00000000-0000-0000-0000-000000000001",
                "as_of": "2026-08-13"
            })),
        )
        .await;
    assert_eq!(status(&response), StatusCode::SERVICE_UNAVAILABLE);
    let body = Harness::body_json(response).await;
    assert_eq!(
        Harness::error_code(&body),
        "OWNER_BETA_PRICE_INPUT_UNAVAILABLE"
    );
    assert_eq!(
        body["error"]["message"],
        "owner-beta price input unavailable"
    );

    let after_runs: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM owner_beta_recommendation_runs
          WHERE owner_user_id = $1",
    )
    .bind(h.owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner-beta run count after request");
    let after_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM jobs
          WHERE owner_user_id = $1",
    )
    .bind(h.owner.user_id)
    .fetch_one(&h.admin_pool)
    .await
    .expect("owner-beta job count after request");
    assert_eq!(
        after_runs, before_runs,
        "price approval failure must not create a run"
    );
    assert_eq!(
        after_jobs, before_jobs,
        "price approval failure must not create a job"
    );

    h.teardown().await;
}

#[tokio::test]
async fn owner_beta_price_only_read_routes_are_owner_only_and_do_not_require_price_mode() {
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // This enables only the owner-beta admission boundary. The sealed price
    // input mode remains disabled, proving GET does not approve or gate on an
    // artifact input that is relevant only to POST.
    h.restart_api_with_owner_beta_access().await;

    let owner = h
        .get(
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&owner), StatusCode::OK);
    let owner_body = Harness::body_json(owner).await;
    assert_eq!(owner_body["items"], json!([]));
    assert_eq!(owner_body["has_more"], false);
    assert!(owner_body.get("next_cursor").is_some());

    let member = h
        .get(
            "/api/v1/recommendations/owner-beta/price-only/runs",
            Some(&h.member),
        )
        .await;
    assert_eq!(status(&member), StatusCode::FORBIDDEN);
    assert_eq!(
        Harness::error_code(&Harness::body_json(member).await),
        "FORBIDDEN"
    );

    let missing = h
        .get(
            &format!(
                "/api/v1/recommendations/owner-beta/price-only/runs/{}",
                uuid::Uuid::new_v4()
            ),
            Some(&h.owner),
        )
        .await;
    assert_eq!(status(&missing), StatusCode::NOT_FOUND);
    assert_eq!(
        Harness::error_code(&Harness::body_json(missing).await),
        "RESOURCE_NOT_FOUND"
    );

    h.teardown().await;
}
