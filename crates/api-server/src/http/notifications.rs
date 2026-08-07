//! Notification routes (FR-RPT-002): the actor's feed and subscriptions,
//! the test-alert endpoint that routes a severity-graded alert and records
//! delivery outcomes (SUCCESS/FAILED), and the Owner-only cross-user
//! delivery view. Alert grades follow design §15.3.

use crate::http::dto::{
    AdminDeliveryDto, DeliveryOutcomeDto, NotificationDeliveryDto, NotificationDto, PageDto,
    SubscriptionBody, SubscriptionDto, TestNotificationBody, TestNotificationResult,
};
use crate::http::error::{api_error, request_id};
use crate::http::pagination::Cursor;
use crate::http::session::{PageParams, Session};
use crate::http::state::ApiState;
use crate::http::{JsonBody, audit, idempotent, tenancy_response};
use crate::notify::{AlertSeverity, CHANNELS, KINDS};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

pub async fn list(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    let actor = session.actor();
    match state
        .notifier()
        .list_notifications(&actor, cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let ids: Vec<uuid::Uuid> = rows.iter().map(|n| n.id).collect();
            let deliveries = match state.notifier().deliveries_for(&actor, &ids).await {
                Ok(d) => d,
                Err(e) => return tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
            };
            let items: Vec<NotificationDto> = rows
                .into_iter()
                .map(|n| NotificationDto {
                    deliveries: deliveries
                        .iter()
                        .filter(|d| d.notification_id == n.id)
                        .map(|d| NotificationDeliveryDto {
                            channel: d.channel.clone(),
                            status: d.status.clone(),
                            error_detail: d.error_detail.clone(),
                        })
                        .collect(),
                    id: n.id.to_string(),
                    kind: n.kind,
                    title: n.title,
                    body: n.body,
                    read_at: n.read_at,
                    created_at: n.created_at,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn list_subscriptions(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let rid = request_id(&headers);
    let actor = session.actor();
    match state.notifier().list_subscriptions(&actor).await {
        Ok(rows) => {
            let items: Vec<SubscriptionDto> = rows
                .into_iter()
                .map(|s| SubscriptionDto {
                    kind: s.kind,
                    channel: s.channel,
                    enabled: s.enabled,
                })
                .collect();
            (StatusCode::OK, Json(PageDto::new(items, None))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

pub async fn upsert_subscription(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<SubscriptionBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    if !KINDS.contains(&body.kind.as_str()) || !CHANNELS.contains(&body.channel.as_str()) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            format!(
                "kind must be one of {} and channel one of {}",
                KINDS.join(","),
                CHANNELS.join(",")
            ),
            &rid,
            None,
        );
    }
    let body_hash = crate::http::idempotency::body_hash(
        &serde_json::json!({ "kind": &body.kind, "channel": &body.channel, "enabled": body.enabled }),
    );
    idempotent(&state, &session, &headers, &body_hash, async {
        let actor = session.actor();
        match state
            .notifier()
            .upsert_subscription(&actor, &body.kind, &body.channel, body.enabled)
            .await
        {
            Ok(s) => {
                audit(
                    &state,
                    &session,
                    &headers,
                    "notifications.subscription.upsert",
                    "notification_subscription",
                    &format!("{}:{}", s.kind, s.channel),
                    None,
                    Some(serde_json::json!({ "enabled": s.enabled })),
                    None,
                )
                .await;
                (
                    StatusCode::OK,
                    Json(SubscriptionDto {
                        kind: s.kind,
                        channel: s.channel,
                        enabled: s.enabled,
                    }),
                )
                    .into_response()
            }
            Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        }
    })
    .await
}

pub async fn test_notification(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    JsonBody(body): JsonBody<TestNotificationBody>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(r) = crate::http::session::require_csrf(&headers, &session.0) {
        return r;
    }
    let severity = match AlertSeverity::parse(&body.severity) {
        Some(s) => s,
        None => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_PARAMETER",
                "severity must be INFO, WARNING, or CRITICAL",
                &rid,
                None,
            );
        }
    };
    if !KINDS.contains(&body.kind.as_str()) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMETER",
            format!("kind must be one of {}", KINDS.join(",")),
            &rid,
            None,
        );
    }
    let body_hash = crate::http::idempotency::body_hash(
        &serde_json::json!({ "severity": &body.severity, "kind": &body.kind, "title": &body.title }),
    );
    idempotent(&state, &session, &headers, &body_hash, async {
        let actor = session.actor();
        match state
            .notifier()
            .route_alert(&actor, severity, &body.kind, &body.title, &body.body)
            .await
        {
            Ok(result) => {
                audit(
                    &state,
                    &session,
                    &headers,
                    "notifications.test",
                    "alert",
                    severity.as_str(),
                    None,
                    Some(serde_json::json!({
                        "notifications": result.notifications.len(),
                        "deliveries": result.deliveries.len(),
                    })),
                    None,
                )
                .await;
                (
                    StatusCode::OK,
                    Json(TestNotificationResult {
                        notifications: result
                            .notifications
                            .iter()
                            .map(|id| id.to_string())
                            .collect(),
                        deliveries: result
                            .deliveries
                            .into_iter()
                            .map(|d| DeliveryOutcomeDto {
                                notification_id: d.notification_id.to_string(),
                                channel: d.channel,
                                status: d.status.to_string(),
                                error_detail: d.error_detail,
                            })
                            .collect(),
                    }),
                )
                    .into_response()
            }
            Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
        }
    })
    .await
}

pub async fn list_deliveries(
    State(state): State<ApiState>,
    session: Session,
    headers: HeaderMap,
    Query(params): Query<PageParams>,
) -> Response {
    let rid = request_id(&headers);
    if !session.actor().is_owner() {
        audit(
            &state,
            &session,
            &headers,
            "admin.notifications.deliveries.list",
            "notification_delivery",
            "all",
            None,
            None,
            Some("FORBIDDEN_MEMBER".to_string()),
        )
        .await;
        return api_error(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "admin notification deliveries are Owner-only",
            &rid,
            None,
        );
    }
    let cursor = match decode_cursor(&state, &rid, params.cursor.as_deref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let limit = params.limit_or(PageParams::DEFAULT_LIMIT);
    match state
        .notifier()
        .list_all_deliveries(cursor.as_ref(), limit)
        .await
    {
        Ok((rows, next)) => {
            let next = next.map(|c| c.encode(&state.cfg.cursor_secret));
            let items: Vec<AdminDeliveryDto> = rows
                .into_iter()
                .map(|d| AdminDeliveryDto {
                    notification_id: d.notification_id.to_string(),
                    owner_user_id: d.owner_user_id.to_string(),
                    channel: d.channel,
                    status: d.status,
                    error_detail: d.error_detail,
                    attempted_at: d.attempted_at,
                })
                .collect();
            audit(
                &state,
                &session,
                &headers,
                "admin.notifications.deliveries.list",
                "notification_delivery",
                "all",
                None,
                None,
                None,
            )
            .await;
            (StatusCode::OK, Json(PageDto::new(items, next))).into_response()
        }
        Err(e) => tenancy_response(e, &rid, "RESOURCE_NOT_FOUND"),
    }
}

#[allow(clippy::result_large_err)]
fn decode_cursor(
    state: &ApiState,
    rid: &str,
    raw: Option<&str>,
) -> Result<Option<Cursor>, Response> {
    match raw {
        None => Ok(None),
        Some(r) => match Cursor::decode(r, &state.cfg.cursor_secret) {
            Ok(c) => Ok(Some(c)),
            Err(_) => Err(crate::http::error::code_error(
                "INVALID_CURSOR",
                "pagination cursor is invalid",
                rid,
            )),
        },
    }
}
