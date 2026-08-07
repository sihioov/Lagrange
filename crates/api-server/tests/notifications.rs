//! Todo 27 notifications and alert routing (design §15.3, FR-RPT-002):
//! per-user subscriptions, durable delivery outcomes (SUCCESS/FAILED), and
//! severity-graded routing (INFO -> web; WARNING/CRITICAL -> web + admin).
//! An email outage is recorded as a FAILED delivery with error detail —
//! never silent. All tests carry the `observability_` prefix so the plan
//! acceptance filter `cargo test -p api-server artifact admin observability`
//! selects them.

mod common;

use axum::http::StatusCode;
use common::{Harness, status};
use serde_json::json;

async fn put_subscription(
    h: &Harness,
    user: Option<&common::UserCtx>,
    kind: &str,
    channel: &str,
    enabled: bool,
) -> axum::http::Response<axum::body::Body> {
    h.send(
        "PUT",
        "/api/v1/notifications/subscriptions",
        user,
        true,
        Some("test-rid-1"),
        Some("sub-key"),
        Some(json!({ "kind": kind, "channel": channel, "enabled": enabled })),
    )
    .await
}

async fn post_test_notification(
    h: &Harness,
    user: Option<&common::UserCtx>,
    severity: &str,
) -> axum::http::Response<axum::body::Body> {
    let key = format!("notify-{}", uuid::Uuid::new_v4());
    h.send(
        "POST",
        "/api/v1/notifications/test",
        user,
        true,
        Some("test-rid-1"),
        Some(&key),
        Some(json!({
            "severity": severity,
            "kind": "job",
            "title": format!("{severity} alert"),
            "body": "test body",
        })),
    )
    .await
}

async fn count_notifications(h: &Harness, owner: &common::UserCtx) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE owner_user_id = $1::uuid")
        .bind(owner.user_id.to_string())
        .fetch_one(&h.admin_pool)
        .await
        .unwrap()
}

async fn count_deliveries(h: &Harness, status_filter: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM notification_deliveries d \
         JOIN notifications n ON n.id = d.notification_id \
         WHERE n.owner_user_id = $1::uuid AND d.status = $2",
    )
    .bind(h.member.user_id.to_string())
    .bind(status_filter)
    .fetch_one(&h.admin_pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn observability_notification_subscription_upsert_and_list() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = put_subscription(&h, Some(&h.member), "backtest", "email", true).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["kind"], "backtest");
    assert_eq!(body["channel"], "email");
    assert_eq!(body["enabled"], true);

    // Member lists own subscriptions.
    let resp = h
        .get("/api/v1/notifications/subscriptions", Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let items = body["items"].as_array().expect("subscriptions");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "backtest");

    // The owner sees no subscriptions of the member (tenant isolation).
    let resp = h
        .get("/api/v1/notifications/subscriptions", Some(&h.owner))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    h.teardown().await;
}

#[tokio::test]
async fn observability_notification_test_records_web_delivery() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let before = count_notifications(&h, &h.member).await;
    let resp = post_test_notification(&h, Some(&h.member), "INFO").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let deliveries = body["deliveries"].as_array().expect("deliveries");
    assert_eq!(deliveries.len(), 1, "INFO routes to web only");
    assert_eq!(deliveries[0]["channel"], "web");
    assert_eq!(deliveries[0]["status"], "SUCCESS");

    assert_eq!(count_notifications(&h, &h.member).await, before + 1);
    assert_eq!(count_deliveries(&h, "SUCCESS").await, 1);
    h.teardown().await;
}

#[tokio::test]
async fn observability_notification_email_outage_records_failed_delivery() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // The member opted into email FOR THE KIND being routed; the email
    // transport is not configured in this release, so the outage must be
    // recorded as a FAILED delivery.
    let resp = put_subscription(&h, Some(&h.member), "job", "email", true).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let before = count_notifications(&h, &h.member).await;
    let resp = post_test_notification(&h, Some(&h.member), "INFO").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let deliveries = body["deliveries"].as_array().expect("deliveries");
    let email = deliveries
        .iter()
        .find(|d| d["channel"] == "email")
        .expect("email delivery attempted for the subscribed channel");
    assert_eq!(email["status"], "FAILED");
    assert!(
        email["error_detail"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "failed deliveries must carry an error detail"
    );
    // The web delivery still succeeded and the notification exists.
    assert_eq!(count_notifications(&h, &h.member).await, before + 1);
    assert_eq!(count_deliveries(&h, "FAILED").await, 1);
    assert_eq!(count_deliveries(&h, "SUCCESS").await, 1);
    h.teardown().await;
}

#[tokio::test]
async fn observability_notification_subscription_is_per_kind() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    // Opting into email for `alert` must NOT add an email leg to a `job`
    // alert. Notifications are configurable per kind, not globally.
    let resp = put_subscription(&h, Some(&h.member), "alert", "email", true).await;
    assert_eq!(status(&resp), StatusCode::OK);
    let resp = post_test_notification(&h, Some(&h.member), "INFO").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let deliveries = body["deliveries"].as_array().expect("deliveries");
    assert_eq!(
        deliveries.len(),
        1,
        "a subscription for another kind must not add a channel"
    );
    assert_eq!(deliveries[0]["channel"], "web");
    assert_eq!(count_deliveries(&h, "FAILED").await, 0);
    h.teardown().await;
}

#[tokio::test]
async fn observability_alert_routing_severity_grades() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let owner = h.owner.user_id.to_string();
    // INFO: web only, member recipient.
    let before_member = count_notifications(&h, &h.member).await;
    let before_owner: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE owner_user_id = $1::uuid")
            .bind(&owner)
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    let resp = post_test_notification(&h, Some(&h.member), "INFO").await;
    assert_eq!(status(&resp), StatusCode::OK);
    assert_eq!(count_notifications(&h, &h.member).await, before_member + 1);
    let owner_now: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE owner_user_id = $1::uuid")
            .bind(&owner)
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert_eq!(owner_now, before_owner, "INFO must not alert the admin");

    // WARNING: member web + owner admin (2 notifications, 2 deliveries).
    let resp = post_test_notification(&h, Some(&h.member), "WARNING").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let deliveries = body["deliveries"].as_array().expect("deliveries");
    assert_eq!(deliveries.len(), 2, "WARNING routes to web + admin");
    let channels: Vec<&str> = deliveries
        .iter()
        .map(|d| d["channel"].as_str().unwrap_or_default())
        .collect();
    assert!(
        channels.contains(&"web") && channels.contains(&"admin"),
        "WARNING channels must be web + admin: {channels:?}"
    );
    let owner_after_warning: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE owner_user_id = $1::uuid")
            .bind(&owner)
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert_eq!(
        owner_after_warning,
        before_owner + 1,
        "admin alert for WARNING"
    );

    // CRITICAL: same immediate admin routing.
    let resp = post_test_notification(&h, Some(&h.member), "CRITICAL").await;
    assert_eq!(status(&resp), StatusCode::OK);
    let owner_after_critical: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE owner_user_id = $1::uuid")
            .bind(&owner)
            .fetch_one(&h.admin_pool)
            .await
            .unwrap();
    assert_eq!(
        owner_after_critical,
        before_owner + 2,
        "admin alert for CRITICAL"
    );
    h.teardown().await;
}

#[tokio::test]
async fn observability_admin_deliveries_view_owner_only() {
    let Some(h) = Harness::new().await else {
        eprintln!("SKIP: DATABASE_URL not set");
        return;
    };
    let resp = post_test_notification(&h, Some(&h.member), "WARNING").await;
    assert_eq!(status(&resp), StatusCode::OK);

    // Member cannot read the cross-user delivery view.
    let resp = h
        .get("/api/v1/admin/notifications/deliveries", Some(&h.member))
        .await;
    assert_eq!(status(&resp), StatusCode::FORBIDDEN);

    // Owner sees the delivery records (read-only cross-user view).
    let resp = h
        .get("/api/v1/admin/notifications/deliveries", Some(&h.owner))
        .await;
    assert_eq!(status(&resp), StatusCode::OK);
    let body = Harness::body_json(resp).await;
    let items = body["items"].as_array().expect("delivery items");
    assert!(
        !items.is_empty(),
        "the WARNING alert deliveries must be visible"
    );
    let statuses: Vec<&str> = items
        .iter()
        .map(|d| d["status"].as_str().unwrap_or_default())
        .collect();
    assert!(
        statuses.iter().all(|s| *s == "SUCCESS" || *s == "FAILED"),
        "delivery statuses must be SUCCESS or FAILED: {statuses:?}"
    );
    h.teardown().await;
}
