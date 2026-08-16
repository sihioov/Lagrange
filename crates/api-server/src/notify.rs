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
use job_queue::paper_execution::set_paper_transaction_timeouts;
use uuid::Uuid;

use crate::repos::pending_targets::PaperSettlementOutboxRow;

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
        Self {
            app_pool,
            admin_pool,
        }
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
        crate::observability::metrics::record_alert(severity.as_str());
        // The actor's own notification: members receive the web leg only —
        // the admin leg of WARNING/CRITICAL belongs to the Owner below
        // (design §15.3 "웹 + 관리자 알림").
        let mut channels: Vec<&str> = if actor.is_owner() {
            severity.channels().to_vec()
        } else {
            vec!["web"]
        };
        // The subscription is looked up for the kind ACTUALLY being routed,
        // so "configurable per kind" means what it says: opting into email
        // for `job` adds the email leg to job alerts and to nothing else.
        if self.has_subscription(actor, kind, "email").await? {
            channels.push("email");
        }
        let outcomes = self
            .notify_recipient(actor, &channels, kind, title, body)
            .await?;
        result
            .notifications
            .extend(outcomes.iter().map(|o| o.notification_id));
        result.deliveries.extend(outcomes);
        // WARNING/CRITICAL: immediate admin alert to the Owner (design 15.3).
        if severity != AlertSeverity::Info
            && !actor.is_owner()
            && let Some(owner_id) = self.owner_user_id().await?
        {
            let owner = Actor::new(owner_id.to_string(), Role::Owner);
            let outcomes = self
                .notify_recipient(&owner, &["admin"], kind, title, body)
                .await?;
            result
                .notifications
                .extend(outcomes.iter().map(|o| o.notification_id));
            result.deliveries.extend(outcomes);
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
        self.notify_recipient_with_source(recipient, channels, kind, title, body, None)
            .await
    }

    /// Idempotent variant used by the Paper settlement outbox.  The source
    /// key is unique per recipient, so a retry after a process kill reuses the
    /// existing notification and each channel's delivery row instead of
    /// creating a second alert.
    async fn notify_recipient_with_source(
        &self,
        recipient: &Actor,
        channels: &[&str],
        kind: &str,
        title: &str,
        body: &str,
        source_key: Option<&str>,
    ) -> TenancyResult<Vec<AlertOutcome>> {
        // First commit a per-channel lease, then invoke the transport.  This
        // ordering is deliberate: transport-before-DB-dedupe lets concurrent
        // retries send the same source key twice.  A process killed after an
        // external send but before the result update can still be retried
        // after lease expiry (explicit at-least-once semantics), but concurrent
        // runners cannot both own the same delivery row.
        let mut tx = begin_actor_tx(&self.app_pool, recipient).await?;
        let owner_user_id = actor_uuid(recipient)?;
        let notification_id: Uuid = match source_key {
            Some(source_key) => sqlx::query_scalar(
                "INSERT INTO notifications \
                 (owner_user_id, kind, title, body, source_key) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (owner_user_id, source_key) WHERE source_key IS NOT NULL \
                 DO UPDATE SET id = notifications.id \
                 RETURNING id",
            )
            .bind(owner_user_id)
            .bind(kind)
            .bind(title)
            .bind(body)
            .bind(source_key)
            .fetch_one(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
            None => sqlx::query_scalar(
                "INSERT INTO notifications (owner_user_id, kind, title, body) \
                 VALUES ($1, $2, $3, $4) RETURNING id",
            )
            .bind(owner_user_id)
            .bind(kind)
            .bind(title)
            .bind(body)
            .fetch_one(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?,
        };

        // A source-key replay must carry exactly the original immutable
        // payload.  Silently accepting a changed body would make a retry look
        // successful while presenting stale or contradictory evidence.
        if source_key.is_some() {
            let existing: (String, String, String) =
                sqlx::query_as("SELECT kind, title, body FROM notifications WHERE id = $1")
                    .bind(notification_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(TenancyError::from_sqlx)?;
            if existing.0 != kind || existing.1 != title || existing.2 != body {
                return Err(TenancyError::InvalidState(
                    "Paper notification source key payload mismatch".to_owned(),
                ));
            }
        }

        let mut claims = Vec::with_capacity(channels.len());
        let mut outcomes = Vec::with_capacity(channels.len());
        for channel in channels {
            let token = Uuid::new_v4();
            let claimed: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
                "INSERT INTO notification_deliveries \
                 (notification_id, owner_user_id, channel, status, error_detail, \
                  delivery_token, delivery_lease_expires_at, delivery_attempts) \
                 VALUES ($1, $2, $3, 'FAILED', 'delivery claim pending', $4, \
                         now() + interval '60 seconds', 1) \
                 ON CONFLICT (notification_id, channel) DO UPDATE SET \
                     status = 'FAILED', \
                     error_detail = 'delivery claim pending', \
                     delivery_token = EXCLUDED.delivery_token, \
                     delivery_lease_expires_at = EXCLUDED.delivery_lease_expires_at, \
                     delivery_attempts = notification_deliveries.delivery_attempts + 1, \
                     attempted_at = now() \
                 WHERE notification_deliveries.status <> 'SUCCESS' \
                   AND (notification_deliveries.delivery_lease_expires_at IS NULL \
                        OR notification_deliveries.delivery_lease_expires_at <= now()) \
                 RETURNING id, channel, error_detail",
            )
            .bind(notification_id)
            .bind(owner_user_id)
            .bind(channel)
            .bind(token)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            if let Some((delivery_id, claimed_channel, _)) = claimed {
                claims.push((delivery_id, claimed_channel, token));
                continue;
            }
            let existing: (String, String, Option<String>) = sqlx::query_as(
                "SELECT channel, status, error_detail \
                 FROM notification_deliveries \
                 WHERE notification_id = $1 AND channel = $2",
            )
            .bind(notification_id)
            .bind(channel)
            .fetch_one(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
            outcomes.push(AlertOutcome {
                notification_id,
                channel: existing.0,
                status: if existing.1 == "SUCCESS" {
                    "SUCCESS"
                } else {
                    "FAILED"
                },
                error_detail: existing.2,
            });
        }
        tx.commit().await.map_err(TenancyError::from_sqlx)?;

        // No database transaction is held while transports run.  Only the
        // rows whose lease was returned above are allowed to invoke a
        // transport; an already-successful row or an unexpired lease is a
        // durable no-op for this runner.
        let attempted: Vec<(Uuid, String, Uuid, &'static str, Option<String>)> = claims
            .into_iter()
            .map(|(delivery_id, channel, token)| {
                let (status, error_detail) =
                    match transport_for(&channel).deliver(&channel, title, body) {
                        Ok(()) => ("SUCCESS", None),
                        Err(error) => ("FAILED", Some(error)),
                    };
                (delivery_id, channel, token, status, error_detail)
            })
            .collect();
        if !attempted.is_empty() {
            let mut finish_tx = begin_actor_tx(&self.app_pool, recipient).await?;
            for (delivery_id, channel, token, status, error_detail) in attempted {
                let persisted: Option<(String, Option<String>)> = sqlx::query_as(
                    "UPDATE notification_deliveries \
                     SET status = $2, error_detail = $3, attempted_at = now(), \
                         delivery_token = NULL, delivery_lease_expires_at = NULL \
                     WHERE id = $1 AND delivery_token = $4 \
                     RETURNING status, error_detail",
                )
                .bind(delivery_id)
                .bind(status)
                .bind(&error_detail)
                .bind(token)
                .fetch_optional(&mut *finish_tx)
                .await
                .map_err(TenancyError::from_sqlx)?;
                let row = match persisted {
                    Some(row) => row,
                    None => sqlx::query_as(
                        "SELECT status, error_detail FROM notification_deliveries \
                         WHERE id = $1 AND channel = $2",
                    )
                    .bind(delivery_id)
                    .bind(&channel)
                    .fetch_one(&mut *finish_tx)
                    .await
                    .map_err(TenancyError::from_sqlx)?,
                };
                outcomes.push(AlertOutcome {
                    notification_id,
                    channel,
                    status: if row.0 == "SUCCESS" {
                        "SUCCESS"
                    } else {
                        "FAILED"
                    },
                    error_detail: row.1,
                });
            }
            finish_tx.commit().await.map_err(TenancyError::from_sqlx)?;
        }
        for outcome in &outcomes {
            crate::observability::metrics::record_delivery(outcome.status);
        }
        Ok(outcomes)
    }

    /// Dispatch one committed Paper settlement intent.  The intent itself is
    /// never deleted, and each recipient/channel is keyed by the outbox id;
    /// retries after cancellation are therefore safe and observable.
    pub async fn dispatch_paper_settlement(
        &self,
        actor: &Actor,
        outbox: &PaperSettlementOutboxRow,
    ) -> TenancyResult<AlertResult> {
        let actor_id = actor_uuid(actor)?;
        if actor_id != outbox.owner_user_id {
            return Err(TenancyError::Forbidden);
        }
        let severity = AlertSeverity::parse(&outbox.severity).ok_or_else(|| {
            TenancyError::InvalidState("invalid Paper outbox severity".to_owned())
        })?;
        crate::observability::metrics::record_alert(severity.as_str());
        let mut result = AlertResult::default();
        let mut channels: Vec<&str> = if actor.is_owner() {
            severity.channels().to_vec()
        } else {
            vec!["web"]
        };
        if self.has_subscription(actor, &outbox.kind, "email").await? {
            channels.push("email");
        }
        let source_key = outbox.id.to_string();
        let outcomes = self
            .notify_recipient_with_source(
                actor,
                &channels,
                &outbox.kind,
                &outbox.title,
                &outbox.body,
                Some(&source_key),
            )
            .await?;
        result
            .notifications
            .extend(outcomes.iter().map(|outcome| outcome.notification_id));
        result.deliveries.extend(outcomes);

        if severity != AlertSeverity::Info
            && !actor.is_owner()
            && let Some(owner_id) = self.owner_user_id().await?
        {
            let owner = Actor::new(owner_id.to_string(), Role::Owner);
            let outcomes = self
                .notify_recipient_with_source(
                    &owner,
                    &["admin"],
                    &outbox.kind,
                    &outbox.title,
                    &outbox.body,
                    Some(&source_key),
                )
                .await?;
            result
                .notifications
                .extend(outcomes.iter().map(|outcome| outcome.notification_id));
            result.deliveries.extend(outcomes);
        }
        Ok(result)
    }

    /// The Owner user id (admin role read; `None` when no owner exists).
    pub async fn owner_user_id(&self) -> TenancyResult<Option<Uuid>> {
        let mut tx = self
            .admin_pool
            .begin()
            .await
            .map_err(TenancyError::from_sqlx)?;
        set_paper_transaction_timeouts(&mut tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT u.id FROM users u \
             JOIN user_roles ur ON ur.user_id = u.id \
             WHERE ur.role_id = 'owner' ORDER BY u.created_at LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
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

    async fn has_subscription(
        &self,
        actor: &Actor,
        kind: &str,
        channel: &str,
    ) -> TenancyResult<bool> {
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
    ) -> TenancyResult<(
        Vec<NotificationRow>,
        Option<crate::http::pagination::Cursor>,
    )> {
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

    /// The actor's OWN delivery attempts for a page of notifications.
    ///
    /// Fetched in one round trip rather than per row: the feed renders each
    /// notification's outcome, and a per-row query would turn one page into
    /// N+1 statements. RLS still scopes the rows to the actor, so passing
    /// ids that are not theirs simply returns nothing.
    pub async fn deliveries_for(
        &self,
        actor: &Actor,
        notification_ids: &[Uuid],
    ) -> TenancyResult<Vec<OwnDeliveryRow>> {
        if notification_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = begin_actor_tx(&self.app_pool, actor).await?;
        let rows = sqlx::query_as::<_, OwnDeliveryRow>(
            "SELECT notification_id, channel, status, error_detail \
             FROM notification_deliveries \
             WHERE owner_user_id = $1 AND notification_id = ANY($2) \
             ORDER BY attempted_at, channel",
        )
        .bind(actor_uuid(actor)?)
        .bind(notification_ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
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

/// One of the actor's own delivery attempts, keyed to its notification.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OwnDeliveryRow {
    pub notification_id: Uuid,
    pub channel: String,
    pub status: String,
    pub error_detail: Option<String>,
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
