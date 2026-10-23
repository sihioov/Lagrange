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

    /// Commit an attributable source event produced by complete Raw revalidation.
    /// The returned SQL event hash must bind the same Owner/range/evidence tuple.
    /// Unattributable IO errors belong to finish_failure, never this method.
    pub async fn finish_source_event(
        mut self,
        event: &market_data::daily_price_only::observation::CanonicalSourceEvent,
    ) -> Result<DailyBlockCode, DailyRuntimeError> {
        if self.completed || event.code() == DailyBlockCode::SourceRevoked {
            return Err(DailyRuntimeError::InvalidInput);
        }
        let payload = json!({
            "source_kind": event.kind().code(),
            "range_start": event.range_start(),
            "range_end": event.range_end(),
            "code": event.code().code(),
            "evidence_sha256": event.observation_sha256(),
        });
        let mut transaction = self
            .session
            .connection()
            .begin()
            .await
            .map_err(database_error)?;
        let result: SqlOutcome = sqlx::query_as(
            "SELECT disposition, admission_sha256 AS identity, reason_code, event_sha256 \
             FROM public.block_owner_beta_daily_input($1, $2)",
        )
        .bind(self.receipt)
        .bind(payload)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let code = block_code(&result)?;
        if code != event.code() || result.event_sha256.as_deref() != Some(event.sha256().as_str()) {
            return Err(DailyRuntimeError::DatabaseIntegrity);
        }
        transaction
            .commit()
            .await
            .map_err(|_| DailyRuntimeError::CommitUnknown)?;
        self.session.release().await?;
        Ok(code)
    }

    /// End a cycle whose calendar operation already committed INPUT_BLOCKED.
    pub async fn finish_blocked(self) -> Result<(), DailyRuntimeError> {
        if !self.completed {
            return Err(DailyRuntimeError::InvalidInput);
        }
        self.session.release().await
    }
}

/// A current DB navigation snapshot for one still-pending poll. It is not a
/// filesystem capability, source refresh, reservation or publication authorization.
pub struct DailyAdmitterContext {
    owner: Uuid,
    policy: ContentHash,
    activation_epoch: i64,
    enabled: bool,
    entitlement_reference: String,
    entitlement_hash: ContentHash,
    first_auto_session: chrono::NaiveDate,
    observed_at: chrono::DateTime<chrono::Utc>,
    latest_admission: Option<ContentHash>,
    source_blocked: bool,
}
impl DailyAdmitterContext {
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy
    }
    pub fn activation_epoch(&self) -> i64 {
        self.activation_epoch
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn entitlement_reference(&self) -> &str {
        &self.entitlement_reference
    }
    pub fn entitlement_sha256(&self) -> &ContentHash {
        &self.entitlement_hash
    }
    pub fn first_auto_session(&self) -> chrono::NaiveDate {
        self.first_auto_session
    }
    pub fn observed_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.observed_at
    }
    pub fn latest_admission_sha256(&self) -> Option<&ContentHash> {
        self.latest_admission.as_ref()
    }
    pub fn source_blocked(&self) -> bool {
        self.source_blocked
    }
}

#[derive(sqlx::FromRow)]
struct ContextRow {
    owner_user_id: Uuid,
    policy_sha256: String,
    activation_epoch: i64,
    enabled: bool,
    entitlement_reference: String,
    entitlement_sha256: String,
    first_auto_session: chrono::NaiveDate,
    observed_at: chrono::DateTime<chrono::Utc>,
    latest_admission_sha256: Option<String>,
    source_blocked: bool,
}
fn readback_hash(value: String) -> Result<ContentHash, DailyRuntimeError> {
    let hash = ContentHash::parse(&value).map_err(|_| DailyRuntimeError::DatabaseIntegrity)?;
    if hash.as_str() != value {
        return Err(DailyRuntimeError::DatabaseIntegrity);
    }
    Ok(hash)
}
impl DailyAdmissionCycle {
    pub async fn context(&mut self) -> Result<DailyAdmitterContext, DailyRuntimeError> {
        if self.completed {
            return Err(DailyRuntimeError::InvalidInput);
        }
        let row: ContextRow = sqlx::query_as(
            "SELECT owner_user_id,policy_sha256,activation_epoch,enabled,entitlement_reference,\
             entitlement_sha256,first_auto_session,observed_at,latest_admission_sha256,source_blocked \
             FROM public.read_owner_beta_daily_admission_context($1)")
            .bind(self.receipt).fetch_one(self.session.connection()).await.map_err(database_error)?;
        if row.owner_user_id.is_nil()
            || row.activation_epoch < 0
            || row.entitlement_reference.is_empty()
            || row.entitlement_reference.chars().count() > 512
            || row.entitlement_reference.chars().any(char::is_control)
            || row.first_auto_session
                <= chrono::NaiveDate::from_ymd_opt(2026, 8, 28).expect("fixed date")
        {
            return Err(DailyRuntimeError::DatabaseIntegrity);
        }
        Ok(DailyAdmitterContext {
            owner: row.owner_user_id,
            policy: readback_hash(row.policy_sha256)?,
            activation_epoch: row.activation_epoch,
            enabled: row.enabled,
            entitlement_reference: row.entitlement_reference,
            entitlement_hash: readback_hash(row.entitlement_sha256)?,
            first_auto_session: row.first_auto_session,
            observed_at: row.observed_at,
            latest_admission: row.latest_admission_sha256.map(readback_hash).transpose()?,
            source_blocked: row.source_blocked,
        })
    }
}

/// Exact committed SQL metadata. Reopen its files, compare every pin and refresh
/// sources independently before use; unblocked_at_read() excludes freshness and activation.
pub struct DailyCommittedInput {
    owner: Uuid,
    pins: market_data::daily_price_only::DailyPricePinsV1Wire,
    parent: Option<ContentHash>,
    frontier: ContentHash,
    range_start: chrono::NaiveDate,
    range_end: chrono::NaiveDate,
    session_count: u32,
    usable: bool,
}
impl DailyCommittedInput {
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn pins(&self) -> &market_data::daily_price_only::DailyPricePinsV1Wire {
        &self.pins
    }
    pub fn parent_admission_sha256(&self) -> Option<&ContentHash> {
        self.parent.as_ref()
    }
    pub fn source_frontier_sha256(&self) -> &ContentHash {
        &self.frontier
    }
    pub fn range_start(&self) -> chrono::NaiveDate {
        self.range_start
    }
    pub fn range_end(&self) -> chrono::NaiveDate {
        self.range_end
    }
    pub fn session_count(&self) -> u32 {
        self.session_count
    }
    pub fn unblocked_at_read(&self) -> bool {
        self.usable
    }
}
#[derive(sqlx::FromRow)]
struct CommittedInputRow {
    owner_user_id: Uuid,
    policy_sha256: String,
    content_sha256: String,
    manifest_sha256: String,
    admission_sha256: String,
    parent_admission_sha256: Option<String>,
    source_frontier_sha256: String,
    range_start: chrono::NaiveDate,
    range_end: chrono::NaiveDate,
    session_count: i32,
    bar_count: i32,
    usable: bool,
}

/// Read-only exact-hash reconciliation, including a lost admission commit reply.
/// Unknown/foreign rows return None. This grants no new current execution input.
pub async fn read_committed_daily_input(
    pool: &PgPool,
    expected: &ContentHash,
) -> Result<Option<DailyCommittedInput>, DailyRuntimeError> {
    let row: Option<CommittedInputRow> = sqlx::query_as(
        "SELECT owner_user_id,policy_sha256,content_sha256,manifest_sha256,admission_sha256,\
         parent_admission_sha256,source_frontier_sha256,range_start,range_end,session_count,\
         bar_count,usable FROM public.read_owner_beta_daily_admitted_input($1)",
    )
    .bind(expected.as_str())
    .fetch_optional(pool)
    .await
    .map_err(database_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.owner_user_id.is_nil()
        || row.admission_sha256 != expected.as_str()
        || !(2453..=20000).contains(&row.session_count)
        || row.bar_count != row.session_count * 11
        || row.parent_admission_sha256.is_none() != (row.session_count == 2453)
        || row.range_start != chrono::NaiveDate::from_ymd_opt(2016, 8, 29).expect("fixed date")
        || row.range_end <= chrono::NaiveDate::from_ymd_opt(2026, 8, 28).expect("fixed date")
    {
        return Err(DailyRuntimeError::DatabaseIntegrity);
    }
    Ok(Some(DailyCommittedInput {
        owner: row.owner_user_id,
        pins: market_data::daily_price_only::DailyPricePinsV1Wire {
            content_sha256: readback_hash(row.content_sha256)?,
            manifest_sha256: readback_hash(row.manifest_sha256)?,
            admission_sha256: readback_hash(row.admission_sha256)?,
            policy_sha256: readback_hash(row.policy_sha256)?,
        },
        parent: row.parent_admission_sha256.map(readback_hash).transpose()?,
        frontier: readback_hash(row.source_frontier_sha256)?,
        range_start: row.range_start,
        range_end: row.range_end,
        session_count: row.session_count as u32,
        usable: row.usable,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedAdmissionOutcome {
    Started,
    Verified {
        admission: ContentHash,
        source_frontier: ContentHash,
    },
    Failed {
        failure: AdmissionPollFailure,
        source_frontier: Option<ContentHash>,
        event: Option<(DailyBlockCode, ContentHash)>,
    },
}
/// An exact completion record, never a source/file capability. In particular,
/// matches_current_cycle() compares only the latest receipt ID and activation epoch.
/// It does not assert freshness, source validity, input usability or activation.
pub struct RecordedAdmissionReceipt {
    receipt: Uuid,
    owner: Uuid,
    policy: ContentHash,
    activation_epoch: i64,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    current: bool,
    outcome: RecordedAdmissionOutcome,
}
impl RecordedAdmissionReceipt {
    pub fn receipt_id(&self) -> Uuid {
        self.receipt
    }
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy
    }
    pub fn activation_epoch(&self) -> i64 {
        self.activation_epoch
    }
    pub fn started_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.started_at
    }
    pub fn completed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.completed_at
    }
    pub fn matches_current_cycle(&self) -> bool {
        self.current
    }
    pub fn outcome(&self) -> &RecordedAdmissionOutcome {
        &self.outcome
    }
}
#[derive(sqlx::FromRow)]
struct ReceiptRow {
    receipt_id: Uuid,
    owner_user_id: Uuid,
    policy_sha256: String,
    activation_epoch: i64,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    result_code: Option<String>,
    source_frontier_sha256: Option<String>,
    admission_sha256: Option<String>,
    event_sha256: Option<String>,
    event_code: Option<String>,
    matches_current_cycle: bool,
}
fn decode_receipt(
    row: ReceiptRow,
    expected: Uuid,
) -> Result<RecordedAdmissionReceipt, DailyRuntimeError> {
    if row.receipt_id != expected
        || row.receipt_id.is_nil()
        || row.owner_user_id.is_nil()
        || row.activation_epoch < 0
        || row
            .completed_at
            .is_some_and(|finished| finished < row.started_at)
        || row.completed_at.is_some() != row.result_code.is_some()
        || row.event_sha256.is_some() != row.event_code.is_some()
    {
        return Err(DailyRuntimeError::DatabaseIntegrity);
    }
    let frontier = row.source_frontier_sha256.map(readback_hash).transpose()?;
    let admission = row.admission_sha256.map(readback_hash).transpose()?;
    let outcome = match row.result_code.as_deref() {
        None if frontier.is_none() && admission.is_none() && row.event_sha256.is_none() => {
            RecordedAdmissionOutcome::Started
        }
        Some("VERIFIED") if row.event_sha256.is_none() => RecordedAdmissionOutcome::Verified {
            admission: admission.ok_or(DailyRuntimeError::DatabaseIntegrity)?,
            source_frontier: frontier.ok_or(DailyRuntimeError::DatabaseIntegrity)?,
        },
        Some(code @ ("INPUT_PENDING" | "INPUT_BLOCKED" | "IO_FAILED")) if admission.is_none() => {
            let failure = match code {
                "INPUT_PENDING" => AdmissionPollFailure::InputPending,
                "INPUT_BLOCKED" => AdmissionPollFailure::InputBlocked,
                _ => AdmissionPollFailure::IoFailed,
            };
            let event = if let Some(hash) = row.event_sha256 {
                if failure != AdmissionPollFailure::InputBlocked || frontier.is_none() {
                    return Err(DailyRuntimeError::DatabaseIntegrity);
                }
                let result = SqlOutcome {
                    disposition: "BLOCKED".into(),
                    identity: None,
                    reason_code: row.event_code,
                    event_sha256: Some(hash.clone()),
                };
                Some((block_code(&result)?, readback_hash(hash)?))
            } else {
                None
            };
            RecordedAdmissionOutcome::Failed {
                failure,
                source_frontier: frontier,
                event,
            }
        }
        _ => return Err(DailyRuntimeError::DatabaseIntegrity),
    };
    Ok(RecordedAdmissionReceipt {
        receipt: row.receipt_id,
        owner: row.owner_user_id,
        policy: readback_hash(row.policy_sha256)?,
        activation_epoch: row.activation_epoch,
        started_at: row.started_at,
        completed_at: row.completed_at,
        current: row.matches_current_cycle,
        outcome,
    })
}
/// No slot is taken or completed by readback. A missing result or interrupted
/// receipt remains unknown; it never becomes successful merely after a restart.
pub async fn read_daily_admission_receipt(
    pool: &PgPool,
    expected: Uuid,
) -> Result<Option<RecordedAdmissionReceipt>, DailyRuntimeError> {
    if expected.is_nil() {
        return Err(DailyRuntimeError::InvalidInput);
    }
    let row: Option<ReceiptRow> = sqlx::query_as(
        "SELECT receipt_id,owner_user_id,policy_sha256,activation_epoch,started_at,completed_at,\
         result_code,source_frontier_sha256,admission_sha256,event_sha256,event_code,matches_current_cycle \
         FROM public.read_owner_beta_daily_admission_receipt($1)")
        .bind(expected).fetch_optional(pool).await.map_err(database_error)?;
    row.map(|row| decode_receipt(row, expected)).transpose()
}

#[cfg(test)]
mod readback_tests {
    use super::*;

    fn receipt_row() -> ReceiptRow {
        ReceiptRow {
            receipt_id: Uuid::from_u128(1),
            owner_user_id: Uuid::from_u128(2),
            policy_sha256: format!("sha256:{}", "a".repeat(64)),
            activation_epoch: 1,
            started_at: "2026-10-15T00:00:00Z".parse().unwrap(),
            completed_at: None,
            result_code: None,
            source_frontier_sha256: None,
            admission_sha256: None,
            event_sha256: None,
            event_code: None,
            matches_current_cycle: true,
        }
    }

    #[test]
    fn daily_receipt_decoder_rejects_partial_or_foreign_identity() {
        for change in 0..6 {
            let mut row = receipt_row();
            match change {
                0 => row.receipt_id = Uuid::from_u128(3),
                1 => row.owner_user_id = Uuid::nil(),
                2 => row.activation_epoch = -1,
                3 => row.completed_at = Some(row.started_at),
                4 => row.result_code = Some("INPUT_PENDING".into()),
                _ => {
                    row.completed_at = Some(row.started_at - chrono::Duration::seconds(1));
                    row.result_code = Some("IO_FAILED".into());
                }
            }
            assert!(matches!(
                decode_receipt(row, Uuid::from_u128(1)),
                Err(DailyRuntimeError::DatabaseIntegrity)
            ));
        }
    }

    #[test]
    fn daily_receipt_decoder_rejects_malformed_hash_and_result_shape() {
        for change in 0..6 {
            let mut row = receipt_row();
            row.completed_at = Some(row.started_at);
            row.result_code = Some("VERIFIED".into());
            row.source_frontier_sha256 = Some(format!("sha256:{}", "b".repeat(64)));
            row.admission_sha256 = Some(format!("sha256:{}", "c".repeat(64)));
            match change {
                0 => row.policy_sha256 = "not-a-hash".into(),
                1 => row.source_frontier_sha256 = None,
                2 => row.admission_sha256 = None,
                3 => row.result_code = Some("CHECKED".into()),
                4 => row.result_code = Some("INPUT_PENDING".into()),
                _ => {
                    row.event_sha256 = Some(format!("sha256:{}", "d".repeat(64)));
                    row.event_code = Some("SOURCE_CONFLICT".into());
                }
            }
            assert!(matches!(
                decode_receipt(row, Uuid::from_u128(1)),
                Err(DailyRuntimeError::DatabaseIntegrity)
            ));
        }
    }

    #[test]
    fn daily_receipt_decoder_preserves_started_vs_noncurrent_completed() {
        let started = decode_receipt(receipt_row(), Uuid::from_u128(1)).unwrap();
        assert_eq!(started.outcome(), &RecordedAdmissionOutcome::Started);
        assert!(started.completed_at().is_none());
        let mut row = receipt_row();
        row.completed_at = Some(row.started_at);
        row.result_code = Some("VERIFIED".into());
        row.source_frontier_sha256 = Some(format!("sha256:{}", "b".repeat(64)));
        row.admission_sha256 = Some(format!("sha256:{}", "c".repeat(64)));
        row.matches_current_cycle = false;
        let recorded = decode_receipt(row, Uuid::from_u128(1)).unwrap();
        assert!(!recorded.matches_current_cycle());
        assert!(matches!(
            recorded.outcome(),
            RecordedAdmissionOutcome::Verified { .. }
        ));
        assert!(recorded.completed_at().is_some());
    }

    #[test]
    fn daily_receipt_decoder_only_pairs_blocked_event_with_evidence() {
        for change in 0..5 {
            let mut row = receipt_row();
            row.completed_at = Some(row.started_at);
            row.result_code = Some("INPUT_BLOCKED".into());
            row.source_frontier_sha256 = Some(format!("sha256:{}", "b".repeat(64)));
            row.event_sha256 = Some(format!("sha256:{}", "d".repeat(64)));
            row.event_code = Some("SOURCE_CONFLICT".into());
            match change {
                0 => (),
                1 => row.event_code = None,
                2 => row.source_frontier_sha256 = None,
                3 => row.event_code = Some("unrecognized".into()),
                _ => row.result_code = Some("IO_FAILED".into()),
            }
            let result = decode_receipt(row, Uuid::from_u128(1));
            if change == 0 {
                assert!(matches!(
                    result.unwrap().outcome(),
                    RecordedAdmissionOutcome::Failed {
                        failure: AdmissionPollFailure::InputBlocked,
                        event: Some((DailyBlockCode::SourceConflict, _)),
                        ..
                    }
                ));
            } else {
                assert!(matches!(result, Err(DailyRuntimeError::DatabaseIntegrity)));
            }
        }
    }
}
