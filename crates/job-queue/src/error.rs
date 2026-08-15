//! Typed errors for the PostgreSQL job queue.
//!
//! Every failure mode is a typed variant; the queue never panics on
//! malformed input or database state and never retries on its own — retry
//! decisions belong to [`crate::types::ErrorClass`] at settle time.

use crate::types::JobStatus;
use sqlx::types::Uuid;
use thiserror::Error;

use crate::types::ErrorClass;

/// Stable event/error code for an uncertain PostgreSQL COMMIT outcome.
pub const BACKTEST_COMMIT_UNKNOWN_CODE: &str = "BACKTEST_PUBLICATION_COMMIT_UNKNOWN";

/// Classify SQLx failures without inspecting display text. Database failures
/// are decided solely from SQLSTATE; client-side contract/decode failures are
/// permanent integrity errors.
pub(crate) fn database_error_class(error: &sqlx::Error) -> ErrorClass {
    match error {
        sqlx::Error::Database(error) => {
            let code = error.code();
            let code = code.as_deref().unwrap_or_default();
            if code.starts_with("08")
                || code.starts_with("53")
                || matches!(
                    code,
                    "40001" | "40P01" | "55P03" | "57014" | "57P01" | "57P02" | "57P03"
                )
            {
                ErrorClass::Transient
            } else {
                ErrorClass::Integrity
            }
        }
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => ErrorClass::Transient,
        _ => ErrorClass::Integrity,
    }
}

/// Classify queue operation failures for callers that own retry policy.
/// Database variants preserve the SQLSTATE-aware transport/contract split;
/// all queue contract failures are permanent integrity errors.
pub(crate) fn queue_error_class(error: &QueueError) -> ErrorClass {
    match error {
        QueueError::Database(error) => database_error_class(error),
        QueueError::CommitUnknown { .. } => ErrorClass::Transient,
        QueueError::JobNotFound(_)
        | QueueError::StaleClaim(_)
        | QueueError::AlreadyTerminal(_, _)
        | QueueError::InvalidInput(_)
        | QueueError::AuditUnavailable
        | QueueError::Internal(_) => ErrorClass::Integrity,
    }
}

/// Errors produced by [`crate::JobQueue`] operations.
#[derive(Debug, Error)]
pub enum QueueError {
    /// A database statement failed (connection, constraint, or — for the
    /// production role matrix — a grant denial surfaced as SQLSTATE 42501).
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// The publication transaction reached PostgreSQL's COMMIT boundary but
    /// the client could not learn whether it committed. The immutable
    /// generation is intentionally retained and must be reconciled against
    /// the authoritative DB before cleanup. This is separate from a normal
    /// database error so operators receive the run/job/path needed to repair
    /// or verify the result.
    #[error(
        "{BACKTEST_COMMIT_UNKNOWN_CODE}: run {run_id}, job {job_id}, generation {generation_path}: {detail}"
    )]
    CommitUnknown {
        run_id: Uuid,
        job_id: Uuid,
        generation_path: String,
        detail: String,
    },

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

impl QueueError {
    /// Stable machine-readable event code for operational logs.
    pub fn event_code(&self) -> &'static str {
        match self {
            Self::CommitUnknown { .. } => BACKTEST_COMMIT_UNKNOWN_CODE,
            Self::Database(_) => "JOB_QUEUE_DATABASE_ERROR",
            Self::JobNotFound(_) => "JOB_NOT_FOUND",
            Self::StaleClaim(_) => "JOB_STALE_CLAIM",
            Self::AlreadyTerminal(_, _) => "JOB_ALREADY_TERMINAL",
            Self::InvalidInput(_) => "JOB_INVALID_INPUT",
            Self::AuditUnavailable => "JOB_AUDIT_UNAVAILABLE",
            Self::Internal(_) => "JOB_QUEUE_INTERNAL",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorClass;
    use sqlx::error::{DatabaseError, ErrorKind};
    use std::borrow::Cow;
    use std::fmt;

    #[derive(Debug)]
    struct SqlState(&'static str);

    impl fmt::Display for SqlState {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl std::error::Error for SqlState {}

    impl DatabaseError for SqlState {
        fn message(&self) -> &str {
            self.0
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn database_error(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(SqlState(code)))
    }

    #[test]
    fn sqlx_transport_and_contract_errors_have_typed_retry_classes() {
        assert_eq!(
            database_error_class(&sqlx::Error::PoolTimedOut),
            ErrorClass::Transient
        );
        assert_eq!(
            database_error_class(&sqlx::Error::PoolClosed),
            ErrorClass::Transient
        );
        assert_eq!(
            database_error_class(&sqlx::Error::RowNotFound),
            ErrorClass::Integrity
        );
        assert_eq!(
            database_error_class(&sqlx::Error::ColumnNotFound("missing".into())),
            ErrorClass::Integrity
        );
        assert_eq!(
            database_error_class(&sqlx::Error::TypeNotFound {
                type_name: "missing".into(),
            }),
            ErrorClass::Integrity
        );
        for code in [
            "23505", "23503", "23514", "42501", "42P01", "42883", "3F000",
        ] {
            assert_eq!(
                database_error_class(&database_error(code)),
                ErrorClass::Integrity,
                "{code}"
            );
        }
        for code in [
            "08006", "40001", "40P01", "55P03", "57014", "57P01", "57P02", "57P03", "53200",
            "53300",
        ] {
            assert_eq!(
                database_error_class(&database_error(code)),
                ErrorClass::Transient,
                "{code}"
            );
        }
    }

    #[test]
    fn commit_unknown_has_a_stable_operational_code_and_transient_class() {
        let error = QueueError::CommitUnknown {
            run_id: Uuid::new_v4(),
            job_id: Uuid::new_v4(),
            generation_path: "/data/artifacts/generation".to_owned(),
            detail: "commit response lost".to_owned(),
        };
        assert_eq!(error.event_code(), BACKTEST_COMMIT_UNKNOWN_CODE);
        assert_eq!(queue_error_class(&error), ErrorClass::Transient);
        assert!(error.to_string().contains("generation"));
    }
}
