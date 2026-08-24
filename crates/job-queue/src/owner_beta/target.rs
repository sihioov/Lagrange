//! Deterministic, price-only owner-beta target snapshots.
//!
//! This boundary deliberately consumes only the sealed owner-beta input and a
//! validated price-only factor snapshot.  It does not read storage, consult a
//! process environment, or execute another program.  Every value covered by
//! the target hash is either a string or an integer; floating point values are
//! used only while applying the already-shipped strategy equations.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use domain::ContentHash;
use factor_engine::{Factor, Field, PriceOnlyFactorSnapshot, snapshot::FactorRow};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde::Serialize;
use serde_json::{Number, Value};

use super::{OwnerBetaPriceRecommendationInput, OwnerBetaPriceRecommendationPins};
use crate::{
    factor_series::factors_for,
    recommendation::compute::{StrategyRequirements, requirements_for},
    resolver::ResolvedConfig,
};

/// Versioned target wire/hash schema.
pub const OWNER_BETA_TARGET_SNAPSHOT_SCHEMA: &str = "owner-beta-target-snapshot-v1";
/// Hash algorithm bound into the target preimage.
pub const OWNER_BETA_TARGET_HASH_ALGORITHM: &str = "sha256";
/// Target weights and cash use signed parts-per-million internally.
pub const OWNER_BETA_TARGET_WEIGHT_SCALE: i64 = 1_000_000;
/// Static, fail-closed target construction errors.  No input values are
/// carried so malformed data cannot enter logs through an error formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaTargetSnapshotError {
    #[error("owner-beta target strategy is unsupported")]
    StrategyUnsupported,
    #[error("owner-beta factor snapshot is invalid")]
    FactorSnapshotInvalid,
    #[error("owner-beta target input is invalid")]
    TargetInputInvalid,
    #[error("owner-beta target arithmetic is invalid")]
    ArithmeticInvalid,
}

/// Closed reason taxonomy used by the five shipped baseline strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OwnerBetaReasonCode {
    SelectedTopN,
    NotSelectedBeyondTopN,
    ExcludedMandatoryFactorNull,
    AllCashNoEligible,
    WeightCappedAtMax,
    WeightRoundingResidueToCash,
    CashFloorApplied,
    BenchmarkHeld,
    TrendPositive,
    TrendNegativeCash,
    AbsoluteMomentumPassed,
    DefensiveCashSelected,
    InverseVolWeighted,
    NotSelectedByStrategy,
}

impl OwnerBetaReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SelectedTopN => "SELECTED_TOP_N",
            Self::NotSelectedBeyondTopN => "NOT_SELECTED_BEYOND_TOP_N",
            Self::ExcludedMandatoryFactorNull => "EXCLUDED_MANDATORY_FACTOR_NULL",
            Self::AllCashNoEligible => "ALL_CASH_NO_ELIGIBLE",
            Self::WeightCappedAtMax => "WEIGHT_CAPPED_AT_MAX",
            Self::WeightRoundingResidueToCash => "WEIGHT_ROUNDING_RESIDUE_TO_CASH",
            Self::CashFloorApplied => "CASH_FLOOR_APPLIED",
            Self::BenchmarkHeld => "BENCHMARK_HELD",
            Self::TrendPositive => "TREND_POSITIVE",
            Self::TrendNegativeCash => "TREND_NEGATIVE_CASH",
            Self::AbsoluteMomentumPassed => "ABSOLUTE_MOMENTUM_PASSED",
            Self::DefensiveCashSelected => "DEFENSIVE_CASH_SELECTED",
            Self::InverseVolWeighted => "INVERSE_VOL_WEIGHTED",
            Self::NotSelectedByStrategy => "NOT_SELECTED_BY_STRATEGY",
        }
    }
}

/// One localized, parameterized reason. Parameters are always stored in
/// lexical order, making both the wire form and target hash deterministic.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerBetaReason {
    code: OwnerBetaReasonCode,
    params: BTreeMap<String, String>,
    text_ko: String,
    text_en: String,
}

impl fmt::Debug for OwnerBetaReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaReason")
            .field("code", &self.code.as_str())
            .field("reason", &"<redacted>")
            .finish()
    }
}

impl OwnerBetaReason {
    pub fn code(&self) -> &'static str {
        self.code.as_str()
    }

    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    pub fn text_ko(&self) -> &str {
        &self.text_ko
    }

    pub fn text_en(&self) -> &str {
        &self.text_en
    }
}

/// One canonical member of a target snapshot.  The internal fixed-point
/// fields are intentionally not floating point.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerBetaTargetItem {
    instrument_id: String,
    rank: Option<u32>,
    score_text: Option<String>,
    factors_text: BTreeMap<String, String>,
    target_weight_ppm: Option<i64>,
    reasons: Vec<OwnerBetaReason>,
}

impl fmt::Debug for OwnerBetaTargetItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaTargetItem")
            .field("instrument_id", &self.instrument_id)
            .field("rank", &self.rank)
            .finish_non_exhaustive()
    }
}

impl OwnerBetaTargetItem {
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub fn rank(&self) -> Option<u32> {
        self.rank
    }

    /// Score evidence in the same shortest-round-trip form as Python's
    /// baseline target generators. The target hash binds this string, never a
    /// JSON floating-point value.
    pub fn score(&self) -> Option<&str> {
        self.score_text.as_deref()
    }

    /// Factor evidence in canonical Python-compatible float strings.
    pub fn factors(&self) -> &BTreeMap<String, String> {
        &self.factors_text
    }

    pub fn target_weight_ppm(&self) -> Option<i64> {
        self.target_weight_ppm
    }

    /// Target weight as a canonical six-decimal string.
    pub fn target_weight(&self) -> Option<String> {
        self.target_weight_ppm.map(format_fixed_six)
    }

    pub fn reasons(&self) -> &[OwnerBetaReason] {
        &self.reasons
    }
}

/// Opaque deterministic target output.  Its hash covers every field except
/// `target_snapshot_sha256`, which is recomputed by [`Self::validate_hash`].
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerBetaTargetSnapshot {
    schema: &'static str,
    hash_algorithm: &'static str,
    input_kind: String,
    capability: String,
    as_of: domain::TradingDate,
    strategy_id: String,
    strategy_version: String,
    strategy_config_sha256: ContentHash,
    factor_snapshot_sha256: ContentHash,
    pins: OwnerBetaPriceRecommendationPins,
    items: Vec<OwnerBetaTargetItem>,
    cash_weight_ppm: i64,
    portfolio_reasons: Vec<OwnerBetaReason>,
    target_snapshot_sha256: ContentHash,
}

impl fmt::Debug for OwnerBetaTargetSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaTargetSnapshot")
            .field("schema", &self.schema)
            .field("item_count", &self.items.len())
            .field("snapshot", &"<redacted>")
            .finish()
    }
}

impl OwnerBetaTargetSnapshot {
    pub fn schema(&self) -> &str {
        self.schema
    }

    pub fn hash_algorithm(&self) -> &str {
        self.hash_algorithm
    }

    pub fn input_kind(&self) -> &str {
        &self.input_kind
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn as_of(&self) -> domain::TradingDate {
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

    pub fn pins(&self) -> &OwnerBetaPriceRecommendationPins {
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

    /// Returns the target hash preimage. It contains no floating-point values
    /// and deliberately excludes the stored target hash itself.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OwnerBetaTargetSnapshotError> {
        let canonical = CanonicalTarget {
            schema: self.schema,
            hash_algorithm: self.hash_algorithm,
            input_kind: &self.input_kind,
            capability: &self.capability,
            as_of: &self.as_of.to_iso(),
            strategy_id: &self.strategy_id,
            strategy_version: &self.strategy_version,
            strategy_config_sha256: self.strategy_config_sha256.as_str(),
            factor_snapshot_sha256: self.factor_snapshot_sha256.as_str(),
            candidate_content_sha256: self.pins.candidate_content_sha256().as_str(),
            artifact_manifest_sha256: self.pins.artifact_manifest_sha256().as_str(),
            stage5_manifest_sha256: self.pins.stage5_manifest_sha256().as_str(),
            action_manifest_sha256: self.pins.action_manifest_sha256().as_str(),
            approval_registry_sha256: self.pins.approval_registry_sha256().as_str(),
            items: self.items.iter().map(CanonicalItem::from_item).collect(),
            cash_weight_ppm: self.cash_weight_ppm,
            portfolio_reasons: self
                .portfolio_reasons
                .iter()
                .map(CanonicalReason::from_reason)
                .collect(),
        };
        serde_json::to_vec(&canonical).map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)
    }

    pub fn recompute_hash(&self) -> Result<ContentHash, OwnerBetaTargetSnapshotError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }

    pub fn validate_hash(&self) -> Result<(), OwnerBetaTargetSnapshotError> {
        let expected = self.recompute_hash()?;
        if expected != self.target_snapshot_sha256 {
            return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
        }
        Ok(())
    }

    /// Constructor alias kept explicit at the type boundary.
    pub fn from_input(
        input: &OwnerBetaPriceRecommendationInput,
        factor_snapshot: &PriceOnlyFactorSnapshot,
    ) -> Result<Self, OwnerBetaTargetSnapshotError> {
        build_target_snapshot(input, factor_snapshot)
    }
}

#[derive(Serialize)]
struct CanonicalTarget<'a> {
    schema: &'a str,
    hash_algorithm: &'a str,
    input_kind: &'a str,
    capability: &'a str,
    as_of: &'a str,
    strategy_id: &'a str,
    strategy_version: &'a str,
    strategy_config_sha256: &'a str,
    factor_snapshot_sha256: &'a str,
    candidate_content_sha256: &'a str,
    artifact_manifest_sha256: &'a str,
    stage5_manifest_sha256: &'a str,
    action_manifest_sha256: &'a str,
    approval_registry_sha256: &'a str,
    items: Vec<CanonicalItem<'a>>,
    cash_weight_ppm: i64,
    portfolio_reasons: Vec<CanonicalReason<'a>>,
}

#[derive(Serialize)]
struct CanonicalItem<'a> {
    instrument_id: &'a str,
    rank: Option<u32>,
    score: Option<&'a str>,
    factors: &'a BTreeMap<String, String>,
    target_weight_ppm: Option<i64>,
    reasons: Vec<CanonicalReason<'a>>,
}

impl<'a> CanonicalItem<'a> {
    fn from_item(item: &'a OwnerBetaTargetItem) -> Self {
        Self {
            instrument_id: &item.instrument_id,
            rank: item.rank,
            score: item.score_text.as_deref(),
            factors: &item.factors_text,
            target_weight_ppm: item.target_weight_ppm,
            reasons: item
                .reasons
                .iter()
                .map(CanonicalReason::from_reason)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct CanonicalReason<'a> {
    code: &'a str,
    params: &'a BTreeMap<String, String>,
    text_ko: &'a str,
    text_en: &'a str,
}

impl<'a> CanonicalReason<'a> {
    fn from_reason(reason: &'a OwnerBetaReason) -> Self {
        Self {
            code: reason.code.as_str(),
            params: &reason.params,
            text_ko: &reason.text_ko,
            text_en: &reason.text_en,
        }
    }
}

/// Build one deterministic target snapshot from the sealed input and factor
/// snapshot.
pub fn build_target_snapshot(
    input: &OwnerBetaPriceRecommendationInput,
    factor_snapshot: &PriceOnlyFactorSnapshot,
) -> Result<OwnerBetaTargetSnapshot, OwnerBetaTargetSnapshotError> {
    input
        .validate_strategy_snapshot()
        .map_err(|_| OwnerBetaTargetSnapshotError::StrategyUnsupported)?;
    let strategy = input.strategy_snapshot();
    let resolved = ResolvedConfig {
        strategy_id: strategy.strategy_id().to_owned(),
        strategy_version: strategy.strategy_version().to_owned(),
        config: strategy.config_json().clone(),
    };
    let requirements = requirements_for(&resolved)
        .map_err(|_| OwnerBetaTargetSnapshotError::StrategyUnsupported)?;
    let factor_impls = factors_for(&requirements.factor_ids)
        .map_err(|_| OwnerBetaTargetSnapshotError::StrategyUnsupported)?;
    validate_factor_definitions(&factor_impls)?;
    validate_factor_snapshot(input, factor_snapshot, &requirements, &factor_impls)?;

    let values = values_at_as_of(factor_snapshot, &requirements.factor_ids, input.as_of())?;
    let (items, cash_weight_ppm, portfolio_reasons) = match resolved.strategy_id.as_str() {
        "buy_and_hold" => build_buy_and_hold(&resolved, &values)?,
        "trend_following" => build_trend_following(&resolved, &requirements.factor_ids, &values)?,
        "relative_momentum" => {
            build_relative_momentum(&resolved, &requirements.factor_ids, &values)?
        }
        "dual_momentum" => build_dual_momentum(&resolved, &requirements.factor_ids, &values)?,
        "inverse_volatility" => {
            build_inverse_volatility(&resolved, &requirements.factor_ids, &values)?
        }
        _ => return Err(OwnerBetaTargetSnapshotError::StrategyUnsupported),
    };

    assemble_snapshot(
        input,
        factor_snapshot,
        items,
        cash_weight_ppm,
        portfolio_reasons,
    )
}

fn validate_factor_definitions(
    factors: &[Box<dyn Factor>],
) -> Result<(), OwnerBetaTargetSnapshotError> {
    for factor in factors {
        let fields = factor.required_fields();
        if fields.len() != 1 || fields[0] != Field::CLOSE {
            return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
        }
    }
    Ok(())
}

fn validate_factor_snapshot(
    input: &OwnerBetaPriceRecommendationInput,
    snapshot: &PriceOnlyFactorSnapshot,
    factor_requirements: &StrategyRequirements,
    factor_impls: &[Box<dyn Factor>],
) -> Result<(), OwnerBetaTargetSnapshotError> {
    input
        .validate_factor_snapshot(snapshot)
        .map_err(|_| OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let computed_hash = snapshot
        .compute_hash()
        .map_err(|_| OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    if computed_hash != snapshot.hash {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }

    let expected_ids = factor_requirements
        .factor_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual_ids = snapshot
        .factor_versions
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected_ids != actual_ids || factor_impls.len() != expected_ids.len() {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    for factor in factor_impls {
        if snapshot.factor_versions.get(factor.id()) != Some(&factor.version().to_string()) {
            return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
        }
    }
    Ok(())
}

fn values_at_as_of(
    snapshot: &PriceOnlyFactorSnapshot,
    required_factor_ids: &[String],
    as_of: domain::TradingDate,
) -> Result<BTreeMap<String, BTreeMap<String, Option<f64>>>, OwnerBetaTargetSnapshotError> {
    let expected_instruments = canonical_instruments().into_iter().collect::<BTreeSet<_>>();
    let expected_factors = required_factor_ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut values = BTreeMap::<String, BTreeMap<String, Option<f64>>>::new();
    let as_of = as_of.to_iso();
    for row in snapshot.rows.iter().filter(|row| row.date == as_of) {
        if !expected_instruments.contains(&row.instrument)
            || !expected_factors.contains(&row.factor)
        {
            return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
        }
        let raw = validate_row_value(row)?;
        let entry = values.entry(row.instrument.clone()).or_default();
        if entry.insert(row.factor.clone(), raw).is_some() {
            return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
        }
    }
    let actual = values
        .iter()
        .flat_map(|(instrument, factors)| {
            factors
                .keys()
                .map(|factor| (instrument.clone(), factor.clone()))
        })
        .collect::<BTreeSet<_>>();
    let expected = expected_instruments
        .iter()
        .flat_map(|instrument| {
            expected_factors
                .iter()
                .map(|factor| (instrument.clone(), factor.clone()))
        })
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    Ok(values)
}

type BuildResult = (Vec<OwnerBetaTargetItem>, i64, Vec<OwnerBetaReason>);

fn assemble_snapshot(
    input: &OwnerBetaPriceRecommendationInput,
    factor_snapshot: &PriceOnlyFactorSnapshot,
    mut items: Vec<OwnerBetaTargetItem>,
    cash_weight_ppm: i64,
    portfolio_reasons: Vec<OwnerBetaReason>,
) -> Result<OwnerBetaTargetSnapshot, OwnerBetaTargetSnapshotError> {
    let expected = canonical_instruments();
    items.sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
    if items.len() != expected.len()
        || items
            .iter()
            .map(|item| item.instrument_id.as_str())
            .collect::<BTreeSet<_>>()
            != expected.iter().map(String::as_str).collect()
        || cash_weight_ppm < 0
    {
        return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
    }
    let mut total = cash_weight_ppm;
    for item in &items {
        match item.target_weight_ppm {
            Some(weight) if weight >= 0 => {
                total = total
                    .checked_add(weight)
                    .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
            }
            Some(_) => return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid),
            None if item.reasons.is_empty() => {
                return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
            }
            None => {}
        }
    }
    if total != OWNER_BETA_TARGET_WEIGHT_SCALE {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let strategy = input.strategy_snapshot();
    let mut snapshot = OwnerBetaTargetSnapshot {
        schema: OWNER_BETA_TARGET_SNAPSHOT_SCHEMA,
        hash_algorithm: OWNER_BETA_TARGET_HASH_ALGORITHM,
        input_kind: factor_snapshot.input_kind.clone(),
        capability: factor_snapshot.capability.clone(),
        as_of: input.as_of(),
        strategy_id: strategy.strategy_id().to_owned(),
        strategy_version: strategy.strategy_version().to_owned(),
        strategy_config_sha256: strategy.config_sha256().clone(),
        factor_snapshot_sha256: factor_snapshot.hash.clone(),
        pins: input.pins().clone(),
        items,
        cash_weight_ppm,
        portfolio_reasons,
        target_snapshot_sha256: ContentHash::from_bytes(b"owner-beta-target-placeholder"),
    };
    snapshot.target_snapshot_sha256 = snapshot.recompute_hash()?;
    Ok(snapshot)
}

fn canonical_instruments() -> Vec<String> {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort_unstable();
    instruments
}

fn values_for(
    values: &BTreeMap<String, BTreeMap<String, Option<f64>>>,
    instrument: &str,
    factor: &str,
) -> Result<Option<f64>, OwnerBetaTargetSnapshotError> {
    let value = values
        .get(instrument)
        .and_then(|factors| factors.get(factor))
        .copied()
        .ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    Ok(value)
}

fn canonical_factor_map(
    factor_values: impl IntoIterator<Item = (String, f64)>,
) -> Result<BTreeMap<String, String>, OwnerBetaTargetSnapshotError> {
    factor_values
        .into_iter()
        .map(|(factor, value)| Ok((factor, python_float_string(value)?)))
        .collect()
}

fn selected_item(
    instrument_id: String,
    rank: u32,
    score: f64,
    factors: BTreeMap<String, f64>,
    target_weight_ppm: i64,
    reasons: Vec<OwnerBetaReason>,
) -> Result<OwnerBetaTargetItem, OwnerBetaTargetSnapshotError> {
    if target_weight_ppm < 0 {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    Ok(OwnerBetaTargetItem {
        instrument_id,
        rank: Some(rank),
        score_text: Some(python_float_string(score)?),
        factors_text: canonical_factor_map(factors)?,
        target_weight_ppm: Some(target_weight_ppm),
        reasons,
    })
}

fn excluded_item(instrument_id: String, reasons: Vec<OwnerBetaReason>) -> OwnerBetaTargetItem {
    OwnerBetaTargetItem {
        instrument_id,
        rank: None,
        score_text: None,
        factors_text: BTreeMap::new(),
        target_weight_ppm: None,
        reasons,
    }
}

fn reason(
    code: OwnerBetaReasonCode,
    params: impl IntoIterator<Item = (&'static str, String)>,
) -> OwnerBetaReason {
    let params = params
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect::<BTreeMap<_, _>>();
    let param = |key: &str| params.get(key).map(String::as_str).unwrap_or("?");
    let (text_ko, text_en) = match code {
        OwnerBetaReasonCode::SelectedTopN => (
            format!(
                "상위 {}개 이내 선정 (순위 {})",
                param("top_n"),
                param("rank")
            ),
            format!("Ranked {} within top {}", param("rank"), param("top_n")),
        ),
        OwnerBetaReasonCode::NotSelectedBeyondTopN => (
            format!("순위 {} — 상위 {} 밖", param("rank"), param("top_n")),
            format!("Rank {} is beyond top {}", param("rank"), param("top_n")),
        ),
        OwnerBetaReasonCode::ExcludedMandatoryFactorNull => (
            format!("필수 팩터 {} 결측(NULL)으로 제외", param("factor")),
            format!("Excluded: mandatory factor {} is NULL", param("factor")),
        ),
        OwnerBetaReasonCode::AllCashNoEligible => (
            "선정 가능한 종목이 없어 전액 현금 유지".to_owned(),
            "No eligible instrument; portfolio held in cash".to_owned(),
        ),
        OwnerBetaReasonCode::WeightCappedAtMax => (
            format!("최대 비중 {} 상한 적용", param("max_weight")),
            format!("Weight capped at max {}", param("max_weight")),
        ),
        OwnerBetaReasonCode::WeightRoundingResidueToCash => (
            format!("반올림 잔여 {}을 현금으로 배분", param("residue")),
            format!("Rounding residue {} allocated to cash", param("residue")),
        ),
        OwnerBetaReasonCode::CashFloorApplied => (
            format!("현금 최소 비중 {} 보장", param("cash_floor")),
            format!("Cash floor {} maintained", param("cash_floor")),
        ),
        OwnerBetaReasonCode::BenchmarkHeld => (
            format!(
                "벤치마크 {} 비중 {} 보유",
                param("benchmark"),
                param("target_weight")
            ),
            format!(
                "Holding benchmark {} at weight {}",
                param("benchmark"),
                param("target_weight")
            ),
        ),
        OwnerBetaReasonCode::TrendPositive => (
            format!(
                "빠른 이평 {} > 느린 이평 {} — 상승 추세",
                param("fast"),
                param("slow")
            ),
            format!(
                "Fast MA {} above slow MA {} — uptrend",
                param("fast"),
                param("slow")
            ),
        ),
        OwnerBetaReasonCode::TrendNegativeCash => (
            format!(
                "빠른 이평 {} <= 느린 이평 {} — 현금 유지",
                param("fast"),
                param("slow")
            ),
            format!(
                "Fast MA {} at or below slow MA {} — hold cash",
                param("fast"),
                param("slow")
            ),
        ),
        OwnerBetaReasonCode::AbsoluteMomentumPassed => (
            format!(
                "12개월 수익률 {}이 절대 모멘텀 기준 {} 초과",
                param("return_"),
                param("threshold")
            ),
            format!(
                "12-month return {} above absolute threshold {}",
                param("return_"),
                param("threshold")
            ),
        ),
        OwnerBetaReasonCode::DefensiveCashSelected => (
            format!(
                "최고 수익률 {}이 절대 모멘텀 기준 {} 이하 — 방어적 현금",
                param("return_"),
                param("threshold")
            ),
            format!(
                "Best return {} at or below absolute threshold {} — defensive cash",
                param("return_"),
                param("threshold")
            ),
        ),
        OwnerBetaReasonCode::InverseVolWeighted => (
            format!("변동성 {} 역가중 비중 {}", param("vol"), param("weight")),
            format!(
                "Inverse-volatility weight {} (vol {})",
                param("weight"),
                param("vol")
            ),
        ),
        OwnerBetaReasonCode::NotSelectedByStrategy => (
            "전략이 이 고정 유니버스 종목을 선택하지 않았습니다.".to_owned(),
            "The strategy did not select this canonical universe member.".to_owned(),
        ),
    };
    OwnerBetaReason {
        code,
        params,
        text_ko,
        text_en,
    }
}

fn strategy_string<'a>(
    config: &'a Value,
    key: &'static str,
) -> Result<&'a str, OwnerBetaTargetSnapshotError> {
    config
        .get(key)
        .and_then(Value::as_str)
        .ok_or(OwnerBetaTargetSnapshotError::TargetInputInvalid)
}

fn strategy_u64(config: &Value, key: &'static str) -> Result<u64, OwnerBetaTargetSnapshotError> {
    config
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(OwnerBetaTargetSnapshotError::TargetInputInvalid)
}

fn strategy_f64(config: &Value, key: &'static str) -> Result<f64, OwnerBetaTargetSnapshotError> {
    let value = config
        .get(key)
        .and_then(Value::as_f64)
        .ok_or(OwnerBetaTargetSnapshotError::TargetInputInvalid)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(OwnerBetaTargetSnapshotError::TargetInputInvalid)
    }
}

fn benchmark(config: &Value) -> Result<String, OwnerBetaTargetSnapshotError> {
    let benchmark = strategy_string(config, "benchmark_instrument")?;
    if !canonical_instruments().iter().any(|id| id == benchmark) {
        return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
    }
    Ok(benchmark.to_owned())
}

fn parse_fixed_units(
    text: &str,
    decimal_places: usize,
) -> Result<i64, OwnerBetaTargetSnapshotError> {
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (whole, fraction) = unsigned
        .split_once('.')
        .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    if fraction.len() != decimal_places
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let scale = pow10(
        u32::try_from(decimal_places)
            .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?,
    )?;
    let whole = whole
        .parse::<i128>()
        .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let fraction = fraction
        .parse::<i128>()
        .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let units = whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction))
        .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let units = if negative { -units } else { units };
    i64::try_from(units).map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)
}

fn round_py4(value: f64) -> Result<i64, OwnerBetaTargetSnapshotError> {
    if !value.is_finite() {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    parse_fixed_units(&format!("{value:.4}"), 4)
}

fn exact_scaled_number(
    number: &Number,
    decimal_places: u32,
) -> Result<i64, OwnerBetaTargetSnapshotError> {
    let text = number.to_string();
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(index) => {
            let exponent = text[index + 1..]
                .parse::<i32>()
                .map_err(|_| OwnerBetaTargetSnapshotError::TargetInputInvalid)?;
            (&text[..index], exponent)
        }
        None => (text.as_str(), 0),
    };
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.strip_prefix(['+', '-']).unwrap_or(mantissa);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
    }
    let digits = format!("{whole}{fraction}");
    let significand = digits
        .parse::<i128>()
        .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let power = exponent
        .checked_add(decimal_places as i32)
        .and_then(|power| power.checked_sub(fraction.len() as i32))
        .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let scaled = if power >= 0 {
        significand
            .checked_mul(pow10(power as u32)?)
            .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?
    } else {
        let divisor = pow10((-power) as u32)?;
        if significand % divisor != 0 {
            return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
        }
        significand / divisor
    };
    let scaled = if negative { -scaled } else { scaled };
    i64::try_from(scaled).map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)
}

fn pow10(power: u32) -> Result<i128, OwnerBetaTargetSnapshotError> {
    let mut result = 1_i128;
    for _ in 0..power {
        result = result
            .checked_mul(10)
            .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    }
    Ok(result)
}

fn format_fixed_six(value: i64) -> String {
    let negative = value < 0;
    let absolute = value.unsigned_abs();
    let whole = absolute / OWNER_BETA_TARGET_WEIGHT_SCALE as u64;
    let fraction = absolute % OWNER_BETA_TARGET_WEIGHT_SCALE as u64;
    if negative {
        format!("-{whole}.{fraction:06}")
    } else {
        format!("{whole}.{fraction:06}")
    }
}

fn python_float_string(value: f64) -> Result<String, OwnerBetaTargetSnapshotError> {
    if !value.is_finite() {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let mut text = format!("{value:?}");
    if let Some(index) = text.find(['e', 'E']) {
        let mantissa = &text[..index];
        let exponent = text[index + 1..]
            .parse::<i32>()
            .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
        let sign = if exponent < 0 { '-' } else { '+' };
        return Ok(format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs()));
    }
    if !text.contains('.') {
        text.push_str(".0");
    }
    Ok(text)
}

fn python_round_string(
    value: f64,
    decimal_places: usize,
) -> Result<String, OwnerBetaTargetSnapshotError> {
    if !value.is_finite() {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let rounded = format!("{value:.decimal_places$}")
        .parse::<f64>()
        .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    python_float_string(rounded)
}

fn python_round4_string(units: i64) -> String {
    let negative = units < 0;
    let absolute = units.unsigned_abs();
    let whole = absolute / 10_000;
    let fraction = absolute % 10_000;
    let mut text = format!("{whole}.{fraction:04}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.push('0');
    }
    if negative && units != 0 {
        text.insert(0, '-');
    }
    text
}

fn scale_weight_units(units: i64) -> Result<i64, OwnerBetaTargetSnapshotError> {
    units
        .checked_mul(100)
        .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)
}

fn cash_from_round4(
    rounded_units: i64,
    selected_count: usize,
) -> Result<i64, OwnerBetaTargetSnapshotError> {
    let selected = rounded_units
        .checked_mul(
            i64::try_from(selected_count)
                .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?,
        )
        .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let selected_total = round_py4(rounded_units as f64 / 10_000.0 * selected_count as f64)?;
    let cash = round_py4(1.0 - selected_total as f64 / 10_000.0)?;
    if cash < 0 {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let cash_ppm = scale_weight_units(cash)?;
    if selected
        .checked_mul(100)
        .and_then(|weights| weights.checked_add(cash_ppm))
        != Some(OWNER_BETA_TARGET_WEIGHT_SCALE)
    {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    Ok(cash_ppm)
}

fn fill_not_selected(selected: &BTreeSet<String>) -> Vec<OwnerBetaTargetItem> {
    canonical_instruments()
        .into_iter()
        .filter(|instrument| !selected.contains(instrument))
        .map(|instrument| {
            excluded_item(
                instrument,
                vec![reason(OwnerBetaReasonCode::NotSelectedByStrategy, [])],
            )
        })
        .collect()
}

fn build_buy_and_hold(
    resolved: &ResolvedConfig,
    values: &BTreeMap<String, BTreeMap<String, Option<f64>>>,
) -> Result<BuildResult, OwnerBetaTargetSnapshotError> {
    if !values.is_empty() {
        // The factor gate already rejects requested-date rows for a no-factor
        // strategy; retaining this check keeps the equation total if called
        // independently in a future unit test.
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    let benchmark = benchmark(&resolved.config)?;
    let number = resolved
        .config
        .get("target_weight")
        .and_then(Value::as_number)
        .ok_or(OwnerBetaTargetSnapshotError::TargetInputInvalid)?;
    let target_weight_ppm = exact_scaled_number(number, 6)?;
    if !(0..=OWNER_BETA_TARGET_WEIGHT_SCALE).contains(&target_weight_ppm) {
        return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
    }
    let target_weight = number
        .as_f64()
        .ok_or(OwnerBetaTargetSnapshotError::TargetInputInvalid)?;
    let cash_units = round_py4(1.0 - target_weight)?;
    if cash_units < 0 {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let cash_weight_ppm = scale_weight_units(cash_units)?;
    if target_weight_ppm.checked_add(cash_weight_ppm) != Some(OWNER_BETA_TARGET_WEIGHT_SCALE) {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let selected = selected_item(
        benchmark.clone(),
        1,
        target_weight,
        BTreeMap::new(),
        target_weight_ppm,
        vec![reason(
            OwnerBetaReasonCode::BenchmarkHeld,
            [
                ("benchmark", benchmark),
                ("target_weight", python_float_string(target_weight)?),
            ],
        )],
    )?;
    let mut selected_ids = BTreeSet::new();
    selected_ids.insert(selected.instrument_id.clone());
    let mut items = vec![selected];
    items.extend(fill_not_selected(&selected_ids));
    let portfolio_reasons = if cash_units == 0 {
        Vec::new()
    } else {
        vec![reason(
            OwnerBetaReasonCode::CashFloorApplied,
            [("cash_floor", python_round4_string(cash_units))],
        )]
    };
    Ok((items, cash_weight_ppm, portfolio_reasons))
}

fn build_trend_following(
    resolved: &ResolvedConfig,
    factor_ids: &[String],
    values: &BTreeMap<String, BTreeMap<String, Option<f64>>>,
) -> Result<BuildResult, OwnerBetaTargetSnapshotError> {
    let benchmark = benchmark(&resolved.config)?;
    let fast = strategy_u64(&resolved.config, "fast_ma")?;
    let slow = strategy_u64(&resolved.config, "slow_ma")?;
    let fast_id = format!("trend_{fast}");
    let slow_id = format!("trend_{slow}");
    if !factor_ids.contains(&fast_id) || !factor_ids.contains(&slow_id) {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    let fast_value = values_for(values, &benchmark, &fast_id)?
        .ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let slow_value = values_for(values, &benchmark, &slow_id)?
        .ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    if fast_value > slow_value {
        let factors = if fast_id == slow_id {
            BTreeMap::from([(fast_id, fast_value)])
        } else {
            BTreeMap::from([(fast_id, fast_value), (slow_id, slow_value)])
        };
        let item = selected_item(
            benchmark,
            1,
            fast_value,
            factors,
            OWNER_BETA_TARGET_WEIGHT_SCALE,
            vec![reason(
                OwnerBetaReasonCode::TrendPositive,
                [("fast", fast.to_string()), ("slow", slow.to_string())],
            )],
        )?;
        let mut selected_ids = BTreeSet::new();
        selected_ids.insert(item.instrument_id.clone());
        let mut items = vec![item];
        items.extend(fill_not_selected(&selected_ids));
        Ok((items, 0, Vec::new()))
    } else {
        let items = fill_not_selected(&BTreeSet::new());
        Ok((
            items,
            OWNER_BETA_TARGET_WEIGHT_SCALE,
            vec![reason(
                OwnerBetaReasonCode::TrendNegativeCash,
                [("fast", fast.to_string()), ("slow", slow.to_string())],
            )],
        ))
    }
}

fn build_relative_momentum(
    resolved: &ResolvedConfig,
    factor_ids: &[String],
    values: &BTreeMap<String, BTreeMap<String, Option<f64>>>,
) -> Result<BuildResult, OwnerBetaTargetSnapshotError> {
    let factor_id = factor_ids
        .first()
        .ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let top_n = usize::try_from(strategy_u64(&resolved.config, "top_n")?)
        .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    let instruments = canonical_instruments();
    let mut ranked = Vec::with_capacity(instruments.len());
    let mut missing = Vec::new();
    for instrument in &instruments {
        match values_for(values, instrument, factor_id)? {
            Some(value) => ranked.push((instrument.clone(), value)),
            None => missing.push(instrument.clone()),
        }
    }
    if ranked.is_empty() {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    ranked.sort_by(|left, right| match right.1.partial_cmp(&left.1) {
        Some(Ordering::Equal) | None => left.0.cmp(&right.0),
        Some(ordering) => ordering,
    });
    let top = ranked
        .get(..top_n.min(ranked.len()))
        .ok_or(OwnerBetaTargetSnapshotError::TargetInputInvalid)?;
    if top.is_empty() {
        return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
    }
    let rounded_units = round_py4(1.0 / top.len() as f64)?;
    if rounded_units <= 0 {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let cash_weight_ppm = cash_from_round4(rounded_units, top.len())?;
    let top_n_text = top_n.to_string();
    let mut items = Vec::with_capacity(instruments.len());
    top.iter()
        .enumerate()
        .map(|(rank, (instrument, value))| {
            let item = selected_item(
                instrument.clone(),
                u32::try_from(rank + 1)
                    .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?,
                *value,
                BTreeMap::from([(factor_id.clone(), *value)]),
                scale_weight_units(rounded_units)?,
                vec![reason(
                    OwnerBetaReasonCode::SelectedTopN,
                    [
                        ("top_n", top_n_text.clone()),
                        ("rank", (rank + 1).to_string()),
                    ],
                )],
            )?;
            items.push(item);
            Ok(())
        })
        .collect::<Result<Vec<_>, OwnerBetaTargetSnapshotError>>()?;
    for (rank, (instrument, _)) in ranked.iter().enumerate().skip(top.len()) {
        items.push(excluded_item(
            instrument.clone(),
            vec![reason(
                OwnerBetaReasonCode::NotSelectedBeyondTopN,
                [
                    ("rank", (rank + 1).to_string()),
                    ("top_n", top_n_text.clone()),
                ],
            )],
        ));
    }
    for instrument in missing {
        items.push(excluded_item(
            instrument,
            vec![reason(
                OwnerBetaReasonCode::ExcludedMandatoryFactorNull,
                [("factor", factor_id.clone())],
            )],
        ));
    }
    Ok((items, cash_weight_ppm, Vec::new()))
}

fn build_dual_momentum(
    resolved: &ResolvedConfig,
    factor_ids: &[String],
    values: &BTreeMap<String, BTreeMap<String, Option<f64>>>,
) -> Result<BuildResult, OwnerBetaTargetSnapshotError> {
    let factor_id = factor_ids
        .first()
        .ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let threshold = strategy_f64(&resolved.config, "absolute_threshold")?;
    let mut best: Option<(String, f64)> = None;
    let mut missing = BTreeSet::new();
    for instrument in canonical_instruments() {
        let Some(value) = values_for(values, &instrument, factor_id)? else {
            missing.insert(instrument);
            continue;
        };
        if best.as_ref().is_none_or(|(best_instrument, best_value)| {
            value > *best_value || (value == *best_value && instrument > *best_instrument)
        }) {
            best = Some((instrument, value));
        }
    }
    let (best_instrument, best_value) =
        best.ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let return_text = python_round_string(best_value, 6)?;
    let threshold_text = python_float_string(threshold)?;
    if best_value > threshold {
        let item = selected_item(
            best_instrument.clone(),
            1,
            best_value,
            BTreeMap::from([(factor_id.clone(), best_value)]),
            OWNER_BETA_TARGET_WEIGHT_SCALE,
            vec![reason(
                OwnerBetaReasonCode::AbsoluteMomentumPassed,
                [
                    ("instrument", best_instrument.clone()),
                    ("return_", return_text),
                    ("threshold", threshold_text),
                ],
            )],
        )?;
        let mut selected = BTreeSet::new();
        selected.insert(best_instrument);
        let mut items = vec![item];
        items.extend(
            fill_not_selected(&selected)
                .into_iter()
                .filter(|item| !missing.contains(item.instrument_id())),
        );
        items.extend(missing.into_iter().map(|instrument| {
            excluded_item(
                instrument,
                vec![reason(
                    OwnerBetaReasonCode::ExcludedMandatoryFactorNull,
                    [("factor", factor_id.clone())],
                )],
            )
        }));
        Ok((items, 0, Vec::new()))
    } else {
        let items = canonical_instruments()
            .into_iter()
            .map(|instrument| {
                if missing.contains(&instrument) {
                    excluded_item(
                        instrument,
                        vec![reason(
                            OwnerBetaReasonCode::ExcludedMandatoryFactorNull,
                            [("factor", factor_id.clone())],
                        )],
                    )
                } else {
                    excluded_item(
                        instrument,
                        vec![reason(OwnerBetaReasonCode::NotSelectedByStrategy, [])],
                    )
                }
            })
            .collect();
        Ok((
            items,
            OWNER_BETA_TARGET_WEIGHT_SCALE,
            vec![reason(
                OwnerBetaReasonCode::DefensiveCashSelected,
                [
                    ("instrument", best_instrument),
                    ("return_", return_text),
                    ("threshold", threshold_text),
                ],
            )],
        ))
    }
}

fn build_inverse_volatility(
    resolved: &ResolvedConfig,
    factor_ids: &[String],
    values: &BTreeMap<String, BTreeMap<String, Option<f64>>>,
) -> Result<BuildResult, OwnerBetaTargetSnapshotError> {
    let factor_id = factor_ids
        .first()
        .ok_or(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)?;
    let max_weight = strategy_f64(&resolved.config, "max_weight")?;
    if !(0.0 < max_weight && max_weight <= 1.0) {
        return Err(OwnerBetaTargetSnapshotError::TargetInputInvalid);
    }
    let mut missing = Vec::new();
    let mut inverse = Vec::new();
    for instrument in canonical_instruments() {
        let Some(volatility) = values_for(values, &instrument, factor_id)? else {
            missing.push(instrument);
            continue;
        };
        if volatility <= 0.0 {
            return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
        }
        let inverse_weight = 1.0 / volatility;
        if !inverse_weight.is_finite() || inverse_weight <= 0.0 {
            return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
        }
        inverse.push((instrument, volatility, inverse_weight));
    }
    if inverse.is_empty() {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    let total_inverse = inverse.iter().try_fold(0.0, |total, (_, _, value)| {
        let next = total + value;
        if next.is_finite() {
            Ok(next)
        } else {
            Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid)
        }
    })?;
    if total_inverse <= 0.0 {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let mut rounded_units = Vec::with_capacity(inverse.len());
    for (instrument, volatility, inverse_value) in &inverse {
        let raw_weight = inverse_value / total_inverse;
        if !raw_weight.is_finite() || raw_weight < 0.0 {
            return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
        }
        let capped = raw_weight.min(max_weight);
        let units = round_py4(capped)?;
        rounded_units.push((instrument, *volatility, raw_weight, units));
    }
    let (sum_units, rounded_sum) = rounded_units.iter().try_fold(
        (0_i64, 0.0_f64),
        |(sum_units, rounded_sum), (_, _, _, units)| {
            let sum_units = sum_units
                .checked_add(*units)
                .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
            let rounded_sum = rounded_sum + *units as f64 / 10_000.0;
            if rounded_sum.is_finite() {
                Ok((sum_units, rounded_sum))
            } else {
                Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid)
            }
        },
    )?;
    let cash_units = round_py4(1.0 - rounded_sum)?;
    if cash_units < 0 {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let cash_weight_ppm = scale_weight_units(cash_units)?;
    let total_ppm = sum_units
        .checked_mul(100)
        .and_then(|sum| sum.checked_add(cash_weight_ppm))
        .ok_or(OwnerBetaTargetSnapshotError::ArithmeticInvalid)?;
    if total_ppm != OWNER_BETA_TARGET_WEIGHT_SCALE {
        return Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid);
    }
    let mut items = Vec::with_capacity(rounded_units.len());
    for (index, (instrument, volatility, raw_weight, units)) in
        rounded_units.into_iter().enumerate()
    {
        let mut reasons = vec![reason(
            OwnerBetaReasonCode::InverseVolWeighted,
            [
                ("instrument", instrument.clone()),
                ("vol", python_round_string(volatility, 6)?),
                ("weight", python_round4_string(units)),
            ],
        )];
        if raw_weight > max_weight {
            reasons.push(reason(
                OwnerBetaReasonCode::WeightCappedAtMax,
                [("max_weight", python_float_string(max_weight)?)],
            ));
        }
        items.push(selected_item(
            instrument.clone(),
            u32::try_from(index + 1)
                .map_err(|_| OwnerBetaTargetSnapshotError::ArithmeticInvalid)?,
            raw_weight,
            BTreeMap::from([(factor_id.clone(), raw_weight)]),
            scale_weight_units(units)?,
            reasons,
        )?);
    }
    items.extend(missing.into_iter().map(|instrument| {
        excluded_item(
            instrument,
            vec![reason(
                OwnerBetaReasonCode::ExcludedMandatoryFactorNull,
                [("factor", factor_id.clone())],
            )],
        )
    }));
    let portfolio_reasons = if cash_units == 0 {
        Vec::new()
    } else {
        vec![reason(
            OwnerBetaReasonCode::WeightRoundingResidueToCash,
            [("residue", python_round4_string(cash_units))],
        )]
    };
    Ok((items, cash_weight_ppm, portfolio_reasons))
}

fn validate_row_value(row: &FactorRow) -> Result<Option<f64>, OwnerBetaTargetSnapshotError> {
    if row.raw.is_some_and(|raw| !raw.is_finite())
        || row
            .normalized
            .is_some_and(|normalized| !normalized.is_finite())
        || (row.raw.is_none() && row.normalized.is_some())
    {
        return Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid);
    }
    Ok(row.raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_beta::OwnerBetaStrategySnapshot;
    use domain::TradingDate;
    use factor_engine::price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND};
    use factor_engine::snapshot::NormalizationMeta;
    use serde_json::json;

    fn date() -> TradingDate {
        TradingDate::parse("2026-08-24").expect("test date")
    }

    fn pin(value: u8) -> String {
        format!("sha256:{value:064x}")
    }

    fn resolved(strategy_id: &str, config: Value) -> ResolvedConfig {
        ResolvedConfig {
            strategy_id: strategy_id.to_owned(),
            strategy_version: "1.0.0".to_owned(),
            config,
        }
    }

    fn input_for(resolved: &ResolvedConfig) -> OwnerBetaPriceRecommendationInput {
        let strategy =
            OwnerBetaStrategySnapshot::from_resolved_config(resolved).expect("strategy snapshot");
        serde_json::from_value(json!({
            "run_id": "00000000-0000-4000-8000-000000000001",
            "strategy_config_id": "00000000-0000-4000-8000-000000000002",
            "as_of": date().to_iso(),
            "pins": {
                "candidate_content_sha256": pin(1),
                "artifact_manifest_sha256": pin(2),
                "stage5_manifest_sha256": pin(3),
                "action_manifest_sha256": pin(4),
                "approval_registry_sha256": pin(5),
            },
            "strategy": serde_json::to_value(strategy).expect("strategy json"),
        }))
        .expect("owner-beta input")
    }

    fn factor_snapshot(
        input: &OwnerBetaPriceRecommendationInput,
        factor_ids: &[&str],
        value: impl Fn(&str, &str) -> f64,
    ) -> PriceOnlyFactorSnapshot {
        let mut rows = Vec::new();
        for instrument in canonical_instruments() {
            for factor in factor_ids {
                rows.push(FactorRow {
                    date: date().to_iso(),
                    instrument: instrument.clone(),
                    factor: (*factor).to_owned(),
                    raw: Some(value(&instrument, factor)),
                    normalized: Some(0.0),
                });
            }
        }
        let factor_versions = factor_ids
            .iter()
            .map(|factor| ((*factor).to_owned(), "1.0.0".to_owned()))
            .collect();
        let mut snapshot = PriceOnlyFactorSnapshot {
            input_kind: PRICE_ONLY_INPUT_KIND.to_owned(),
            capability: PRICE_ONLY_CAPABILITY.to_owned(),
            as_of: date(),
            candidate_content_sha256: input.pins().candidate_content_sha256().to_string(),
            artifact_manifest_sha256: input.pins().artifact_manifest_sha256().to_string(),
            stage5_manifest_sha256: input.pins().stage5_manifest_sha256().to_string(),
            action_manifest_sha256: input.pins().action_manifest_sha256().to_string(),
            approval_registry_sha256: input.pins().approval_registry_sha256().to_string(),
            factor_versions,
            normalization: NormalizationMeta {
                id: "test".to_owned(),
                version: "1.0.0".to_owned(),
                params: BTreeMap::new(),
            },
            rows,
            hash: ContentHash::from_bytes(b"placeholder"),
        };
        snapshot.hash = snapshot.compute_hash().expect("factor hash");
        snapshot
    }

    fn rows_for_ids(snapshot: &PriceOnlyFactorSnapshot) -> BTreeSet<(String, String)> {
        snapshot
            .rows
            .iter()
            .map(|row| (row.instrument.clone(), row.factor.clone()))
            .collect()
    }

    #[test]
    fn five_baselines_build_exact_eleven_item_snapshots() {
        let buy = input_for(&resolved(
            "buy_and_hold",
            json!({"benchmark_instrument": "069500.KRX", "target_weight": 1.0}),
        ));
        let buy_factors = factor_snapshot(&buy, &[], |_, _| 0.0);
        let buy_target = build_target_snapshot(&buy, &buy_factors).expect("buy target");
        assert_eq!(buy_target.items().len(), 11);
        assert_eq!(buy_target.cash_weight(), "0.000000");
        assert_eq!(
            buy_target
                .items()
                .iter()
                .find(|item| item.instrument_id() == "069500.KRX")
                .and_then(OwnerBetaTargetItem::target_weight),
            Some("1.000000".to_owned())
        );
        assert!(
            buy_target
                .items()
                .iter()
                .filter(|item| item.target_weight_ppm().is_none())
                .all(|item| item.reasons()[0].code() == "NOT_SELECTED_BY_STRATEGY")
        );

        let trend = input_for(&resolved(
            "trend_following",
            json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 100,
                "slow_ma": 200
            }),
        ));
        let trend_factors =
            factor_snapshot(&trend, &["trend_100", "trend_200"], |instrument, factor| {
                if instrument == "069500.KRX" && factor == "trend_100" {
                    2.0
                } else if instrument == "069500.KRX" {
                    1.0
                } else {
                    0.0
                }
            });
        let trend_target = build_target_snapshot(&trend, &trend_factors).expect("trend target");
        assert_eq!(trend_target.cash_weight_ppm(), 0);
        assert_eq!(
            trend_target
                .items()
                .iter()
                .find(|item| item.instrument_id() == "069500.KRX")
                .and_then(OwnerBetaTargetItem::target_weight),
            Some("1.000000".to_owned())
        );

        let relative = input_for(&resolved(
            "relative_momentum",
            json!({"top_n": 3, "lookback_months": 12}),
        ));
        let relative_factors = factor_snapshot(&relative, &["momentum_12_1"], |_, _| 1.0);
        let relative_target =
            build_target_snapshot(&relative, &relative_factors).expect("relative target");
        assert_eq!(relative_target.cash_weight_ppm(), 100);
        assert_eq!(
            relative_target
                .items()
                .iter()
                .filter(|item| item.target_weight_ppm().is_some())
                .count(),
            3
        );
        assert_eq!(
            relative_target
                .items()
                .iter()
                .filter(|item| item.target_weight_ppm().is_some())
                .map(OwnerBetaTargetItem::instrument_id)
                .collect::<Vec<_>>(),
            vec!["069500.KRX", "102110.KRX", "114260.KRX"]
        );

        let dual = input_for(&resolved(
            "dual_momentum",
            json!({"absolute_threshold": 0.0, "lookback_months": 12}),
        ));
        let dual_factors = factor_snapshot(&dual, &["return_12m"], |_, _| 0.1);
        let dual_target = build_target_snapshot(&dual, &dual_factors).expect("dual target");
        assert_eq!(
            dual_target
                .items()
                .iter()
                .find(|item| item.target_weight_ppm().is_some())
                .map(OwnerBetaTargetItem::instrument_id),
            Some("229200.KRX")
        );

        let inverse = input_for(&resolved(
            "inverse_volatility",
            json!({"vol_window": 60, "max_weight": 0.3}),
        ));
        let inverse_factors = factor_snapshot(&inverse, &["vol_60"], |instrument, _| {
            canonical_instruments()
                .iter()
                .position(|candidate| candidate == instrument)
                .map(|index| (index + 1) as f64)
                .unwrap_or(1.0)
        });
        let inverse_target =
            build_target_snapshot(&inverse, &inverse_factors).expect("inverse target");
        assert_eq!(inverse_target.items().len(), 11);
        assert!(
            inverse_target
                .items()
                .iter()
                .all(|item| item.target_weight_ppm().is_some_and(|weight| weight > 0))
        );
        assert_eq!(
            inverse_target
                .items()
                .iter()
                .map(|item| item.target_weight_ppm().unwrap_or_default())
                .sum::<i64>()
                + inverse_target.cash_weight_ppm(),
            OWNER_BETA_TARGET_WEIGHT_SCALE
        );
    }

    #[test]
    fn defensive_and_boundary_strategy_cases_match_the_contract() {
        let trend = input_for(&resolved(
            "trend_following",
            json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 100,
                "slow_ma": 100
            }),
        ));
        let trend_factors = factor_snapshot(&trend, &["trend_100"], |_, _| 1.0);
        let trend_target = build_target_snapshot(&trend, &trend_factors).expect("cash trend");
        assert_eq!(
            trend_target.cash_weight_ppm(),
            OWNER_BETA_TARGET_WEIGHT_SCALE
        );
        assert_eq!(
            trend_target.portfolio_reasons()[0].code(),
            "TREND_NEGATIVE_CASH"
        );

        let dual = input_for(&resolved(
            "dual_momentum",
            json!({"absolute_threshold": 0.1, "lookback_months": 12}),
        ));
        let dual_factors = factor_snapshot(&dual, &["return_12m"], |_, _| 0.1);
        let dual_target = build_target_snapshot(&dual, &dual_factors).expect("threshold cash");
        assert_eq!(
            dual_target.cash_weight_ppm(),
            OWNER_BETA_TARGET_WEIGHT_SCALE
        );
        assert_eq!(
            dual_target.portfolio_reasons()[0].code(),
            "DEFENSIVE_CASH_SELECTED"
        );

        assert_eq!(round_py4(0.00005).expect("round"), 1);
        assert_eq!(round_py4(0.00015).expect("round"), 1);
        assert_eq!(round_py4(0.00025).expect("round"), 3);
        assert_eq!(round_py4(0.00035).expect("round"), 3);
        assert_eq!(round_py4(0.33335).expect("round"), 3333);
        assert_eq!(round_py4(1.23445).expect("round"), 12345);
        assert_eq!(round_py4(1.0 / 3.0).expect("round"), 3333);
        assert_eq!(python_float_string(0.123456789).unwrap(), "0.123456789");
        assert_eq!(python_float_string(1e-7).unwrap(), "1e-07");
        assert_eq!(python_round_string(0.0000025, 6).unwrap(), "3e-06");

        let invalid_benchmark = input_for(&resolved(
            "buy_and_hold",
            json!({"benchmark_instrument": "999999.KRX", "target_weight": 1.0}),
        ));
        let invalid_factors = factor_snapshot(&invalid_benchmark, &[], |_, _| 0.0);
        assert_eq!(
            build_target_snapshot(&invalid_benchmark, &invalid_factors),
            Err(OwnerBetaTargetSnapshotError::TargetInputInvalid)
        );

        let unsupported_fraction = input_for(&resolved(
            "buy_and_hold",
            json!({"benchmark_instrument": "069500.KRX", "target_weight": 0.333333}),
        ));
        let unsupported_factors = factor_snapshot(&unsupported_fraction, &[], |_, _| 0.0);
        assert_eq!(
            build_target_snapshot(&unsupported_fraction, &unsupported_factors),
            Err(OwnerBetaTargetSnapshotError::ArithmeticInvalid)
        );
    }

    fn set_null(snapshot: &mut PriceOnlyFactorSnapshot, instrument: &str, factor: Option<&str>) {
        for row in &mut snapshot.rows {
            if row.instrument == instrument && factor.is_none_or(|factor| row.factor == factor) {
                row.raw = None;
                row.normalized = None;
            }
        }
        snapshot.hash = snapshot.compute_hash().expect("rehash null snapshot");
    }

    #[test]
    fn partial_nulls_match_the_strategy_exclusion_contract() {
        let relative = input_for(&resolved(
            "relative_momentum",
            json!({"top_n": 3, "lookback_months": 12}),
        ));
        let mut relative_factors = factor_snapshot(&relative, &["momentum_12_1"], |_, _| 1.0);
        set_null(&mut relative_factors, "102110.KRX", Some("momentum_12_1"));
        let relative_target =
            build_target_snapshot(&relative, &relative_factors).expect("relative partial null");
        let missing = relative_target
            .items()
            .iter()
            .find(|item| item.instrument_id() == "102110.KRX")
            .expect("relative missing member");
        assert_eq!(missing.target_weight_ppm(), None);
        assert_eq!(
            missing.reasons()[0].code(),
            "EXCLUDED_MANDATORY_FACTOR_NULL"
        );

        let dual = input_for(&resolved(
            "dual_momentum",
            json!({"absolute_threshold": 0.0, "lookback_months": 12}),
        ));
        let mut dual_factors = factor_snapshot(&dual, &["return_12m"], |_, _| 0.1);
        set_null(&mut dual_factors, "102110.KRX", Some("return_12m"));
        let dual_target = build_target_snapshot(&dual, &dual_factors).expect("dual partial null");
        assert_eq!(
            dual_target
                .items()
                .iter()
                .find(|item| item.instrument_id() == "102110.KRX")
                .expect("dual missing member")
                .reasons()[0]
                .code(),
            "EXCLUDED_MANDATORY_FACTOR_NULL"
        );

        let inverse = input_for(&resolved(
            "inverse_volatility",
            json!({"vol_window": 60, "max_weight": 0.3}),
        ));
        let mut inverse_factors = factor_snapshot(&inverse, &["vol_60"], |_, _| 1.0);
        set_null(&mut inverse_factors, "102110.KRX", Some("vol_60"));
        let inverse_target =
            build_target_snapshot(&inverse, &inverse_factors).expect("inverse partial null");
        assert_eq!(
            inverse_target
                .items()
                .iter()
                .find(|item| item.instrument_id() == "102110.KRX")
                .expect("inverse missing member")
                .reasons()[0]
                .code(),
            "EXCLUDED_MANDATORY_FACTOR_NULL"
        );

        let trend = input_for(&resolved(
            "trend_following",
            json!({
                "benchmark_instrument": "069500.KRX",
                "fast_ma": 50,
                "slow_ma": 200
            }),
        ));
        let mut trend_factors =
            factor_snapshot(&trend, &["trend_50", "trend_200"], |instrument, factor| {
                if instrument == "069500.KRX" && factor == "trend_50" {
                    2.0
                } else {
                    1.0
                }
            });
        for instrument in canonical_instruments() {
            if instrument != "069500.KRX" {
                set_null(&mut trend_factors, &instrument, None);
            }
        }
        let trend_target =
            build_target_snapshot(&trend, &trend_factors).expect("trend ignores nonbenchmark null");
        assert_eq!(trend_target.cash_weight_ppm(), 0);
    }

    #[test]
    fn inverse_rounding_and_python_evidence_have_concrete_goldens() {
        let inverse = input_for(&resolved(
            "inverse_volatility",
            json!({"vol_window": 60, "max_weight": 0.09}),
        ));
        let factors = factor_snapshot(&inverse, &["vol_60"], |instrument, _| {
            if instrument == "069500.KRX" {
                1999.9
            } else {
                1.0
            }
        });
        let target = build_target_snapshot(&inverse, &factors).expect("inverse rounding golden");
        let first = target
            .items()
            .iter()
            .find(|item| item.instrument_id() == "069500.KRX")
            .expect("first member");
        assert_eq!(first.score(), Some("5e-05"));
        assert_eq!(first.target_weight(), Some("0.000100".to_owned()));
        assert_eq!(target.cash_weight(), "0.099900");

        let dual = input_for(&resolved(
            "dual_momentum",
            json!({"absolute_threshold": 1e-7, "lookback_months": 12}),
        ));
        let dual_factors = factor_snapshot(&dual, &["return_12m"], |_, _| 0.0000025);
        let dual_target = build_target_snapshot(&dual, &dual_factors).expect("dual evidence");
        let selected = dual_target
            .items()
            .iter()
            .find(|item| item.target_weight_ppm().is_some())
            .expect("selected dual member");
        assert_eq!(selected.score(), Some("2.5e-06"));
        assert_eq!(selected.reasons()[0].params()["return_"], "3e-06");
        assert_eq!(selected.reasons()[0].params()["threshold"], "1e-07");
    }

    #[test]
    fn factor_inventory_rows_and_hash_are_fail_closed() {
        let input = input_for(&resolved(
            "relative_momentum",
            json!({"top_n": 3, "lookback_months": 12}),
        ));
        let mut snapshot = factor_snapshot(&input, &["momentum_12_1"], |_, _| 1.0);
        let expected_rows = rows_for_ids(&snapshot);
        snapshot.rows.pop();
        snapshot.hash = snapshot.compute_hash().expect("rehash");
        assert_eq!(
            build_target_snapshot(&input, &snapshot),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );

        let mut duplicate = factor_snapshot(&input, &["momentum_12_1"], |_, _| 1.0);
        duplicate.rows.push(duplicate.rows[0].clone());
        duplicate.hash = duplicate.compute_hash().expect("rehash");
        assert_eq!(
            build_target_snapshot(&input, &duplicate),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );

        let mut foreign = factor_snapshot(&input, &["momentum_12_1"], |_, _| 1.0);
        foreign.rows[0].instrument = "SPY.ARCA".to_owned();
        foreign.hash = foreign.compute_hash().expect("rehash");
        assert_eq!(
            build_target_snapshot(&input, &foreign),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );

        let mut null = factor_snapshot(&input, &["momentum_12_1"], |_, _| 1.0);
        null.rows[0].raw = None;
        null.hash = null.compute_hash().expect("rehash");
        assert_eq!(
            build_target_snapshot(&input, &null),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );

        let mut extra_factor = factor_snapshot(&input, &["momentum_12_1"], |_, _| 1.0);
        extra_factor
            .factor_versions
            .insert("return_6m".to_owned(), "1.0.0".to_owned());
        extra_factor.hash = extra_factor.compute_hash().expect("rehash");
        assert_eq!(
            build_target_snapshot(&input, &extra_factor),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );

        let mut hash_mismatch = factor_snapshot(&input, &["momentum_12_1"], |_, _| 1.0);
        hash_mismatch.hash = ContentHash::from_bytes(b"wrong");
        assert_eq!(
            build_target_snapshot(&input, &hash_mismatch),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );
        assert_eq!(expected_rows.len(), 11);
    }

    #[test]
    fn target_hash_and_debug_redact_identity_evidence() {
        let input = input_for(&resolved(
            "dual_momentum",
            json!({"absolute_threshold": 0.0, "lookback_months": 12}),
        ));
        let factors = factor_snapshot(&input, &["return_12m"], |_, _| 0.123456789);
        let target = build_target_snapshot(&input, &factors).expect("target");
        target.validate_hash().expect("target hash");
        let debug = format!("{target:?}");
        assert!(!debug.contains(input.pins().candidate_content_sha256().as_str()));
        assert!(!debug.contains(target.strategy_config_sha256().as_str()));
        assert!(!debug.contains(target.factor_snapshot_sha256().as_str()));
        assert!(!debug.contains("0.123456789"));

        let mut tampered = target.clone();
        tampered.target_snapshot_sha256 = ContentHash::from_bytes(b"tampered");
        assert_eq!(
            tampered.validate_hash(),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );
        let mut identity_tampered = target;
        identity_tampered.strategy_id = "trend_following".to_owned();
        assert_eq!(
            identity_tampered.validate_hash(),
            Err(OwnerBetaTargetSnapshotError::FactorSnapshotInvalid)
        );
    }
}
