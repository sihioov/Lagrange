//! Deterministic, indicative recommendation-to-Paper rebalance previews.
//!
//! The preview deliberately delegates affordability and order sizing to
//! [`portfolio_model::sizing::plan_rebalance`]. This module only adds the
//! immutable recommendation-close price basis, explainable fee components,
//! lineage, bounded canonical JSON, and a stable content token.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use domain::{
    ContentHash, Currency, DatasetId, FixedPoint, InstrumentId, Money, Price, Quantity,
    TradingDate, UtcTimestamp, WEIGHT_SCALE, Weight,
};
use market_data::CurateStore;
use market_data::curate::schema::read_bars;
use market_data::curate::{CurateError, dataset_manifest_hash};
use portfolio_model::sizing::{
    SizingAction, SizingInput, SkipReason, TargetAllocation, plan_rebalance,
};
use portfolio_model::{CostProfile, PortfolioError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

use crate::error::{QueueError, database_error_class, queue_error_class};
use crate::queue::JobQueue;
use crate::types::{ClaimedJob, ErrorClass, HeartbeatStatus, JobStatus, SettleResult};

/// Maximum JSON result accepted by the database boundary.
pub const MAX_PREVIEW_RESULT_BYTES: usize = 262_144;

/// Immutable identities that make a preview reproducible and stale-checkable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewLineage {
    pub account_id: Uuid,
    pub recommendation_run_id: Uuid,
    pub target_portfolio_id: Uuid,
    pub strategy_config_id: Uuid,
    pub dataset_version_id: Uuid,
    pub curated_version: u32,
    pub dataset_manifest_sha256: String,
    pub account_state_version: i64,
    pub account_state_sha256: String,
    pub target_portfolio_sha256: String,
}

/// Complete fixed-point input to the pure calculation.
#[derive(Debug, Clone)]
pub struct PreviewCalculationInput {
    pub cash: Money,
    pub positions: BTreeMap<InstrumentId, Quantity>,
    pub close_prices: BTreeMap<InstrumentId, Price>,
    pub targets: Vec<TargetAllocation>,
    pub lot_sizes: BTreeMap<InstrumentId, u64>,
    pub profile: CostProfile,
    pub price_date: TradingDate,
    pub proposed_effective_date: TradingDate,
    pub lineage: PreviewLineage,
}

/// Versioned response stored in PostgreSQL and returned by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewResultV1 {
    pub schema_version: u32,
    pub price_basis: String,
    pub price_date: String,
    pub proposed_effective_date: String,
    pub equity: String,
    pub cash_before: String,
    pub available_cash: String,
    pub leftover_cash: String,
    pub buy_notional: String,
    pub sell_notional: String,
    pub explicit_fees: String,
    pub informational_slippage: String,
    pub decisions: Vec<PreviewDecisionV1>,
    pub orders: Vec<PreviewOrderV1>,
    pub warning_code: String,
    pub lineage: PreviewLineage,
}

/// Canonical per-instrument explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewDecisionV1 {
    pub instrument_id: String,
    pub current_quantity: String,
    pub current_value: String,
    pub current_weight: String,
    pub target_value: String,
    pub target_weight: String,
    pub delta_value: String,
    pub action: String,
    pub skip_reason: Option<String>,
}

/// Canonical per-order estimate. Slippage is informational because it is
/// already embedded once in `estimated_execution_price`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewOrderV1 {
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    pub raw_price: String,
    pub estimated_execution_price: String,
    pub notional: String,
    pub commission: String,
    pub tax: String,
    pub informational_slippage: String,
}

/// Retry semantics shared by the later queue worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewErrorClass {
    Transient,
    DataBlocked,
    Integrity,
}

/// Typed preview boundary errors. No caller branches on rendered messages.
#[derive(Debug, thiserror::Error)]
pub enum PaperPreviewError {
    #[error("preview database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("invalid preview payload: {0}")]
    InvalidPayload(String),
    #[error("preview input is unavailable: {0}")]
    PreviewUnavailable(String),
    #[error("Paper account changed while previewing")]
    AccountChanged,
    #[error("target portfolio changed after preview submission")]
    TargetChanged,
    #[error("missing recommendation close for {instrument_id}")]
    MissingPrice { instrument_id: String },
    #[error("malformed curated preview data: {0}")]
    MalformedCuratedData(String),
    #[error("curated preview I/O failed: {0}")]
    CuratedIo(String),
    #[error("rebalance planning failed: {0}")]
    Plan(String),
    #[error("preview job lease was lost")]
    LeaseLost,
    #[error("preview job was canceled")]
    Canceled,
    #[error("preview result exceeds {MAX_PREVIEW_RESULT_BYTES} bytes: {bytes}")]
    ResultTooLarge { bytes: usize },
}

impl PaperPreviewError {
    pub fn class(&self) -> PreviewErrorClass {
        match self {
            Self::Database(_) | Self::CuratedIo(_) | Self::AccountChanged | Self::LeaseLost => {
                PreviewErrorClass::Transient
            }
            Self::PreviewUnavailable(_) | Self::MissingPrice { .. } => {
                PreviewErrorClass::DataBlocked
            }
            Self::InvalidPayload(_)
            | Self::TargetChanged
            | Self::MalformedCuratedData(_)
            | Self::Plan(_)
            | Self::Canceled
            | Self::ResultTooLarge { .. } => PreviewErrorClass::Integrity,
        }
    }
}

/// Loads the exact raw closes used by the recommendation's attested curated
/// numeric version. Partition identity and date are rechecked from Parquet;
/// paths or owners never come from queue payloads.
pub fn load_recommendation_closes(
    dataset_root: &Path,
    curated_version: u32,
    price_date: TradingDate,
    instruments: &[InstrumentId],
) -> Result<BTreeMap<InstrumentId, Price>, PaperPreviewError> {
    if curated_version == 0 {
        return Err(PaperPreviewError::InvalidPayload(
            "curated version must be positive".into(),
        ));
    }
    let mut needed = instruments.to_vec();
    needed.sort();
    if needed.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(PaperPreviewError::InvalidPayload(
            "duplicate close instrument".into(),
        ));
    }

    let store = CurateStore::new(dataset_root.join("curated"));
    let year = price_date.as_naive_date().year();
    let now = UtcTimestamp::now();
    let mut closes = BTreeMap::new();
    for instrument in needed {
        let path = store.bars_path("kr", &instrument.to_string(), year, curated_version);
        if !path.is_file() {
            return Err(PaperPreviewError::MissingPrice {
                instrument_id: instrument.to_string(),
            });
        }
        let rows = read_bars(&path).map_err(classify_curate_read)?;
        if rows.iter().any(|row| {
            row.instrument_id != instrument
                || row.trading_date.as_naive_date().year() != year
                || row.currency != Currency::KRW
        }) {
            return Err(PaperPreviewError::MalformedCuratedData(format!(
                "partition identity mismatch for {instrument}"
            )));
        }
        if rows.iter().any(|row| row.trading_date > price_date) {
            return Err(PaperPreviewError::MalformedCuratedData(format!(
                "future row in attested partition for {instrument}"
            )));
        }
        let mut exact = rows.iter().filter(|row| row.trading_date == price_date);
        let Some(row) = exact.next() else {
            return Err(PaperPreviewError::MissingPrice {
                instrument_id: instrument.to_string(),
            });
        };
        if exact.next().is_some() {
            return Err(PaperPreviewError::MalformedCuratedData(format!(
                "duplicate close for {instrument} on {price_date}"
            )));
        }
        if row.market_close_ts.as_datetime() > now.as_datetime() {
            return Err(PaperPreviewError::PreviewUnavailable(format!(
                "recommendation close is not yet available for {instrument}"
            )));
        }
        closes.insert(instrument, row.close);
    }
    Ok(closes)
}

fn classify_curate_read(error: CurateError) -> PaperPreviewError {
    match error {
        CurateError::StoreIo { context, detail } | CurateError::RawIo { context, detail } => {
            PaperPreviewError::CuratedIo(format!("{context}: {detail}"))
        }
        other => PaperPreviewError::MalformedCuratedData(other.to_string()),
    }
}

/// The queue payload is intentionally closed: all sensitive identity is read
/// back from PostgreSQL under the claimed job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaperPreviewPayload {
    pub preview_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewRunOutcome {
    Idle,
    Published { job_id: Uuid, preview_id: Uuid },
    Retrying { job_id: Uuid, code: String },
    Failed { job_id: Uuid, code: String },
    Canceled { job_id: Uuid },
    LeaseLost { job_id: Uuid },
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewRunnerError {
    #[error("Paper preview queue unavailable")]
    Queue(#[from] QueueError),
}

#[derive(Debug, sqlx::FromRow)]
struct SnapshotRow {
    account_id: Uuid,
    recommendation_run_id: Uuid,
    target_portfolio_id: Uuid,
    strategy_config_id: Uuid,
    price_date: NaiveDate,
    proposed_effective_date: NaiveDate,
    dataset_version_id: Uuid,
    dataset_id: String,
    curated_version: i32,
    dataset_manifest_sha256: String,
    target_portfolio_sha256: String,
    cost_profile_id: String,
    cost_profile_version: i32,
    account_state_version: i64,
    cash_balance: String,
    positions_json: Value,
    weights_json: Value,
}

struct PreparedPreview {
    preview_id: Uuid,
    account_state_version: i64,
    account_state_sha256: String,
    target_weights_json: Value,
    cost_profile_id: String,
    cost_profile_version: i32,
    proposed_effective_date: NaiveDate,
    token: String,
    result: PreviewResultV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewLeaseOutcome {
    Stopped,
    Canceled,
    LeaseLost,
    Failed(ErrorClass),
}

/// Claims and executes at most one preview job. The final result and queue
/// settlement share one transaction, so cancellation/lease loss can never
/// expose a READY preview.
pub async fn run_preview_once(
    pool: &PgPool,
    queue: &JobQueue,
    dataset_root: &Path,
    worker_id: &str,
    seoul_today: NaiveDate,
    heartbeat_interval: Duration,
) -> Result<PreviewRunOutcome, PreviewRunnerError> {
    if heartbeat_interval.is_zero() || heartbeat_interval >= queue.config().lease {
        return Err(PreviewRunnerError::Queue(QueueError::InvalidInput(
            "preview heartbeat must be positive and shorter than the queue lease".into(),
        )));
    }
    let Some(claim) = queue
        .claim_next_for(worker_id, "paper_rebalance_preview")
        .await?
    else {
        return Ok(PreviewRunOutcome::Idle);
    };
    let payload =
        match serde_json::from_value::<PaperPreviewPayload>(claim.job.payload_json.clone()) {
            Ok(payload) => payload,
            Err(_) => {
                return settle_preview_failure(
                    pool,
                    queue,
                    &claim,
                    ErrorClass::Input,
                    "PAPER_PREVIEW_INVALID_PAYLOAD",
                    "Paper preview payload is invalid",
                )
                .await;
            }
        };

    let (stop_tx, stop_rx) = watch::channel(false);
    let monitor_queue = queue.clone();
    let monitor_claim = claim.clone();
    let mut monitor = tokio::spawn(async move {
        monitor_preview_lease(&monitor_queue, &monitor_claim, heartbeat_interval, stop_rx).await
    });
    let preparation = prepare_preview(pool, dataset_root, &claim, payload.preview_id, seoul_today);
    tokio::pin!(preparation);

    let outcome = tokio::select! {
        biased;
        monitor_result = &mut monitor => {
            match monitor_result {
                Ok(PreviewLeaseOutcome::Canceled) => settle_preview_canceled(pool, queue, &claim).await,
                Ok(PreviewLeaseOutcome::LeaseLost) => Ok(PreviewRunOutcome::LeaseLost { job_id: claim.job.id }),
                Ok(PreviewLeaseOutcome::Failed(class)) => settle_preview_failure(
                    pool, queue, &claim, class,
                    "PAPER_PREVIEW_HEARTBEAT_FAILED",
                    "Paper preview heartbeat failed",
                ).await,
                Ok(PreviewLeaseOutcome::Stopped) | Err(_) => settle_preview_failure(
                    pool, queue, &claim, ErrorClass::Transient,
                    "PAPER_PREVIEW_HEARTBEAT_STOPPED",
                    "Paper preview heartbeat stopped unexpectedly",
                ).await,
            }
        }
        prepared = &mut preparation => {
            let _ = stop_tx.send(true);
            match monitor.await {
                Ok(PreviewLeaseOutcome::Stopped) => match prepared {
                    Ok(prepared) => finish_preview(pool, queue, &claim, prepared).await,
                    Err(error) => {
                        let (class, code, summary) = preview_failure(&error);
                        settle_preview_failure(pool, queue, &claim, class, code, summary).await
                    }
                },
                Ok(PreviewLeaseOutcome::Canceled) => settle_preview_canceled(pool, queue, &claim).await,
                Ok(PreviewLeaseOutcome::LeaseLost) => Ok(PreviewRunOutcome::LeaseLost { job_id: claim.job.id }),
                Ok(PreviewLeaseOutcome::Failed(class)) => settle_preview_failure(
                    pool, queue, &claim, class,
                    "PAPER_PREVIEW_HEARTBEAT_FAILED",
                    "Paper preview heartbeat failed",
                ).await,
                Err(_) => settle_preview_failure(
                    pool, queue, &claim, ErrorClass::Transient,
                    "PAPER_PREVIEW_HEARTBEAT_STOPPED",
                    "Paper preview heartbeat stopped unexpectedly",
                ).await,
            }
        }
    };
    match outcome {
        Err(PreviewRunnerError::Queue(QueueError::StaleClaim(_))) => {
            stale_preview_outcome(pool, queue, &claim).await
        }
        other => other,
    }
}

async fn stale_preview_outcome(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<PreviewRunOutcome, PreviewRunnerError> {
    if queue.check_canceled(claim.job.id).await? {
        settle_preview_canceled(pool, queue, claim).await
    } else {
        Ok(PreviewRunOutcome::LeaseLost {
            job_id: claim.job.id,
        })
    }
}

async fn monitor_preview_lease(
    queue: &JobQueue,
    claim: &ClaimedJob,
    interval: Duration,
    mut stop: watch::Receiver<bool>,
) -> PreviewLeaseOutcome {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return PreviewLeaseOutcome::Stopped;
                }
            }
            _ = ticker.tick() => match queue.heartbeat(claim).await {
                Ok(HeartbeatStatus::Extended) => {}
                Ok(HeartbeatStatus::Canceled) => return PreviewLeaseOutcome::Canceled,
                Ok(HeartbeatStatus::LeaseLost) => return PreviewLeaseOutcome::LeaseLost,
                Err(error) => return PreviewLeaseOutcome::Failed(queue_error_class(&error)),
            }
        }
    }
}

async fn prepare_preview(
    pool: &PgPool,
    dataset_root: &Path,
    claim: &ClaimedJob,
    preview_id: Uuid,
    seoul_today: NaiveDate,
) -> Result<PreparedPreview, PaperPreviewError> {
    let snapshot = sqlx::query_as::<_, SnapshotRow>(
        "SELECT account_id, recommendation_run_id, target_portfolio_id, \
                strategy_config_id, price_date, proposed_effective_date, \
                dataset_version_id, dataset_id, curated_version, dataset_manifest_sha256, \
                target_portfolio_sha256, cost_profile_id, cost_profile_version, \
                account_state_version, cash_balance, positions_json, weights_json \
         FROM snapshot_paper_rebalance_preview($1, $2, $3)",
    )
    .bind(preview_id)
    .bind(claim.job.id)
    .bind(seoul_today)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        PaperPreviewError::PreviewUnavailable(
            "preview inputs no longer satisfy the attested contract".into(),
        )
    })?;
    let cash = Money::parse(&snapshot.cash_balance, Currency::KRW)
        .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
    let positions = parse_positions(&snapshot.positions_json)?;
    let current_target_sha256 = raw_sha256(&serde_json::to_vec(&snapshot.weights_json).map_err(
        |error| {
            PaperPreviewError::InvalidPayload(format!(
                "target portfolio cannot be serialized: {error}"
            ))
        },
    )?);
    if current_target_sha256 != snapshot.target_portfolio_sha256 {
        return Err(PaperPreviewError::TargetChanged);
    }
    let target_weights_json = snapshot.weights_json.clone();
    let targets = parse_targets(&snapshot.weights_json)?;
    let profile = CostProfile::resolve(&snapshot.cost_profile_id)
        .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
    if i32::try_from(profile.version).ok() != Some(snapshot.cost_profile_version) {
        return Err(PaperPreviewError::InvalidPayload(
            "cost profile version is unsupported".into(),
        ));
    }
    let curated_version = u32::try_from(snapshot.curated_version)
        .map_err(|_| PaperPreviewError::InvalidPayload("curated version is invalid".into()))?;
    let price_date = TradingDate::parse(&snapshot.price_date.to_string())
        .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
    let proposed_effective_date = TradingDate::parse(&snapshot.proposed_effective_date.to_string())
        .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
    let mut instrument_ids: Vec<_> = positions.keys().cloned().collect();
    instrument_ids.extend(targets.iter().map(|target| target.instrument_id.clone()));
    instrument_ids.sort();
    instrument_ids.dedup();
    let root = dataset_root.to_path_buf();
    let dataset_id = snapshot.dataset_id.clone();
    let manifest_sha256 = snapshot.dataset_manifest_sha256.clone();
    let account_state_sha256 = account_state_sha256(
        snapshot.account_state_version,
        &snapshot.cash_balance,
        &snapshot.positions_json,
    )?;
    let lineage = PreviewLineage {
        account_id: snapshot.account_id,
        recommendation_run_id: snapshot.recommendation_run_id,
        target_portfolio_id: snapshot.target_portfolio_id,
        strategy_config_id: snapshot.strategy_config_id,
        dataset_version_id: snapshot.dataset_version_id,
        curated_version,
        dataset_manifest_sha256: snapshot.dataset_manifest_sha256,
        account_state_version: snapshot.account_state_version,
        account_state_sha256: account_state_sha256.clone(),
        target_portfolio_sha256: snapshot.target_portfolio_sha256,
    };
    let calculation = tokio::task::spawn_blocking(move || {
        attest_preview_dataset(&root, &dataset_id, curated_version, &manifest_sha256)?;
        let close_prices =
            load_recommendation_closes(&root, curated_version, price_date, &instrument_ids)?;
        calculate_preview(PreviewCalculationInput {
            cash,
            positions,
            close_prices,
            targets,
            lot_sizes: BTreeMap::new(),
            profile,
            price_date,
            proposed_effective_date,
            lineage,
        })
    })
    .await
    .map_err(|_| PaperPreviewError::PreviewUnavailable("preview compute task stopped".into()))??;
    Ok(PreparedPreview {
        preview_id,
        account_state_version: snapshot.account_state_version,
        account_state_sha256,
        target_weights_json,
        cost_profile_id: snapshot.cost_profile_id,
        cost_profile_version: snapshot.cost_profile_version,
        proposed_effective_date: snapshot.proposed_effective_date,
        token: calculation.1,
        result: calculation.0,
    })
}

fn attest_preview_dataset(
    dataset_root: &Path,
    dataset_id: &str,
    curated_version: u32,
    expected_manifest_sha256: &str,
) -> Result<(), PaperPreviewError> {
    let dataset_id = DatasetId::parse(dataset_id)
        .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
    let store = CurateStore::new(dataset_root.join("curated"));
    let manifest = store
        .read_dataset_manifest(&dataset_id, curated_version)
        .map_err(classify_curate_read)?
        .ok_or_else(|| {
            PaperPreviewError::PreviewUnavailable(format!(
                "attested dataset manifest is missing for version {curated_version}"
            ))
        })?;
    if manifest.dataset_id != dataset_id || manifest.version != curated_version {
        return Err(PaperPreviewError::MalformedCuratedData(
            "dataset manifest identity does not match its attested path".into(),
        ));
    }
    let canonical = dataset_manifest_hash(&manifest)
        .map_err(|error| PaperPreviewError::MalformedCuratedData(error.to_string()))?;
    if canonical != manifest.content_hash
        || canonical
            .as_str()
            .strip_prefix("sha256:")
            .is_none_or(|hash| hash != expected_manifest_sha256)
    {
        return Err(PaperPreviewError::MalformedCuratedData(
            "dataset manifest does not match its canonical database attestation".into(),
        ));
    }
    Ok(())
}

fn parse_positions(value: &Value) -> Result<BTreeMap<InstrumentId, Quantity>, PaperPreviewError> {
    let object = value.as_object().ok_or_else(|| {
        PaperPreviewError::InvalidPayload("positions snapshot is not an object".into())
    })?;
    let mut positions = BTreeMap::new();
    for (instrument, quantity) in object {
        let instrument_id = InstrumentId::parse(instrument)
            .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
        let quantity = quantity.as_str().ok_or_else(|| {
            PaperPreviewError::InvalidPayload("position quantity is not a string".into())
        })?;
        let quantity = Quantity::parse(quantity)
            .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
        if !quantity.is_zero() {
            positions.insert(instrument_id, quantity);
        }
    }
    Ok(positions)
}

fn parse_targets(value: &Value) -> Result<Vec<TargetAllocation>, PaperPreviewError> {
    let object = value.as_object().ok_or_else(|| {
        PaperPreviewError::InvalidPayload("target weights are not an object".into())
    })?;
    let mut targets = Vec::new();
    for (instrument, weight) in object {
        let instrument_id = InstrumentId::parse(instrument)
            .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
        let weight = weight.as_str().ok_or_else(|| {
            PaperPreviewError::InvalidPayload("target weight is not a string".into())
        })?;
        let weight = Weight::parse(weight)
            .map_err(|error| PaperPreviewError::InvalidPayload(error.to_string()))?;
        if !weight.is_zero() {
            targets.push(TargetAllocation {
                instrument_id,
                weight,
            });
        }
    }
    Ok(targets)
}

/// Hash the canonical Paper cash/position snapshot used by preview publication
/// and explicit application. Callers must hold the account/input locks while
/// deriving these values so the digest and state version share one snapshot.
pub fn account_state_sha256(
    version: i64,
    cash: &str,
    positions: &Value,
) -> Result<String, PaperPreviewError> {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "paper_state_version": version,
        "cash": cash,
        "positions": positions,
    }))
    .map_err(|error| PaperPreviewError::Plan(error.to_string()))?;
    Ok(raw_sha256(&bytes))
}

fn raw_sha256(bytes: &[u8]) -> String {
    ContentHash::from_bytes(bytes)
        .as_str()
        .strip_prefix("sha256:")
        .expect("ContentHash always has sha256 prefix")
        .to_owned()
}

async fn finish_preview(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    prepared: PreparedPreview,
) -> Result<PreviewRunOutcome, PreviewRunnerError> {
    match queue.heartbeat(claim).await? {
        HeartbeatStatus::Canceled => return settle_preview_canceled(pool, queue, claim).await,
        HeartbeatStatus::LeaseLost => {
            return Ok(PreviewRunOutcome::LeaseLost {
                job_id: claim.job.id,
            });
        }
        HeartbeatStatus::Extended => {}
    }
    let mut transaction = pool.begin().await.map_err(QueueError::Database)?;
    match queue.lock_claim_in(&mut transaction, claim).await {
        Ok(_) => {}
        Err(QueueError::StaleClaim(_)) => {
            transaction.rollback().await.map_err(QueueError::Database)?;
            return stale_preview_outcome(pool, queue, claim).await;
        }
        Err(error) => {
            transaction.rollback().await.map_err(QueueError::Database)?;
            return Err(error.into());
        }
    }
    let publication =
        sqlx::query_scalar(
            "SELECT publish_paper_rebalance_preview( \
         $1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(prepared.preview_id)
        .bind(claim.job.id)
        .bind(prepared.account_state_version)
        .bind(&prepared.account_state_sha256)
        .bind(&prepared.cost_profile_id)
        .bind(prepared.cost_profile_version)
        .bind(prepared.proposed_effective_date)
        .bind(&prepared.token)
        .bind(&prepared.target_weights_json)
        .bind(serde_json::to_value(&prepared.result).map_err(|error| {
            QueueError::Internal(format!("serialize preview publication: {error}"))
        })?)
        .fetch_one(&mut *transaction)
        .await;
    let published: bool = match publication {
        Ok(published) => published,
        Err(error) => {
            let class = database_error_class(&error);
            let (code, summary) = match class {
                ErrorClass::Transient => (
                    "PAPER_PREVIEW_PUBLICATION_UNAVAILABLE",
                    "Paper preview publication is temporarily unavailable",
                ),
                _ => (
                    "PAPER_PREVIEW_PUBLICATION_INTEGRITY",
                    "Paper preview publication failed validation",
                ),
            };
            transaction.rollback().await.map_err(QueueError::Database)?;
            return settle_preview_failure(pool, queue, claim, class, code, summary).await;
        }
    };
    if !published {
        transaction.rollback().await.map_err(QueueError::Database)?;
        return settle_preview_failure(
            pool,
            queue,
            claim,
            ErrorClass::Transient,
            "PAPER_PREVIEW_ACCOUNT_CHANGED",
            "Paper account changed while previewing",
        )
        .await;
    }
    match queue.settle_success_in(&mut transaction, claim).await? {
        SettleResult::Committed(job) if job.status == JobStatus::Succeeded => {}
        SettleResult::Canceled(_) => {
            transaction.rollback().await.map_err(QueueError::Database)?;
            return Ok(PreviewRunOutcome::Canceled {
                job_id: claim.job.id,
            });
        }
        SettleResult::Committed(_) => {
            transaction.rollback().await.map_err(QueueError::Database)?;
            return Err(QueueError::Internal(
                "preview success settlement returned an invalid status".into(),
            )
            .into());
        }
    }
    transaction.commit().await.map_err(QueueError::Database)?;
    Ok(PreviewRunOutcome::Published {
        job_id: claim.job.id,
        preview_id: prepared.preview_id,
    })
}

async fn settle_preview_failure(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
    class: ErrorClass,
    code: &str,
    summary: &str,
) -> Result<PreviewRunOutcome, PreviewRunnerError> {
    let mut transaction = pool.begin().await.map_err(QueueError::Database)?;
    let settlement = queue
        .settle_failure_in(&mut transaction, claim, class, code, summary)
        .await?;
    let outcome = match settlement {
        SettleResult::Committed(job) if job.status == JobStatus::Queued => {
            PreviewRunOutcome::Retrying {
                job_id: claim.job.id,
                code: code.into(),
            }
        }
        SettleResult::Committed(job) if job.status == JobStatus::Failed => {
            fail_preview_in(&mut transaction, claim, code, summary).await?;
            PreviewRunOutcome::Failed {
                job_id: claim.job.id,
                code: code.into(),
            }
        }
        SettleResult::Canceled(_) => {
            fail_preview_in(
                &mut transaction,
                claim,
                "PAPER_PREVIEW_CANCELED",
                "Paper preview was canceled",
            )
            .await?;
            PreviewRunOutcome::Canceled {
                job_id: claim.job.id,
            }
        }
        SettleResult::Committed(_) => {
            transaction.rollback().await.map_err(QueueError::Database)?;
            return Err(QueueError::Internal(
                "preview failure settlement returned an invalid status".into(),
            )
            .into());
        }
    };
    transaction.commit().await.map_err(QueueError::Database)?;
    Ok(outcome)
}

async fn settle_preview_canceled(
    pool: &PgPool,
    queue: &JobQueue,
    claim: &ClaimedJob,
) -> Result<PreviewRunOutcome, PreviewRunnerError> {
    let mut transaction = pool.begin().await.map_err(QueueError::Database)?;
    queue
        .settle_aborted_in(&mut transaction, claim, "Paper preview was canceled")
        .await?;
    fail_preview_in(
        &mut transaction,
        claim,
        "PAPER_PREVIEW_CANCELED",
        "Paper preview was canceled",
    )
    .await?;
    transaction.commit().await.map_err(QueueError::Database)?;
    Ok(PreviewRunOutcome::Canceled {
        job_id: claim.job.id,
    })
}

async fn fail_preview_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &ClaimedJob,
    code: &str,
    summary: &str,
) -> Result<(), PreviewRunnerError> {
    let preview_id = claim
        .job
        .payload_json
        .get("preview_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| QueueError::Internal("claimed preview payload lost its identity".into()))?;
    let failed: bool = sqlx::query_scalar("SELECT fail_paper_rebalance_preview($1, $2, $3)")
        .bind(preview_id)
        .bind(claim.job.id)
        .bind(json!({ "code": code, "message": summary }))
        .fetch_one(&mut **transaction)
        .await
        .map_err(QueueError::Database)?;
    if !failed {
        return Err(QueueError::Internal("preview failure row was not updated".into()).into());
    }
    Ok(())
}

fn preview_failure(error: &PaperPreviewError) -> (ErrorClass, &'static str, &'static str) {
    let class = match error {
        PaperPreviewError::Database(error) => database_error_class(error),
        PaperPreviewError::InvalidPayload(_) => ErrorClass::Input,
        PaperPreviewError::PreviewUnavailable(_) | PaperPreviewError::MissingPrice { .. } => {
            ErrorClass::DataBlocked
        }
        PaperPreviewError::CuratedIo(_)
        | PaperPreviewError::AccountChanged
        | PaperPreviewError::LeaseLost => ErrorClass::Transient,
        PaperPreviewError::MalformedCuratedData(_)
        | PaperPreviewError::TargetChanged
        | PaperPreviewError::Plan(_)
        | PaperPreviewError::Canceled
        | PaperPreviewError::ResultTooLarge { .. } => ErrorClass::Integrity,
    };
    let code = match error {
        PaperPreviewError::InvalidPayload(_) => "PAPER_PREVIEW_INVALID_INPUT",
        PaperPreviewError::PreviewUnavailable(_) => "PAPER_PREVIEW_INPUT_UNAVAILABLE",
        PaperPreviewError::MissingPrice { .. } => "PAPER_PREVIEW_CLOSE_MISSING",
        PaperPreviewError::MalformedCuratedData(_) => "PAPER_PREVIEW_CURATED_INTEGRITY",
        PaperPreviewError::CuratedIo(_) | PaperPreviewError::Database(_) => {
            "PAPER_PREVIEW_TEMPORARILY_UNAVAILABLE"
        }
        PaperPreviewError::AccountChanged => "PAPER_PREVIEW_ACCOUNT_CHANGED",
        PaperPreviewError::TargetChanged => "PAPER_PREVIEW_TARGET_CHANGED",
        PaperPreviewError::Plan(_) => "PAPER_PREVIEW_PLAN_FAILED",
        PaperPreviewError::LeaseLost => "PAPER_PREVIEW_LEASE_LOST",
        PaperPreviewError::Canceled => "PAPER_PREVIEW_CANCELED",
        PaperPreviewError::ResultTooLarge { .. } => "PAPER_PREVIEW_RESULT_TOO_LARGE",
    };
    let summary = match class {
        ErrorClass::Transient => "Paper preview is temporarily unavailable",
        ErrorClass::DataBlocked => "Paper preview data is unavailable",
        _ => "Paper preview failed validation",
    };
    (class, code, summary)
}

/// Produces one bounded result and its raw lowercase SHA-256 token.
pub fn calculate_preview(
    input: PreviewCalculationInput,
) -> Result<(PreviewResultV1, String), PaperPreviewError> {
    validate_lineage(&input.lineage)?;
    if input.proposed_effective_date <= input.price_date {
        return Err(PaperPreviewError::InvalidPayload(
            "proposed effective date must follow the price date".into(),
        ));
    }

    let mut target_weights = BTreeMap::new();
    for target in &input.targets {
        if target_weights
            .insert(target.instrument_id.clone(), target.weight)
            .is_some()
        {
            return Err(PaperPreviewError::InvalidPayload(format!(
                "duplicate target instrument {}",
                target.instrument_id
            )));
        }
    }

    // This is the one and only affordability/sizing call.
    let report = plan_rebalance(&SizingInput {
        cash: input.cash,
        positions: input.positions.clone(),
        open_prices: input.close_prices.clone(),
        targets: input.targets,
        lot_sizes: input.lot_sizes,
        profile: input.profile.clone(),
    })
    .map_err(map_plan_error)?;

    let currency = input.cash.currency();
    let mut buy_notional = Money::zero(currency);
    let mut sell_notional = Money::zero(currency);
    let mut explicit_fees = Money::zero(currency);
    let mut informational_slippage = Money::zero(currency);
    let mut orders = Vec::with_capacity(report.orders.len());
    for order in &report.orders {
        let raw = input
            .close_prices
            .get(&order.instrument_id)
            .ok_or_else(|| PaperPreviewError::MissingPrice {
                instrument_id: order.instrument_id.to_string(),
            })?;
        let execution = input
            .profile
            .execution_price(raw, order.side)
            .map_err(|error| PaperPreviewError::Plan(error.to_string()))?;
        let cost = input
            .profile
            .estimate(order.side, &order.quantity, &execution)
            .map_err(|error| PaperPreviewError::Plan(error.to_string()))?;
        explicit_fees = explicit_fees
            .checked_add(&cost.cash_fees().map_err(map_domain_plan)?)
            .map_err(map_domain_plan)?;
        informational_slippage = informational_slippage
            .checked_add(&cost.slippage)
            .map_err(map_domain_plan)?;
        if order.side.is_buy() {
            buy_notional = buy_notional
                .checked_add(&order.order_value)
                .map_err(map_domain_plan)?;
        } else {
            sell_notional = sell_notional
                .checked_add(&order.order_value)
                .map_err(map_domain_plan)?;
        }
        orders.push(PreviewOrderV1 {
            instrument_id: order.instrument_id.to_string(),
            side: if order.side.is_buy() { "BUY" } else { "SELL" }.into(),
            quantity: order.quantity.as_decimal_string(),
            raw_price: raw.as_decimal_string(),
            estimated_execution_price: execution.as_decimal_string(),
            notional: order.order_value.as_decimal_string(),
            commission: cost.commission.as_decimal_string(),
            tax: cost.tax.as_decimal_string(),
            informational_slippage: cost.slippage.as_decimal_string(),
        });
    }

    let mut decisions = Vec::with_capacity(report.decisions.len());
    for decision in &report.decisions {
        let current_quantity = input
            .positions
            .get(&decision.instrument_id)
            .copied()
            .unwrap_or(Quantity::zero().map_err(map_domain_plan)?);
        let target_weight = target_weights
            .get(&decision.instrument_id)
            .copied()
            .unwrap_or(Weight::zero().map_err(map_domain_plan)?);
        let current_weight = if report.equity.is_zero() {
            FixedPoint::ZERO
        } else {
            decision
                .current_value
                .amount()
                .checked_div(&report.equity.amount(), WEIGHT_SCALE)
                .map_err(map_domain_plan)?
        };
        let (action, skip_reason) = describe_action(&decision.action);
        decisions.push(PreviewDecisionV1 {
            instrument_id: decision.instrument_id.to_string(),
            current_quantity: current_quantity.as_decimal_string(),
            current_value: decision.current_value.as_decimal_string(),
            current_weight: current_weight.to_string(),
            target_value: decision.target_value.as_decimal_string(),
            target_weight: target_weight.as_decimal_string(),
            delta_value: decision.order_value.to_string(),
            action: action.into(),
            skip_reason: skip_reason.map(str::to_owned),
        });
    }

    // `plan_rebalance` already emits canonical decisions and sell-then-buy
    // orders. Keep a defensive assertion at the serialization boundary.
    ensure_canonical(&decisions, &orders)?;
    let result = PreviewResultV1 {
        schema_version: 1,
        price_basis: "RECOMMENDATION_CLOSE".into(),
        price_date: input.price_date.to_iso(),
        proposed_effective_date: input.proposed_effective_date.to_iso(),
        equity: report.equity.as_decimal_string(),
        cash_before: input.cash.as_decimal_string(),
        available_cash: report.available_cash.as_decimal_string(),
        leftover_cash: report.leftover_cash.as_decimal_string(),
        buy_notional: buy_notional.as_decimal_string(),
        sell_notional: sell_notional.as_decimal_string(),
        explicit_fees: explicit_fees.as_decimal_string(),
        informational_slippage: informational_slippage.as_decimal_string(),
        decisions,
        orders,
        warning_code: "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED".into(),
        lineage: input.lineage,
    };
    let bytes = serde_json::to_vec(&result)
        .map_err(|error| PaperPreviewError::Plan(format!("serialize preview: {error}")))?;
    if bytes.len() > MAX_PREVIEW_RESULT_BYTES {
        return Err(PaperPreviewError::ResultTooLarge { bytes: bytes.len() });
    }
    let token = ContentHash::from_bytes(&bytes)
        .as_str()
        .strip_prefix("sha256:")
        .expect("ContentHash always has sha256 prefix")
        .to_owned();
    Ok((result, token))
}

fn validate_lineage(lineage: &PreviewLineage) -> Result<(), PaperPreviewError> {
    for (field, hash) in [
        ("dataset_manifest_sha256", &lineage.dataset_manifest_sha256),
        ("account_state_sha256", &lineage.account_state_sha256),
        ("target_portfolio_sha256", &lineage.target_portfolio_sha256),
    ] {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(PaperPreviewError::InvalidPayload(format!(
                "{field} is not a canonical SHA-256"
            )));
        }
    }
    if lineage.account_state_version < 0 {
        return Err(PaperPreviewError::InvalidPayload(
            "account_state_version is negative".into(),
        ));
    }
    Ok(())
}

fn map_plan_error(error: PortfolioError) -> PaperPreviewError {
    match error {
        PortfolioError::MissingPrice { instrument_id } => PaperPreviewError::MissingPrice {
            instrument_id: instrument_id.to_string(),
        },
        other => PaperPreviewError::Plan(other.to_string()),
    }
}

fn map_domain_plan(error: impl std::fmt::Display) -> PaperPreviewError {
    PaperPreviewError::Plan(error.to_string())
}

fn describe_action(action: &SizingAction) -> (&'static str, Option<&'static str>) {
    match action {
        SizingAction::Sell(_) => ("SELL", None),
        SizingAction::Buy(_) => ("BUY", None),
        SizingAction::Skip(reason) => (
            "SKIP",
            Some(match reason {
                SkipReason::BelowRebalanceThreshold { .. } => "BELOW_REBALANCE_THRESHOLD",
                SkipReason::BelowMinTrade { .. } => "BELOW_MIN_TRADE",
                SkipReason::NoAvailableCash => "NO_AVAILABLE_CASH",
                SkipReason::NoAffordableLot { .. } => "NO_AFFORDABLE_LOT",
            }),
        ),
    }
}

fn ensure_canonical(
    decisions: &[PreviewDecisionV1],
    orders: &[PreviewOrderV1],
) -> Result<(), PaperPreviewError> {
    if decisions
        .windows(2)
        .any(|pair| pair[0].instrument_id >= pair[1].instrument_id)
    {
        return Err(PaperPreviewError::Plan(
            "decisions are not in canonical instrument order".into(),
        ));
    }
    let mut buy_seen = false;
    let mut sell_ids = BTreeSet::new();
    let mut buy_ids = BTreeSet::new();
    for order in orders {
        match order.side.as_str() {
            "SELL" if !buy_seen => {
                if !sell_ids.insert(&order.instrument_id) {
                    return Err(PaperPreviewError::Plan("duplicate sell order".into()));
                }
            }
            "BUY" => {
                buy_seen = true;
                if !buy_ids.insert(&order.instrument_id) {
                    return Err(PaperPreviewError::Plan("duplicate buy order".into()));
                }
            }
            _ => {
                return Err(PaperPreviewError::Plan(
                    "orders are not sell-before-buy".into(),
                ));
            }
        }
    }
    Ok(())
}
