//! Close-driven scheduling for the common stock-candidate analysis.

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike, Utc};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

const KST_CLOSE_HOUR: u32 = 16;
const KST_CLOSE_MINUTE: u32 = 30;
const MIN_PRICE_CONTEXT_SESSIONS: i32 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSchedulePin {
    pub id: Uuid,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateScheduleRequest {
    pub as_of_date: NaiveDate,
    pub cutoff_at: DateTime<Utc>,
    pub scoring_config_version: String,
    pub scoring_config_sha256: String,
    pub universe_snapshot_id: Uuid,
    pub price: DatasetSchedulePin,
    pub price_curated_version: u32,
    pub status: DatasetSchedulePin,
    pub flow: DatasetSchedulePin,
    pub fundamental: DatasetSchedulePin,
    pub sector_version_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateScheduleReport {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub computation_seq: i32,
    pub as_of_date: NaiveDate,
}

#[derive(Debug, Error)]
pub enum CandidateScheduleError {
    #[error("candidate scheduling database failure: {0}")]
    Database(#[from] sqlx::Error),
    #[error("candidate scheduling input is invalid: {0}")]
    Invalid(String),
    #[error("no coherent point-in-time candidate source set is available")]
    SourceUnavailable,
}

#[derive(Debug, FromRow)]
struct ConfigRow {
    version: String,
    content_sha256: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct PinRow {
    id: Uuid,
    manifest_sha256: String,
    available_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct PricePinRow {
    id: Uuid,
    manifest_sha256: String,
    available_at: DateTime<Utc>,
    curated_generation: i64,
}

#[derive(Debug, FromRow)]
struct IdentityRow {
    id: Uuid,
    available_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct CalendarRow {
    session_date: NaiveDate,
    retrieved_at: DateTime<Utc>,
}

/// Schedule one fully pinned analysis. PostgreSQL computes the immutable input
/// identity and queue idempotency key; callers cannot choose either value.
pub async fn schedule_candidate_run(
    pool: &PgPool,
    request: &CandidateScheduleRequest,
) -> Result<CandidateScheduleReport, CandidateScheduleError> {
    if request.price_curated_version == 0
        || request.scoring_config_version.trim().is_empty()
        || !is_sha256(&request.scoring_config_sha256)
        || !is_sha256(&request.price.manifest_sha256)
        || !is_sha256(&request.status.manifest_sha256)
        || !is_sha256(&request.flow.manifest_sha256)
        || !is_sha256(&request.fundamental.manifest_sha256)
    {
        return Err(CandidateScheduleError::Invalid(
            "versions and exact lowercase SHA-256 pins are required".to_owned(),
        ));
    }
    let curated_version = i32::try_from(request.price_curated_version).map_err(|_| {
        CandidateScheduleError::Invalid("price curated version exceeds PostgreSQL integer".into())
    })?;
    let (run_id, job_id, computation_seq): (Uuid, Uuid, i32) = sqlx::query_as(
        "SELECT run_id, job_id, computation_seq
           FROM public.schedule_candidate_run(
             $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
           )",
    )
    .bind(request.as_of_date)
    .bind(request.cutoff_at)
    .bind(&request.scoring_config_version)
    .bind(&request.scoring_config_sha256)
    .bind(request.universe_snapshot_id)
    .bind(request.price.id)
    .bind(curated_version)
    .bind(&request.price.manifest_sha256)
    .bind(request.status.id)
    .bind(&request.status.manifest_sha256)
    .bind(request.flow.id)
    .bind(&request.flow.manifest_sha256)
    .bind(request.fundamental.id)
    .bind(&request.fundamental.manifest_sha256)
    .bind(request.sector_version_id)
    .fetch_one(pool)
    .await?;
    Ok(CandidateScheduleReport {
        run_id,
        job_id,
        computation_seq,
        as_of_date: request.as_of_date,
    })
}

/// Discover the newest coherent post-close source set and schedule it. The
/// exact curated Parquet generation comes from the immutable price-publication
/// row; deployment configuration cannot substitute a different generation.
pub async fn schedule_latest_candidate_run(
    pool: &PgPool,
    now_kst: DateTime<FixedOffset>,
) -> Result<CandidateScheduleReport, CandidateScheduleError> {
    let latest_date = if (now_kst.hour(), now_kst.minute()) < (KST_CLOSE_HOUR, KST_CLOSE_MINUTE) {
        now_kst
            .date_naive()
            .pred_opt()
            .ok_or(CandidateScheduleError::SourceUnavailable)?
    } else {
        now_kst.date_naive()
    };
    let calendar: CalendarRow = sqlx::query_as(
        "SELECT calendar.session_date, calendar.retrieved_at
           FROM trading_calendars AS calendar
          WHERE calendar.exchange = 'KRX'
            AND calendar.session_type = 'TRADING'
            AND calendar.timezone = 'Asia/Seoul'
            AND calendar.session_date <= $1
            AND calendar.source_batch_id IS NOT NULL
            AND calendar.content_sha256 IS NOT NULL
            AND calendar.retrieved_at IS NOT NULL
          ORDER BY calendar.session_date DESC LIMIT 1",
    )
    .bind(latest_date)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let as_of_date = calendar.session_date;
    let discovery_at = now_kst.with_timezone(&Utc);
    let required_fetch_mode: String = sqlx::query_scalar(
        "SELECT required_fetch_mode FROM candidate_scheduler_control
          WHERE control_key='scheduler' AND active",
    )
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;

    let config: ConfigRow = sqlx::query_as(
        "SELECT version, content_sha256, created_at
           FROM candidate_scoring_configs
          WHERE created_at <= $1
          ORDER BY created_at DESC, version DESC LIMIT 1",
    )
    .bind(discovery_at)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let universe: IdentityRow = sqlx::query_as(
        "SELECT id, available_at FROM candidate_universe_snapshots
          WHERE index_id = 'kospi200' AND as_of_date <= $1 AND available_at <= $2
            AND member_count = (
                SELECT count(*) FROM candidate_universe_members AS member
                 WHERE member.universe_snapshot_id = candidate_universe_snapshots.id
                   AND member.effective_from <= $1
                   AND (member.effective_until IS NULL OR member.effective_until >= $1))
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=candidate_universe_snapshots.dataset_version_id
                 AND binding.response_kind='index_membership'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
          ORDER BY as_of_date DESC, available_at DESC, id LIMIT 1",
    )
    .bind(as_of_date)
    .bind(discovery_at)
    .bind(&required_fetch_mode)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let price: PricePinRow = sqlx::query_as(
        "SELECT dataset.id, dataset.manifest_sha256, price.available_at,
                price.curated_generation
           FROM candidate_price_publications AS price
           JOIN dataset_versions AS dataset ON dataset.id = price.dataset_version_id
          WHERE dataset.dataset_id = 'krx_eod_bars'
            AND dataset.status IN ('READY', 'WARNING')
            AND price.market = 'kr'
            AND price.first_session <= $1 AND price.last_session >= $1
            AND price.available_at <= $2
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=price.dataset_version_id
                 AND binding.response_kind='bars'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
          ORDER BY price.available_at DESC, dataset.id LIMIT 1",
    )
    .bind(as_of_date)
    .bind(discovery_at)
    .bind(&required_fetch_mode)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let flow: PinRow = sqlx::query_as(
        "SELECT dataset.id, dataset.manifest_sha256,
                max(flow.available_at) AS available_at
           FROM candidate_investor_flows AS flow
           JOIN candidate_investor_flow_snapshot_rows AS member
             ON member.flow_observation_id=flow.id
           JOIN dataset_versions AS dataset ON dataset.id = member.dataset_version_id
          WHERE flow.trade_date = $1 AND flow.available_at <= $2
            AND dataset.status IN ('READY', 'WARNING')
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=dataset.id
                 AND binding.response_kind='investor_flow'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
          GROUP BY dataset.id, dataset.manifest_sha256
          ORDER BY max(flow.available_at) DESC, dataset.id LIMIT 1",
    )
    .bind(as_of_date)
    .bind(discovery_at)
    .bind(&required_fetch_mode)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let status: PinRow = sqlx::query_as(
        "SELECT dataset.id, dataset.manifest_sha256,
                max(status.available_at) AS available_at
           FROM candidate_market_status_observations AS status
           JOIN dataset_versions AS dataset ON dataset.id = status.dataset_version_id
          WHERE status.trade_date = $1 AND status.available_at <= $2
            AND dataset.status IN ('READY', 'WARNING')
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=dataset.id
                 AND binding.response_kind='market_status'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
          GROUP BY dataset.id, dataset.manifest_sha256
          ORDER BY max(status.available_at) DESC, dataset.id LIMIT 1",
    )
    .bind(as_of_date)
    .bind(discovery_at)
    .bind(&required_fetch_mode)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let fundamental: PinRow = sqlx::query_as(
        "SELECT dataset.id, dataset.manifest_sha256,
                max(fact.available_at) AS available_at
           FROM candidate_fundamental_observations AS fact
           JOIN dataset_versions AS dataset ON dataset.id = fact.dataset_version_id
          WHERE fact.fiscal_period_end <= $1 AND fact.available_at <= $2
            AND dataset.status IN ('READY', 'WARNING')
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=dataset.id
                 AND binding.response_kind='fundamentals'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
          GROUP BY dataset.id, dataset.manifest_sha256
          ORDER BY max(fact.available_at) DESC, dataset.id LIMIT 1",
    )
    .bind(as_of_date)
    .bind(discovery_at)
    .bind(&required_fetch_mode)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let sector: IdentityRow = sqlx::query_as(
        "SELECT id, available_at FROM candidate_sector_versions
          WHERE effective_from <= $1 AND available_at <= $2
            AND EXISTS (
                SELECT 1 FROM candidate_raw_batch_datasets AS binding
                JOIN candidate_raw_batch_publications AS batch
                  ON batch.batch_id=binding.batch_id AND batch.surface=binding.surface
               WHERE binding.dataset_version_id=candidate_sector_versions.dataset_version_id
                 AND binding.response_kind='sector_classification'
                 AND batch.state='PUBLISHED' AND batch.fetch_mode=$3)
          ORDER BY effective_from DESC, available_at DESC, id LIMIT 1",
    )
    .bind(as_of_date)
    .bind(discovery_at)
    .bind(&required_fetch_mode)
    .fetch_optional(pool)
    .await?
    .ok_or(CandidateScheduleError::SourceUnavailable)?;
    let viable_count: i64 = sqlx::query_scalar(
        "WITH required_sessions AS MATERIALIZED (
             SELECT calendar.session_date FROM trading_calendars AS calendar
              WHERE calendar.exchange='KRX' AND calendar.session_type='TRADING'
                AND calendar.timezone='Asia/Seoul' AND calendar.session_date <= $1
                AND calendar.source_batch_id IS NOT NULL
                AND calendar.content_sha256 IS NOT NULL AND calendar.retrieved_at IS NOT NULL
              ORDER BY calendar.session_date DESC LIMIT $9)
         SELECT count(*) FROM candidate_universe_members AS member
          WHERE member.universe_snapshot_id=$2
            AND member.effective_from <= $1
            AND (member.effective_until IS NULL OR member.effective_until >= $1)
            AND (SELECT count(*) FROM required_sessions)=$9
            AND NOT EXISTS (
                SELECT 1 FROM required_sessions AS required WHERE NOT EXISTS (
                    SELECT 1 FROM candidate_price_instrument_sessions AS price_session
                     WHERE price_session.dataset_version_id=$3
                       AND price_session.instrument_id=member.instrument_id
                       AND price_session.session_date=required.session_date))
            AND NOT EXISTS (
                SELECT 1 FROM required_sessions AS required
                CROSS JOIN (VALUES ('FOREIGN'),('INSTITUTION')) AS class(investor_class)
                 WHERE NOT EXISTS (
                    SELECT 1 FROM candidate_investor_flows AS history
                    JOIN candidate_investor_flow_snapshot_rows AS flow_member
                      ON flow_member.flow_observation_id=history.id
                     WHERE flow_member.dataset_version_id=$4
                       AND history.instrument_id=member.instrument_id
                       AND history.trade_date=required.session_date
                       AND history.investor_class=class.investor_class
                       AND history.available_at <= $5))
            AND EXISTS (SELECT 1 FROM candidate_market_status_observations AS status
                         WHERE status.dataset_version_id=$6
                           AND status.instrument_id=member.instrument_id
                           AND status.trade_date=$1 AND status.available_at <= $5)
            AND EXISTS (SELECT 1 FROM candidate_fundamental_observations AS fact
                         WHERE fact.dataset_version_id=$7
                           AND fact.instrument_id=member.instrument_id
                           AND fact.fiscal_period_end <= $1 AND fact.available_at <= $5)
            AND EXISTS (SELECT 1 FROM candidate_sector_entries AS entry
                         WHERE entry.sector_version_id=$8
                           AND entry.instrument_id=member.instrument_id
                           AND entry.effective_from <= $1
                           AND entry.available_at <= $5
                           AND (entry.effective_until IS NULL OR entry.effective_until >= $1))",
    )
    .bind(as_of_date)
    .bind(universe.id)
    .bind(price.id)
    .bind(flow.id)
    .bind(discovery_at)
    .bind(status.id)
    .bind(fundamental.id)
    .bind(sector.id)
    .bind(MIN_PRICE_CONTEXT_SESSIONS)
    .fetch_one(pool)
    .await?;
    if viable_count < 5 {
        return Err(CandidateScheduleError::SourceUnavailable);
    }
    let cutoff_at = canonical_cutoff([
        calendar.retrieved_at,
        config.created_at,
        universe.available_at,
        price.available_at,
        status.available_at,
        flow.available_at,
        fundamental.available_at,
        sector.available_at,
    ]);
    let price_curated_version = u32::try_from(price.curated_generation)
        .ok()
        .filter(|generation| *generation > 0)
        .ok_or_else(|| {
            CandidateScheduleError::Invalid(
                "published price curated generation exceeds the worker contract".to_owned(),
            )
        })?;

    schedule_candidate_run(
        pool,
        &CandidateScheduleRequest {
            as_of_date,
            cutoff_at,
            scoring_config_version: config.version,
            scoring_config_sha256: config.content_sha256,
            universe_snapshot_id: universe.id,
            price: DatasetSchedulePin {
                id: price.id,
                manifest_sha256: price.manifest_sha256,
            },
            price_curated_version,
            status: DatasetSchedulePin {
                id: status.id,
                manifest_sha256: status.manifest_sha256,
            },
            flow: DatasetSchedulePin {
                id: flow.id,
                manifest_sha256: flow.manifest_sha256,
            },
            fundamental: DatasetSchedulePin {
                id: fundamental.id,
                manifest_sha256: fundamental.manifest_sha256,
            },
            sector_version_id: sector.id,
        },
    )
    .await
}

fn canonical_cutoff<const N: usize>(timestamps: [DateTime<Utc>; N]) -> DateTime<Utc> {
    timestamps
        .into_iter()
        .max()
        .expect("candidate cutoff always has pinned inputs")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_strict_lowercase() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"A".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
    }

    #[test]
    fn canonical_cutoff_depends_on_pins_not_poll_clock() {
        let earlier = DateTime::parse_from_rfc3339("2026-08-14T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let latest = DateTime::parse_from_rfc3339("2026-08-14T07:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(canonical_cutoff([earlier, latest, earlier]), latest);
    }
}
