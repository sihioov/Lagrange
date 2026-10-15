//! Latest-only daily reservations through the fixed 0060 SQL authority.
//! No caller-selected Owner/date/pins and no provider or artifact access.

use chrono::NaiveDate;
use domain::ContentHash;
use serde_json::Value;
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

use super::daily_runtime::{
    DailyCycleSession, DailyProcessMode, DailyRuntimeError, DailyStage, database_error,
};
use crate::{owner_beta::OwnerBetaStrategySnapshot, resolver::ResolvedConfig};

// Resource bound only: overflow fails the whole discovery, never silently
// truncates the Owner's eligible set or chooses a different session.
const MAX_CANDIDATES_PER_CYCLE: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyScheduleOutcome {
    Disabled,
    Busy,
    NotDue,
    Completed(DailyScheduleReport),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DailyScheduleReport {
    pub checked: usize,
    pub reserved: usize,
    pub replayed: usize,
    pub deferred: usize,
    pub blocked: usize,
}

#[derive(sqlx::FromRow)]
struct Candidate {
    strategy_config_id: Uuid,
    expected_session: NaiveDate,
    admission_sha256: String,
    config_epoch: i64,
    strategy_id: String,
    strategy_version: String,
    config_json: Value,
}

#[derive(sqlx::FromRow)]
struct Reservation {
    run_id: Option<Uuid>,
    job_id: Option<Uuid>,
    disposition: String,
}

pub async fn schedule_daily_once(
    pool: &PgPool,
    mode: DailyProcessMode,
) -> Result<DailyScheduleOutcome, DailyRuntimeError> {
    if mode == DailyProcessMode::Disabled {
        return Ok(DailyScheduleOutcome::Disabled);
    }
    let Some(mut session) = DailyCycleSession::acquire(pool, DailyStage::Schedule).await? else {
        return Ok(DailyScheduleOutcome::Busy);
    };
    let result = schedule_on(session.connection()).await;
    let released = session.release().await;
    match result {
        Err(error) => Err(error),
        Ok(outcome) => released.map(|()| outcome),
    }
}

async fn schedule_on(
    connection: &mut PgConnection,
) -> Result<DailyScheduleOutcome, DailyRuntimeError> {
    // Autocommit before discovery: restart/error cannot roll back the minute gate.
    let receipt: Option<Uuid> = sqlx::query_scalar(
        "SELECT receipt_id FROM public.take_owner_beta_daily_schedule_poll_slot()",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(database_error)?;
    let Some(receipt) = receipt else {
        return Ok(DailyScheduleOutcome::NotDue);
    };
    let work = reserve_candidates(connection).await;
    // Failure completes BLOCKED where authority still allows it. A broken
    // connection or revoked control can leave only a started receipt, not CHECKED.
    let code = match &work {
        Ok(report) if report.blocked > 0 => "BLOCKED",
        Ok(report) if report.deferred > 0 => "DEFERRED",
        Ok(_) => "CHECKED",
        Err(_) => "BLOCKED",
    };
    let completed = finish_schedule(connection, receipt, code).await;
    match work {
        Err(error) => Err(error),
        Ok(report) => completed.map(|()| DailyScheduleOutcome::Completed(report)),
    }
}

async fn reserve_candidates(
    connection: &mut PgConnection,
) -> Result<DailyScheduleReport, DailyRuntimeError> {
    let candidates: Vec<Candidate> = sqlx::query_as(
        "SELECT strategy_config_id, expected_session, admission_sha256, config_epoch, \
                strategy_id, strategy_version, config_json \
         FROM public.owner_beta_daily_schedule_candidates() \
         ORDER BY strategy_config_id LIMIT 1025",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?;
    if candidates.len() > MAX_CANDIDATES_PER_CYCLE {
        return Err(DailyRuntimeError::ResourceLimit);
    }
    let mut report = DailyScheduleReport::default();
    for candidate in candidates {
        if candidate.config_epoch < 0
            || !ContentHash::parse(&candidate.admission_sha256)
                .is_ok_and(|hash| hash.as_str() == candidate.admission_sha256)
        {
            return Err(DailyRuntimeError::InvalidInput);
        }
        let strategy = OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
            strategy_id: candidate.strategy_id,
            strategy_version: candidate.strategy_version,
            config: candidate.config_json,
        })
        .map_err(|_| DailyRuntimeError::InvalidInput)?;
        let strategy =
            serde_json::to_value(strategy).map_err(|_| DailyRuntimeError::InvalidInput)?;
        let mut transaction = connection.begin().await.map_err(database_error)?;
        let reservation: Reservation = sqlx::query_as(
            "SELECT run_id, job_id, disposition \
             FROM public.reserve_owner_beta_daily_run($1, $2, $3, $4, $5)",
        )
        .bind(candidate.strategy_config_id)
        .bind(candidate.expected_session)
        .bind(candidate.admission_sha256)
        .bind(candidate.config_epoch)
        .bind(strategy)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        record_reservation(&mut report, &reservation)?;
        // A lost commit reply never causes an in-cycle duplicate attempt. The
        // permanent unique reservation makes the next permitted cycle converge.
        transaction
            .commit()
            .await
            .map_err(|_| DailyRuntimeError::CommitUnknown)?;
    }
    Ok(report)
}

fn record_reservation(
    report: &mut DailyScheduleReport,
    result: &Reservation,
) -> Result<(), DailyRuntimeError> {
    match result.disposition.as_str() {
        "RESERVED" if result.run_id.is_some() && result.job_id.is_some() => report.reserved += 1,
        "REPLAY" if result.run_id.is_some() && result.job_id.is_some() => report.replayed += 1,
        "DEFERRED" if result.run_id.is_none() && result.job_id.is_none() => report.deferred += 1,
        "BLOCKED" if result.run_id.is_none() && result.job_id.is_none() => report.blocked += 1,
        _ => return Err(DailyRuntimeError::DatabaseIntegrity),
    }
    report.checked += 1;
    Ok(())
}

async fn finish_schedule(
    connection: &mut PgConnection,
    receipt: Uuid,
    code: &'static str,
) -> Result<(), DailyRuntimeError> {
    let mut transaction = connection.begin().await.map_err(database_error)?;
    let outcome: String =
        sqlx::query_scalar("SELECT public.finish_owner_beta_daily_schedule_poll($1, $2)")
            .bind(receipt)
            .bind(code)
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)?;
    if !matches!(outcome.as_str(), "COMPLETED" | "REPLAY") {
        return Err(DailyRuntimeError::DatabaseIntegrity);
    }
    transaction
        .commit()
        .await
        .map_err(|_| DailyRuntimeError::CommitUnknown)
}
