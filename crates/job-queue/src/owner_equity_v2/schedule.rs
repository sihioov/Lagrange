//! Confirmed-close incremental scheduling for Owner Equity V2.

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike};
use domain::{CodeCommit, ContentHash};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

const KST_CLOSE_HOUR: u32 = 16;
const KST_CLOSE_MINUTE: u32 = 30;

pub fn eligible_schedule_date(now_kst: DateTime<FixedOffset>) -> Option<NaiveDate> {
    let local_date = now_kst.date_naive();
    if (now_kst.hour(), now_kst.minute()) < (KST_CLOSE_HOUR, KST_CLOSE_MINUTE) {
        local_date.pred_opt()
    } else {
        Some(local_date)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquitySchedulePins {
    code_commit: String,
    entitlement_reference: String,
    entitlement_sha256: String,
}

impl OwnerEquitySchedulePins {
    pub fn new(
        code_commit: String,
        entitlement_reference: String,
        entitlement_sha256: String,
    ) -> Result<Self, OwnerEquityScheduleError> {
        if CodeCommit::parse(&code_commit).is_err()
            || ContentHash::parse(&entitlement_sha256).is_err()
            || entitlement_reference.trim().is_empty()
            || entitlement_reference.len() > 512
            || entitlement_reference.chars().any(char::is_control)
        {
            return Err(OwnerEquityScheduleError::InvalidPins);
        }
        Ok(Self {
            code_commit,
            entitlement_reference,
            entitlement_sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEquityScheduleReport {
    pub as_of: NaiveDate,
    pub scheduled: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerEquityScheduleError {
    #[error("owner equity schedule pins are invalid")]
    InvalidPins,
    #[error("no confirmed published KRX close is available")]
    NoConfirmedClose,
    #[error("owner equity scheduling database is unavailable")]
    Database,
}

#[derive(Debug, FromRow)]
struct ScheduleCandidate {
    owner_user_id: Uuid,
    membership_id: Uuid,
}

/// Enqueue at most one deterministic incremental job per READY membership.
/// PostgreSQL revalidates every pin and owns the idempotent job insertion.
pub async fn run_owner_equity_schedule_cycle(
    pool: &PgPool,
    pins: &OwnerEquitySchedulePins,
    now_kst: DateTime<FixedOffset>,
) -> Result<OwnerEquityScheduleReport, OwnerEquityScheduleError> {
    let latest_eligible =
        eligible_schedule_date(now_kst).ok_or(OwnerEquityScheduleError::NoConfirmedClose)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| OwnerEquityScheduleError::Database)?;
    let as_of: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT calendar.session_date
           FROM public.trading_calendars AS calendar
          WHERE calendar.exchange = 'KRX'
            AND calendar.session_type = 'TRADING'
            AND calendar.timezone = 'Asia/Seoul'
            AND calendar.session_date <= $1
            AND calendar.source_batch_id IS NOT NULL
            AND calendar.content_sha256 IS NOT NULL
            AND calendar.retrieved_at IS NOT NULL
          ORDER BY calendar.session_date DESC LIMIT 1",
    )
    .bind(latest_eligible)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| OwnerEquityScheduleError::Database)?;
    let Some(as_of) = as_of else {
        tx.rollback()
            .await
            .map_err(|_| OwnerEquityScheduleError::Database)?;
        return Err(OwnerEquityScheduleError::NoConfirmedClose);
    };

    let candidates: Vec<ScheduleCandidate> = sqlx::query_as(
        "SELECT membership.owner_user_id, membership.id AS membership_id
           FROM public.owner_equity_memberships AS membership
           JOIN LATERAL (
                SELECT generation.last_session
                  FROM public.owner_equity_instrument_generations AS generation
                  JOIN public.owner_equity_generation_admissions AS admission
                    ON admission.generation_id = generation.id
                   AND admission.owner_user_id = generation.owner_user_id
                   AND admission.membership_id = generation.membership_id
                   AND admission.instrument_id = generation.instrument_id
                   AND admission.generation = generation.generation
                 WHERE generation.owner_user_id = membership.owner_user_id
                   AND generation.membership_id = membership.id
                 ORDER BY generation.generation DESC LIMIT 1
           ) AS latest ON true
          WHERE membership.state = 'READY'
            AND latest.last_session < $1
          ORDER BY membership.owner_user_id, membership.id",
    )
    .bind(as_of)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| OwnerEquityScheduleError::Database)?;

    let mut scheduled = 0;
    for candidate in candidates {
        let result: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT job_id, inserted
               FROM public.schedule_owner_equity_incremental($1, $2, $3, $4, $5, $6)",
        )
        .bind(candidate.owner_user_id)
        .bind(candidate.membership_id)
        .bind(as_of)
        .bind(&pins.code_commit)
        .bind(&pins.entitlement_reference)
        .bind(&pins.entitlement_sha256)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| OwnerEquityScheduleError::Database)?;
        if result.is_some_and(|(_, inserted)| inserted) {
            scheduled += 1;
        }
    }
    tx.commit()
        .await
        .map_err(|_| OwnerEquityScheduleError::Database)?;
    Ok(OwnerEquityScheduleReport { as_of, scheduled })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn cutoff_excludes_the_unconfirmed_current_session() {
        let seoul = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        let before = seoul.with_ymd_and_hms(2026, 8, 31, 16, 29, 59).unwrap();
        let at = seoul.with_ymd_and_hms(2026, 8, 31, 16, 30, 0).unwrap();
        assert_eq!(
            eligible_schedule_date(before).unwrap().to_string(),
            "2026-08-30"
        );
        assert_eq!(
            eligible_schedule_date(at).unwrap().to_string(),
            "2026-08-31"
        );
    }

    #[test]
    fn pins_fail_closed() {
        assert_eq!(
            OwnerEquitySchedulePins::new(String::new(), "ref".into(), "sha256:x".into()),
            Err(OwnerEquityScheduleError::InvalidPins)
        );
        assert!(
            OwnerEquitySchedulePins::new(
                "a".repeat(40),
                "repo://entitlement".into(),
                format!("sha256:{}", "b".repeat(64)),
            )
            .is_ok()
        );
    }
}
