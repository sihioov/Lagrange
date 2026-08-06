//! Typed tenancy errors: the repository layer never leaks row internals of a
//! foreign actor.
//!
//! Mapping rules:
//! - a SELECT/UPDATE/DELETE that Row-Level Security filters to zero rows
//!   surfaces as [`TenancyError::NotFound`] (404-class: the resource either
//!   does not exist or is not owned — indistinguishable, by design);
//! - a policy denial (`SQLSTATE 42501`, e.g. a crafted `owner_user_id` on
//!   INSERT) surfaces as [`TenancyError::Forbidden`] (403-class);
//! - everything else stays a typed [`TenancyError::Database`] passthrough.

use sqlx::postgres::PgDatabaseError;

/// Result alias for the tenancy repositories.
pub type TenancyResult<T> = Result<T, TenancyError>;

/// Errors produced by the tenancy repositories.
#[derive(Debug, thiserror::Error)]
pub enum TenancyError {
    /// The row does not exist for this actor (missing, or not owned).
    #[error("resource not found")]
    NotFound,
    /// The actor is not permitted to perform this operation.
    #[error("forbidden")]
    Forbidden,
    /// The operation is not implemented yet (red-phase stub).
    #[error("not implemented")]
    NotImplemented,
    /// Database-level failure (connection, constraint, or other SQLSTATE).
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

impl TenancyError {
    /// Classify a `sqlx` error against the RLS policy matrix.
    pub fn from_sqlx(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e
            && db.code().as_deref() == Some("42501")
        {
            return Self::Forbidden;
        }
        if matches!(e, sqlx::Error::RowNotFound) {
            return Self::NotFound;
        }
        Self::Database(e)
    }

    /// True when the underlying database error carries the given SQLSTATE.
    pub fn sqlstate(&self, code: &str) -> bool {
        match self {
            Self::Database(sqlx::Error::Database(db)) => db.code().as_deref() == Some(code),
            _ => false,
        }
    }

    /// For denial transcripts: the SQLSTATE of a database-class error.
    pub fn database_sqlstate(&self) -> Option<String> {
        match self {
            Self::Database(sqlx::Error::Database(db)) => db.code().map(|c| c.into_owned()),
            _ => None,
        }
    }
}

/// Extract a `PgDatabaseError` reference for denial transcript capture.
pub fn as_pg_error(e: &sqlx::Error) -> Option<&PgDatabaseError> {
    match e {
        sqlx::Error::Database(db) => Some(db.downcast_ref::<PgDatabaseError>()),
        _ => None,
    }
}

/// Map the result of an UPDATE/DELETE that `RETURNING`s the touched rows:
/// zero rows (RLS-filtered or genuinely absent) is [`TenancyError::NotFound`].
pub(crate) fn map_optional<T>(row: Option<T>) -> TenancyResult<T> {
    row.ok_or(TenancyError::NotFound)
}
