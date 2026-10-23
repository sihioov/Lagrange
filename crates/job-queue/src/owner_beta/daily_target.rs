//! Separate daily target preimage over the unchanged pure strategy arithmetic.

use std::fmt;

use domain::{ContentHash, TradingDate};
use factor_engine::daily_price_only::{DAILY_PRICE_ONLY_INPUT_KIND, DailyPriceOnlyFactorSnapshot};
use factor_engine::price_only::PRICE_ONLY_CAPABILITY;
use serde::Serialize;

use super::daily_input::{OwnerBetaDailyRecommendationInputV1, OwnerBetaDailyRecommendationPins};
use super::target::{
    CanonicalItem, CanonicalReason, OWNER_BETA_TARGET_HASH_ALGORITHM, OwnerBetaReason,
    OwnerBetaTargetItem, OwnerBetaTargetSnapshotError, compute_target_parts, format_fixed_six,
};

pub const OWNER_BETA_DAILY_TARGET_SNAPSHOT_SCHEMA: &str = "owner-beta-daily-target-snapshot-v1";

/// Hashed computation output. Publication still needs its independent DB gate.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerBetaDailyTargetSnapshotV1 {
    as_of: TradingDate,
    strategy_id: String,
    strategy_version: String,
    strategy_config_sha256: ContentHash,
    factor_snapshot_sha256: ContentHash,
    pins: OwnerBetaDailyRecommendationPins,
    items: Vec<OwnerBetaTargetItem>,
    cash_weight_ppm: i64,
    portfolio_reasons: Vec<OwnerBetaReason>,
    target_snapshot_sha256: ContentHash,
}

impl fmt::Debug for OwnerBetaDailyTargetSnapshotV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerBetaDailyTargetSnapshotV1")
            .field("schema", &OWNER_BETA_DAILY_TARGET_SNAPSHOT_SCHEMA)
            .field("item_count", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl OwnerBetaDailyTargetSnapshotV1 {
    pub fn schema(&self) -> &'static str {
        OWNER_BETA_DAILY_TARGET_SNAPSHOT_SCHEMA
    }
    pub fn hash_algorithm(&self) -> &'static str {
        OWNER_BETA_TARGET_HASH_ALGORITHM
    }
    pub fn input_kind(&self) -> &'static str {
        DAILY_PRICE_ONLY_INPUT_KIND
    }
    pub fn capability(&self) -> &'static str {
        PRICE_ONLY_CAPABILITY
    }
    pub fn as_of(&self) -> TradingDate {
        self.as_of
    }
    pub fn strategy_id(&self) -> &str {
        &self.strategy_id
    }
    pub fn strategy_version(&self) -> &str {
        &self.strategy_version
    }
    pub fn strategy_config_sha256(&self) -> &ContentHash {
        &self.strategy_config_sha256
    }
    pub fn factor_snapshot_sha256(&self) -> &ContentHash {
        &self.factor_snapshot_sha256
    }
    pub fn pins(&self) -> &OwnerBetaDailyRecommendationPins {
        &self.pins
    }
    pub fn items(&self) -> &[OwnerBetaTargetItem] {
        &self.items
    }
    pub fn cash_weight_ppm(&self) -> i64 {
        self.cash_weight_ppm
    }
    pub fn cash_weight(&self) -> String {
        format_fixed_six(self.cash_weight_ppm)
    }
    pub fn portfolio_reasons(&self) -> &[OwnerBetaReason] {
        &self.portfolio_reasons
    }
    pub fn target_snapshot_sha256(&self) -> &ContentHash {
        &self.target_snapshot_sha256
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OwnerBetaTargetSnapshotError> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            schema: &'a str,
            hash_algorithm: &'a str,
            input_kind: &'a str,
            capability: &'a str,
            as_of: String,
            strategy_id: &'a str,
            strategy_version: &'a str,
            strategy_config_sha256: &'a str,
            factor_snapshot_sha256: &'a str,
            content_sha256: &'a str,
            manifest_sha256: &'a str,
            admission_sha256: &'a str,
            policy_sha256: &'a str,
            items: Vec<CanonicalItem<'a>>,
            cash_weight_ppm: i64,
            portfolio_reasons: Vec<CanonicalReason<'a>>,
        }
        let value = Canonical {
            schema: self.schema(),
            hash_algorithm: self.hash_algorithm(),
            input_kind: self.input_kind(),
            capability: self.capability(),
            as_of: self.as_of.to_iso(),
            strategy_id: &self.strategy_id,
            strategy_version: &self.strategy_version,
            strategy_config_sha256: self.strategy_config_sha256.as_str(),
            factor_snapshot_sha256: self.factor_snapshot_sha256.as_str(),
            content_sha256: self.pins.content_sha256().as_str(),
            manifest_sha256: self.pins.manifest_sha256().as_str(),
            admission_sha256: self.pins.admission_sha256().as_str(),
            policy_sha256: self.pins.policy_sha256().as_str(),
            items: self.items.iter().map(CanonicalItem::from_item).collect(),
            cash_weight_ppm: self.cash_weight_ppm,
            portfolio_reasons: self
                .portfolio_reasons
                .iter()
                .map(CanonicalReason::from_reason)
                .collect(),
        };
        serde_json::to_vec(&value).map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)
    }

    pub fn recompute_hash(&self) -> Result<ContentHash, OwnerBetaTargetSnapshotError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }

    pub fn validate_hash(&self) -> Result<(), OwnerBetaTargetSnapshotError> {
        if self.recompute_hash()? != self.target_snapshot_sha256 {
            return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
        }
        Ok(())
    }
}

pub fn build_daily_target_snapshot(
    input: &OwnerBetaDailyRecommendationInputV1,
    snapshot: &DailyPriceOnlyFactorSnapshot,
) -> Result<OwnerBetaDailyTargetSnapshotV1, OwnerBetaTargetSnapshotError> {
    input
        .validate()
        .map_err(|_| OwnerBetaTargetSnapshotError::StrategyUnsupported)?;
    input
        .validate_factor_snapshot(snapshot)
        .map_err(|_| OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let strategy = input.strategy_snapshot();
    let computed = compute_target_parts(
        strategy,
        input.as_of(),
        &snapshot.factor_versions,
        &snapshot.rows,
    )?;
    let mut result = OwnerBetaDailyTargetSnapshotV1 {
        as_of: input.as_of(),
        strategy_id: strategy.strategy_id().to_owned(),
        strategy_version: strategy.strategy_version().to_owned(),
        strategy_config_sha256: strategy.config_sha256().clone(),
        factor_snapshot_sha256: snapshot.hash.clone(),
        pins: input.pins().clone(),
        items: computed.items,
        cash_weight_ppm: computed.cash_weight_ppm,
        portfolio_reasons: computed.portfolio_reasons,
        target_snapshot_sha256: ContentHash::from_bytes(b"daily-target-placeholder"),
    };
    result.target_snapshot_sha256 = result.recompute_hash()?;
    Ok(result)
}
