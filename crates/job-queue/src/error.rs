//! Typed errors for the PostgreSQL job queue.
//!
//! Every failure mode is a typed variant; the queue never panics on
//! malformed input or database state and never retries on its own — retry
//! decisions belong to [`crate::types::ErrorClass`] at settle time.

use crate::types::JobStatus;
use sqlx::types::Uuid;
use thiserror::Error;

/// Errors produced by [`crate::JobQueue`] operations.
#[derive(Debug, Error)]
pub enum QueueError {
    /// A database statement failed (connection, constraint, or — for the
    /// production role matrix — a grant denial surfaced as SQLSTATE 42501).
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// No row matched the requested job id.
    #[error("job {0} not found")]
    JobNotFound(Uuid),

    /// The claim no longer exists: the lease expired and a sweeper requeued
    /// the job (or another actor settled it) before this worker settled.
    /// The worker must stop working and never touch the attempt again.
    #[error("job {0} is no longer owned by this worker (lease lost or already settled)")]
    StaleClaim(Uuid),

    /// The requested transition targets a job already in a terminal state
    /// (SUCCEEDED, FAILED, or CANCELED).
    #[error("job {0} is already in terminal state {1}")]
    AlreadyTerminal(Uuid, JobStatus),

    /// Submission was rejected before touching the database.
    #[error("invalid submission: {0}")]
    InvalidInput(String),

    /// Cancellation was requested without an audit-writer pool. Cancels are
    /// cooperative AND audited by contract; an un-auditable cancel is refused.
    #[error("cancellation requires an audit-writer pool (audited cancels are mandatory)")]
    AuditUnavailable,

    /// An invariant that should be impossible was observed. Never panics;
    /// surfaces as a typed error so callers can decide how to degrade.
    #[error("internal invariant broken: {0}")]
    Internal(String),
}
