//! The sealed price-only input is fail-closed while the embedded approval
//! registry has no approved artifact.  The handler must reject before the
//! durable repository can create either queue or run state.

mod common;

use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;

#[tokio::test]
async fn owner_beta_price_only_empty_registry_is_static_503_without_enqueue_rows() {
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
            Some("owner-beta-empty-registry"),
            Some("owner-beta-empty-registry-key"),
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
