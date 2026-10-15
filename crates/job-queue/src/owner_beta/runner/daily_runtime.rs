//! Local-only daily cycle coordination. Process mode is an upper bound; SQL
//! remains the current Owner, entitlement, policy and activation authority.

use sqlx::{PgConnection, PgPool, Postgres, pool::PoolConnection};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DailyProcessMode {
    #[default]
    Disabled,
    OwnerOnly,
}

impl DailyProcessMode {
    pub fn parse(value: Option<&str>) -> Result<Self, DailyRuntimeError> {
        match value {
            None | Some("disabled") => Ok(Self::Disabled),
            Some("owner_only") => Ok(Self::OwnerOnly),
            _ => Err(DailyRuntimeError::InvalidConfiguration),
        }
    }
}

/// Value-free errors: database messages, payloads and paths are never retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DailyRuntimeError {
    #[error("daily runtime configuration is invalid")]
    InvalidConfiguration,
    #[error("daily runtime database is unavailable")]
    DatabaseUnavailable,
    #[error("daily runtime database contract is invalid")]
    DatabaseIntegrity,
    #[error("daily runtime role is invalid")]
    InvalidRole,
    #[error("daily runtime input is invalid")]
    InvalidInput,
    #[error("daily runtime resource bound exceeded")]
    ResourceLimit,
    #[error("daily runtime commit outcome is unknown")]
    CommitUnknown,
}

pub(super) fn database_error(error: sqlx::Error) -> DailyRuntimeError {
    if crate::error::database_error_class(&error) == crate::ErrorClass::Transient {
        DailyRuntimeError::DatabaseUnavailable
    } else {
        DailyRuntimeError::DatabaseIntegrity
    }
}

#[derive(Clone, Copy)]
pub(super) enum DailyStage {
    Admission,
    Schedule,
}
impl DailyStage {
    fn lock_key(self) -> &'static str {
        match self {
            Self::Admission => "daily-cycle:ADMISSION",
            Self::Schedule => "daily-cycle:SCHEDULE",
        }
    }
    fn role(self) -> &'static str {
        match self {
            Self::Admission => "daily_input_admitter",
            Self::Schedule => "worker",
        }
    }
}

/// A dedicated session, never returned to the pool with an advisory lock.
/// Cancellation/panic closes it as well. The persistent SQL minute gate is a
/// separate autocommit operation; this lock excludes cycles lasting over a minute.
pub(super) struct DailyCycleSession {
    connection: PoolConnection<Postgres>,
    stage: DailyStage,
}
impl DailyCycleSession {
    pub(super) async fn acquire(
        pool: &PgPool,
        stage: DailyStage,
    ) -> Result<Option<Self>, DailyRuntimeError> {
        let mut connection = pool.acquire().await.map_err(database_error)?;
        // Set before the first query, including an interrupted lock acquisition.
        connection.close_on_drop();
        let valid_role: bool = sqlx::query_scalar(
            "SELECT session_user::text = $1 AND current_user::text = $1 \
                    AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole \
                    AND NOT r.rolreplication AND NOT r.rolbypassrls \
             FROM pg_catalog.pg_roles r WHERE r.rolname = current_user",
        )
        .bind(stage.role())
        .fetch_one(&mut *connection)
        .await
        .map_err(database_error)?;
        if !valid_role {
            return Err(DailyRuntimeError::InvalidRole);
        }
        let locked: bool = sqlx::query_scalar(
            "SELECT pg_catalog.pg_try_advisory_lock(pg_catalog.hashtextextended($1, 7922))",
        )
        .bind(stage.lock_key())
        .fetch_one(&mut *connection)
        .await
        .map_err(database_error)?;
        if !locked {
            connection.close().await.map_err(database_error)?;
            return Ok(None);
        }
        Ok(Some(Self { connection, stage }))
    }

    pub(super) fn connection(&mut self) -> &mut PgConnection {
        &mut self.connection
    }

    pub(super) async fn release(mut self) -> Result<(), DailyRuntimeError> {
        let unlocked = sqlx::query_scalar::<_, bool>(
            "SELECT pg_catalog.pg_advisory_unlock(pg_catalog.hashtextextended($1, 7922))",
        )
        .bind(self.stage.lock_key())
        .fetch_one(&mut *self.connection)
        .await;
        // Explicit close even when unlock fails. close_on_drop covers cancellation.
        let closed = self.connection.close().await;
        if !unlocked.map_err(database_error)? {
            return Err(DailyRuntimeError::DatabaseIntegrity);
        }
        closed.map_err(database_error)
    }
}
