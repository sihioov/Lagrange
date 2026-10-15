//! Read-only Owner/RLS projection. No Raw, filesystem, provider or definer call.
use super::OwnerBetaRecommendationRepo;
use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use job_queue::owner_beta::runner::daily_runtime::DailyProcessMode;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DailyStateCode {
    Disabled,
    Unverifiable,
    InputPending,
    InputBlocked,
    Queued,
    Running,
    Retrying,
    Failed,
    Current,
    Stale,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DailyReasonCode {
    RuntimeDisabled,
    ConfigDisabled,
    ConfigInactive,
    ControlUnavailable,
    CalendarCoverageGap,
    VerificationExpired,
    InputUnavailable,
    InputNotReady,
    BeforeActivation,
    AwaitingSchedule,
    JobQueued,
    JobRunning,
    JobRetrying,
    JobFailed,
    CapacityExceeded,
    Superseded,
    ConfigurationChanged,
    FutureEvidence,
    SourceConflict,
    SealedSourceChanged,
    SourceRevoked,
    SourceIntegrity,
    CalendarConflict,
    NoncashAction,
}
#[derive(Debug, Clone, Serialize)]
pub struct DailyState {
    pub enabled: bool,
    pub runtime_enabled: bool,
    pub expected_session: Option<NaiveDate>,
    pub data_support_end: Option<NaiveDate>,
    pub recommendation_as_of: Option<NaiveDate>,
    pub published_at: Option<DateTime<Utc>>,
    pub input_kind: Option<&'static str>,
    pub trigger_kind: Option<&'static str>,
    pub current_reservation_run_id: Option<Uuid>,
    pub state: DailyStateCode,
    pub reason_code: Option<DailyReasonCode>,
    pub data_due_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
}
#[derive(Debug, Clone, sqlx::FromRow)]
struct StateRow {
    enabled: bool,
    active: bool,
    config_epoch: i64,
    control_enabled: Option<bool>,
    activation_epoch: Option<i64>,
    first_auto_session: Option<NaiveDate>,
    rights: bool,
    capacity_blocked: bool,
    observed_at: DateTime<Utc>,
    civil: NaiveDate,
    expected_session: Option<NaiveDate>,
    support_end: Option<NaiveDate>,
    source_block: Option<String>,
    admission_block: bool,
    future_input: bool,
    receipt_completed_at: Option<DateTime<Utc>>,
    receipt_started_at: Option<DateTime<Utc>>,
    receipt_result: Option<String>,
    receipt_binding: bool,
    reservation_id: Option<Uuid>,
    run_status: Option<String>,
    job_status: Option<String>,
    run_binding: bool,
    run_error: Option<String>,
    reservation_epoch: Option<i64>,
    reservation_activation: Option<i64>,
    attempt_count: Option<i32>,
    run_as_of: Option<NaiveDate>,
    run_finished_at: Option<DateTime<Utc>>,
    published_as_of: Option<NaiveDate>,
    published_at: Option<DateTime<Utc>>,
    published_binding: bool,
}
fn invalid() -> TenancyError {
    TenancyError::ResultIntegrity("owner-beta daily state is inconsistent".into())
}
fn source_reason(code: &str) -> TenancyResult<DailyReasonCode> {
    Ok(match code {
        "SOURCE_CONFLICT" => DailyReasonCode::SourceConflict,
        "SEALED_SOURCE_CHANGED" => DailyReasonCode::SealedSourceChanged,
        "SOURCE_REVOKED" => DailyReasonCode::SourceRevoked,
        "SOURCE_INTEGRITY" => DailyReasonCode::SourceIntegrity,
        "CALENDAR_CONFLICT" => DailyReasonCode::CalendarConflict,
        "NONCASH_ACTION" => DailyReasonCode::NoncashAction,
        _ => return Err(invalid()),
    })
}
fn project(row: StateRow, mode: DailyProcessMode) -> TenancyResult<DailyState> {
    use DailyReasonCode as R;
    use DailyStateCode as S;
    if row.control_enabled.is_some() != row.activation_epoch.is_some()
        || row.control_enabled.is_some() != row.first_auto_session.is_some()
        || row.control_enabled == Some(true) && row.activation_epoch.is_some_and(|v| v == 0)
        || row.receipt_completed_at.is_some() && row.receipt_started_at.is_none()
        || row.config_epoch < 0
        || row.activation_epoch.is_some_and(|v| v < 0)
        || row.reservation_epoch.is_some_and(|v| v < 0)
        || row.reservation_activation.is_some_and(|v| v < 0)
        || row.attempt_count.is_some_and(|v| v < 0)
        || row.expected_session.is_some_and(|d| d > row.civil)
        || row.support_end.is_some_and(|d| d > row.civil)
        || row.published_as_of.is_some() != row.published_at.is_some()
        || row.published_as_of.is_some_and(|d| d > row.civil)
        || row.published_at.is_some_and(|t| t > row.observed_at)
        || row.run_finished_at.is_some_and(|t| t > row.observed_at)
        || row.reservation_id.is_some() != row.run_status.is_some()
        || row
            .run_status
            .as_deref()
            .is_some_and(|s| !["PENDING", "SUCCEEDED", "FAILED", "CANCELED"].contains(&s))
        || row.reservation_id.is_some()
            && (!row.run_binding
                || row.job_status.is_none()
                || row.reservation_epoch.is_none()
                || row.reservation_activation.is_none()
                || row.attempt_count.is_none()
                || row.run_as_of != row.expected_session)
        || row.published_at.is_some() && !row.published_binding
        || row.receipt_completed_at.is_some() != row.receipt_result.is_some()
        || row
            .receipt_completed_at
            .zip(row.receipt_started_at)
            .is_some_and(|(end, start)| end < start)
        || row.receipt_result.as_deref().is_some_and(|s| {
            !["VERIFIED", "INPUT_PENDING", "INPUT_BLOCKED", "IO_FAILED"].contains(&s)
        })
    {
        return Err(invalid());
    }
    if row.control_enabled.is_some() && !row.rights {
        return Err(TenancyError::Forbidden);
    }
    let source_block = row.source_block.as_deref().map(source_reason).transpose()?;
    let due = row
        .expected_session
        .map(|date| {
            let local = date.and_hms_opt(18, 30, 0).ok_or_else(invalid)?;
            chrono::FixedOffset::east_opt(9 * 3600)
                .ok_or_else(invalid)?
                .from_local_datetime(&local)
                .single()
                .map(|v| v.with_timezone(&Utc))
                .ok_or_else(invalid)
        })
        .transpose()?;
    let runtime = mode == DailyProcessMode::OwnerOnly && row.control_enabled == Some(true);
    let future_receipt = row.receipt_started_at.is_some_and(|t| t > row.observed_at)
        || row
            .receipt_completed_at
            .is_some_and(|t| t > row.observed_at);
    let fresh = row.receipt_completed_at.is_some_and(|t| {
        t <= row.observed_at
            && row.observed_at.signed_duration_since(t) <= chrono::Duration::seconds(180)
    });
    let after_due = due.is_some_and(|t| row.observed_at >= t);
    let delayed = |state, reason| {
        if after_due {
            (S::Stale, Some(reason))
        } else {
            (state, Some(reason))
        }
    };
    let (state, reason) = if !row.enabled {
        (S::Disabled, Some(R::ConfigDisabled))
    } else if !row.active {
        (S::Disabled, Some(R::ConfigInactive))
    } else if mode == DailyProcessMode::Disabled {
        (S::Disabled, Some(R::RuntimeDisabled))
    } else if row.control_enabled.is_none() {
        (S::Unverifiable, Some(R::ControlUnavailable))
    } else if !runtime {
        (S::Disabled, Some(R::RuntimeDisabled))
    } else if let Some(code) = source_block {
        (S::InputBlocked, Some(code))
    } else if row.admission_block {
        (S::InputBlocked, Some(R::SourceRevoked))
    } else if row.future_input || future_receipt {
        (S::Unverifiable, Some(R::FutureEvidence))
    } else if row.expected_session.is_none() {
        (S::Unverifiable, Some(R::CalendarCoverageGap))
    } else if row
        .first_auto_session
        .zip(row.expected_session)
        .is_some_and(|(first, expected)| expected < first)
    {
        (S::InputPending, Some(R::BeforeActivation))
    } else if row.run_status.as_deref() == Some("FAILED") {
        (S::Failed, Some(R::JobFailed))
    } else if row.run_status.as_deref() == Some("CANCELED")
        || row.job_status.as_deref() == Some("CANCELED")
    {
        (
            S::Failed,
            Some(
                if row.run_error.as_deref() == Some("DAILY_SESSION_SUPERSEDED") {
                    R::Superseded
                } else {
                    R::JobFailed
                },
            ),
        )
    } else if row.support_end.is_none() {
        delayed(S::InputPending, R::InputNotReady)
    } else if !fresh {
        if row.run_status.as_deref() == Some("SUCCEEDED")
            && row.published_as_of == row.expected_session
        {
            (S::Unverifiable, Some(R::VerificationExpired))
        } else {
            delayed(S::Unverifiable, R::VerificationExpired)
        }
    } else if row.receipt_result.as_deref() == Some("IO_FAILED") {
        delayed(S::Unverifiable, R::InputUnavailable)
    } else if row.receipt_result.as_deref() == Some("INPUT_BLOCKED") {
        (S::InputBlocked, Some(R::SourceIntegrity))
    } else if row.receipt_result.as_deref() != Some("VERIFIED")
        || !row.receipt_binding
        || row.support_end != row.expected_session
    {
        delayed(S::InputPending, R::InputNotReady)
    } else if row.reservation_id.is_some()
        && (row.reservation_epoch != Some(row.config_epoch)
            || row.reservation_activation != row.activation_epoch)
    {
        (S::Stale, Some(R::ConfigurationChanged))
    } else {
        match row.run_status.as_deref() {
            Some("SUCCEEDED")
                if row.run_as_of == row.expected_session
                    && row.run_finished_at.is_some()
                    && row.published_as_of == row.expected_session
                    && row.published_at == row.run_finished_at =>
            {
                (S::Current, None)
            }
            Some("SUCCEEDED") => return Err(invalid()),
            Some("FAILED") => (S::Failed, Some(R::JobFailed)),
            Some("CANCELED") => (
                S::Failed,
                Some(
                    if row.run_error.as_deref() == Some("DAILY_SESSION_SUPERSEDED") {
                        R::Superseded
                    } else {
                        R::JobFailed
                    },
                ),
            ),
            Some("PENDING") if row.job_status.as_deref() == Some("RUNNING") => {
                delayed(S::Running, R::JobRunning)
            }
            Some("PENDING") if row.attempt_count.unwrap_or(0) > 0 => {
                delayed(S::Retrying, R::JobRetrying)
            }
            Some("PENDING") => delayed(S::Queued, R::JobQueued),
            None if row.capacity_blocked => delayed(S::InputPending, R::CapacityExceeded),
            None => delayed(S::InputPending, R::AwaitingSchedule),
            _ => return Err(invalid()),
        }
    };
    Ok(DailyState {
        enabled: row.enabled,
        runtime_enabled: runtime,
        expected_session: row.expected_session,
        data_support_end: if fresh
            && row.receipt_result.as_deref() == Some("VERIFIED")
            && row.receipt_binding
            && !future_receipt
            && !row.future_input
            && source_block.is_none()
            && !row.admission_block
        {
            row.support_end
        } else {
            None
        },
        recommendation_as_of: row.published_as_of,
        published_at: row.published_at,
        input_kind: row
            .published_at
            .map(|_| "owner_beta_daily_etf11_price_only_v1"),
        trigger_kind: row.published_at.map(|_| "DAILY"),
        current_reservation_run_id: row.reservation_id,
        state,
        reason_code: reason,
        data_due_at: due,
        observed_at: row.observed_at,
    })
}

impl OwnerBetaRecommendationRepo {
    pub async fn read_daily_state(
        &self,
        actor: &Actor,
        config: Uuid,
        mode: DailyProcessMode,
    ) -> TenancyResult<DailyState> {
        if !actor.is_owner() {
            return Err(TenancyError::Forbidden);
        }
        if config.is_nil() {
            return Err(TenancyError::NotFound);
        }
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as::<_, StateRow>(READ_STATE)
            .bind(config)
            .bind(actor_uuid(actor)?)
            .fetch_optional(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?;
        let result = project(row.ok_or(TenancyError::NotFound)?, mode)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(result)
    }
}

// One SQL snapshot and one sampled DB clock. Every join has an explicit Owner,
// policy or exact immutable identity in addition to the existing app RLS policy.
const READ_STATE: &str = r#"
WITH observed AS MATERIALIZED (SELECT clock_timestamp() AS now),
clock AS (SELECT now,(now AT TIME ZONE 'Asia/Seoul')::date AS today,
 ((now AT TIME ZONE 'Asia/Seoul')::date-
 CASE WHEN (now AT TIME ZONE 'Asia/Seoul')::time<time '16:30' THEN 1 ELSE 0 END) AS civil FROM observed),
cfg AS (SELECT * FROM public.user_strategy_configs WHERE id=$1 AND owner_user_id=$2),
control AS (SELECT * FROM public.owner_beta_daily_control WHERE owner_user_id=$2 AND id='etf11'),
calendar AS (
 SELECT d.session_date,min(d.session_type) AS kind,count(DISTINCT d.session_type) AS kinds
 FROM public.owner_beta_daily_calendar_days d
 JOIN public.owner_beta_daily_calendar_sources s USING(owner_user_id,calendar_evidence_sha256)
 JOIN control c ON c.owner_user_id=d.owner_user_id AND c.policy_sha256=s.policy_sha256
 CROSS JOIN clock t WHERE d.session_date BETWEEN t.civil-369 AND t.civil
 GROUP BY d.session_date),
latest AS (SELECT max(session_date) AS day FROM calendar WHERE kind='TRADING'),
expected AS (SELECT CASE WHEN l.day IS NOT NULL
 AND (SELECT count(*) FROM calendar WHERE session_date BETWEEN l.day AND t.civil)=t.civil-l.day+1
 AND NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_calendar_days d
   WHERE d.owner_user_id=$2 AND d.session_date BETWEEN l.day AND t.civil
   GROUP BY d.session_date HAVING count(DISTINCT d.session_type)>1)
 AND NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_source_events e
   WHERE e.owner_user_id=$2 AND e.source_kind='CALENDAR'
     AND e.range_start<=t.civil AND e.range_end>=l.day)
 THEN l.day END AS day FROM latest l CROSS JOIN clock t),
admission AS (SELECT a.* FROM public.owner_beta_daily_admissions a JOIN control c
 ON c.owner_user_id=a.owner_user_id AND c.policy_sha256=a.policy_sha256 CROSS JOIN clock t
 WHERE a.range_end<=t.civil ORDER BY a.range_end DESC,a.admission_sha256 LIMIT 1),
receipt AS (SELECT r.*,s.verified_admission_sha256 FROM public.owner_beta_daily_poll_state s
 JOIN public.owner_beta_daily_poll_receipts r ON r.receipt_id=s.latest_receipt_id AND r.stage=s.stage
 JOIN control c ON c.owner_user_id=r.owner_user_id AND c.policy_sha256=r.policy_sha256
 AND c.activation_epoch=r.activation_epoch WHERE s.stage='ADMISSION'),
run_rows AS (
 SELECT r.*,j.status AS job_status,j.attempt_count,
 COALESCE((
q.run_id=r.id AND q.job_id=r.job_id AND q.owner_user_id=r.owner_user_id
AND q.strategy_config_id=r.strategy_config_id AND q.session_date=r.as_of
AND q.config_epoch=r.daily_config_epoch AND q.activation_epoch=r.daily_activation_epoch
AND q.trigger_version=r.daily_trigger_version AND q.strategy_config_sha256=r.strategy_config_sha256
AND q.admission_sha256=r.daily_pins->>'admission_sha256'
AND r.input_kind='owner_beta_daily_etf11_price_only_v1' AND r.trigger_kind='DAILY'
AND a.owner_user_id=r.owner_user_id AND a.range_end=r.as_of
AND r.daily_pins=jsonb_build_object('admission_sha256',a.admission_sha256,
 'content_sha256',a.content_sha256,'manifest_sha256',a.manifest_sha256,'policy_sha256',a.policy_sha256)
AND j.owner_user_id=r.owner_user_id AND j.job_type='owner_beta_price_recommendation'
AND j.payload_json=jsonb_build_object(
 'schema','owner-beta-daily-recommendation-v1','run_id',r.id,'strategy_config_id',r.strategy_config_id,
 'as_of',to_char(r.as_of,'YYYY-MM-DD'),
 'input',r.daily_pins||jsonb_build_object('kind',r.input_kind),
 'strategy',jsonb_build_object('strategy_id',r.strategy_id,'strategy_version',r.strategy_version,
  'config_json',r.strategy_config_json,'config_sha256',r.strategy_config_sha256),
 'trigger',jsonb_build_object('kind','DAILY','version',r.daily_trigger_version,
  'config_epoch',r.daily_config_epoch,'activation_epoch',r.daily_activation_epoch))
AND r.candidate_content_sha256 IS NULL AND r.artifact_manifest_sha256 IS NULL
AND r.stage5_manifest_sha256 IS NULL AND r.action_manifest_sha256 IS NULL
AND r.approval_registry_sha256 IS NULL

 AND r.capability='PRICE_RETURN_ONLY' AND r.audience='OWNER_ONLY'
 AND r.vendor_snapshot AND NOT r.strict_pit AND j.max_attempts=3 AND j.attempt_count BETWEEN 0 AND 3
 AND ((r.status='PENDING' AND j.status IN ('QUEUED','RUNNING','CANCELED') AND r.finished_at IS NULL
       AND r.error_code IS NULL AND r.started_at IS NULL)
   OR (r.status=j.status AND r.status IN ('SUCCEEDED','FAILED','CANCELED') AND r.finished_at IS NOT NULL))
 AND ((r.status='SUCCEEDED' AND r.factor_snapshot_sha256 IS NOT NULL
       AND r.target_snapshot_sha256 IS NOT NULL AND r.cash_weight IS NOT NULL AND r.error_code IS NULL
       AND r.started_at IS NOT NULL
       AND (SELECT count(*)=11 AND count(DISTINCT item.instrument_id)=11
          AND bool_and(item.instrument_id=ANY(ARRAY['069500.KRX','102110.KRX','114260.KRX','132030.KRX','133690.KRX','143850.KRX','148070.KRX','153130.KRX','192090.KRX','195930.KRX','229200.KRX']))
          AND COALESCE(sum(item.target_weight),0)+r.cash_weight=1
         FROM public.owner_beta_recommendation_items item
         WHERE item.recommendation_run_id=r.id AND item.owner_user_id=r.owner_user_id))
    OR (r.status<>'SUCCEEDED' AND r.factor_snapshot_sha256 IS NULL AND r.target_snapshot_sha256 IS NULL
        AND r.cash_weight IS NULL AND NOT EXISTS(SELECT 1 FROM public.owner_beta_recommendation_items item
         WHERE item.recommendation_run_id=r.id AND item.owner_user_id=r.owner_user_id)))
 ),false) AS binding
 FROM public.owner_beta_recommendation_runs r JOIN cfg sc
 ON sc.id=r.strategy_config_id AND sc.owner_user_id=r.owner_user_id
 LEFT JOIN public.owner_beta_daily_reservations q ON q.run_id=r.id
 LEFT JOIN public.jobs j ON j.id=r.job_id
 LEFT JOIN public.owner_beta_daily_admissions a ON a.admission_sha256=q.admission_sha256),
current_run AS (
 SELECT q.run_id AS id,r.status,r.error_code,r.as_of,r.finished_at,q.config_epoch,q.activation_epoch,
 r.job_status,r.attempt_count,COALESCE(r.binding,false) AS binding
 FROM public.owner_beta_daily_reservations q JOIN cfg sc
 ON sc.id=q.strategy_config_id AND sc.owner_user_id=q.owner_user_id
 JOIN expected x ON x.day=q.session_date LEFT JOIN run_rows r ON r.id=q.run_id),
published AS (
 SELECT as_of,finished_at,binding FROM run_rows WHERE trigger_kind='DAILY'
 AND status='SUCCEEDED' ORDER BY as_of DESC,finished_at DESC,id DESC LIMIT 1)
SELECT sc.daily_enabled AS enabled,sc.is_active AS active,sc.daily_epoch AS config_epoch,
 c.enabled AS control_enabled,c.activation_epoch,c.first_auto_session,
 EXISTS(SELECT 1 FROM public.data_entitlements e WHERE e.contract_reference=c.entitlement_reference
 AND e.contract_document_sha256=substring(c.entitlement_sha256,8) AND e.status='ACTIVE'
 AND e.effective_from<=LEAST(date '2016-08-29',t.today)
 AND e.effective_until>=GREATEST(COALESCE(a.range_end,t.today),t.today)
 AND e.covered_datasets @> '["krx_eod_bars"]'::jsonb AND e.covered_uses @> '["recommendation"]'::jsonb) AS rights,
 (SELECT count(*) FROM public.jobs j WHERE j.owner_user_id=$2
   AND j.status IN ('QUEUED','RUNNING'))>=COALESCE(c.max_jobs_per_owner,2147483647) AS capacity_blocked,
 t.now AS observed_at,t.civil,x.day AS expected_session,a.range_end AS support_end,
 (SELECT e.code FROM public.owner_beta_daily_source_events e WHERE e.owner_user_id=$2
  AND e.range_start<=t.civil AND e.range_end>=date '2016-08-29'
  ORDER BY e.recorded_at DESC,e.event_sha256 LIMIT 1) AS source_block,
 EXISTS(SELECT 1 FROM public.owner_beta_daily_admission_events e
  JOIN public.owner_beta_daily_admissions old ON old.admission_sha256=e.admission_sha256
  WHERE e.owner_user_id=$2 AND old.owner_user_id=$2 AND old.range_end<=a.range_end) AS admission_block,
 EXISTS(SELECT 1 FROM public.owner_beta_daily_admissions future WHERE future.owner_user_id=$2
  AND future.range_end>t.civil) AS future_input,
 v.completed_at AS receipt_completed_at,v.started_at AS receipt_started_at,v.result_code AS receipt_result,
 COALESCE(v.admission_sha256=a.admission_sha256 AND v.verified_admission_sha256=a.admission_sha256
 AND v.completion_binding_sha256=('sha256:'||encode(sha256(convert_to('sql-admission-v1'||a.evidence_json::text,'UTF8')),'hex')),false) AS receipt_binding,
 r.id AS reservation_id,r.status AS run_status,r.job_status,COALESCE(r.binding,false) AS run_binding,r.error_code AS run_error,
 r.config_epoch AS reservation_epoch,r.activation_epoch AS reservation_activation,
 r.attempt_count,r.as_of AS run_as_of,r.finished_at AS run_finished_at,
 p.as_of AS published_as_of,p.finished_at AS published_at,COALESCE(p.binding,false) AS published_binding
FROM cfg sc CROSS JOIN clock t CROSS JOIN expected x
LEFT JOIN control c ON true LEFT JOIN admission a ON true LEFT JOIN receipt v ON true
LEFT JOIN current_run r ON true LEFT JOIN published p ON true
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    fn sample() -> StateRow {
        let date = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-15T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        StateRow {
            enabled: true,
            active: true,
            config_epoch: 2,
            control_enabled: Some(true),
            activation_epoch: Some(1),
            first_auto_session: Some(date),
            rights: true,
            capacity_blocked: false,
            observed_at: now,
            civil: date,
            expected_session: Some(date),
            support_end: Some(date),
            source_block: None,
            admission_block: false,
            future_input: false,
            receipt_completed_at: Some(now - Duration::seconds(1)),
            receipt_started_at: Some(now - Duration::seconds(2)),
            receipt_result: Some("VERIFIED".into()),
            receipt_binding: true,
            reservation_id: None,
            run_status: None,
            job_status: None,
            run_binding: false,
            run_error: None,
            reservation_epoch: None,
            reservation_activation: None,
            attempt_count: None,
            run_as_of: None,
            run_finished_at: None,
            published_as_of: Some(date - Duration::days(1)),
            published_at: Some(now - Duration::days(1)),
            published_binding: true,
        }
    }
    fn reserved(row: &mut StateRow, job: &str) {
        row.reservation_id = Some(Uuid::from_u128(1));
        row.run_status = Some("PENDING".into());
        row.job_status = Some(job.into());
        row.run_binding = true;
        row.reservation_epoch = Some(2);
        row.reservation_activation = Some(1);
        row.attempt_count = Some(0);
        row.run_as_of = row.expected_session;
    }
    fn output(row: StateRow) -> DailyState {
        project(row, DailyProcessMode::OwnerOnly).unwrap()
    }
    #[test]
    fn daily_state_dates_remain_separate_when_new_input_has_old_recommendation() {
        let row = sample();
        let value = output(row.clone());
        assert_eq!(value.state, DailyStateCode::InputPending);
        assert_eq!(value.data_support_end, row.expected_session);
        assert_eq!(value.recommendation_as_of, row.published_as_of);
        assert_eq!(value.published_at, row.published_at);
        assert_ne!(value.recommendation_as_of, value.expected_session);
        assert_eq!(
            value.data_due_at.unwrap().to_rfc3339(),
            "2026-10-15T09:30:00+00:00"
        );
    }
    #[test]
    fn daily_state_deadline_is_inclusive_and_never_moves_for_queue_or_retry() {
        for (job, attempt) in [
            (None, 0),
            (Some("QUEUED"), 0),
            (Some("QUEUED"), 1),
            (Some("RUNNING"), 1),
        ] {
            let mut row = sample();
            if let Some(job) = job {
                reserved(&mut row, job);
                row.attempt_count = Some(attempt);
            }
            row.observed_at = DateTime::parse_from_rfc3339("2026-10-15T09:29:59Z")
                .unwrap()
                .with_timezone(&Utc);
            row.receipt_started_at = Some(row.observed_at - Duration::seconds(2));
            row.receipt_completed_at = Some(row.observed_at);
            let before = output(row.clone());
            assert_ne!(before.state, DailyStateCode::Stale);
            row.observed_at += Duration::seconds(1);
            let due = output(row);
            assert_eq!(due.state, DailyStateCode::Stale);
            assert_eq!(due.data_due_at, before.data_due_at);
            assert_eq!(due.reason_code, before.reason_code);
        }
    }
    #[test]
    fn daily_state_uses_queue_running_and_retry_without_fabricating_run_running() {
        let mut row = sample();
        reserved(&mut row, "RUNNING");
        row.attempt_count = Some(1);
        assert_eq!(output(row.clone()).state, DailyStateCode::Running);
        row.job_status = Some("QUEUED".into());
        assert_eq!(output(row.clone()).state, DailyStateCode::Retrying);
        row.attempt_count = Some(0);
        assert_eq!(output(row.clone()).state, DailyStateCode::Queued);
        row.run_status = Some("RUNNING".into());
        assert!(project(row, DailyProcessMode::OwnerOnly).is_err());
    }
    #[test]
    fn daily_state_verification_exact_expiry_preserves_publication_but_loses_eligibility() {
        let mut row = sample();
        row.receipt_started_at = Some(row.observed_at - Duration::seconds(181));
        row.receipt_completed_at = Some(row.observed_at - Duration::seconds(180));
        assert!(output(row.clone()).data_support_end.is_some());
        row.observed_at += Duration::milliseconds(1);
        let value = output(row.clone());
        assert_eq!(value.state, DailyStateCode::Unverifiable);
        assert_eq!(
            value.reason_code,
            Some(DailyReasonCode::VerificationExpired)
        );
        assert_eq!(value.data_support_end, None);
        assert_eq!(value.published_at, row.published_at);
        row.observed_at = DateTime::parse_from_rfc3339("2026-10-15T09:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let overdue = output(row.clone());
        assert_eq!(overdue.state, DailyStateCode::Stale);
        assert_eq!(
            overdue.reason_code,
            Some(DailyReasonCode::VerificationExpired)
        );
        // A new in-progress scan has no completion and cannot reuse an old
        // VERIFIED result, even while it retains the same admitted input.
        row.receipt_started_at = Some(row.observed_at);
        row.receipt_completed_at = None;
        row.receipt_result = None;
        assert_eq!(output(row).state, DailyStateCode::Stale);
    }
    #[test]
    fn daily_state_current_requires_same_daily_run_date_epoch_and_publication() {
        let mut row = sample();
        reserved(&mut row, "SUCCEEDED");
        row.run_status = Some("SUCCEEDED".into());
        row.run_finished_at = Some(row.observed_at - Duration::seconds(1));
        row.published_at = row.run_finished_at;
        row.published_as_of = row.expected_session;
        assert_eq!(output(row.clone()).state, DailyStateCode::Current);
        row.reservation_epoch = Some(1);
        assert_eq!(output(row.clone()).state, DailyStateCode::Stale);
        row.reservation_epoch = Some(2);
        row.published_as_of = None;
        row.published_at = None;
        assert!(project(row, DailyProcessMode::OwnerOnly).is_err());
    }
    #[test]
    fn daily_state_disable_paths_keep_the_previous_result_without_current_claim() {
        let row = sample();
        let disabled = project(row.clone(), DailyProcessMode::Disabled).unwrap();
        assert_eq!(disabled.state, DailyStateCode::Disabled);
        assert!(!disabled.runtime_enabled);
        for kind in 0..3 {
            let mut item = row.clone();
            match kind {
                0 => item.enabled = false,
                1 => item.active = false,
                _ => item.control_enabled = Some(false),
            }
            let value = output(item);
            assert_eq!(value.state, DailyStateCode::Disabled);
            assert_eq!(value.published_at, row.published_at);
        }
    }
    #[test]
    fn daily_state_calendar_gap_missing_input_and_capacity_are_distinct() {
        let mut row = sample();
        row.expected_session = None;
        let value = output(row);
        assert_eq!(value.state, DailyStateCode::Unverifiable);
        assert_eq!(
            value.reason_code,
            Some(DailyReasonCode::CalendarCoverageGap)
        );
        assert_eq!(value.data_due_at, None);
        let mut row = sample();
        row.support_end = None;
        row.receipt_completed_at = None;
        row.receipt_result = None;
        assert_eq!(output(row).state, DailyStateCode::InputPending);
        let mut row = sample();
        row.capacity_blocked = true;
        assert_eq!(
            output(row).reason_code,
            Some(DailyReasonCode::CapacityExceeded)
        );
    }
    #[test]
    fn daily_state_block_and_failed_codes_keep_specific_status_after_deadline() {
        for code in [
            "SOURCE_CONFLICT",
            "SEALED_SOURCE_CHANGED",
            "SOURCE_REVOKED",
            "SOURCE_INTEGRITY",
            "CALENDAR_CONFLICT",
            "NONCASH_ACTION",
        ] {
            let mut row = sample();
            row.observed_at += Duration::hours(3);
            row.source_block = Some(code.into());
            let value = output(row.clone());
            assert_eq!(value.state, DailyStateCode::InputBlocked);
            assert_eq!(value.data_support_end, None);
            assert_eq!(value.published_at, row.published_at);
        }
        let mut row = sample();
        reserved(&mut row, "FAILED");
        row.run_status = Some("FAILED".into());
        row.run_error = Some("OWNER_BETA_COMPUTATION_FAILED".into());
        row.run_finished_at = Some(row.observed_at);
        row.observed_at += Duration::hours(3);
        assert_eq!(output(row).state, DailyStateCode::Failed);
        let mut row = sample();
        reserved(&mut row, "CANCELED");
        row.run_status = Some("CANCELED".into());
        row.run_error = Some("DAILY_SESSION_SUPERSEDED".into());
        row.run_finished_at = Some(row.observed_at);
        assert_eq!(output(row).reason_code, Some(DailyReasonCode::Superseded));
        let mut draining = sample();
        reserved(&mut draining, "CANCELED");
        assert_eq!(output(draining).state, DailyStateCode::Failed);
    }
    #[test]
    fn daily_state_future_receipts_inputs_and_publications_never_become_current() {
        let mut row = sample();
        row.receipt_completed_at = Some(row.observed_at + Duration::seconds(1));
        assert_eq!(
            output(row).reason_code,
            Some(DailyReasonCode::FutureEvidence)
        );
        let mut row = sample();
        row.future_input = true;
        assert_eq!(
            output(row).reason_code,
            Some(DailyReasonCode::FutureEvidence)
        );
        let mut row = sample();
        row.published_at = Some(row.observed_at + Duration::seconds(1));
        assert!(project(row, DailyProcessMode::OwnerOnly).is_err());
    }
    #[test]
    fn daily_state_broken_reservation_receipt_and_publication_witnesses_fail_closed() {
        let mut row = sample();
        reserved(&mut row, "QUEUED");
        row.run_binding = false;
        assert!(project(row, DailyProcessMode::OwnerOnly).is_err());
        let mut row = sample();
        row.published_binding = false;
        assert!(project(row, DailyProcessMode::OwnerOnly).is_err());
        let mut row = sample();
        row.receipt_result = Some("CHECKED".into());
        assert!(project(row, DailyProcessMode::OwnerOnly).is_err());
        let mut row = sample();
        row.rights = false;
        assert!(matches!(
            project(row, DailyProcessMode::OwnerOnly),
            Err(TenancyError::Forbidden)
        ));
    }
}
