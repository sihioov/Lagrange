//! One close-driven scheduling cycle for opted-in Paper configurations.

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

use super::input::DatasetPin;

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
pub struct ScheduleReport {
    pub as_of: NaiveDate,
    pub scheduled: usize,
}

#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("recommendation scheduling database failure: {0}")]
    Database(#[from] sqlx::Error),
    #[error("no confirmed published KRX close is available")]
    NoConfirmedClose,
    #[error("configured recommendation dataset is not usable or its lineage changed")]
    DatasetUnavailable,
}

#[derive(Debug, FromRow)]
struct Candidate {
    owner_user_id: Uuid,
    strategy_config_id: Uuid,
}

/// Run the startup/daily scheduler once.
///
/// Before 16:30 KST the current local date cannot yet be a confirmed close,
/// so startup catch-up considers only earlier dates. At/after the cutoff the
/// current date is eligible, but only if PostgreSQL holds both published KRX
/// calendar provenance and credentialed EOD evidence for it.
pub async fn run_schedule_cycle(
    pool: PgPool,
    dataset: DatasetPin,
    now_kst: DateTime<FixedOffset>,
) -> Result<ScheduleReport, ScheduleError> {
    let latest_eligible_date =
        eligible_schedule_date(now_kst).ok_or(ScheduleError::NoConfirmedClose)?;

    let mut tx = pool.begin().await?;
    if latest_eligible_date == now_kst.date_naive() {
        let current_session_locked: bool =
            sqlx::query_scalar("SELECT public.lock_recommendation_calendar_coverage($1)")
                .bind(latest_eligible_date)
                .fetch_one(&mut *tx)
                .await?;
        if !current_session_locked {
            tx.rollback().await?;
            return Err(ScheduleError::NoConfirmedClose);
        }
    }
    let as_of: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT calendar.session_date \
           FROM trading_calendars AS calendar \
          WHERE calendar.exchange = 'KRX' \
            AND calendar.session_type = 'TRADING' \
            AND calendar.timezone = 'Asia/Seoul' \
            AND calendar.session_date <= $1 \
            AND calendar.source_batch_id IS NOT NULL \
            AND calendar.content_sha256 IS NOT NULL \
            AND calendar.retrieved_at IS NOT NULL \
          ORDER BY calendar.session_date DESC LIMIT 1",
    )
    .bind(latest_eligible_date)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(as_of) = as_of else {
        tx.rollback().await?;
        return Err(ScheduleError::NoConfirmedClose);
    };

    let dataset_locked: bool =
        sqlx::query_scalar("SELECT public.lock_recommendation_schedule_inputs($1, $2, $3, $4, $5)")
            .bind(as_of)
            .bind(dataset.id)
            .bind(&dataset.dataset_id)
            .bind(&dataset.version)
            .bind(&dataset.manifest_sha256)
            .fetch_one(&mut *tx)
            .await?;
    if !dataset_locked {
        tx.rollback().await?;
        return Err(ScheduleError::DatasetUnavailable);
    }

    let candidates: Vec<Candidate> = sqlx::query_as(
        "SELECT DISTINCT binding.owner_user_id, binding.strategy_config_id \
           FROM account_strategy_bindings AS binding \
           JOIN accounts AS account \
             ON account.id = binding.account_id \
            AND account.owner_user_id = binding.owner_user_id \
           JOIN user_strategy_configs AS config \
             ON config.id = binding.strategy_config_id \
            AND config.owner_user_id = binding.owner_user_id \
          WHERE binding.unbound_at IS NULL \
            AND binding.auto_apply_recommendations \
            AND account.account_type = 'PAPER' \
            AND account.status = 'ACTIVE' \
            AND config.is_active \
          ORDER BY binding.owner_user_id, binding.strategy_config_id",
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut scheduled = 0;
    for candidate in candidates {
        let entitled: bool =
            sqlx::query_scalar("SELECT public.lock_recommendation_entitlement($1, $2, $3)")
                .bind(candidate.owner_user_id)
                .bind(&dataset.dataset_id)
                .bind(as_of)
                .fetch_one(&mut *tx)
                .await?;
        if !entitled {
            continue;
        }
        let identity = format!(
            "{}|{}|{}|{}",
            candidate.owner_user_id, candidate.strategy_config_id, as_of, dataset.id
        );
        let key: String =
            sqlx::query_scalar("SELECT 'recommendation:scheduled:' || pg_catalog.md5($1)")
                .bind(identity)
                .fetch_one(&mut *tx)
                .await?;
        let _: (Uuid, Uuid) = sqlx::query_as(
            "SELECT run_id, job_id FROM public.schedule_recommendation_run(\
                $1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(candidate.owner_user_id)
        .bind(candidate.strategy_config_id)
        .bind(as_of)
        .bind(dataset.id)
        .bind(&dataset.manifest_sha256)
        .bind(i32::try_from(dataset.curated_version).unwrap_or(i32::MAX))
        .bind(key)
        .fetch_one(&mut *tx)
        .await?;
        scheduled += 1;
    }
    tx.commit().await?;
    Ok(ScheduleReport { as_of, scheduled })
}
