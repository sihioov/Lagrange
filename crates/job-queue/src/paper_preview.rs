//! Deterministic, indicative recommendation-to-Paper rebalance previews.
//!
//! The preview deliberately delegates affordability and order sizing to
//! [`portfolio_model::sizing::plan_rebalance`]. This module only adds the
//! immutable recommendation-close price basis, explainable fee components,
//! lineage, bounded canonical JSON, and a stable content token.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::Datelike;
use domain::{
    ContentHash, Currency, FixedPoint, InstrumentId, Money, Price, Quantity, TradingDate,
    UtcTimestamp, WEIGHT_SCALE, Weight,
};
use market_data::CurateStore;
use market_data::curate::CurateError;
use market_data::curate::schema::read_bars;
use portfolio_model::sizing::{
    SizingAction, SizingInput, SkipReason, TargetAllocation, plan_rebalance,
};
use portfolio_model::{CostProfile, PortfolioError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
