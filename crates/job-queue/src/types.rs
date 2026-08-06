//! Public row types, lifecycle enums, and result enums of the job queue.
//!
//! Statuses mirror the frozen T3 schema exactly: `jobs.status` has FIVE
//! public states (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELED) — `ORPHANED` is
//! an attempt-level outcome only, never a job status (NFR-REL-002/003).
//! Attempts are immutable records: a row is inserted `RUNNING` at claim time
//! and written exactly once more to a terminal outcome.

use chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use std::fmt;
use std::str::FromStr;

/// The five public job lifecycle states (design §6.8, NFR-REL-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl JobStatus {
    pub const ALL: [JobStatus; 5] = [
        JobStatus::Queued,
        JobStatus::Running,
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Canceled,
    ];

    /// Uppercase DB representation (`jobs.status` CHECK constraint values).
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "QUEUED",
            JobStatus::Running => "RUNNING",
            JobStatus::Succeeded => "SUCCEEDED",
            JobStatus::Failed => "FAILED",
            JobStatus::Canceled => "CANCELED",
        }
    }

    pub fn parse(s: &str) -> Option<JobStatus> {
        match s {
            "QUEUED" => Some(JobStatus::Queued),
            "RUNNING" => Some(JobStatus::Running),
            "SUCCEEDED" => Some(JobStatus::Succeeded),
            "FAILED" => Some(JobStatus::Failed),
            "CANCELED" => Some(JobStatus::Canceled),
            _ => None,
        }
    }

    /// Terminal states: no further transitions are ever legal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStatus::Succeeded | JobStatus::Failed | JobStatus::Canceled
        )
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for JobStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        JobStatus::parse(s).ok_or_else(|| format!("unknown job status {s:?}"))
    }
}

/// Attempt-level outcome. `ORPHANED` exists HERE and never on `jobs.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AttemptOutcome {
    Running,
    Succeeded,
    Failed,
    Orphaned,
}

impl AttemptOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AttemptOutcome::Running => "RUNNING",
            AttemptOutcome::Succeeded => "SUCCEEDED",
            AttemptOutcome::Failed => "FAILED",
            AttemptOutcome::Orphaned => "ORPHANED",
        }
    }

    pub fn parse(s: &str) -> Option<AttemptOutcome> {
        match s {
            "RUNNING" => Some(AttemptOutcome::Running),
            "SUCCEEDED" => Some(AttemptOutcome::Succeeded),
            "FAILED" => Some(AttemptOutcome::Failed),
            "ORPHANED" => Some(AttemptOutcome::Orphaned),
            _ => None,
        }
    }
}

impl fmt::Display for AttemptOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AttemptOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AttemptOutcome::parse(s).ok_or_else(|| format!("unknown attempt outcome {s:?}"))
    }
}

/// Failure classification at settle time (design §6.8 retry policy).
///
/// Only [`ErrorClass::Transient`] ever retries (exponential backoff). Input
/// errors, blocked data, integrity violations, and engine determinism
/// violations are non-retryable by contract: they fail the job immediately,
/// no matter how many attempts remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// Invalid input (bad parameters, impossible dates...): never retry.
    Input,
    /// Data blocked / missing / stale (entitlement, quality policy): never retry.
    DataBlocked,
    /// Integrity violation (ledger mismatch, NaN, hash mismatch): never retry.
    Integrity,
    /// Transient file/DB/network error: retry with exponential backoff.
    Transient,
    /// Engine determinism violation or result-validation failure: no auto retry.
    Determinism,
}

impl ErrorClass {
    /// `true` only for transient errors (design §6.8: 입력 오류·데이터 차단,
    /// 엔진 결정론 위반·결과 검증 실패 → 재시도 없음; 일시적 오류 → 지수 백오프 1회).
    pub fn retryable(self) -> bool {
        matches!(self, ErrorClass::Transient)
    }
}

/// A `jobs` row (all 18 columns, status decoded to [`JobStatus`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub job_type: String,
    pub status: JobStatus,
    pub priority: i32,
    pub idempotency_key: Option<String>,
    pub payload_json: serde_json::Value,
    pub max_attempts: i32,
    pub attempt_count: i32,
    pub available_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A `job_attempts` row — an immutable per-attempt record.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct JobAttempt {
    pub id: Uuid,
    pub job_id: Uuid,
    pub attempt_no: i32,
    pub outcome: AttemptOutcome,
    pub claimed_by: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A claimed job plus its fresh `RUNNING` attempt and the current lease bound.
///
/// `lease_expires_at = jobs.locked_at + configured lease`; the worker must
/// heartbeat before that instant or the sweeper may orphan the attempt.
#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub job: Job,
    pub attempt: JobAttempt,
    pub lease_expires_at: DateTime<Utc>,
    /// Worker id that holds this claim (mirrors `attempt.claimed_by`).
    pub worker_id: String,
}

/// Input of [`crate::JobQueue::submit`]. Validated before any SQL runs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubmitJob {
    pub owner_user_id: Uuid,
    /// e.g. `backtest` | `paper` | `report`; `[a-z0-9_-]{1,64}`.
    pub job_type: String,
    /// Arbitrary JSON object carried to the worker.
    pub payload: serde_json::Value,
    /// Higher claims first (claim index `priority DESC, created_at`).
    pub priority: i32,
    /// Per-owner deduplication: a duplicate key returns the SAME job
    /// (FR-BT-008 / AT-03), never a second row.
    pub idempotency_key: Option<String>,
    /// Total claims allowed (>= 1; orphan/retry exhaustion caps here).
    pub max_attempts: i32,
    /// Earliest claim instant (default: now).
    pub available_at: Option<DateTime<Utc>>,
}

/// Outcome of a settle call.
#[derive(Debug, Clone)]
pub enum SettleResult {
    /// The settle committed; inspect the returned job for the new state
    /// (SUCCEEDED, requeued QUEUED, or FAILED).
    Committed(Job),
    /// The job was canceled while this worker was working; its attempt was
    /// recorded `FAILED(error_code='canceled')` and the job stays CANCELED.
    Canceled(Job),
}

/// Outcome of [`crate::JobQueue::request_cancel`].
#[derive(Debug, Clone)]
pub enum CancelResult {
    /// The cancel transitioned the job QUEUED/RUNNING -> CANCELED (audited).
    Canceled(Job),
    /// The job was already terminal; nothing changed.
    AlreadyTerminal(Job),
}

/// Outcome of [`crate::JobQueue::heartbeat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatStatus {
    /// Lease anchor advanced; the claim is safe for another lease period.
    Extended,
    /// The job was canceled; the worker should abort at the next checkpoint.
    Canceled,
    /// The lease had already expired (no heartbeat in time); the sweeper now
    /// owns the attempt — stop working.
    LeaseLost,
}

/// Who requested an audited action (cancel). The queue records it verbatim
/// in `audit_logs` (actor_role/actor_user_id/correlation_id).
#[derive(Debug, Clone)]
pub struct AuditActor {
    pub role: String,
    pub user_id: Option<Uuid>,
    pub correlation_id: Option<String>,
}

impl AuditActor {
    pub fn new(role: impl Into<String>) -> AuditActor {
        AuditActor {
            role: role.into(),
            user_id: None,
            correlation_id: None,
        }
    }
}

/// Result of [`crate::JobQueue::sweep`]: one pass over expired leases.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SweepReport {
    /// Expired-lease jobs examined.
    pub jobs_checked: usize,
    /// Attempts marked `ORPHANED` (attempt-level outcome only).
    pub attempts_orphaned: usize,
    /// Jobs requeued as QUEUED after an orphan (at most one per pass per job).
    pub jobs_requeued: usize,
    /// Jobs resolved FAILED after retry exhaustion (worker-crash retries
    /// exhausted). Canceled jobs whose orphaned attempt was finalized are
    /// counted in `attempts_orphaned` only — they are never requeued or
    /// re-resolved.
    pub jobs_failed: usize,
}

// ---------------------------------------------------------------------------
// sqlx Type/Decode for the enum columns (both are TEXT-backed CHECK columns).
// ---------------------------------------------------------------------------

macro_rules! impl_text_enum {
    ($ty:ty, $parse:expr) => {
        impl sqlx::Type<sqlx::Postgres> for $ty {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <String as sqlx::Type<sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <String as sqlx::Type<sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $ty {
            fn decode(
                value: sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                let s = <&str as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
                ($parse)(s).ok_or_else(|| format!("unknown enum value {s:?}").into())
            }
        }
    };
}

impl_text_enum!(JobStatus, JobStatus::parse);
impl_text_enum!(AttemptOutcome, AttemptOutcome::parse);
