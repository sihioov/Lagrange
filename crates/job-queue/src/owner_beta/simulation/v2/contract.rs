//! Closed v2 identities and canonical target allocation types.

use std::{collections::BTreeMap, fmt};

use domain::ContentHash;
use market_data::KR_ETF_CORE_SYMBOLS;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::{
    CostProfile, EngineIdentity, Money4, Ratio12, Sha256Digest, SimulationContractError,
    canonical_json, derive_benchmark_return_difference, parse_strict_json,
};
use super::ledger::V2LedgerRun;
use super::schedule::{V2DecisionKind, V2Schedule};

pub const RESULT_SCHEMA_VERSION: &str = "owner-beta-price-only-result-v2";
pub const CONTRACT_VERSION: &str = "owner-beta-price-only-simulation-v2-draft-1";
pub const APPROVED_CONTRACT_SHA256: &str =
    "sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3";
pub const V2_WEIGHT_SCALE_PPM: u32 = 1_000_000;
pub const V2_INITIAL_CAPITAL: &str = "10000000.0000";

/// Failures exposed by the v2 pure calculation boundary. Errors never include
/// rejected JSON, market values, or provider text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum V2ContractError {
    #[error("v2 simulation contract identity is invalid")]
    InvalidIdentity,
    #[error("v2 simulation JSON is malformed, duplicated, or non-canonical")]
    InvalidJson,
    #[error("v2 simulation JSON contains an unknown field or version")]
    InvalidSchema,
    #[error("v2 simulation value is invalid")]
    InvalidValue,
    #[error("v2 simulation schedule is invalid")]
    InvalidSchedule,
    #[error("v2 simulation ledger is invalid")]
    InvalidLedger,
    #[error("v2 simulation arithmetic overflowed")]
    ArithmeticOverflow,
}

impl From<SimulationContractError> for V2ContractError {
    fn from(error: SimulationContractError) -> Self {
        match error {
            SimulationContractError::InvalidJson
            | SimulationContractError::DuplicateJsonKey
            | SimulationContractError::NonCanonicalInteger => Self::InvalidJson,
            SimulationContractError::InvalidSchema => Self::InvalidSchema,
            SimulationContractError::InvalidSchedule => Self::InvalidSchedule,
            SimulationContractError::InvalidLedger => Self::InvalidLedger,
            SimulationContractError::ArithmeticOverflow => Self::ArithmeticOverflow,
            SimulationContractError::InvalidValue => Self::InvalidValue,
        }
    }
}

/// The five existing v1.0.0 DEFAULT strategies admitted by this contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum V2Strategy {
    #[serde(rename = "buy_and_hold")]
    BuyAndHold,
    #[serde(rename = "trend_following")]
    TrendFollowing,
    #[serde(rename = "relative_momentum")]
    RelativeMomentum,
    #[serde(rename = "dual_momentum")]
    DualMomentum,
    #[serde(rename = "inverse_volatility")]
    InverseVolatility,
}

impl V2Strategy {
    pub const ALL: [Self; 5] = [
        Self::BuyAndHold,
        Self::TrendFollowing,
        Self::RelativeMomentum,
        Self::DualMomentum,
        Self::InverseVolatility,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuyAndHold => "buy_and_hold",
            Self::TrendFollowing => "trend_following",
            Self::RelativeMomentum => "relative_momentum",
            Self::DualMomentum => "dual_momentum",
            Self::InverseVolatility => "inverse_volatility",
        }
    }

    pub const fn factor_names(self) -> &'static [&'static str] {
        match self {
            Self::BuyAndHold => &[],
            Self::TrendFollowing => &["trend_50", "trend_200"],
            Self::RelativeMomentum => &["momentum_12_1"],
            Self::DualMomentum => &["return_12m"],
            Self::InverseVolatility => &["vol_60"],
        }
    }

    pub const fn is_monthly(self) -> bool {
        matches!(
            self,
            Self::RelativeMomentum | Self::DualMomentum | Self::InverseVolatility
        )
    }

    pub const fn default_config_json(self) -> &'static str {
        match self {
            Self::BuyAndHold => {
                r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}"#
            }
            Self::TrendFollowing => {
                r#"{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}"#
            }
            Self::RelativeMomentum => r#"{"lookback_months":12,"top_n":3}"#,
            Self::DualMomentum => r#"{"absolute_threshold":0.0,"lookback_months":12}"#,
            Self::InverseVolatility => r#"{"max_weight":0.3,"vol_window":60}"#,
        }
    }
}

/// One closed factor-definition reference included in the result identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2FactorDefinition {
    name: String,
    version: String,
    sha256: Sha256Digest,
}

impl V2FactorDefinition {
    pub fn new(name: &str, version: &str, sha256: &str) -> Result<Self, V2ContractError> {
        if !is_ascii_identifier(name) || !is_ascii_identifier(version) {
            return Err(V2ContractError::InvalidIdentity);
        }
        Ok(Self {
            name: name.to_owned(),
            version: version.to_owned(),
            sha256: Sha256Digest::parse(sha256).map_err(V2ContractError::from)?,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
}

/// Checked identity for one exact strategy DEFAULT snapshot and its existing
/// factor definitions. Custom and unknown strategy configurations are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2StrategyIdentity {
    strategy: V2Strategy,
    version: String,
    canonical_config_json: String,
    snapshot_sha256: Sha256Digest,
    factors: Vec<V2FactorDefinition>,
}

impl V2StrategyIdentity {
    pub fn from_default_snapshot(
        strategy_id: &str,
        version: &str,
        config_json: &str,
        snapshot_sha256: &str,
        factors: Vec<V2FactorDefinition>,
    ) -> Result<Self, V2ContractError> {
        let strategy = V2Strategy::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == strategy_id)
            .ok_or(V2ContractError::InvalidIdentity)?;
        if version != "1.0.0" {
            return Err(V2ContractError::InvalidIdentity);
        }
        if config_json != strategy.default_config_json() {
            return Err(V2ContractError::InvalidIdentity);
        }
        validate_default_config(strategy, config_json)?;
        let factor_names = factors
            .iter()
            .map(V2FactorDefinition::name)
            .collect::<Vec<_>>();
        if factor_names.as_slice() != strategy.factor_names() {
            return Err(V2ContractError::InvalidIdentity);
        }
        let expected_hash = strategy_snapshot_hash(strategy, strategy.default_config_json())?;
        if Sha256Digest::parse(snapshot_sha256).map_err(V2ContractError::from)? != expected_hash {
            return Err(V2ContractError::InvalidIdentity);
        }
        Ok(Self {
            strategy,
            version: version.to_owned(),
            canonical_config_json: strategy.default_config_json().to_owned(),
            snapshot_sha256: Sha256Digest::parse(snapshot_sha256).map_err(V2ContractError::from)?,
            factors,
        })
    }

    pub const fn strategy(&self) -> V2Strategy {
        self.strategy
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn canonical_config_json(&self) -> &str {
        &self.canonical_config_json
    }

    pub fn snapshot_sha256(&self) -> &Sha256Digest {
        &self.snapshot_sha256
    }

    pub fn factors(&self) -> &[V2FactorDefinition] {
        &self.factors
    }
}

fn strategy_snapshot_hash(
    strategy: V2Strategy,
    config_json: &str,
) -> Result<Sha256Digest, V2ContractError> {
    let config: Value =
        serde_json::from_str(config_json).map_err(|_| V2ContractError::InvalidIdentity)?;
    let mut tagged = BTreeMap::new();
    tagged.insert("config_json", config);
    tagged.insert(
        "schema",
        Value::String(
            crate::owner_beta::input::OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA.to_owned(),
        ),
    );
    tagged.insert("strategy_id", Value::String(strategy.as_str().to_owned()));
    tagged.insert("strategy_version", Value::String("1.0.0".to_owned()));
    let bytes = serde_json::to_vec(&tagged).map_err(|_| V2ContractError::InvalidIdentity)?;
    let hash = ContentHash::from_bytes(&bytes);
    Sha256Digest::parse(hash.as_str()).map_err(V2ContractError::from)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuyAndHoldConfig {
    benchmark_instrument: String,
    rebalance_cadence: String,
    target_weight: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrendConfig {
    benchmark_instrument: String,
    fast_ma: u16,
    slow_ma: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelativeMomentumConfig {
    lookback_months: u16,
    top_n: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DualMomentumConfig {
    absolute_threshold: f64,
    lookback_months: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InverseVolatilityConfig {
    max_weight: f64,
    vol_window: u16,
}

fn validate_default_config(strategy: V2Strategy, json: &str) -> Result<(), V2ContractError> {
    let valid = match strategy {
        V2Strategy::BuyAndHold => {
            serde_json::from_str::<BuyAndHoldConfig>(json).is_ok_and(|config| {
                config.benchmark_instrument == "069500.KRX"
                    && config.rebalance_cadence == "none"
                    && config.target_weight == 1.0
            })
        }
        V2Strategy::TrendFollowing => serde_json::from_str::<TrendConfig>(json).is_ok_and(|c| {
            c.benchmark_instrument == "069500.KRX" && c.fast_ma == 50 && c.slow_ma == 200
        }),
        V2Strategy::RelativeMomentum => serde_json::from_str::<RelativeMomentumConfig>(json)
            .is_ok_and(|c| c.lookback_months == 12 && c.top_n == 3),
        V2Strategy::DualMomentum => serde_json::from_str::<DualMomentumConfig>(json)
            .is_ok_and(|c| c.absolute_threshold == 0.0 && c.lookback_months == 12),
        V2Strategy::InverseVolatility => serde_json::from_str::<InverseVolatilityConfig>(json)
            .is_ok_and(|c| c.max_weight == 0.3 && c.vol_window == 60),
    };
    if valid {
        Ok(())
    } else {
        Err(V2ContractError::InvalidIdentity)
    }
}

/// The immutable five pins for the already-approved v3 ETF11 artifact.
/// Constructing this value asserts only the fixed identity; WP-2 still has to
/// validate the approved artifact and its factor readiness before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2InputIdentity {
    candidate_content_sha256: Sha256Digest,
    artifact_manifest_sha256: Sha256Digest,
    stage5_manifest_sha256: Sha256Digest,
    action_manifest_sha256: Sha256Digest,
    approval_registry_sha256: Sha256Digest,
    bars_sha256: Sha256Digest,
    ownership_scope: &'static str,
    return_method: &'static str,
    vendor_snapshot: bool,
    strict_pit: bool,
}

impl V2InputIdentity {
    pub fn approved_v3_etf11() -> Result<Self, V2ContractError> {
        use super::super::{
            APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256,
            APPROVED_BARS_SHA256, APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_REGISTRY_SHA256,
            APPROVED_STAGE5_MANIFEST_SHA256,
        };

        let parse = |value: &str| Sha256Digest::parse(value).map_err(V2ContractError::from);
        Ok(Self {
            candidate_content_sha256: parse(APPROVED_CANDIDATE_CONTENT_SHA256)?,
            artifact_manifest_sha256: parse(APPROVED_ARTIFACT_MANIFEST_SHA256)?,
            stage5_manifest_sha256: parse(APPROVED_STAGE5_MANIFEST_SHA256)?,
            action_manifest_sha256: parse(APPROVED_ACTION_MANIFEST_SHA256)?,
            approval_registry_sha256: parse(APPROVED_REGISTRY_SHA256)?,
            bars_sha256: parse(APPROVED_BARS_SHA256)?,
            ownership_scope: "OWNER_ONLY",
            return_method: "PRICE_RETURN_ONLY",
            vendor_snapshot: true,
            strict_pit: false,
        })
    }

    pub fn candidate_content_sha256(&self) -> &Sha256Digest {
        &self.candidate_content_sha256
    }

    pub fn artifact_manifest_sha256(&self) -> &Sha256Digest {
        &self.artifact_manifest_sha256
    }

    pub fn stage5_manifest_sha256(&self) -> &Sha256Digest {
        &self.stage5_manifest_sha256
    }

    pub fn action_manifest_sha256(&self) -> &Sha256Digest {
        &self.action_manifest_sha256
    }

    pub fn approval_registry_sha256(&self) -> &Sha256Digest {
        &self.approval_registry_sha256
    }

    pub fn bars_sha256(&self) -> &Sha256Digest {
        &self.bars_sha256
    }

    pub fn ownership_scope(&self) -> &'static str {
        self.ownership_scope
    }

    pub fn return_method(&self) -> &'static str {
        self.return_method
    }

    pub const fn vendor_snapshot(&self) -> bool {
        self.vendor_snapshot
    }

    pub const fn strict_pit(&self) -> bool {
        self.strict_pit
    }
}

/// One instrument's fixed-point target weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2InstrumentWeight {
    instrument_id: String,
    weight_ppm: u32,
}

impl V2InstrumentWeight {
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub const fn weight_ppm(&self) -> u32 {
        self.weight_ppm
    }
}

/// Complete ETF11 target snapshot reference. Inventory is complete, sorted,
/// unique, and sums with cash to exactly one million ppm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2TargetAllocation {
    target_sha256: Sha256Digest,
    allocation_sha256: Sha256Digest,
    lineage_sha256: Sha256Digest,
    weights: Vec<V2InstrumentWeight>,
    cash_weight_ppm: u32,
}

impl V2TargetAllocation {
    pub fn new(
        target_sha256: &str,
        weights: Vec<(String, u32)>,
        cash_weight_ppm: u32,
    ) -> Result<Self, V2ContractError> {
        let expected = approved_instruments();
        if weights.len() != expected.len()
            || weights
                .iter()
                .map(|(instrument, _)| instrument.as_str())
                .ne(expected.iter().map(String::as_str))
        {
            return Err(V2ContractError::InvalidValue);
        }
        let total = weights
            .iter()
            .try_fold(cash_weight_ppm, |sum, (_, weight)| {
                if *weight > V2_WEIGHT_SCALE_PPM {
                    return None;
                }
                sum.checked_add(*weight)
            });
        if total != Some(V2_WEIGHT_SCALE_PPM) {
            return Err(V2ContractError::InvalidValue);
        }
        let weights = weights
            .into_iter()
            .map(|(instrument_id, weight_ppm)| V2InstrumentWeight {
                instrument_id,
                weight_ppm,
            })
            .collect::<Vec<_>>();
        let preimage = TargetAllocationPreimage {
            weights: &weights,
            cash_weight_ppm,
        };
        let allocation_bytes =
            serde_json::to_vec(&preimage).map_err(|_| V2ContractError::InvalidValue)?;
        let target_sha256 = Sha256Digest::parse(target_sha256).map_err(V2ContractError::from)?;
        let allocation_sha256 = digest(&allocation_bytes);
        let lineage_bytes = serde_json::to_vec(&TargetLineagePreimage {
            target_sha256: &target_sha256,
            allocation_sha256: &allocation_sha256,
        })
        .map_err(|_| V2ContractError::InvalidValue)?;
        Ok(Self {
            target_sha256,
            allocation_sha256,
            lineage_sha256: digest(&lineage_bytes),
            weights,
            cash_weight_ppm,
        })
    }

    pub fn target_sha256(&self) -> &Sha256Digest {
        &self.target_sha256
    }

    pub fn allocation_sha256(&self) -> &Sha256Digest {
        &self.allocation_sha256
    }

    pub fn lineage_sha256(&self) -> &Sha256Digest {
        &self.lineage_sha256
    }

    pub fn weights(&self) -> &[V2InstrumentWeight] {
        &self.weights
    }

    pub const fn cash_weight_ppm(&self) -> u32 {
        self.cash_weight_ppm
    }
}

#[derive(Serialize)]
struct TargetAllocationPreimage<'a> {
    weights: &'a [V2InstrumentWeight],
    cash_weight_ppm: u32,
}

#[derive(Serialize)]
struct TargetLineagePreimage<'a> {
    target_sha256: &'a Sha256Digest,
    allocation_sha256: &'a Sha256Digest,
}

pub(crate) fn approved_instruments() -> Vec<String> {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort_unstable();
    instruments
}

pub(crate) fn digest(bytes: &[u8]) -> Sha256Digest {
    let hash = ContentHash::from_bytes(bytes);
    Sha256Digest::parse(hash.as_str()).expect("domain SHA-256 output is valid")
}

pub(crate) fn is_ascii_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

impl fmt::Display for V2Strategy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One fully checked strategy result for one cost profile. The public type has
/// no `Deserialize` implementation; untrusted JSON enters through
/// [`Self::from_json`] and is rebuilt through the same validators as creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2SimulationResult {
    schema: String,
    contract_version: String,
    contract_sha256: Sha256Digest,
    input: V2InputIdentity,
    strategy: V2StrategyIdentity,
    engine: EngineIdentity,
    schedule: V2Schedule,
    cost_profile: CostProfile,
    ledger: V2LedgerRun,
    benchmark: V2LedgerRun,
    benchmark_return_difference: Ratio12,
}

impl V2SimulationResult {
    pub fn new(
        strategy: V2StrategyIdentity,
        engine: EngineIdentity,
        schedule: V2Schedule,
        cost_profile: CostProfile,
        ledger: V2LedgerRun,
        benchmark: V2LedgerRun,
    ) -> Result<Self, V2ContractError> {
        let initial = Money4::parse(V2_INITIAL_CAPITAL).map_err(V2ContractError::from)?;
        if strategy.strategy() != schedule.strategy()
            || ledger.profile != cost_profile
            || benchmark.profile != cost_profile
            || ledger.initial_capital != initial
            || ledger.initial_capital != benchmark.initial_capital
            || ledger.market_sessions != benchmark.market_sessions
        {
            return Err(V2ContractError::InvalidIdentity);
        }
        ledger.validate_trace(&schedule)?;
        let benchmark_schedule = benchmark_schedule(&schedule)?;
        benchmark.validate_trace(&benchmark_schedule)?;
        validate_benchmark(&benchmark, &benchmark_schedule)?;
        if strategy.strategy() == V2Strategy::BuyAndHold && !ledger.economically_equal(&benchmark) {
            return Err(V2ContractError::InvalidLedger);
        }
        let benchmark_return_difference = derive_benchmark_return_difference(
            &initial,
            ledger.ending_nav(),
            benchmark.ending_nav(),
        )
        .map_err(V2ContractError::from)?;
        Ok(Self {
            schema: RESULT_SCHEMA_VERSION.to_owned(),
            contract_version: CONTRACT_VERSION.to_owned(),
            contract_sha256: Sha256Digest::parse(APPROVED_CONTRACT_SHA256)
                .map_err(V2ContractError::from)?,
            input: V2InputIdentity::approved_v3_etf11()?,
            strategy,
            engine,
            schedule,
            cost_profile,
            ledger,
            benchmark,
            benchmark_return_difference,
        })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, V2ContractError> {
        let value = parse_strict_json(bytes).map_err(V2ContractError::from)?;
        let wire: V2SimulationResultWire =
            serde_json::from_value(value).map_err(|_| V2ContractError::InvalidSchema)?;
        if wire.schema != RESULT_SCHEMA_VERSION
            || wire.contract_version != CONTRACT_VERSION
            || wire.contract_sha256 != APPROVED_CONTRACT_SHA256
        {
            return Err(V2ContractError::InvalidIdentity);
        }

        let input = V2InputIdentity::approved_v3_etf11()?;
        if serde_json::to_value(&input).map_err(|_| V2ContractError::InvalidJson)?
            != serde_json::to_value(&wire.input).map_err(|_| V2ContractError::InvalidJson)?
        {
            return Err(V2ContractError::InvalidIdentity);
        }
        let factors = wire
            .strategy
            .factors
            .iter()
            .map(|factor| V2FactorDefinition::new(&factor.name, &factor.version, &factor.sha256))
            .collect::<Result<Vec<_>, _>>()?;
        let strategy = V2StrategyIdentity::from_default_snapshot(
            wire.strategy.strategy.as_str(),
            &wire.strategy.version,
            &wire.strategy.canonical_config_json,
            &wire.strategy.snapshot_sha256,
            factors,
        )?;
        if serde_json::to_value(&strategy).map_err(|_| V2ContractError::InvalidJson)?
            != serde_json::to_value(&wire.strategy).map_err(|_| V2ContractError::InvalidJson)?
        {
            return Err(V2ContractError::InvalidIdentity);
        }
        let engine = EngineIdentity::new(
            &wire.engine.git_commit,
            &wire.engine.binary_sha256,
            &wire.engine.toolchain,
            &wire.engine.target,
            &wire.engine.profile,
        )
        .map_err(V2ContractError::from)?;
        if serde_json::to_value(&engine).map_err(|_| V2ContractError::InvalidJson)?
            != serde_json::to_value(&wire.engine).map_err(|_| V2ContractError::InvalidJson)?
        {
            return Err(V2ContractError::InvalidIdentity);
        }
        let preparation_sessions = wire
            .schedule
            .preparation_sessions
            .iter()
            .map(|date| date.to_owned())
            .collect();
        let evaluation_sessions = wire
            .schedule
            .evaluation_sessions
            .iter()
            .map(|date| date.to_owned())
            .collect();
        let schedule = V2Schedule::new(
            wire.schedule.strategy,
            preparation_sessions,
            evaluation_sessions,
        )?;
        if serde_json::to_value(&schedule).map_err(|_| V2ContractError::InvalidJson)?
            != serde_json::to_value(&wire.schedule).map_err(|_| V2ContractError::InvalidJson)?
        {
            return Err(V2ContractError::InvalidSchedule);
        }
        if schedule.strategy() != strategy.strategy() {
            return Err(V2ContractError::InvalidIdentity);
        }
        let benchmark_schedule = benchmark_schedule(&schedule)?;
        let ledger = V2LedgerRun::from_json_value(wire.ledger, &schedule)?;
        let benchmark = V2LedgerRun::from_json_value(wire.benchmark, &benchmark_schedule)?;
        let supplied_difference =
            Ratio12::parse(&wire.benchmark_return_difference).map_err(V2ContractError::from)?;
        let result = Self::new(
            strategy,
            engine,
            schedule,
            wire.cost_profile,
            ledger,
            benchmark,
        )?;
        if result.benchmark_return_difference != supplied_difference {
            return Err(V2ContractError::InvalidLedger);
        }
        Ok(result)
    }

    pub fn canonical_json(&self) -> Result<Vec<u8>, V2ContractError> {
        canonical_json(self).map_err(V2ContractError::from)
    }

    pub fn sha256(&self) -> Result<Sha256Digest, V2ContractError> {
        Ok(digest(&self.canonical_json()?))
    }

    pub fn strategy(&self) -> &V2StrategyIdentity {
        &self.strategy
    }

    pub fn schedule(&self) -> &V2Schedule {
        &self.schedule
    }

    pub const fn cost_profile(&self) -> CostProfile {
        self.cost_profile
    }

    pub fn ledger(&self) -> &V2LedgerRun {
        &self.ledger
    }

    pub fn benchmark(&self) -> &V2LedgerRun {
        &self.benchmark
    }

    pub fn benchmark_return_difference(&self) -> &Ratio12 {
        &self.benchmark_return_difference
    }
}

/// Atomic two-profile outcome for exactly one strategy and one shared period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2SimulationPair {
    illustrative_10bp_per_side: V2SimulationResult,
    zero_cost_reference: V2SimulationResult,
}

impl V2SimulationPair {
    pub fn new(
        illustrative_10bp_per_side: V2SimulationResult,
        zero_cost_reference: V2SimulationResult,
    ) -> Result<Self, V2ContractError> {
        if illustrative_10bp_per_side.cost_profile != CostProfile::Illustrative10BpPerSideV1
            || zero_cost_reference.cost_profile != CostProfile::ZeroCostReferenceV1
            || illustrative_10bp_per_side.strategy != zero_cost_reference.strategy
            || illustrative_10bp_per_side.engine != zero_cost_reference.engine
            || illustrative_10bp_per_side.schedule != zero_cost_reference.schedule
            || illustrative_10bp_per_side.input != zero_cost_reference.input
            || !same_target_path(
                &illustrative_10bp_per_side.ledger,
                &zero_cost_reference.ledger,
            )
            || !same_target_path(
                &illustrative_10bp_per_side.benchmark,
                &zero_cost_reference.benchmark,
            )
            || illustrative_10bp_per_side.ledger.market_sessions
                != zero_cost_reference.ledger.market_sessions
            || illustrative_10bp_per_side.benchmark.market_sessions
                != zero_cost_reference.benchmark.market_sessions
            || illustrative_10bp_per_side.ledger.target_observations
                != zero_cost_reference.ledger.target_observations
            || illustrative_10bp_per_side.benchmark.target_observations
                != zero_cost_reference.benchmark.target_observations
        {
            return Err(V2ContractError::InvalidLedger);
        }
        Ok(Self {
            illustrative_10bp_per_side,
            zero_cost_reference,
        })
    }

    pub fn illustrative_10bp_per_side(&self) -> &V2SimulationResult {
        &self.illustrative_10bp_per_side
    }

    pub fn zero_cost_reference(&self) -> &V2SimulationResult {
        &self.zero_cost_reference
    }

    pub fn canonical_json(&self) -> Result<(Vec<u8>, Vec<u8>), V2ContractError> {
        Ok((
            self.illustrative_10bp_per_side.canonical_json()?,
            self.zero_cost_reference.canonical_json()?,
        ))
    }

    /// Decodes and cross-checks both independently stored profile bodies.
    pub fn from_json(
        illustrative_10bp_per_side: &[u8],
        zero_cost_reference: &[u8],
    ) -> Result<Self, V2ContractError> {
        Self::new(
            V2SimulationResult::from_json(illustrative_10bp_per_side)?,
            V2SimulationResult::from_json(zero_cost_reference)?,
        )
    }
}

fn benchmark_schedule(schedule: &V2Schedule) -> Result<V2Schedule, V2ContractError> {
    V2Schedule::new(
        V2Strategy::BuyAndHold,
        schedule
            .preparation_sessions()
            .iter()
            .map(|date| date.to_iso())
            .collect(),
        schedule
            .evaluation_sessions()
            .iter()
            .map(|date| date.to_iso())
            .collect(),
    )
}

fn validate_benchmark(
    benchmark: &V2LedgerRun,
    schedule: &V2Schedule,
) -> Result<(), V2ContractError> {
    if benchmark.rebalances.len() != 1
        || schedule.decisions().len() != 1
        || benchmark.rebalances[0].execution_date != schedule.evaluation_start()
        || benchmark.rebalances[0].decision_kind != V2DecisionKind::InitialCommonSession
        || benchmark.rebalances[0].target.cash_weight_ppm() != 0
    {
        return Err(V2ContractError::InvalidLedger);
    }
    let weights = benchmark.rebalances[0].target.weights();
    if weights.len() != KR_ETF_CORE_SYMBOLS.len()
        || weights.iter().any(|weight| {
            if weight.instrument_id() == "069500.KRX" {
                weight.weight_ppm() != V2_WEIGHT_SCALE_PPM
            } else {
                weight.weight_ppm() != 0
            }
        })
    {
        return Err(V2ContractError::InvalidLedger);
    }
    Ok(())
}

fn same_target_path(left: &V2LedgerRun, right: &V2LedgerRun) -> bool {
    left.rebalances.len() == right.rebalances.len()
        && left.rebalances.iter().zip(&right.rebalances).all(|(a, b)| {
            a.signal_date == b.signal_date
                && a.execution_date == b.execution_date
                && a.decision_kind == b.decision_kind
                && a.target == b.target
                && a.open_prices == b.open_prices
        })
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2SimulationResultWire {
    schema: String,
    contract_version: String,
    contract_sha256: String,
    input: V2InputIdentityWire,
    strategy: V2StrategyIdentityWire,
    engine: V2EngineIdentityWire,
    schedule: V2ScheduleWire,
    cost_profile: CostProfile,
    ledger: Value,
    benchmark: Value,
    benchmark_return_difference: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2InputIdentityWire {
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
    bars_sha256: String,
    ownership_scope: String,
    return_method: String,
    vendor_snapshot: bool,
    strict_pit: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2StrategyIdentityWire {
    strategy: V2Strategy,
    version: String,
    canonical_config_json: String,
    snapshot_sha256: String,
    factors: Vec<V2FactorDefinitionWire>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2FactorDefinitionWire {
    name: String,
    version: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2EngineIdentityWire {
    git_commit: String,
    binary_sha256: String,
    toolchain: String,
    target: String,
    profile: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2ScheduleWire {
    strategy: V2Strategy,
    preparation_session_count: u32,
    first_signal_date: String,
    preparation_sessions: Vec<String>,
    evaluation_sessions: Vec<String>,
    decisions: Vec<V2DecisionWire>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct V2DecisionWire {
    signal_date: String,
    execution_date: String,
    kind: V2DecisionKind,
}
