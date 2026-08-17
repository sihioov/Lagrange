//! Strict candidate payload parsing and point-in-time database attestation.

use chrono::{DateTime, NaiveDate, Utc};
use domain::{InstrumentId, TradingDate};
use factor_engine::{CandidateFlags, CandidateScoringConfig};
use market_data::FundamentalProfile;
use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

use super::CandidateUniverseKey;

fn default_universe_key() -> CandidateUniverseKey {
    CandidateUniverseKey::Kospi200
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePayload {
    pub run_id: Uuid,
    /// Omitted legacy payloads retain the documented KOSPI200 meaning.
    #[serde(default = "default_universe_key")]
    pub universe_key: CandidateUniverseKey,
    #[serde(deserialize_with = "deserialize_date")]
    pub as_of_date: NaiveDate,
    pub cutoff_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_nonempty")]
    pub scoring_config_version: String,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub scoring_config_sha256: String,
    pub universe_snapshot_id: Uuid,
    pub universe_entitlement_id: Uuid,
    pub price_dataset_version_id: Uuid,
    pub price_entitlement_id: Uuid,
    #[serde(deserialize_with = "deserialize_positive_u32")]
    pub price_curated_version: u32,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub price_manifest_sha256: String,
    pub status_dataset_version_id: Uuid,
    pub status_entitlement_id: Uuid,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub status_manifest_sha256: String,
    pub flow_dataset_version_id: Uuid,
    pub flow_entitlement_id: Uuid,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub flow_manifest_sha256: String,
    pub fundamental_dataset_version_id: Uuid,
    pub fundamental_entitlement_id: Uuid,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub fundamental_manifest_sha256: String,
    pub sector_version_id: Uuid,
    pub sector_entitlement_id: Uuid,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub input_identity_sha256: String,
}

impl TryFrom<serde_json::Value> for CandidatePayload {
    type Error = CandidateInputError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map_err(|error| CandidateInputError::Malformed {
            detail: error.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttestedPriceDataset {
    pub dataset_id: String,
    pub storage_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateMemberSource {
    pub instrument: InstrumentId,
    pub sector_code: String,
    pub fundamental_profile: FundamentalProfile,
    pub flags: CandidateFlags,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateFlowSource {
    pub instrument: InstrumentId,
    pub trade_date: TradingDate,
    pub investor_class: String,
    pub net_amount: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateFundamentalSource {
    pub instrument: InstrumentId,
    pub metric: String,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct AttestedCandidateInput {
    pub payload: CandidatePayload,
    pub scoring: CandidateScoringConfig,
    pub price: AttestedPriceDataset,
    pub members: Vec<CandidateMemberSource>,
    pub flows: Vec<CandidateFlowSource>,
    pub fundamentals: Vec<CandidateFundamentalSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CandidateInputError {
    #[error("malformed candidate payload: {detail}")]
    Malformed { detail: String },
    #[error("candidate input does not exist")]
    NotFound,
    #[error("candidate input integrity failure: {detail}")]
    Integrity { detail: String },
    #[error("candidate source data is blocked: {detail}")]
    DataBlocked { detail: String },
    #[error("candidate input is temporarily unavailable: {detail}")]
    Unavailable { detail: String },
}

impl CandidateInputError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Malformed { .. } => "CANDIDATE_INPUT_MALFORMED",
            Self::NotFound => "CANDIDATE_INPUT_NOT_FOUND",
            Self::Integrity { .. } => "CANDIDATE_INPUT_INTEGRITY",
            Self::DataBlocked { .. } => "CANDIDATE_DATA_BLOCKED",
            Self::Unavailable { .. } => "CANDIDATE_INPUT_UNAVAILABLE",
        }
    }

    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

#[derive(Debug, FromRow)]
struct RunRow {
    job_id: Option<Uuid>,
    as_of_date: NaiveDate,
    cutoff_at: DateTime<Utc>,
    status: String,
    scoring_config_version: String,
    scoring_config_sha256: String,
    universe_snapshot_id: Uuid,
    universe_key: String,
    universe_entitlement_id: Uuid,
    price_dataset_version_id: Uuid,
    price_entitlement_id: Uuid,
    price_curated_version: i32,
    price_manifest_sha256: String,
    status_dataset_version_id: Uuid,
    status_entitlement_id: Uuid,
    status_manifest_sha256: String,
    flow_dataset_version_id: Uuid,
    flow_entitlement_id: Uuid,
    flow_manifest_sha256: String,
    fundamental_dataset_version_id: Uuid,
    fundamental_entitlement_id: Uuid,
    fundamental_manifest_sha256: String,
    sector_version_id: Uuid,
    sector_entitlement_id: Uuid,
    input_identity_sha256: String,
}

#[derive(Debug, FromRow)]
struct ConfigRow {
    canonical_json: String,
    config_json: serde_json::Value,
    content_sha256: String,
}

#[derive(Debug, FromRow)]
struct DatasetRow {
    id: Uuid,
    dataset_id: String,
    status: String,
    manifest_sha256: String,
    storage_path: String,
}

#[derive(Debug, FromRow)]
struct MemberRow {
    instrument_id: String,
    sector_code: String,
    fundamental_profile: String,
    instrument_status: String,
    membership_eligible: bool,
    status_found: bool,
    flow_found: bool,
    suspended: bool,
    administrative: bool,
    liquidation: bool,
    status_inactive: bool,
    disqualifying_audit_opinion: bool,
    complete_capital_impairment: bool,
}

#[derive(Debug, FromRow)]
struct FlowRow {
    instrument_id: String,
    trade_date: NaiveDate,
    investor_class: String,
    net_amount: f64,
}

#[derive(Debug, FromRow)]
struct FundamentalRow {
    instrument_id: String,
    metric: String,
    value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredConfig {
    context_sessions: Vec<u32>,
    evidence: StoredEvidence,
    financial_sector_profile: String,
    min_average_trading_value_20: f64,
    primary_horizon_sessions: u32,
    sector_min_members: usize,
    weights: StoredWeights,
    winsorize: StoredWinsor,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEvidence {
    axis_min_coverage: f64,
    strong_coverage: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWeights {
    flow: f64,
    fundamental: f64,
    technical: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWinsor {
    lower: f64,
    upper: f64,
}

pub(crate) async fn attest_candidate_input(
    pool: &PgPool,
    claimed_job_id: Uuid,
    claimed_owner_user_id: Uuid,
    payload: CandidatePayload,
) -> Result<AttestedCandidateInput, CandidateInputError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| unavailable("begin attestation", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| unavailable("configure attestation snapshot", error))?;

    let service_owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT service_user_id FROM candidate_scheduler_control
          WHERE control_key = 'scheduler' AND active",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("read scheduler control", error))?;
    require(
        service_owner == Some(claimed_owner_user_id),
        "claim owner is not the active candidate service principal",
    )?;

    let run: RunRow = sqlx::query_as(
        "SELECT job_id, as_of_date, cutoff_at, status,
                scoring_config_version, scoring_config_sha256,
                universe_snapshot_id, universe_key, universe_entitlement_id,
                price_dataset_version_id, price_entitlement_id,
                price_curated_version, price_manifest_sha256,
                status_dataset_version_id, status_entitlement_id, status_manifest_sha256,
                flow_dataset_version_id, flow_entitlement_id, flow_manifest_sha256,
                fundamental_dataset_version_id, fundamental_entitlement_id,
                fundamental_manifest_sha256,
                sector_version_id, sector_entitlement_id, input_identity_sha256
           FROM stock_analysis_runs WHERE id = $1",
    )
    .bind(payload.run_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("read candidate run", error))?
    .ok_or(CandidateInputError::NotFound)?;
    attest_run(&run, claimed_job_id, &payload)?;

    let config: ConfigRow = sqlx::query_as(
        "SELECT canonical_json, config_json, content_sha256
           FROM candidate_scoring_configs WHERE version = $1",
    )
    .bind(&payload.scoring_config_version)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("read scoring config", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned scoring config is missing".into(),
    })?;
    let scoring = attest_config(&payload, config)?;

    let price = attest_dataset(
        &mut tx,
        payload.price_dataset_version_id,
        &payload.price_manifest_sha256,
        "price",
    )
    .await?;
    let status = attest_dataset(
        &mut tx,
        payload.status_dataset_version_id,
        &payload.status_manifest_sha256,
        "market status",
    )
    .await?;
    let flow = attest_dataset(
        &mut tx,
        payload.flow_dataset_version_id,
        &payload.flow_manifest_sha256,
        "flow",
    )
    .await?;
    let fundamental = attest_dataset(
        &mut tx,
        payload.fundamental_dataset_version_id,
        &payload.fundamental_manifest_sha256,
        "fundamental",
    )
    .await?;
    let price_license_ref: String = sqlx::query_scalar(
        "SELECT price.license_ref
           FROM candidate_price_publications AS price
          WHERE price.dataset_version_id = $1
            AND price.entitlement_id = $2
            AND price.manifest_sha256 = $3
            AND price.curated_generation = $4
            AND price.first_session <= $5 AND price.last_session >= $5
            AND price.available_at <= $6",
    )
    .bind(payload.price_dataset_version_id)
    .bind(payload.price_entitlement_id)
    .bind(&payload.price_manifest_sha256)
    .bind(i32::try_from(payload.price_curated_version).unwrap_or(i32::MAX))
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("attest price publication", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned price publication is unavailable at cutoff".into(),
    })?;
    let status_license_ref: String = sqlx::query_scalar(
        "SELECT status.license_ref
           FROM candidate_market_status_observations AS status
          WHERE status.dataset_version_id = $1 AND status.entitlement_id = $2
            AND status.trade_date = $3 AND status.available_at <= $4
          ORDER BY status.available_at DESC, status.id LIMIT 1",
    )
    .bind(payload.status_dataset_version_id)
    .bind(payload.status_entitlement_id)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("attest market-status source", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned market-status source is unavailable at cutoff".into(),
    })?;
    let flow_license_ref: String = sqlx::query_scalar(
        "SELECT member.license_ref
           FROM candidate_investor_flows AS flow
           JOIN candidate_investor_flow_snapshot_rows AS member
             ON member.flow_observation_id=flow.id
          WHERE member.dataset_version_id = $1 AND member.entitlement_id = $2
            AND flow.trade_date = $3 AND flow.available_at <= $4
          ORDER BY flow.available_at DESC, flow.id LIMIT 1",
    )
    .bind(payload.flow_dataset_version_id)
    .bind(payload.flow_entitlement_id)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("attest flow source", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned flow source is unavailable at cutoff".into(),
    })?;
    let fundamental_license_ref: String = sqlx::query_scalar(
        "SELECT fact.license_ref
           FROM candidate_fundamental_observations AS fact
          WHERE fact.dataset_version_id = $1 AND fact.entitlement_id = $2
            AND fact.fiscal_period_end <= $3 AND fact.available_at <= $4
          ORDER BY fact.available_at DESC, fact.id LIMIT 1",
    )
    .bind(payload.fundamental_dataset_version_id)
    .bind(payload.fundamental_entitlement_id)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("attest fundamental source", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned fundamental source is unavailable at cutoff".into(),
    })?;
    let (universe_dataset_id, universe_license_ref): (String, String) = sqlx::query_as(
        "SELECT dataset.dataset_id, universe.license_ref
           FROM candidate_universe_snapshots AS universe
           JOIN dataset_versions AS dataset ON dataset.id = universe.dataset_version_id
          WHERE universe.id = $1 AND universe.as_of_date <= $2
            AND universe.entitlement_id = $4
            AND universe.available_at <= $3
            AND universe.member_count = (
                SELECT count(*) FROM candidate_universe_members AS member
                 WHERE member.universe_snapshot_id = universe.id
                   AND member.effective_from <= $2
                   AND (member.effective_until IS NULL OR member.effective_until >= $2))
            AND dataset.manifest_sha256 = universe.manifest_sha256
            AND dataset.status IN ('READY', 'WARNING')",
    )
    .bind(payload.universe_snapshot_id)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .bind(payload.universe_entitlement_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("attest universe", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned universe is unavailable at cutoff".into(),
    })?;
    let (sector_dataset_id, sector_license_ref): (String, String) = sqlx::query_as(
        "SELECT dataset.dataset_id, sector.license_ref
           FROM candidate_sector_versions AS sector
           JOIN dataset_versions AS dataset ON dataset.id = sector.dataset_version_id
          WHERE sector.id = $1 AND sector.effective_from <= $2
            AND sector.entitlement_id = $4
            AND sector.available_at <= $3
            AND dataset.manifest_sha256 = sector.manifest_sha256
            AND dataset.status IN ('READY', 'WARNING')",
    )
    .bind(payload.sector_version_id)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .bind(payload.sector_entitlement_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| unavailable("attest sector version", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: "pinned sector version is unavailable at cutoff".into(),
    })?;
    let (required_first_session, required_session_count): (Option<NaiveDate>, i64) =
        sqlx::query_as(
            "SELECT min(required.session_date), count(*)
               FROM (
                   SELECT calendar.session_date
                     FROM trading_calendars AS calendar
                    WHERE calendar.exchange='KRX'
                      AND calendar.session_type='TRADING'
                      AND calendar.timezone='Asia/Seoul'
                      AND calendar.session_date <= $1
                      AND calendar.source_batch_id IS NOT NULL
                      AND calendar.content_sha256 IS NOT NULL
                      AND calendar.retrieved_at IS NOT NULL
                    ORDER BY calendar.session_date DESC LIMIT 60
               ) AS required",
        )
        .bind(payload.as_of_date)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| unavailable("derive candidate entitlement window", error))?;
    let required_first_session = required_first_session
        .filter(|_| required_session_count == 60)
        .ok_or_else(|| CandidateInputError::DataBlocked {
            detail: "candidate input requires 60 confirmed KRX sessions".into(),
        })?;
    for (entitlement_id, license_ref, dataset_id, first_use_date) in [
        (
            payload.price_entitlement_id,
            price_license_ref.as_str(),
            price.dataset_id.as_str(),
            required_first_session,
        ),
        (
            payload.status_entitlement_id,
            status_license_ref.as_str(),
            status.dataset_id.as_str(),
            payload.as_of_date,
        ),
        (
            payload.flow_entitlement_id,
            flow_license_ref.as_str(),
            flow.dataset_id.as_str(),
            required_first_session,
        ),
        (
            payload.fundamental_entitlement_id,
            fundamental_license_ref.as_str(),
            fundamental.dataset_id.as_str(),
            payload.as_of_date,
        ),
        (
            payload.universe_entitlement_id,
            universe_license_ref.as_str(),
            universe_dataset_id.as_str(),
            payload.as_of_date,
        ),
        (
            payload.sector_entitlement_id,
            sector_license_ref.as_str(),
            sector_dataset_id.as_str(),
            payload.as_of_date,
        ),
    ] {
        let entitled: bool = sqlx::query_scalar(
            "SELECT public.candidate_source_entitlement_is_valid($1, $2, $3, $4, $5)",
        )
        .bind(entitlement_id)
        .bind(license_ref)
        .bind(dataset_id)
        .bind(first_use_date)
        .bind(payload.as_of_date)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| unavailable("re-attest candidate entitlement", error))?;
        if !entitled {
            return Err(CandidateInputError::DataBlocked {
                detail: format!("candidate entitlement is inactive for {dataset_id}"),
            });
        }
    }

    let member_rows: Vec<MemberRow> = sqlx::query_as(
        "SELECT member.instrument_id,
                COALESCE(sector.sector_code, 'UNCLASSIFIED') AS sector_code,
                COALESCE(sector.fundamental_profile, 'UNSUPPORTED') AS fundamental_profile,
                instrument.status AS instrument_status,
                (member.available_at <= $3 AND member.announced_at <= $3
                 AND member.effective_from <= $2
                 AND (member.effective_until IS NULL OR member.effective_until >= $2))
                    AS membership_eligible,
                (market_status.id IS NOT NULL) AS status_found,
                (SELECT count(DISTINCT daily_flow.investor_class) = 2
                   FROM candidate_investor_flows AS daily_flow
                   JOIN candidate_investor_flow_snapshot_rows AS flow_member
                     ON flow_member.flow_observation_id=daily_flow.id
                   JOIN dataset_versions AS flow_dataset
                     ON flow_dataset.id=flow_member.dataset_version_id
                  WHERE daily_flow.instrument_id = member.instrument_id
                    AND flow_member.dataset_version_id = $7
                    AND flow_dataset.manifest_sha256 = $8
                    AND flow_member.entitlement_id = $9
                    AND daily_flow.trade_date = $2
                    AND daily_flow.available_at <= $3) AS flow_found,
                COALESCE(market_status.suspended, false) AS suspended,
                COALESCE(market_status.administrative, false) AS administrative,
                COALESCE(market_status.liquidation, false) AS liquidation,
                COALESCE(market_status.inactive, false) AS status_inactive,
                COALESCE(market_status.disqualifying_audit_opinion, false)
                    AS disqualifying_audit_opinion,
                COALESCE(market_status.complete_capital_impairment, false)
                    AS complete_capital_impairment
           FROM candidate_universe_members AS member
           JOIN instruments AS instrument ON instrument.id = member.instrument_id
           LEFT JOIN candidate_sector_entries AS sector
             ON sector.sector_version_id = $4
            AND sector.instrument_id = member.instrument_id
            AND sector.effective_from <= $2
            AND (sector.effective_until IS NULL OR sector.effective_until >= $2)
            AND sector.available_at <= $3
           LEFT JOIN LATERAL (
                SELECT status.*
                  FROM candidate_market_status_observations AS status
                 WHERE status.instrument_id = member.instrument_id
                   AND status.dataset_version_id = $5
                   AND status.manifest_sha256 = $6
                   AND status.entitlement_id = $10
                   AND status.trade_date = $2
                   AND status.available_at <= $3
                 ORDER BY status.trade_date DESC, status.available_at DESC,
                          status.source_revision DESC, status.id DESC
                 LIMIT 1
           ) AS market_status ON true
          WHERE member.universe_snapshot_id = $1
          ORDER BY member.instrument_id",
    )
    .bind(payload.universe_snapshot_id)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .bind(payload.sector_version_id)
    .bind(payload.status_dataset_version_id)
    .bind(&payload.status_manifest_sha256)
    .bind(payload.flow_dataset_version_id)
    .bind(&payload.flow_manifest_sha256)
    .bind(payload.flow_entitlement_id)
    .bind(payload.status_entitlement_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| unavailable("read candidate members", error))?;
    if member_rows.is_empty() {
        return Err(CandidateInputError::DataBlocked {
            detail: "pinned universe contains no members".into(),
        });
    }

    let flow_rows: Vec<FlowRow> = sqlx::query_as(
        "SELECT DISTINCT ON (flow.instrument_id, flow.trade_date, flow.investor_class)
                flow.instrument_id, flow.trade_date, flow.investor_class,
                flow.net_amount::double precision AS net_amount
           FROM candidate_investor_flows AS flow
           JOIN candidate_investor_flow_snapshot_rows AS flow_member
             ON flow_member.flow_observation_id=flow.id
           JOIN dataset_versions AS flow_dataset
             ON flow_dataset.id=flow_member.dataset_version_id
           JOIN candidate_universe_members AS member
             ON member.universe_snapshot_id = $1
            AND member.instrument_id = flow.instrument_id
          WHERE flow_member.dataset_version_id = $2 AND flow_dataset.manifest_sha256 = $3
            AND flow_member.entitlement_id = $6
            AND flow.trade_date <= $4 AND flow.trade_date >= $4 - 180
            AND flow.available_at <= $5
          ORDER BY flow.instrument_id, flow.trade_date, flow.investor_class,
                   flow.available_at DESC, flow.source_revision DESC, flow.id DESC",
    )
    .bind(payload.universe_snapshot_id)
    .bind(payload.flow_dataset_version_id)
    .bind(&payload.flow_manifest_sha256)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .bind(payload.flow_entitlement_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| unavailable("read point-in-time flow", error))?;

    let fundamental_rows: Vec<FundamentalRow> = sqlx::query_as(
        "SELECT DISTINCT ON (fact.instrument_id, fact.metric)
                fact.instrument_id, fact.metric,
                fact.value::double precision AS value
           FROM candidate_fundamental_observations AS fact
           JOIN candidate_universe_members AS member
             ON member.universe_snapshot_id = $1
            AND member.instrument_id = fact.instrument_id
          WHERE fact.dataset_version_id = $2 AND fact.manifest_sha256 = $3
            AND fact.entitlement_id = $6
            AND fact.fiscal_period_end <= $4
            AND fact.disclosed_at <= $5 AND fact.available_at <= $5
          ORDER BY fact.instrument_id, fact.metric, fact.fiscal_period_end DESC,
                   fact.disclosed_at DESC, fact.available_at DESC,
                   fact.source_revision DESC, fact.id DESC",
    )
    .bind(payload.universe_snapshot_id)
    .bind(payload.fundamental_dataset_version_id)
    .bind(&payload.fundamental_manifest_sha256)
    .bind(payload.as_of_date)
    .bind(payload.cutoff_at)
    .bind(payload.fundamental_entitlement_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| unavailable("read point-in-time fundamentals", error))?;

    tx.commit()
        .await
        .map_err(|error| unavailable("commit attestation", error))?;

    let members = member_rows
        .into_iter()
        .map(parse_member)
        .collect::<Result<Vec<_>, _>>()?;
    let flows = flow_rows
        .into_iter()
        .map(|row| {
            Ok(CandidateFlowSource {
                instrument: parse_instrument(&row.instrument_id)?,
                trade_date: parse_date(row.trade_date)?,
                investor_class: row.investor_class,
                net_amount: row.net_amount,
            })
        })
        .collect::<Result<Vec<_>, CandidateInputError>>()?;
    let fundamentals = fundamental_rows
        .into_iter()
        .map(|row| {
            Ok(CandidateFundamentalSource {
                instrument: parse_instrument(&row.instrument_id)?,
                metric: row.metric,
                value: row.value,
            })
        })
        .collect::<Result<Vec<_>, CandidateInputError>>()?;

    Ok(AttestedCandidateInput {
        payload,
        scoring,
        price: AttestedPriceDataset {
            dataset_id: price.dataset_id,
            storage_path: price.storage_path,
        },
        members,
        flows,
        fundamentals,
    })
}

fn attest_run(
    run: &RunRow,
    claimed_job_id: Uuid,
    payload: &CandidatePayload,
) -> Result<(), CandidateInputError> {
    require(
        run.job_id == Some(claimed_job_id),
        "run job does not match claim",
    )?;
    require(run.status == "PENDING", "run is not PENDING")?;
    require(run.as_of_date == payload.as_of_date, "run as-of mismatch")?;
    require(run.cutoff_at == payload.cutoff_at, "run cutoff mismatch")?;
    require(
        run.scoring_config_version == payload.scoring_config_version
            && run.scoring_config_sha256 == payload.scoring_config_sha256,
        "run scoring config mismatch",
    )?;
    require(
        run.universe_snapshot_id == payload.universe_snapshot_id
            && run.universe_entitlement_id == payload.universe_entitlement_id,
        "run universe mismatch",
    )?;
    let run_universe = CandidateUniverseKey::parse(&run.universe_key).ok_or_else(|| {
        CandidateInputError::Integrity {
            detail: format!("run has unknown universe {}", run.universe_key),
        }
    })?;
    require(
        run_universe == payload.universe_key,
        "run universe key mismatch",
    )?;
    require(
        run.price_dataset_version_id == payload.price_dataset_version_id
            && run.price_entitlement_id == payload.price_entitlement_id
            && run.price_curated_version
                == i32::try_from(payload.price_curated_version).unwrap_or(i32::MAX)
            && run.price_manifest_sha256 == payload.price_manifest_sha256,
        "run price pin mismatch",
    )?;
    require(
        run.status_dataset_version_id == payload.status_dataset_version_id
            && run.status_entitlement_id == payload.status_entitlement_id
            && run.status_manifest_sha256 == payload.status_manifest_sha256,
        "run status pin mismatch",
    )?;
    require(
        run.flow_dataset_version_id == payload.flow_dataset_version_id
            && run.flow_entitlement_id == payload.flow_entitlement_id
            && run.flow_manifest_sha256 == payload.flow_manifest_sha256,
        "run flow pin mismatch",
    )?;
    require(
        run.fundamental_dataset_version_id == payload.fundamental_dataset_version_id
            && run.fundamental_entitlement_id == payload.fundamental_entitlement_id
            && run.fundamental_manifest_sha256 == payload.fundamental_manifest_sha256,
        "run fundamental pin mismatch",
    )?;
    require(
        run.sector_version_id == payload.sector_version_id
            && run.sector_entitlement_id == payload.sector_entitlement_id,
        "run sector pin mismatch",
    )?;
    require(
        run.input_identity_sha256 == payload.input_identity_sha256,
        "run input identity mismatch",
    )
}

fn attest_config(
    payload: &CandidatePayload,
    row: ConfigRow,
) -> Result<CandidateScoringConfig, CandidateInputError> {
    require(
        row.content_sha256 == payload.scoring_config_sha256,
        "scoring config hash mismatch",
    )?;
    let hash = lower_hex(&Sha256::digest(row.canonical_json.as_bytes()));
    require(
        hash == row.content_sha256,
        "scoring config content hash mismatch",
    )?;
    let canonical_value: serde_json::Value =
        serde_json::from_str(&row.canonical_json).map_err(|_| CandidateInputError::Integrity {
            detail: "scoring canonical JSON is invalid".into(),
        })?;
    require(
        canonical_value == row.config_json,
        "scoring canonical JSON mismatch",
    )?;
    let stored: StoredConfig = serde_json::from_value(row.config_json).map_err(|error| {
        CandidateInputError::Integrity {
            detail: format!("scoring config schema is invalid: {error}"),
        }
    })?;
    require(
        stored.context_sessions == [5, 60]
            && stored.primary_horizon_sessions == 20
            && stored.sector_min_members == 8
            && stored.financial_sector_profile == "candidate-financial-v1"
            && (stored.evidence.axis_min_coverage - 0.60).abs() < 1e-12
            && (stored.evidence.strong_coverage - 0.80).abs() < 1e-12,
        "unsupported scoring config semantics",
    )?;
    Ok(CandidateScoringConfig {
        version: payload.scoring_config_version.clone(),
        flow_weight: stored.weights.flow,
        fundamental_weight: stored.weights.fundamental,
        technical_weight: stored.weights.technical,
        min_average_trading_value_20: stored.min_average_trading_value_20,
        winsor_lower: stored.winsorize.lower,
        winsor_upper: stored.winsorize.upper,
    })
}

async fn attest_dataset(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    manifest_sha256: &str,
    label: &str,
) -> Result<DatasetRow, CandidateInputError> {
    let row: DatasetRow = sqlx::query_as(
        "SELECT id, dataset_id, status, manifest_sha256, storage_path
           FROM dataset_versions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| unavailable("read dataset pin", error))?
    .ok_or_else(|| CandidateInputError::DataBlocked {
        detail: format!("pinned {label} dataset is missing"),
    })?;
    require(row.id == id, "dataset id mismatch")?;
    require(
        row.manifest_sha256 == manifest_sha256,
        "dataset manifest mismatch",
    )?;
    if !matches!(row.status.as_str(), "READY" | "WARNING") {
        return Err(CandidateInputError::DataBlocked {
            detail: format!("pinned {label} dataset is not usable"),
        });
    }
    Ok(row)
}

fn parse_member(row: MemberRow) -> Result<CandidateMemberSource, CandidateInputError> {
    let fundamental_profile = match row.fundamental_profile.as_str() {
        "NON_FINANCIAL" => FundamentalProfile::NonFinancial,
        "FINANCIAL" => FundamentalProfile::Financial,
        "UNSUPPORTED" => FundamentalProfile::Unsupported,
        _ => {
            return Err(CandidateInputError::Integrity {
                detail: "unknown fundamental profile".into(),
            });
        }
    };
    Ok(CandidateMemberSource {
        instrument: parse_instrument(&row.instrument_id)?,
        sector_code: row.sector_code,
        fundamental_profile,
        flags: CandidateFlags {
            suspended: row.suspended,
            administrative: row.administrative,
            liquidation: row.liquidation,
            inactive: row.status_inactive
                || row.instrument_status != "ACTIVE"
                || !row.membership_eligible,
            disqualifying_audit_opinion: row.disqualifying_audit_opinion,
            complete_capital_impairment: row.complete_capital_impairment,
            data_stale: !row.status_found || !row.flow_found,
            entitlement_active: true,
            fundamental_profile_supported: fundamental_profile != FundamentalProfile::Unsupported,
        },
    })
}

fn parse_instrument(value: &str) -> Result<InstrumentId, CandidateInputError> {
    InstrumentId::parse(value).map_err(|_| CandidateInputError::Integrity {
        detail: format!("database contains invalid instrument id {value:?}"),
    })
}

fn parse_date(value: NaiveDate) -> Result<TradingDate, CandidateInputError> {
    TradingDate::parse(&value.format("%Y-%m-%d").to_string()).map_err(|_| {
        CandidateInputError::Integrity {
            detail: "database contains invalid trading date".into(),
        }
    })
}

fn require(matches: bool, detail: &'static str) -> Result<(), CandidateInputError> {
    if matches {
        Ok(())
    } else {
        Err(CandidateInputError::Integrity {
            detail: detail.into(),
        })
    }
}

fn unavailable(context: &'static str, error: sqlx::Error) -> CandidateInputError {
    CandidateInputError::Unavailable {
        detail: format!("{context}: {error}"),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

fn deserialize_date<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
    {
        return Err(de::Error::custom("date must use YYYY-MM-DD"));
    }
    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(de::Error::custom)
}

fn deserialize_nonempty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(de::Error::custom("value must not be empty"));
    }
    Ok(value)
}

fn deserialize_positive_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value == 0 {
        return Err(de::Error::custom("value must be positive"));
    }
    Ok(value)
}

fn deserialize_sha256<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(de::Error::custom("value must be lowercase 64-hex SHA-256"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn payload_rejects_unknown_fields_and_noncanonical_hashes() {
        let payload = json!({
            "run_id": Uuid::nil(),
            "as_of_date": "2026-08-14",
            "cutoff_at": "2026-08-14T08:00:00Z",
            "scoring_config_version": "candidate-score-v1",
            "scoring_config_sha256": "A".repeat(64),
            "universe_snapshot_id": Uuid::nil(),
            "universe_entitlement_id": Uuid::nil(),
            "price_dataset_version_id": Uuid::nil(),
            "price_entitlement_id": Uuid::nil(),
            "price_curated_version": 1,
            "price_manifest_sha256": "a".repeat(64),
            "status_dataset_version_id": Uuid::nil(),
            "status_entitlement_id": Uuid::nil(),
            "status_manifest_sha256": "a".repeat(64),
            "flow_dataset_version_id": Uuid::nil(),
            "flow_entitlement_id": Uuid::nil(),
            "flow_manifest_sha256": "a".repeat(64),
            "fundamental_dataset_version_id": Uuid::nil(),
            "fundamental_entitlement_id": Uuid::nil(),
            "fundamental_manifest_sha256": "a".repeat(64),
            "sector_version_id": Uuid::nil(),
            "sector_entitlement_id": Uuid::nil(),
            "input_identity_sha256": "a".repeat(64)
        });
        assert!(CandidatePayload::try_from(payload).is_err());
    }

    #[test]
    fn payload_preserves_explicit_universe_key_and_legacy_default() {
        let mut payload = json!({
            "run_id": Uuid::nil(),
            "as_of_date": "2026-08-14",
            "cutoff_at": "2026-08-14T08:00:00Z",
            "scoring_config_version": "candidate-score-v1",
            "scoring_config_sha256": "a".repeat(64),
            "universe_snapshot_id": Uuid::nil(),
            "universe_entitlement_id": Uuid::nil(),
            "price_dataset_version_id": Uuid::nil(),
            "price_entitlement_id": Uuid::nil(),
            "price_curated_version": 1,
            "price_manifest_sha256": "a".repeat(64),
            "status_dataset_version_id": Uuid::nil(),
            "status_entitlement_id": Uuid::nil(),
            "status_manifest_sha256": "a".repeat(64),
            "flow_dataset_version_id": Uuid::nil(),
            "flow_entitlement_id": Uuid::nil(),
            "flow_manifest_sha256": "a".repeat(64),
            "fundamental_dataset_version_id": Uuid::nil(),
            "fundamental_entitlement_id": Uuid::nil(),
            "fundamental_manifest_sha256": "a".repeat(64),
            "sector_version_id": Uuid::nil(),
            "sector_entitlement_id": Uuid::nil(),
            "input_identity_sha256": "a".repeat(64)
        });

        let legacy = CandidatePayload::try_from(payload.clone()).expect("legacy KOSPI payload");
        assert_eq!(legacy.universe_key, CandidateUniverseKey::Kospi200);

        payload["universe_key"] = json!("kosdaq150");
        let explicit = CandidatePayload::try_from(payload).expect("explicit KOSDAQ payload");
        assert_eq!(explicit.universe_key, CandidateUniverseKey::Kosdaq150);
    }
}
