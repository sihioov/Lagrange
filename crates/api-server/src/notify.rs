//! Notification and alert-routing service (design §15.3, FR-RPT-002).
//!
//! Alert grades: INFO routes to the user's web feed; WARNING and CRITICAL
//! additionally route an immediate admin alert to the Owner (the "관리자
//! 알림" of the design table). A per-user `email` subscription adds an email
//! attempt to the same alert. Every attempt writes a durable delivery
//! outcome (`notification_deliveries`: SUCCESS/FAILED + error detail), so an
//! outage is recorded, never silent. All rows are tenant rows written under
//! the RECIPIENT's RLS actor context.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::{Actor, Role};
use uuid::Uuid;

/// One delivery attempt outcome, recorded durably.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertOutcome {
    pub notification_id: Uuid,
    pub channel: String,
    pub status: &'static str,
    pub error_detail: Option<String>,
}

/// The result of routing one alert.
#[derive(Debug, Clone, Default)]
pub struct AlertResult {
    pub notifications: Vec<Uuid>,
    pub deliveries: Vec<AlertOutcome>,
}

/// Alert grades of the design §15.3 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl AlertSeverity {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "INFO" => Some(Self::Info),
            "WARNING" => Some(Self::Warning),
            "CRITICAL" => Some(Self::Critical),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Critical => "CRITICAL",
        }
    }

    /// Design §15.3: INFO -> 웹; WARNING/CRITICAL -> 웹 + 관리자.
    fn channels(&self) -> &'static [&'static str] {
        match self {
            Self::Info => &["web"],
            Self::Warning | Self::Critical => &["web", "admin"],
        }
    }
}

/// One delivery transport. `web`/`admin` succeed by writing the row; the
/// email transport is not configured in this release and fails RECORDED
/// (FR-RPT-002: 전달 결과가 기록된다 — an outage is never silent).
pub trait Transport: Send + Sync {
    fn deliver(&self, channel: &str, title: &str, body: &str) -> Result<(), String>;
}

/// Web/in-app delivery: the notification row itself is the delivery.
pub struct WebTransport;

impl Transport for WebTransport {
    fn deliver(&self, _channel: &str, _title: &str, _body: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Email is optional in this release; attempts fail with a typed detail.
pub struct EmailTransport;

impl Transport for EmailTransport {
    fn deliver(&self, _channel: &str, _title: &str, _body: &str) -> Result<(), String> {
        Err("email delivery not configured in this release".to_string())
    }
}

fn transport_for(channel: &str) -> Box<dyn Transport> {
    match channel {
        "email" => Box::new(EmailTransport),
        _ => Box::new(WebTransport),
    }
}

/// Valid subscription/notification kinds and channels (mirror of the 0011
/// CHECK constraints; validated in code for typed 400s).
pub const KINDS: &[&str] = &["job", "recommendation", "backtest", "alert"];
pub const CHANNELS: &[&str] = &["web", "email", "admin"];

/// The notification service: subscriptions, alert routing, deliveries.
#[derive(Debug, Clone)]
pub struct Notifier {
    app_pool: sqlx::PgPool,
    admin_pool: sqlx::PgPool,
}

impl Notifier {
    pub fn new(app_pool: sqlx::PgPool, admin_pool: sqlx::PgPool) -> Self {
        Self { app_pool, admin_pool }
    }

    /// Route one alert: severity determines the channel set and whether the
    /// Owner receives an immediate admin alert (design §15.3). Every attempt
    /// records a delivery outcome.
    pub async fn route_alert(
        &self,
        actor: &Actor,
        severity: AlertSeverity,
        kind: &str,
        title: &str,
        body: &str,
    ) -> TenancyResult<AlertResult> {
        let mut result = AlertResult::default();
        // The actor's own notification: members receive the web leg only —
        // the admin leg of WARNING/CRITICAL belongs to the Owner below
        // (design §15.3 "웹 + 관리자 알림").
        let mut channels: Vec<&str> = if actor.is_owner() {
            severity.channels().to_vec()
        } else {
            vec!["web"]
        };
        if self.has_subscription(actor, "alert", "email").await? {
            channels.push("email");
        }
        let outcomes = self
            .notify_recipient(actor, &channels, kind, title, body)
            .await?;
        result.notifications.extend(outcomes.iter().map(|o| o.notification_id));
        result.deliveries.extend(outcomes);
        // WARNING/CRITICAL: immediate admin alert to the Owner (design 15.3).
        if severity != AlertSeverity::Info && !actor.is_owner() {
            if let Some(owner_id) = self.owner_user_id().await? {
                let owner = Actor::new(owner_id.to_string(), Role::Owner);
                let outcomes = self.notify_recipient(&owner, &["admin"], kind, title, body).await?;
                result.notifications.extend(outcomes.iter().map(|o| o.notification_id));
                result.deliveries.extend(outcomes);
            }
        }
        Ok(result)
    }

    /// Create ONE notification for the recipient and record a delivery
    /// outcome per channel, all under the RECIPIENT's RLS context (one
    /// short transaction).
    async fn notify_recipient(
        &self,
        recipient: &Actor,
        channels: &[&str],
        kind: &str,
        title: &str,
        body: &str,
    ) -> TenancyResult<Vec<AlertOutcome>> {
        let mut tx = begin_actor_tx(&self.app_pool, recipient).await?;
        let notification_id: Uuid = sqlx::query_scalar(
            "INSERT INTO notifications (owner_user_id, kind, title, body) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(actor_uuid(recipient)?)
        .bind(kind)
        .bind(title)
        .bind(body)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let mut outcomes = Vec::with_capacity(channels.len());
        for channel in channels {
            let delivery = transport_for(channel).deliver(channel, title, body);
            let (status, error_detail) = match delivery {
                Ok(()) => ("SUCCESS", None),
                Err(e) => ("FAILED", Some(e)),
            };
            sqlx::query(
                "INSERT INTO notification_deliveries \
                 (notification_id, owner_user_id, channel, status, error_detail) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(notification_id)
            .bind(actor_uuid(recipient)?)
            .bind(channel)
            .bind(status)
            .bind(&error_detail)
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            outcomes.push(AlertOutcome {
                notification_id,
                channel: channel.to_string(),
                status,
                error_detail,
            });
        }
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(outcomes)
    }

    /// The Owner user id (admin role read; `None` when no owner exists).
    pub async fn owner_user_id(&self) -> TenancyResult<Option<Uuid>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT u.id FROM users u \
             JOIN user_roles ur ON ur.user_id = u.id \
             WHERE ur.role_id = 'owner' ORDER BY u.created_at LIMIT 1",
        )
        .fetch_optional(&self.admin_pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(id)
    }

    /// Upsert one subscription (INSERT ... ON CONFLICT under the actor GUC).
    pub async fn upsert_subscription(
        &self,
        actor: &Actor,
        kind: &str,
        channel: &str,
        enabled: bool,
    ) -> TenancyResult<SubscriptionRow> {
        let mut tx = begin_actor_tx(&self.app_pool, actor).await?;
        let row = sqlx::query_as::<_, SubscriptionRow>(
            "INSERT INTO notification_subscriptions (owner_user_id, kind, channel, enabled) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (owner_user_id, kind, channel) \
             DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = now() \
             RETURNING kind, channel, enabled",
        )
        .bind(actor_uuid(actor)?)
        .bind(kind)
        .bind(channel)
        .bind(enabled)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    /// The actor's subscriptions.
    pub async fn list_subscriptions(&self, actor: &Actor) -> TenancyResult<Vec<SubscriptionRow>> {
        let mut tx = begin_actor_tx(&self.app_pool, actor).await?;
        let rows = sqlx::query_as::<_, SubscriptionRow>(
            "SELECT kind, channel, enabled FROM notification_subscriptions \
             WHERE owner_user_id = $1 ORDER BY kind, channel",
        )
        .bind(actor_uuid(actor)?)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    async fn has_subscription(&self, actor: &Actor, kind: &str, channel: &str) -> TenancyResult<bool> {
        let mut tx = begin_actor_tx(&self.app_pool, actor).await?;
        let enabled: Option<bool> = sqlx::query_scalar(
            "SELECT enabled FROM notification_subscriptions \
             WHERE owner_user_id = $1 AND kind = $2 AND channel = $3",
        )
        .bind(actor_uuid(actor)?)
        .bind(kind)
        .bind(channel)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(enabled.unwrap_or(false))
    }

    /// The actor's notification feed (paginated).
    pub async fn list_notifications(
        &self,
        actor: &Actor,
        after: Option<&crate::http::pagination::Cursor>,
        limit: usize,
    ) -> TenancyResult<(Vec<NotificationRow>, Option<crate::http::pagination::Cursor>)> {
        let mut tx = begin_actor_tx(&self.app_pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT id, owner_user_id, kind, title, body, read_at, created_at \
                 FROM notifications \
                 WHERE owner_user_id = $3 AND (created_at, id) > ($1::timestamptz, $2::uuid) \
                 ORDER BY created_at, id LIMIT $4"
            }
            None => {
                "SELECT id, owner_user_id, kind, title, body, read_at, created_at \
                 FROM notifications WHERE owner_user_id = $1 \
                 ORDER BY created_at, id LIMIT $2"
            }
        };
        let mut q = sqlx::query_as::<_, NotificationRow>(sql);
        if let Some(c) = after {
            q = q.bind(c.k.clone()).bind(parse_uuid(c)?);
        }
        let rows = q
            .bind(actor_uuid(actor)?)
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(crate::repos::split_page(rows, limit, |r| {
            (r.created_at.to_rfc3339(), r.id.to_string())
        }))
    }

    /// Cross-user delivery records for the admin view (Owner-only upstream).
    pub async fn list_all_deliveries(
        &self,
        after: Option<&crate::http::pagination::Cursor>,
        limit: usize,
    ) -> TenancyResult<(Vec<DeliveryRow>, Option<crate::http::pagination::Cursor>)> {
        let sql = match after {
            Some(_) => {
                "SELECT notification_id, owner_user_id, channel, status, error_detail, attempted_at \
                 FROM notification_deliveries \
                 WHERE (attempted_at, id) > ($1::timestamptz, $2::uuid) \
                 ORDER BY attempted_at, id LIMIT $3"
            }
            None => {
                "SELECT notification_id, owner_user_id, channel, status, error_detail, attempted_at \
                 FROM notification_deliveries ORDER BY attempted_at, id LIMIT $1"
            }
        };
        let mut q = sqlx::query_as::<_, DeliveryRow>(sql);
        if let Some(c) = after {
            q = q.bind(c.k.clone()).bind(parse_uuid(c)?);
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&self.admin_pool)
            .await
            .map_err(TenancyError::from_sqlx)?;
        Ok(crate::repos::split_page(rows, limit, |r| {
            (r.attempted_at.to_rfc3339(), r.notification_id.to_string())
        }))
    }
}

fn parse_uuid(c: &crate::http::pagination::Cursor) -> TenancyResult<Uuid> {
    Uuid::parse_str(&c.i).map_err(|_| TenancyError::NotFound)
}

/// One subscription row.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SubscriptionRow {
    pub kind: String,
    pub channel: String,
    pub enabled: bool,
}

/// One notification-feed row.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub read_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One cross-user delivery record (admin view).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DeliveryRow {
    pub notification_id: Uuid,
    pub owner_user_id: Uuid,
    pub channel: String,
    pub status: String,
    pub error_detail: Option<String>,
    pub attempted_at: chrono::DateTime<chrono::Utc>,
}
