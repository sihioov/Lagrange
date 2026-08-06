//! Operations repository (admin surface): cross-user reads over the
//! dedicated `admin` role, Owner-gated and audited like
//! [`crate::repos::admin::AdminRepo`]. Also owns the two server-side
//! mutations that serving roles cannot express directly:
//!
//! - `retry_job`: requeues a FAILED job. No serving role may UPDATE another
//!   user's `jobs` row (app RLS is actor-local; admin is SELECT-only), so the
//!   requeue runs as the app role with the GUC pinned to the JOB OWNER (the
//!   GUC is server-controlled; the operation is Owner-gated and audited
//!   before/after).
//! - dataset approve/block: `dataset_versions` is shared and SELECT-only for
//!   every serving role (owner = migration_owner, the data pipeline), so the
//!   API applies the Todo 11 approval POLICY as an audited verdict: a BLOCKED
//!   dataset can never be approved without a NEW dataset version
//!   (`DATASET_BLOCKED`); a WARNING dataset approves. The durable record is
//!   the append-only audit row.

use crate::actor_tx::{actor_uuid, begin_actor_tx, pool_for_actor};
use crate::error::{TenancyError, TenancyResult};
use crate::http::pagination::Cursor;
use crate::repos::audit::{AuditEntry, AuditWriter};
use crate::repos::split_page;
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AdminDatasetRow {
    pub id: Uuid,
    pub dataset_id: String,
    pub version: String,
    pub status: String,
    pub manifest_sha256: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct QualityIssueRow {
    pub issue_code: String,
    pub severity: String,
    pub detail_json: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AdminJobRow {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub job_type: String,
    pub status: String,
    pub priority: i32,
    pub idempotency_key: Option<String>,
    pub attempt_count: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct WorkerRow {
    pub worker_id: String,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub active_job_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub action: String,
    pub actor_role: String,
    pub actor_user_id: Option<Uuid>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub reason: Option<String>,
    pub correlation_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The policy verdict of a dataset approve/block call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetVerdict {
    pub dataset_id: String,
    pub version: String,
    pub status: String,
    pub verdict: &'static str,
    pub reason: String,
}

/// Admin operations over the dedicated admin pool (Owner-gated, audited).
#[derive(Debug, Clone)]
pub struct OpsRepo {
    admin_pool: sqlx::PgPool,
    audit: AuditWriter,
    app_url: String,
}

impl OpsRepo {
    pub fn new(admin_pool: sqlx::PgPool, audit_pool: sqlx::PgPool, app_url: String) -> Self {
        Self {
            admin_pool,
            audit: AuditWriter::new(audit_pool),
            app_url,
        }
    }

    async fn require_owner(
        &self,
        actor: &Actor,
        action: &str,
        target: (&str, &str),
        correlation_id: &str,
    ) -> TenancyResult<()> {
        if !actor.is_owner() {
            self.audit
                .record(
                    actor,
                    &AuditEntry {
                        action: action.to_string(),
                        target_type: target.0.to_string(),
                        target_id: target.1.to_string(),
                        before_json: None,
                        after_json: None,
                        reason: Some("FORBIDDEN_MEMBER".to_string()),
                        correlation_id: Some(correlation_id.to_string()),
                    },
                )
                .await?;
            return Err(TenancyError::Forbidden);
        }
        Ok(())
    }

    /// Cross-user dataset versions with their quality issues.
    pub async fn list_datasets(
        &self,
        actor: &Actor,
        correlation_id: &str,
    ) -> TenancyResult<Vec<(AdminDatasetRow, Vec<QualityIssueRow>)>> {
        self.require_owner(
            actor,
            "admin.datasets.list",
            ("dataset", "all"),
            correlation_id,
        )
        .await?;
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        let rows = sqlx::query_as::<_, AdminDatasetRow>(
            "SELECT id, dataset_id, version, status, manifest_sha256, created_at \
             FROM dataset_versions ORDER BY dataset_id, version",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let issues = self.quality_issues(&r.dataset_id, &r.version).await?;
            out.push((r, issues));
        }
        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: "admin.datasets.list".to_string(),
                    target_type: "dataset".to_string(),
                    target_id: "all".to_string(),
                    before_json: None,
                    after_json: None,
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(out)
    }

    async fn quality_issues(
        &self,
        dataset_id: &str,
        version: &str,
    ) -> TenancyResult<Vec<QualityIssueRow>> {
        let rows = sqlx::query_as::<_, QualityIssueRow>(
            "SELECT issue_code, severity, detail_json FROM data_quality_issues \
             WHERE dataset_id = $1 AND dataset_version = $2 ORDER BY created_at",
        )
        .bind(dataset_id)
        .bind(version)
        .fetch_all(&self.admin_pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Apply the Todo 11 dataset approval policy as an audited verdict.
    pub async fn dataset_verdict(
        &self,
        actor: &Actor,
        dataset_id: Uuid,
        action: &str,
        correlation_id: &str,
    ) -> TenancyResult<DatasetVerdict> {
        self.require_owner(
            actor,
            &format!("admin.dataset.{action}"),
            ("dataset", &dataset_id.to_string()),
            correlation_id,
        )
        .await?;
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        let row = sqlx::query_as::<_, AdminDatasetRow>(
            "SELECT id, dataset_id, version, status, manifest_sha256, created_at \
             FROM dataset_versions WHERE id = $1",
        )
        .bind(dataset_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        let row = crate::error::map_optional(row)?;

        let verdict = match (action, row.status.as_str()) {
            ("approve", "BLOCKED") => {
                let issues = self.quality_issues(&row.dataset_id, &row.version).await?;
                let codes: Vec<String> = issues.iter().map(|i| i.issue_code.clone()).collect();
                self.audit
                    .record(
                        actor,
                        &AuditEntry {
                            action: format!("admin.dataset.{action}"),
                            target_type: "dataset".to_string(),
                            target_id: row.id.to_string(),
                            before_json: Some(serde_json::json!({ "status": row.status })),
                            after_json: None,
                            reason: Some("DATASET_BLOCKED".to_string()),
                            correlation_id: Some(correlation_id.to_string()),
                        },
                    )
                    .await?;
                return Err(TenancyError::DatasetBlocked(format!(
                    "dataset {} is quality-blocked ({}); structural blocking issues require a NEW dataset version, not approval",
                    row.dataset_id,
                    codes.join(",")
                )));
            }
            ("approve", "READY") => DatasetVerdict {
                dataset_id: row.dataset_id.clone(),
                version: row.version.clone(),
                status: row.status.clone(),
                verdict: "APPROVED",
                reason: "dataset is already READY".to_string(),
            },
            ("approve", "WARNING") => {
                let issues = self.quality_issues(&row.dataset_id, &row.version).await?;
                let blocking: Vec<String> = issues
                    .iter()
                    .filter(|i| i.severity == "ERROR")
                    .map(|i| i.issue_code.clone())
                    .collect();
                if blocking.is_empty() {
                    DatasetVerdict {
                        dataset_id: row.dataset_id.clone(),
                        version: row.version.clone(),
                        status: row.status.clone(),
                        verdict: "APPROVED",
                        reason: format!(
                            "WARNING dataset approved ({} advisory issue(s))",
                            issues.len()
                        ),
                    }
                } else {
                    DatasetVerdict {
                        dataset_id: row.dataset_id.clone(),
                        version: row.version.clone(),
                        status: row.status.clone(),
                        verdict: "APPROVED_WITH_WARNINGS",
                        reason: format!("advisory issues: {}", blocking.join(", ")),
                    }
                }
            }
            ("block", _) => DatasetVerdict {
                dataset_id: row.dataset_id.clone(),
                version: row.version.clone(),
                status: row.status.clone(),
                verdict: "BLOCKED",
                reason: "dataset blocked for new runs by Owner action".to_string(),
            },
            ("approve", other) => DatasetVerdict {
                dataset_id: row.dataset_id.clone(),
                version: row.version.clone(),
                status: row.status.clone(),
                verdict: "UNCHANGED",
                reason: format!("dataset state {other} is not approvable"),
            },
            _ => unreachable!("action is approve or block"),
        };

        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: format!("admin.dataset.{action}"),
                    target_type: "dataset".to_string(),
                    target_id: row.id.to_string(),
                    before_json: Some(serde_json::json!({ "status": row.status })),
                    after_json: Some(serde_json::json!({ "verdict": verdict.verdict })),
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(verdict)
    }

    /// Cross-user queue view (paginated), Owner-only.
    pub async fn list_jobs(
        &self,
        actor: &Actor,
        after: Option<&Cursor>,
        limit: usize,
        correlation_id: &str,
    ) -> TenancyResult<(Vec<AdminJobRow>, Option<Cursor>)> {
        self.require_owner(actor, "admin.jobs.list", ("job", "all"), correlation_id)
            .await?;
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT id, owner_user_id, job_type, status, priority, idempotency_key, \
                        attempt_count, created_at, started_at, finished_at, error_code, error_message \
                 FROM jobs WHERE (created_at, id) > ($1::timestamptz, $2::uuid) ORDER BY created_at, id LIMIT $3"
            }
            None => {
                "SELECT id, owner_user_id, job_type, status, priority, idempotency_key, \
                        attempt_count, created_at, started_at, finished_at, error_code, error_message \
                 FROM jobs ORDER BY created_at, id LIMIT $1"
            }
        };
        let mut q = sqlx::query_as::<_, AdminJobRow>(sql);
        if let Some(c) = after {
            q = q.bind(c.k.clone()).bind(parse_id(c)?);
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: "admin.jobs.list".to_string(),
                    target_type: "job".to_string(),
                    target_id: "all".to_string(),
                    before_json: None,
                    after_json: None,
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(split_page(rows, limit, |r| {
            (r.created_at.to_rfc3339(), r.id.to_string())
        }))
    }

    /// Requeue a FAILED job (Owner-only, audited; runs as the app role with
    /// the GUC pinned to the job owner).
    pub async fn retry_job(
        &self,
        actor: &Actor,
        job_id: Uuid,
        correlation_id: &str,
    ) -> TenancyResult<AdminJobRow> {
        self.require_owner(
            actor,
            "admin.job.retry",
            ("job", &job_id.to_string()),
            correlation_id,
        )
        .await?;
        // Cross-user read to find the owner + verify state.
        let job: Option<AdminJobRow> = sqlx::query_as(
            "SELECT id, owner_user_id, job_type, status, priority, idempotency_key, \
                    attempt_count, created_at, started_at, finished_at, error_code, error_message \
             FROM jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(&self.admin_pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let job = crate::error::map_optional(job)?;
        if job.status != "FAILED" {
            return Err(TenancyError::InvalidState(format!(
                "only FAILED jobs can be retried (job is {})",
                job.status
            )));
        }
        // Requeue with the GUC pinned to the job owner (server-side, audited).
        let owner = job.owner_user_id.to_string();
        let pool = pool_for_actor(&self.app_url, &owner, 2).await?;
        let row = sqlx::query_as::<_, AdminJobRow>(
            "UPDATE jobs SET status = 'QUEUED', attempt_count = 0, error_code = NULL, \
                    error_message = NULL, available_at = now(), updated_at = now() \
             WHERE id = $1 AND status = 'FAILED' \
             RETURNING id, owner_user_id, job_type, status, priority, idempotency_key, \
                       attempt_count, created_at, started_at, finished_at, error_code, error_message",
        )
        .bind(job_id)
        .fetch_optional(&pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let row = crate::error::map_optional(row)?;
        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: "admin.job.retry".to_string(),
                    target_type: "job".to_string(),
                    target_id: job_id.to_string(),
                    before_json: Some(serde_json::json!({ "status": "FAILED" })),
                    after_json: Some(serde_json::json!({ "status": "QUEUED" })),
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(row)
    }

    /// Worker liveness derived from RUNNING job claims (`worker_heartbeats`
    /// has no serving grant by design; jobs carry the live claim heartbeat).
    pub async fn list_workers(
        &self,
        actor: &Actor,
        correlation_id: &str,
    ) -> TenancyResult<Vec<WorkerRow>> {
        self.require_owner(
            actor,
            "admin.workers.list",
            ("worker", "all"),
            correlation_id,
        )
        .await?;
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        let rows = sqlx::query_as::<_, WorkerRow>(
            "SELECT locked_by AS worker_id, max(locked_at) AS last_heartbeat_at, \
                    count(*) AS active_job_count \
             FROM jobs WHERE status = 'RUNNING' AND locked_by IS NOT NULL \
             GROUP BY locked_by ORDER BY locked_by",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: "admin.workers.list".to_string(),
                    target_type: "worker".to_string(),
                    target_id: "all".to_string(),
                    before_json: None,
                    after_json: None,
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(rows)
    }

    /// Cross-user audit trail (paginated), Owner-only.
    pub async fn list_audit_logs(
        &self,
        actor: &Actor,
        after: Option<&Cursor>,
        limit: usize,
        correlation_id: &str,
    ) -> TenancyResult<(Vec<AuditLogRow>, Option<Cursor>)> {
        self.require_owner(actor, "admin.audit.list", ("audit", "all"), correlation_id)
            .await?;
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        let sql = match after {
            Some(_) => {
                "SELECT id, action, actor_role, actor_user_id, target_type, target_id, \
                        reason, correlation_id, created_at \
                 FROM audit_logs WHERE (created_at, id) > ($1::timestamptz, $2::uuid) \
                 ORDER BY created_at, id LIMIT $3"
            }
            None => {
                "SELECT id, action, actor_role, actor_user_id, target_type, target_id, \
                        reason, correlation_id, created_at \
                 FROM audit_logs ORDER BY created_at, id LIMIT $1"
            }
        };
        let mut q = sqlx::query_as::<_, AuditLogRow>(sql);
        if let Some(c) = after {
            q = q.bind(c.k.clone()).bind(parse_id(c)?);
        }
        let rows = q
            .bind(limit as i64 + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        self.audit
            .record(
                actor,
                &AuditEntry {
                    action: "admin.audit.list".to_string(),
                    target_type: "audit".to_string(),
                    target_id: "all".to_string(),
                    before_json: None,
                    after_json: None,
                    reason: None,
                    correlation_id: Some(correlation_id.to_string()),
                },
            )
            .await?;
        Ok(split_page(rows, limit, |r| {
            (r.created_at.to_rfc3339(), r.id.to_string())
        }))
    }

    /// Count a tenant's QUEUED+RUNNING jobs (capacity gate; actor-scoped).
    pub async fn count_active_jobs(&self, actor: &Actor) -> TenancyResult<i64> {
        let mut tx = begin_actor_tx(&self.admin_pool, actor).await?;
        // The admin role sees all jobs; filter to the actor's own.
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs \
             WHERE owner_user_id = $1 AND status IN ('QUEUED', 'RUNNING')",
        )
        .bind(actor_uuid(actor)?)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(count)
    }
}

fn parse_id(c: &Cursor) -> TenancyResult<Uuid> {
    Uuid::parse_str(&c.i).map_err(|_| TenancyError::NotFound)
}
