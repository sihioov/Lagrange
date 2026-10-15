//! The independent admitter's SQL boundary. Only verified calendar projections
//! and reopened sealed file capabilities enter these methods. Wire decoding or
//! producer output alone grants no authority here.

use domain::ContentHash;
use market_data::daily_price_only::{
    calendar::VerifiedCalendarProjection, continuation::SealedDailyParent,
    frontier::SealedSourceFrontier, observation::DailyBlockCode,
};
use serde_json::{Value, json};
use sqlx::{Connection, PgPool};
use uuid::Uuid;

use super::daily_runtime::{
    DailyCycleSession, DailyProcessMode, DailyRuntimeError, DailyStage, database_error,
};

pub enum DailyAdmissionStart {
    Disabled,
    Busy,
    NotDue,
    Started(DailyAdmissionCycle),
}

/// Fields are private and have no wire constructor. Drop closes the dedicated
/// session, releasing its lock without returning a locked connection to the pool.
pub struct DailyAdmissionCycle {
    session: DailyCycleSession,
    receipt: Uuid,
    calendar_binding: Option<(Uuid, ContentHash)>,
    completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarProjectionOutcome {
    Projected,
    Replay,
    Blocked(DailyBlockCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyAdmissionOutcome {
    Admitted,
    Replay,
    Blocked(DailyBlockCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPollFailure {
    InputPending,
    InputBlocked,
    IoFailed,
}
impl AdmissionPollFailure {
    fn code(self) -> &'static str {
        match self {
            Self::InputPending => "INPUT_PENDING",
            Self::InputBlocked => "INPUT_BLOCKED",
            Self::IoFailed => "IO_FAILED",
        }
    }
}

#[derive(sqlx::FromRow)]
struct SqlOutcome {
    disposition: String,
    identity: Option<String>,
    reason_code: Option<String>,
    event_sha256: Option<String>,
}

fn canonical_hash(value: &str) -> bool {
    ContentHash::parse(value).is_ok_and(|hash| hash.as_str() == value)
}

fn block_code(result: &SqlOutcome) -> Result<DailyBlockCode, DailyRuntimeError> {
    if result.disposition != "BLOCKED"
        || result.identity.is_some()
        || !result.event_sha256.as_deref().is_some_and(canonical_hash)
    {
        return Err(DailyRuntimeError::DatabaseIntegrity);
    }
    match result.reason_code.as_deref() {
        Some("SOURCE_CONFLICT") => Ok(DailyBlockCode::SourceConflict),
        Some("SEALED_SOURCE_CHANGED") => Ok(DailyBlockCode::SealedSourceChanged),
        Some("SOURCE_REVOKED") => Ok(DailyBlockCode::SourceRevoked),
        Some("SOURCE_INTEGRITY") => Ok(DailyBlockCode::SourceIntegrity),
        Some("CALENDAR_CONFLICT") => Ok(DailyBlockCode::CalendarConflict),
        Some("NONCASH_ACTION") => Ok(DailyBlockCode::NoncashAction),
        _ => Err(DailyRuntimeError::DatabaseIntegrity),
    }
}

fn success_identity(result: &SqlOutcome, expected: &ContentHash) -> bool {
    result.identity.as_deref() == Some(expected.as_str())
        && result.reason_code.is_none()
        && result.event_sha256.is_none()
}

impl DailyAdmissionCycle {
    pub async fn start(
        pool: &PgPool,
        mode: DailyProcessMode,
    ) -> Result<DailyAdmissionStart, DailyRuntimeError> {
        if mode == DailyProcessMode::Disabled {
            return Ok(DailyAdmissionStart::Disabled);
        }
        let Some(mut session) = DailyCycleSession::acquire(pool, DailyStage::Admission).await?
        else {
            return Ok(DailyAdmissionStart::Busy);
        };
        // Commit this gate before any filesystem scan. An incomplete cycle is
        // not VERIFIED, but still consumes its DB-clock minute slot.
        let receipt: Option<Uuid> = sqlx::query_scalar(
            "SELECT receipt_id FROM public.take_owner_beta_daily_admission_poll_slot()",
        )
        .fetch_optional(session.connection())
        .await
        .map_err(database_error)?;
        let Some(receipt) = receipt else {
            session.release().await?;
            return Ok(DailyAdmissionStart::NotDue);
        };
        Ok(DailyAdmissionStart::Started(Self {
            session,
            receipt,
            calendar_binding: None,
            completed: false,
        }))
    }

    pub fn receipt_id(&self) -> Uuid {
        self.receipt
    }

    pub async fn project_calendar(
        &mut self,
        projection: &VerifiedCalendarProjection<'_>,
    ) -> Result<CalendarProjectionOutcome, DailyRuntimeError> {
        if self.completed
            || self
                .calendar_binding
                .as_ref()
                .is_some_and(|(owner, policy)| {
                    *owner != projection.owner() || policy != projection.policy_sha256()
                })
        {
            return Err(DailyRuntimeError::InvalidInput);
        }
        let file = serde_json::to_value(projection.source_file())
            .map_err(|_| DailyRuntimeError::InvalidInput)?;
        let days =
            serde_json::to_value(projection.days()).map_err(|_| DailyRuntimeError::InvalidInput)?;
        let mut transaction = self
            .session
            .connection()
            .begin()
            .await
            .map_err(database_error)?;
        let result: SqlOutcome = sqlx::query_as(
            "SELECT disposition, calendar_evidence_sha256 AS identity, reason_code, event_sha256 \
             FROM public.project_owner_beta_daily_calendar($1, $2, $3, $4)",
        )
        .bind(self.receipt)
        .bind(projection.evidence_sha256().as_str())
        .bind(file)
        .bind(days)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let outcome = match result.disposition.as_str() {
            "PROJECTED" if success_identity(&result, projection.evidence_sha256()) => {
                CalendarProjectionOutcome::Projected
            }
            "REPLAY" if success_identity(&result, projection.evidence_sha256()) => {
                CalendarProjectionOutcome::Replay
            }
            "BLOCKED" => CalendarProjectionOutcome::Blocked(block_code(&result)?),
            _ => return Err(DailyRuntimeError::DatabaseIntegrity),
        };
        transaction
            .commit()
            .await
            .map_err(|_| DailyRuntimeError::CommitUnknown)?;
        self.calendar_binding = Some((projection.owner(), projection.policy_sha256().clone()));
        self.completed = matches!(outcome, CalendarProjectionOutcome::Blocked(_));
        Ok(outcome)
    }

    /// Calendar facts precede admission. BLOCKED is committed just like ADMITTED
    /// and REPLAY, including its source event and completion receipt.
    pub async fn admit(
        mut self,
        input: &SealedDailyParent<'_, '_>,
        frontier: &SealedSourceFrontier,
    ) -> Result<DailyAdmissionOutcome, DailyRuntimeError> {
        if self.completed
            || self
                .calendar_binding
                .as_ref()
                .is_none_or(|(owner, policy)| {
                    *owner != frontier.owner() || policy != frontier.policy_sha256()
                })
            || input.pins().admission_sha256 != *frontier.admission_sha256()
            || input.pins().policy_sha256 != *frontier.policy_sha256()
            || input.manifest().range_end != frontier.range_end()
        {
            return Err(DailyRuntimeError::InvalidInput);
        }
        input
            .revalidate()
            .map_err(|_| DailyRuntimeError::InvalidInput)?;
        frontier
            .revalidate()
            .map_err(|_| DailyRuntimeError::InvalidInput)?;
        let admission = match input {
            SealedDailyParent::First(value) => value.admission(),
            SealedDailyParent::Continued(value) => value.admission(),
        };
        let transport: Value = json!({
            "admission_sha256": input.pins().admission_sha256,
            "source_frontier_sha256": frontier.sha256(),
            "admission": admission,
            "manifest": input.manifest(),
        });
        let mut transaction = self
            .session
            .connection()
            .begin()
            .await
            .map_err(database_error)?;
        let result: SqlOutcome = sqlx::query_as(
            "SELECT disposition, admission_sha256 AS identity, reason_code, event_sha256 \
             FROM public.admit_owner_beta_daily_input($1, $2)",
        )
        .bind(self.receipt)
        .bind(transport)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let outcome = match result.disposition.as_str() {
            "ADMITTED" if success_identity(&result, &input.pins().admission_sha256) => {
                DailyAdmissionOutcome::Admitted
            }
            "REPLAY" if success_identity(&result, &input.pins().admission_sha256) => {
                DailyAdmissionOutcome::Replay
            }
            "BLOCKED" => DailyAdmissionOutcome::Blocked(block_code(&result)?),
            _ => return Err(DailyRuntimeError::DatabaseIntegrity),
        };
        input
            .revalidate()
            .map_err(|_| DailyRuntimeError::InvalidInput)?;
        frontier
            .revalidate()
            .map_err(|_| DailyRuntimeError::InvalidInput)?;
        transaction
            .commit()
            .await
            .map_err(|_| DailyRuntimeError::CommitUnknown)?;
        self.session.release().await?;
        Ok(outcome)
    }

    /// Unattributable IO/parse failures have no invented evidence hash or event.
    pub async fn finish_failure(
        mut self,
        failure: AdmissionPollFailure,
    ) -> Result<(), DailyRuntimeError> {
        if self.completed {
            return Err(DailyRuntimeError::InvalidInput);
        }
        let mut transaction = self
            .session
            .connection()
            .begin()
            .await
            .map_err(database_error)?;
        let result: String = sqlx::query_scalar(
            "SELECT public.finish_owner_beta_daily_admission_poll($1, $2, NULL)",
        )
        .bind(self.receipt)
        .bind(failure.code())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if !matches!(result.as_str(), "COMPLETED" | "REPLAY") {
            return Err(DailyRuntimeError::DatabaseIntegrity);
        }
        transaction
            .commit()
            .await
            .map_err(|_| DailyRuntimeError::CommitUnknown)?;
        self.session.release().await
    }

    /// End a cycle whose calendar operation already committed INPUT_BLOCKED.
    pub async fn finish_blocked(self) -> Result<(), DailyRuntimeError> {
        if !self.completed {
            return Err(DailyRuntimeError::InvalidInput);
        }
        self.session.release().await
    }
}
