//! Closed, checked result types for the owner-beta price-only simulation.
//!
//! No public unchecked success constructor is provided. The public factories
//! either install the approved immutable values or validate all caller input;
//! JSON enters through [`SimulationResult::from_json`], which checks duplicate
//! keys before serde sees an object map.

use std::{collections::BTreeMap, fmt, str::FromStr};

use domain::{ContentHash, Currency, Money, Price, TradingDate};
use market_data::KR_ETF_CORE_SYMBOLS;
use serde::{Deserialize, Serialize};

use super::canonical::{canonical_json, parse_strict_json};
use super::metrics::{derive_benchmark_return_difference, derive_performance_metrics};

pub const RESULT_SCHEMA_VERSION: &str = "owner-beta-price-only-result-v1";
pub const CONTRACT_VERSION: &str = "owner-beta-price-only-simulation-v1-draft-1";
pub const APPROVED_CONTRACT_SHA256: &str =
    "sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd";
pub const APPROVED_BARS_SHA256: &str =
    "sha256:23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe";
pub const APPROVED_SESSION_COUNT: u32 = 2_452;
pub const APPROVED_BAR_COUNT: u32 = 26_972;
pub const APPROVED_START_DATE: &str = "2016-08-29";
pub const APPROVED_END_DATE: &str = "2026-08-28";
pub const PRODUCTION_INITIAL_CAPITAL: &str = "10000000.0000";

const APPROVED_CANDIDATE_CONTENT_SHA256: &str =
    "sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d";
const APPROVED_ARTIFACT_MANIFEST_SHA256: &str =
    "sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6";
const APPROVED_STAGE5_MANIFEST_SHA256: &str =
    "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a";
const APPROVED_ACTION_MANIFEST_SHA256: &str =
    "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550";
const APPROVED_REGISTRY_SHA256: &str =
    "sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707";
const TREND_INSTRUMENT: &str = "069500.KRX";

/// Typed validation failures intentionally contain no rejected input values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationContractError {
    #[error("simulation result JSON is malformed or non-canonical")]
    InvalidJson,
    #[error("simulation result JSON contains a duplicate object key")]
    DuplicateJsonKey,
    #[error("simulation result JSON contains a non-canonical integer")]
    NonCanonicalInteger,
    #[error("simulation result schema is invalid")]
    InvalidSchema,
    #[error("simulation result value is invalid")]
    InvalidValue,
    #[error("simulation result schedule is invalid")]
    InvalidSchedule,
    #[error("simulation result ledger is invalid")]
    InvalidLedger,
    #[error("simulation result arithmetic overflowed")]
    ArithmeticOverflow,
}

/// A lower-case `sha256:` digest. Unlike `ContentHash::parse`, this wrapper
/// rejects uppercase hex rather than normalizing it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(ContentHash);

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Sha256Digest(<redacted>)")
    }
}

impl Sha256Digest {
    pub fn parse(value: &str) -> Result<Self, SimulationContractError> {
        let parsed =
            ContentHash::parse(value).map_err(|_| SimulationContractError::InvalidValue)?;
        if parsed.as_str() != value {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(Self(parsed))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A full 40-character lower-case Git commit identifier.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FullGitRevision(String);

impl fmt::Debug for FullGitRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FullGitRevision(<redacted>)")
    }
}

impl FullGitRevision {
    pub fn parse(value: &str) -> Result<Self, SimulationContractError> {
        if value.len() != 40
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A fixed-scale, non-negative KRW amount represented as exactly four decimal
/// places. The domain `Money` validates arithmetic and currency semantics.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Money4(String);

impl fmt::Debug for Money4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Money4(<amount>)")
    }
}

impl Money4 {
    pub fn parse(value: &str) -> Result<Self, SimulationContractError> {
        let amount = Money::parse(value, Currency::KRW)
            .map_err(|_| SimulationContractError::InvalidValue)?;
        let canonical = amount.as_decimal_string();
        if canonical != value || !is_fixed_decimal(value, 4, false) {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(Self(canonical))
    }

    pub fn zero() -> Self {
        Self("0.0000".to_owned())
    }

    #[cfg(test)]
    pub(super) fn synthetic_test_capital() -> Self {
        Self("1000.0000".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Exact value in 0.0001-KRW units.
    pub fn units(&self) -> Result<i128, SimulationContractError> {
        decimal_units(&self.0, 4)
    }

    /// Constructs from exact 0.0001-KRW units without rounding.
    pub fn from_units(units: i128) -> Result<Self, SimulationContractError> {
        if units < 0 {
            return Err(SimulationContractError::InvalidValue);
        }
        Self::parse(&format_units(units, 4)?)
    }
}

/// A fixed-scale, strictly positive price string at four decimal places.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Price4(String);

impl fmt::Debug for Price4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Price4(<price>)")
    }
}

impl Price4 {
    pub fn parse(value: &str) -> Result<Self, SimulationContractError> {
        let price = Price::parse(value).map_err(|_| SimulationContractError::InvalidValue)?;
        let canonical = price.as_decimal_string();
        if canonical != value || !is_fixed_decimal(value, 4, false) || value == "0.0000" {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Exact value in 0.0001-KRW price units.
    pub fn units(&self) -> Result<i128, SimulationContractError> {
        decimal_units(&self.0, 4)
    }
}

/// A signed fixed-scale ratio. Negative zero is rejected and values always
/// serialize as exactly twelve fractional digits.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Ratio12(String);

impl fmt::Debug for Ratio12 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Ratio12(<ratio>)")
    }
}

impl Ratio12 {
    pub fn parse(value: &str) -> Result<Self, SimulationContractError> {
        if !is_fixed_decimal(value, 12, true) {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn zero() -> Self {
        Self("0.000000000000".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Exact value in 10^-12 ratio units.
    pub fn units(&self) -> Result<i128, SimulationContractError> {
        decimal_units(&self.0, 12)
    }

    /// Constructs from exact 10^-12 ratio units without rounding.
    pub fn from_units(units: i128) -> Result<Self, SimulationContractError> {
        Self::parse(&format_signed_units(units, 12)?)
    }
}

fn is_fixed_decimal(value: &str, scale: usize, signed: bool) -> bool {
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(rest) if signed => (true, rest),
        Some(_) => return false,
        None => (false, value),
    };
    let Some((integer, fraction)) = unsigned.split_once('.') else {
        return false;
    };
    if fraction.len() != scale
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
    {
        return false;
    }
    let is_zero =
        integer.bytes().all(|byte| byte == b'0') && fraction.bytes().all(|byte| byte == b'0');
    !negative || !is_zero
}

fn decimal_units(value: &str, scale: usize) -> Result<i128, SimulationContractError> {
    if !is_fixed_decimal(value, scale, true) {
        return Err(SimulationContractError::InvalidValue);
    }
    let negative = value.starts_with('-');
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (integer, fraction) = unsigned
        .split_once('.')
        .ok_or(SimulationContractError::InvalidValue)?;
    let digits = format!("{integer}{fraction}");
    let units = i128::from_str(&digits).map_err(|_| SimulationContractError::ArithmeticOverflow)?;
    if negative {
        units
            .checked_neg()
            .ok_or(SimulationContractError::ArithmeticOverflow)
    } else {
        Ok(units)
    }
}

/// The one supported simulated cost assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostProfile {
    #[serde(rename = "ILLUSTRATIVE_10BP_PER_SIDE_V1")]
    Illustrative10BpPerSideV1,
    #[serde(rename = "ZERO_COST_REFERENCE_V1")]
    ZeroCostReferenceV1,
}

impl CostProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Illustrative10BpPerSideV1 => "ILLUSTRATIVE_10BP_PER_SIDE_V1",
            Self::ZeroCostReferenceV1 => "ZERO_COST_REFERENCE_V1",
        }
    }

    const fn fee_rate(self) -> (u32, u32) {
        match self {
            Self::Illustrative10BpPerSideV1 => (1, 1_000),
            Self::ZeroCostReferenceV1 => (0, 1),
        }
    }
}

/// Validated identity declared by an execution. This type checks syntax and
/// fixed fields only; proof that the running binary supplied them belongs to
/// WP-4/G2's verified execution context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineIdentity {
    git_commit: FullGitRevision,
    binary_sha256: Sha256Digest,
    toolchain: String,
    target: String,
    profile: String,
}

impl EngineIdentity {
    pub fn new(
        git_commit: &str,
        binary_sha256: &str,
        toolchain: &str,
        target: &str,
        profile: &str,
    ) -> Result<Self, SimulationContractError> {
        for value in [toolchain, target, profile] {
            if value.is_empty()
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
            {
                return Err(SimulationContractError::InvalidValue);
            }
        }
        Ok(Self {
            git_commit: FullGitRevision::parse(git_commit)?,
            binary_sha256: Sha256Digest::parse(binary_sha256)?,
            toolchain: toolchain.to_owned(),
            target: target.to_owned(),
            profile: profile.to_owned(),
        })
    }
}

/// The fixed first strategy and the existing factor definition identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StrategyIdentity {
    strategy_id: String,
    version: String,
    config: TrendFollowingConfig,
    strategy_snapshot_sha256: Sha256Digest,
    factor_definition: FactorDefinitionIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TrendFollowingConfig {
    benchmark_instrument: String,
    fast_ma: u16,
    slow_ma: u16,
    target_rule: TrendTargetRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FactorDefinitionIdentity {
    version: String,
    sha256: Sha256Digest,
}

impl StrategyIdentity {
    pub fn trend_following_v1(
        strategy_snapshot_sha256: &str,
        factor_definition_version: &str,
        factor_definition_sha256: &str,
    ) -> Result<Self, SimulationContractError> {
        if !is_ascii_identifier(factor_definition_version) {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(Self {
            strategy_id: "trend_following".to_owned(),
            version: "1.0.0".to_owned(),
            config: TrendFollowingConfig {
                benchmark_instrument: TREND_INSTRUMENT.to_owned(),
                fast_ma: 50,
                slow_ma: 200,
                target_rule: TrendTargetRule::BuyWhenFastTrendBelowSlowTrendOtherwiseCash,
            },
            strategy_snapshot_sha256: Sha256Digest::parse(strategy_snapshot_sha256)?,
            factor_definition: FactorDefinitionIdentity {
                version: factor_definition_version.to_owned(),
                sha256: Sha256Digest::parse(factor_definition_sha256)?,
            },
        })
    }
}

fn is_ascii_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ApprovedInputPins {
    candidate_content_sha256: Sha256Digest,
    artifact_manifest_sha256: Sha256Digest,
    stage5_manifest_sha256: Sha256Digest,
    action_manifest_sha256: Sha256Digest,
    approval_registry_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct EvidenceLabels {
    data_scope: EvidenceScope,
    return_basis: ReturnBasis,
    vendor_snapshot: bool,
    strict_pit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum EvidenceScope {
    #[serde(rename = "OWNER_ONLY")]
    OwnerOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ReturnBasis {
    #[serde(rename = "PRICE_RETURN_ONLY")]
    PriceReturnOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimulationInputIdentity {
    pins: ApprovedInputPins,
    bars_sha256: Sha256Digest,
    approved_start_date: TradingDate,
    approved_end_date: TradingDate,
    session_count: u32,
    bar_count: u32,
    instruments: Vec<String>,
    labels: EvidenceLabels,
}

impl SimulationInputIdentity {
    fn approved() -> Result<Self, SimulationContractError> {
        let instruments = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX"))
            .collect::<Vec<_>>();
        let mut instruments = instruments;
        instruments.sort();
        Ok(Self {
            pins: ApprovedInputPins {
                candidate_content_sha256: Sha256Digest::parse(APPROVED_CANDIDATE_CONTENT_SHA256)?,
                artifact_manifest_sha256: Sha256Digest::parse(APPROVED_ARTIFACT_MANIFEST_SHA256)?,
                stage5_manifest_sha256: Sha256Digest::parse(APPROVED_STAGE5_MANIFEST_SHA256)?,
                action_manifest_sha256: Sha256Digest::parse(APPROVED_ACTION_MANIFEST_SHA256)?,
                approval_registry_sha256: Sha256Digest::parse(APPROVED_REGISTRY_SHA256)?,
            },
            bars_sha256: Sha256Digest::parse(APPROVED_BARS_SHA256)?,
            approved_start_date: TradingDate::parse(APPROVED_START_DATE)
                .map_err(|_| SimulationContractError::InvalidValue)?,
            approved_end_date: TradingDate::parse(APPROVED_END_DATE)
                .map_err(|_| SimulationContractError::InvalidValue)?,
            session_count: APPROVED_SESSION_COUNT,
            bar_count: APPROVED_BAR_COUNT,
            instruments,
            labels: EvidenceLabels {
                data_scope: EvidenceScope::OwnerOnly,
                return_basis: ReturnBasis::PriceReturnOnly,
                vendor_snapshot: true,
                strict_pit: false,
            },
        })
    }
}

/// Each signal's existing snapshot hashes and the complete ETF11/cash target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalSnapshot {
    signal_date: TradingDate,
    factor_snapshot_sha256: Sha256Digest,
    target_snapshot_sha256: Sha256Digest,
    targets: Vec<InstrumentTarget>,
    cash_weight_ppm: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct InstrumentTarget {
    instrument_id: String,
    weight_ppm: u32,
}

impl SignalSnapshot {
    pub fn new(
        signal_date: &str,
        factor_snapshot_sha256: &str,
        target_snapshot_sha256: &str,
        targets: Vec<(String, u32)>,
        cash_weight_ppm: u32,
    ) -> Result<Self, SimulationContractError> {
        let signal = Self {
            signal_date: TradingDate::parse(signal_date)
                .map_err(|_| SimulationContractError::InvalidValue)?,
            factor_snapshot_sha256: Sha256Digest::parse(factor_snapshot_sha256)?,
            target_snapshot_sha256: Sha256Digest::parse(target_snapshot_sha256)?,
            targets: targets
                .into_iter()
                .map(|(instrument_id, weight_ppm)| InstrumentTarget {
                    instrument_id,
                    weight_ppm,
                })
                .collect(),
            cash_weight_ppm,
        };
        signal.validate()?;
        Ok(signal)
    }

    fn validate(&self) -> Result<(), SimulationContractError> {
        let expected = approved_instruments();
        if self.targets.len() != expected.len()
            || self
                .targets
                .iter()
                .zip(expected.iter())
                .any(|(target, expected)| target.instrument_id != *expected)
        {
            return Err(SimulationContractError::InvalidValue);
        }
        let sum = self
            .targets
            .iter()
            .try_fold(self.cash_weight_ppm, |sum, target| {
                sum.checked_add(target.weight_ppm)
                    .ok_or(SimulationContractError::ArithmeticOverflow)
            })?;
        if sum != 1_000_000 {
            return Err(SimulationContractError::InvalidValue);
        }
        let trend_weight = self
            .targets
            .iter()
            .find(|target| target.instrument_id == TREND_INSTRUMENT)
            .map(|target| target.weight_ppm)
            .ok_or(SimulationContractError::InvalidValue)?;
        if !((trend_weight == 1_000_000 && self.cash_weight_ppm == 0)
            || (trend_weight == 0 && self.cash_weight_ppm == 1_000_000))
            || self
                .targets
                .iter()
                .any(|target| target.instrument_id != TREND_INSTRUMENT && target.weight_ppm != 0)
        {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(())
    }

    pub fn signal_date(&self) -> TradingDate {
        self.signal_date
    }

    pub fn targets(&self) -> impl Iterator<Item = (&str, u32)> {
        self.targets
            .iter()
            .map(|target| (target.instrument_id.as_str(), target.weight_ppm))
    }

    pub fn cash_weight_ppm(&self) -> u32 {
        self.cash_weight_ppm
    }
}

fn approved_instruments() -> Vec<String> {
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort();
    instruments
}

/// Signal-to-next-session execution pairs and the complete date-only phase
/// sequence. The date lists must be chronological and mutually consistent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimulationSchedule {
    warmup_sessions: Vec<TradingDate>,
    signal_execution: Vec<SignalExecutionPair>,
    evaluation_sessions: Vec<TradingDate>,
    phase_order: Vec<LedgerPhase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SignalExecutionPair {
    signal_date: TradingDate,
    execution_date: TradingDate,
}

impl SimulationSchedule {
    pub fn new(
        warmup_sessions: Vec<String>,
        evaluation_sessions: Vec<String>,
    ) -> Result<Self, SimulationContractError> {
        let schedule = Self {
            warmup_sessions: warmup_sessions
                .iter()
                .map(|date| {
                    TradingDate::parse(date).map_err(|_| SimulationContractError::InvalidSchedule)
                })
                .collect::<Result<_, _>>()?,
            signal_execution: Vec::new(),
            evaluation_sessions: evaluation_sessions
                .iter()
                .map(|date| {
                    TradingDate::parse(date).map_err(|_| SimulationContractError::InvalidSchedule)
                })
                .collect::<Result<_, _>>()?,
            phase_order: LedgerPhase::canonical_order(),
        }
        .with_pairs()?;
        schedule.validate()?;
        Ok(schedule)
    }

    fn with_pairs(mut self) -> Result<Self, SimulationContractError> {
        let all = self
            .warmup_sessions
            .iter()
            .chain(self.evaluation_sessions.iter())
            .copied()
            .collect::<Vec<_>>();
        self.signal_execution = all
            .windows(2)
            .skip(self.warmup_sessions.len().saturating_sub(1))
            .map(|window| SignalExecutionPair {
                signal_date: window[0],
                execution_date: window[1],
            })
            .collect();
        if self.warmup_sessions.last().is_none() {
            return Err(SimulationContractError::InvalidSchedule);
        }
        Ok(self)
    }

    fn validate(&self) -> Result<(), SimulationContractError> {
        if self.warmup_sessions.len() < 200
            || self.evaluation_sessions.len() < 2
            || self.phase_order != LedgerPhase::canonical_order()
        {
            return Err(SimulationContractError::InvalidSchedule);
        }
        let all = self
            .warmup_sessions
            .iter()
            .chain(self.evaluation_sessions.iter())
            .collect::<Vec<_>>();
        if all.windows(2).any(|window| window[0] >= window[1])
            || all.iter().any(|date| date.is_weekend())
            || self.signal_execution.len() != self.evaluation_sessions.len()
        {
            return Err(SimulationContractError::InvalidSchedule);
        }
        for (index, pair) in self.signal_execution.iter().enumerate() {
            if pair.signal_date != *all[self.warmup_sessions.len() + index - 1]
                || pair.execution_date != self.evaluation_sessions[index]
            {
                return Err(SimulationContractError::InvalidSchedule);
            }
        }
        Ok(())
    }

    pub fn warmup_sessions(&self) -> &[TradingDate] {
        &self.warmup_sessions
    }

    pub fn evaluation_sessions(&self) -> &[TradingDate] {
        &self.evaluation_sessions
    }

    pub fn signal_execution_pairs(&self) -> Vec<(TradingDate, TradingDate)> {
        self.signal_execution
            .iter()
            .map(|pair| (pair.signal_date, pair.execution_date))
            .collect()
    }
}

/// Explicit fixed convention tree carried into the result digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimulationConventions {
    precision: PrecisionConvention,
    portfolio: PortfolioConvention,
    pricing: PricingConvention,
    cost: CostConvention,
    benchmark: BenchmarkConvention,
    turnover: TurnoverConvention,
    metrics: MetricsConvention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PrecisionConvention {
    price_scale: u8,
    money_scale: u8,
    ratio_scale: u8,
    quantity_scale: u8,
    target_weight_scale_ppm: u32,
    ratio_rounding: RoundingRule,
    negative_zero: NegativeZeroRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PortfolioConvention {
    initial_capital: Money4,
    currency: Currency,
    quantity_unit: QuantityUnit,
    share_sizing: ShareSizingRule,
    target_transition: TargetTransitionRule,
    residual_cash: ResidualCashRule,
    borrowing: bool,
    short_selling: bool,
    cash_interest: CashInterestRule,
    additional_cash_flows: bool,
    dividends: DividendRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PricingConvention {
    signal_price: SignalPriceRule,
    execution_price: ExecutionPriceRule,
    valuation_price: ValuationPriceRule,
    corporate_actions: CorporateActionRule,
    missing_data: MissingDataRule,
    volume: VolumeRule,
    market_impact: MarketImpactRule,
    partial_fills: bool,
    trade_order: TradeOrderRule,
    ending: EndingRule,
    warmup_returns_included: bool,
    market_timezone: String,
    timestamp_fields_included: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CostConvention {
    profile: CostProfile,
    per_side_rate_numerator: u32,
    per_side_rate_denominator: u32,
    minimum_cost: Money4,
    separate_tax: Money4,
    price_slippage: Money4,
    rounding: CostRoundingRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct BenchmarkConvention {
    instrument_id: String,
    policy: BenchmarkRule,
    initial_purchase: InitialPurchaseRule,
    rebalancing: bool,
    additional_cash_purchases: bool,
    forced_final_liquidation: bool,
    same_cost_profile: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnoverConvention {
    formula: TurnoverFormula,
    include_initial_trade: bool,
    fees_and_deposits_included: bool,
    annualized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MetricsConvention {
    minimum_evaluation_sessions: u32,
    allowed_metrics: Vec<MetricName>,
    cagr_included: bool,
    annualized_volatility_included: bool,
    sharpe_included: bool,
    sortino_included: bool,
    win_rate_included: bool,
    trade_return_included: bool,
    monthly_metrics_included: bool,
}

macro_rules! closed_wire_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }
    };
}

closed_wire_enum!(RoundingRule { TiesToEven => "TIES_TO_EVEN" });
closed_wire_enum!(NegativeZeroRule { NormalizeToZero => "NORMALIZE_TO_ZERO" });
closed_wire_enum!(QuantityUnit { WholeShares => "WHOLE_SHARES" });
closed_wire_enum!(ShareSizingRule { MaximumCostInclusiveWholeShares => "MAXIMUM_COST_INCLUSIVE_WHOLE_SHARES" });
closed_wire_enum!(TargetTransitionRule { FullPositionOrCashOnlyOnTargetChange => "FULL_POSITION_OR_CASH_ONLY_ON_TARGET_CHANGE" });
closed_wire_enum!(ResidualCashRule { NoAdditionalPurchaseAfterInitialFill => "NO_ADDITIONAL_PURCHASE_AFTER_INITIAL_FILL" });
closed_wire_enum!(CashInterestRule { None => "NONE" });
closed_wire_enum!(DividendRule { Excluded => "EXCLUDED" });
closed_wire_enum!(SignalPriceRule { AdjustedClose => "ADJUSTED_CLOSE" });
closed_wire_enum!(ExecutionPriceRule { RawOpen => "RAW_OPEN" });
closed_wire_enum!(ValuationPriceRule { RawClose => "RAW_CLOSE" });
closed_wire_enum!(CorporateActionRule { RejectNoncashAndRequireRawAdjustedEquality => "REJECT_NONCASH_AND_REQUIRE_RAW_ADJUSTED_OHLC_EQUAL" });
closed_wire_enum!(MissingDataRule { FailWholeRunNoCarryForward => "FAIL_WHOLE_RUN_NO_CARRY_FORWARD" });
closed_wire_enum!(VolumeRule { PositiveOnTrade => "REQUIRE_POSITIVE_VOLUME_WHEN_TRADING" });
closed_wire_enum!(MarketImpactRule { None => "NONE" });
closed_wire_enum!(TradeOrderRule { SellBeforeBuy => "SELL_BEFORE_BUY" });
closed_wire_enum!(EndingRule { FinalCloseNoLiquidation => "FINAL_CLOSE_NO_LIQUIDATION" });
closed_wire_enum!(CostRoundingRule { CeilingToMoneyScale => "CEILING_TO_MONEY_SCALE" });
closed_wire_enum!(BenchmarkRule { BuyAndHoldAtStartOpen => "BUY_AND_HOLD_AT_START_OPEN" });
closed_wire_enum!(InitialPurchaseRule { CostInclusiveMaximumWholeShares => "COST_INCLUSIVE_MAXIMUM_WHOLE_SHARES" });
closed_wire_enum!(TurnoverFormula { SumDailyBuyAndSellOverTwicePretradeOpenNav => "SUM_DAILY_BUY_PLUS_SELL_OVER_TWICE_PRETRADE_OPEN_NAV" });
closed_wire_enum!(MetricName {
    DailyReturns => "daily_returns",
    PeriodReturn => "period_return",
    EndingNav => "ending_nav",
    EndingCash => "ending_cash",
    EndingHoldings => "ending_holdings",
    CumulativeCosts => "cumulative_costs",
    BuyNotional => "buy_notional",
    SellNotional => "sell_notional",
    MaximumDrawdown => "maximum_drawdown",
    PeriodTurnover => "period_turnover",
    FillCount => "fill_count",
    BenchmarkReturnDifference => "benchmark_return_difference"
});
closed_wire_enum!(TrendTargetRule {
    BuyWhenFastTrendBelowSlowTrendOtherwiseCash => "BUY_069500_IF_TREND_50_LT_TREND_200_ELSE_CASH"
});

impl SimulationConventions {
    fn approved(profile: CostProfile) -> Result<Self, SimulationContractError> {
        let (numerator, denominator) = profile.fee_rate();
        Ok(Self {
            precision: PrecisionConvention {
                price_scale: 4,
                money_scale: 4,
                ratio_scale: 12,
                quantity_scale: 0,
                target_weight_scale_ppm: 1_000_000,
                ratio_rounding: RoundingRule::TiesToEven,
                negative_zero: NegativeZeroRule::NormalizeToZero,
            },
            portfolio: PortfolioConvention {
                initial_capital: Money4::parse(PRODUCTION_INITIAL_CAPITAL)?,
                currency: Currency::KRW,
                quantity_unit: QuantityUnit::WholeShares,
                share_sizing: ShareSizingRule::MaximumCostInclusiveWholeShares,
                target_transition: TargetTransitionRule::FullPositionOrCashOnlyOnTargetChange,
                residual_cash: ResidualCashRule::NoAdditionalPurchaseAfterInitialFill,
                borrowing: false,
                short_selling: false,
                cash_interest: CashInterestRule::None,
                additional_cash_flows: false,
                dividends: DividendRule::Excluded,
            },
            pricing: approved_pricing(),
            cost: CostConvention {
                profile,
                per_side_rate_numerator: numerator,
                per_side_rate_denominator: denominator,
                minimum_cost: Money4::zero(),
                separate_tax: Money4::zero(),
                price_slippage: Money4::zero(),
                rounding: CostRoundingRule::CeilingToMoneyScale,
            },
            benchmark: approved_benchmark(),
            turnover: TurnoverConvention {
                formula: TurnoverFormula::SumDailyBuyAndSellOverTwicePretradeOpenNav,
                include_initial_trade: true,
                fees_and_deposits_included: false,
                annualized: false,
            },
            metrics: approved_metrics(),
        })
    }

    #[cfg(test)]
    fn synthetic_test(profile: CostProfile) -> Result<Self, SimulationContractError> {
        let mut conventions = Self::approved(profile)?;
        conventions.portfolio.initial_capital = Money4::synthetic_test_capital();
        Ok(conventions)
    }

    fn validate(&self, allow_synthetic_capital: bool) -> Result<(), SimulationContractError> {
        let expected = Self::approved(self.cost.profile)?;
        let valid_capital = self.portfolio.initial_capital.as_str() == PRODUCTION_INITIAL_CAPITAL
            || (allow_synthetic_capital && self.portfolio.initial_capital.as_str() == "1000.0000");
        if !valid_capital
            || self.precision != expected.precision
            || self.portfolio.currency != expected.portfolio.currency
            || self.portfolio.quantity_unit != expected.portfolio.quantity_unit
            || self.portfolio.share_sizing != expected.portfolio.share_sizing
            || self.portfolio.target_transition != expected.portfolio.target_transition
            || self.portfolio.residual_cash != expected.portfolio.residual_cash
            || self.portfolio.borrowing != expected.portfolio.borrowing
            || self.portfolio.short_selling != expected.portfolio.short_selling
            || self.portfolio.cash_interest != expected.portfolio.cash_interest
            || self.portfolio.additional_cash_flows != expected.portfolio.additional_cash_flows
            || self.portfolio.dividends != expected.portfolio.dividends
            || self.pricing != expected.pricing
            || self.cost != expected.cost
            || self.benchmark != expected.benchmark
            || self.turnover != expected.turnover
            || self.metrics != expected.metrics
        {
            return Err(SimulationContractError::InvalidValue);
        }
        Ok(())
    }

    pub fn cost_profile(&self) -> CostProfile {
        self.cost.profile
    }

    pub fn initial_capital(&self) -> &Money4 {
        &self.portfolio.initial_capital
    }
}

fn approved_pricing() -> PricingConvention {
    PricingConvention {
        signal_price: SignalPriceRule::AdjustedClose,
        execution_price: ExecutionPriceRule::RawOpen,
        valuation_price: ValuationPriceRule::RawClose,
        corporate_actions: CorporateActionRule::RejectNoncashAndRequireRawAdjustedEquality,
        missing_data: MissingDataRule::FailWholeRunNoCarryForward,
        volume: VolumeRule::PositiveOnTrade,
        market_impact: MarketImpactRule::None,
        partial_fills: false,
        trade_order: TradeOrderRule::SellBeforeBuy,
        ending: EndingRule::FinalCloseNoLiquidation,
        warmup_returns_included: false,
        market_timezone: "Asia/Seoul".to_owned(),
        timestamp_fields_included: false,
    }
}

fn approved_benchmark() -> BenchmarkConvention {
    BenchmarkConvention {
        instrument_id: TREND_INSTRUMENT.to_owned(),
        policy: BenchmarkRule::BuyAndHoldAtStartOpen,
        initial_purchase: InitialPurchaseRule::CostInclusiveMaximumWholeShares,
        rebalancing: false,
        additional_cash_purchases: false,
        forced_final_liquidation: false,
        same_cost_profile: true,
    }
}

fn approved_metrics() -> MetricsConvention {
    MetricsConvention {
        minimum_evaluation_sessions: 2,
        allowed_metrics: vec![
            MetricName::DailyReturns,
            MetricName::PeriodReturn,
            MetricName::EndingNav,
            MetricName::EndingCash,
            MetricName::EndingHoldings,
            MetricName::CumulativeCosts,
            MetricName::BuyNotional,
            MetricName::SellNotional,
            MetricName::MaximumDrawdown,
            MetricName::PeriodTurnover,
            MetricName::FillCount,
            MetricName::BenchmarkReturnDifference,
        ],
        cagr_included: false,
        annualized_volatility_included: false,
        sharpe_included: false,
        sortino_included: false,
        win_rate_included: false,
        trade_return_included: false,
        monthly_metrics_included: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerPhase {
    #[serde(rename = "INITIAL_CASH")]
    InitialCash,
    #[serde(rename = "PRE_OPEN_MARK")]
    PreOpenMark,
    #[serde(rename = "SELL")]
    Sell,
    #[serde(rename = "BUY")]
    Buy,
    #[serde(rename = "CLOSE_MARK")]
    CloseMark,
    #[serde(rename = "SIGNAL_FOR_NEXT_SESSION")]
    SignalForNextSession,
}

impl LedgerPhase {
    fn canonical_order() -> Vec<Self> {
        vec![
            Self::InitialCash,
            Self::PreOpenMark,
            Self::Sell,
            Self::Buy,
            Self::CloseMark,
            Self::SignalForNextSession,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeSide {
    #[serde(rename = "BUY")]
    Buy,
    #[serde(rename = "SELL")]
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnfilledReason {
    #[serde(rename = "NOT_APPLICABLE")]
    NotApplicable,
    #[serde(rename = "NO_ORDER_REQUIRED")]
    NoOrderRequired,
    #[serde(rename = "ZERO_SHARE_QUANTITY")]
    ZeroShareQuantity,
    #[serde(rename = "FILLED")]
    Filled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TradeEvent {
    instrument_id: String,
    side: TradeSide,
    quantity: u64,
    price: Price4,
    notional: Money4,
    cost: Money4,
    cash_after: Money4,
    position_after: u64,
}

impl TradeEvent {
    pub fn new(
        instrument_id: &str,
        side: TradeSide,
        quantity: u64,
        price: Price4,
        cost: Money4,
        cash_after: Money4,
        position_after: u64,
    ) -> Result<Self, SimulationContractError> {
        if instrument_id != TREND_INSTRUMENT || quantity == 0 {
            return Err(SimulationContractError::InvalidLedger);
        }
        let notional_units = price
            .units()?
            .checked_mul(i128::from(quantity))
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        let notional = Money4::parse(&format_units(notional_units, 4)?)?;
        let event = Self {
            instrument_id: instrument_id.to_owned(),
            side,
            quantity,
            price,
            notional,
            cost,
            cash_after,
            position_after,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), SimulationContractError> {
        if self.instrument_id != TREND_INSTRUMENT || self.quantity == 0 {
            return Err(SimulationContractError::InvalidLedger);
        }
        let expected_notional = self
            .price
            .units()?
            .checked_mul(i128::from(self.quantity))
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        if self.notional.units()? != expected_notional {
            return Err(SimulationContractError::InvalidLedger);
        }
        match self.side {
            TradeSide::Buy if self.position_after != self.quantity => {
                Err(SimulationContractError::InvalidLedger)
            }
            TradeSide::Sell if self.position_after != 0 => {
                Err(SimulationContractError::InvalidLedger)
            }
            _ => Ok(()),
        }
    }

    pub fn side(&self) -> TradeSide {
        self.side
    }

    pub fn quantity(&self) -> u64 {
        self.quantity
    }

    pub fn price(&self) -> &Price4 {
        &self.price
    }

    pub fn notional(&self) -> &Money4 {
        &self.notional
    }

    pub fn cost(&self) -> &Money4 {
        &self.cost
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PositionMark {
    instrument_id: String,
    quantity: u64,
    price: Price4,
    market_value: Money4,
}

impl PositionMark {
    pub fn new(
        instrument_id: &str,
        quantity: u64,
        price: Price4,
    ) -> Result<Self, SimulationContractError> {
        if instrument_id != TREND_INSTRUMENT || quantity == 0 {
            return Err(SimulationContractError::InvalidLedger);
        }
        let value = price
            .units()?
            .checked_mul(i128::from(quantity))
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        Ok(Self {
            instrument_id: instrument_id.to_owned(),
            quantity,
            market_value: Money4::parse(&format_units(value, 4)?)?,
            price,
        })
    }

    pub fn quantity(&self) -> u64 {
        self.quantity
    }

    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub fn market_value(&self) -> &Money4 {
        &self.market_value
    }

    pub fn price(&self) -> &Price4 {
        &self.price
    }
}

fn format_units(units: i128, scale: usize) -> Result<String, SimulationContractError> {
    let divisor = 10i128
        .checked_pow(scale as u32)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let whole = units / divisor;
    let fraction = units % divisor;
    if fraction < 0 {
        return Err(SimulationContractError::InvalidValue);
    }
    Ok(format!("{whole}.{fraction:0scale$}", scale = scale))
}

fn format_signed_units(units: i128, scale: usize) -> Result<String, SimulationContractError> {
    if units >= 0 {
        return format_units(units, scale);
    }
    let absolute = units
        .checked_abs()
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    Ok(format!("-{}", format_units(absolute, scale)?))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerPhaseRecord {
    session_date: TradingDate,
    phase: LedgerPhase,
    events: Vec<TradeEvent>,
    unfilled_reason: UnfilledReason,
    cash: Money4,
    positions: Vec<PositionMark>,
    nav: Money4,
}

impl LedgerPhaseRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_date: &str,
        phase: LedgerPhase,
        events: Vec<TradeEvent>,
        unfilled_reason: UnfilledReason,
        cash: Money4,
        positions: Vec<PositionMark>,
        nav: Money4,
    ) -> Result<Self, SimulationContractError> {
        let record = Self {
            session_date: TradingDate::parse(session_date)
                .map_err(|_| SimulationContractError::InvalidLedger)?,
            phase,
            events,
            unfilled_reason,
            cash,
            positions,
            nav,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), SimulationContractError> {
        if self.positions.len() > 1
            || self
                .positions
                .windows(2)
                .any(|pair| pair[0].instrument_id >= pair[1].instrument_id)
            || self
                .events
                .windows(2)
                .any(|pair| pair[0].instrument_id >= pair[1].instrument_id)
        {
            return Err(SimulationContractError::InvalidLedger);
        }
        for event in &self.events {
            event.validate()?;
        }
        let marked_value = self.positions.iter().try_fold(0i128, |sum, position| {
            sum.checked_add(position.market_value.units()?)
                .ok_or(SimulationContractError::ArithmeticOverflow)
        })?;
        if self.nav.units()?
            != self
                .cash
                .units()?
                .checked_add(marked_value)
                .ok_or(SimulationContractError::ArithmeticOverflow)?
        {
            return Err(SimulationContractError::InvalidLedger);
        }
        let is_trade_phase = matches!(self.phase, LedgerPhase::Sell | LedgerPhase::Buy);
        if !is_trade_phase {
            if !self.events.is_empty() || self.unfilled_reason != UnfilledReason::NotApplicable {
                return Err(SimulationContractError::InvalidLedger);
            }
        } else if self.events.is_empty() {
            if !matches!(
                self.unfilled_reason,
                UnfilledReason::NoOrderRequired | UnfilledReason::ZeroShareQuantity
            ) || (self.phase == LedgerPhase::Sell
                && self.unfilled_reason == UnfilledReason::ZeroShareQuantity)
            {
                return Err(SimulationContractError::InvalidLedger);
            }
        } else if self.unfilled_reason != UnfilledReason::Filled || self.events.len() != 1 {
            return Err(SimulationContractError::InvalidLedger);
        }
        if self.events.iter().any(|event| {
            (self.phase == LedgerPhase::Buy && event.side != TradeSide::Buy)
                || (self.phase == LedgerPhase::Sell && event.side != TradeSide::Sell)
        }) {
            return Err(SimulationContractError::InvalidLedger);
        }
        if let Some(event) = self.events.last() {
            let position_after = self
                .positions
                .first()
                .map_or(0, |position| position.quantity);
            if event.cash_after != self.cash || event.position_after != position_after {
                return Err(SimulationContractError::InvalidLedger);
            }
        }
        Ok(())
    }

    pub fn session_date(&self) -> TradingDate {
        self.session_date
    }

    pub fn phase(&self) -> LedgerPhase {
        self.phase
    }

    pub fn events(&self) -> &[TradeEvent] {
        &self.events
    }

    pub fn cash(&self) -> &Money4 {
        &self.cash
    }

    pub fn positions(&self) -> &[PositionMark] {
        &self.positions
    }

    pub fn nav(&self) -> &Money4 {
        &self.nav
    }

    pub fn unfilled_reason(&self) -> UnfilledReason {
        self.unfilled_reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DailyReturn {
    session_date: TradingDate,
    value: Ratio12,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Holding {
    instrument_id: String,
    quantity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerMetrics {
    daily_returns: Vec<DailyReturn>,
    period_return: Ratio12,
    ending_nav: Money4,
    ending_cash: Money4,
    ending_holdings: Vec<Holding>,
    cumulative_costs: Money4,
    buy_notional: Money4,
    sell_notional: Money4,
    maximum_drawdown: Ratio12,
    period_turnover: Ratio12,
    fill_count: u64,
}

impl LedgerMetrics {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        daily_returns: Vec<(String, Ratio12)>,
        period_return: Ratio12,
        ending_nav: Money4,
        ending_cash: Money4,
        ending_holdings: Vec<(String, u64)>,
        cumulative_costs: Money4,
        buy_notional: Money4,
        sell_notional: Money4,
        maximum_drawdown: Ratio12,
        period_turnover: Ratio12,
        fill_count: u64,
    ) -> Result<Self, SimulationContractError> {
        let result = Self {
            daily_returns: daily_returns
                .into_iter()
                .map(|(date, value)| {
                    Ok(DailyReturn {
                        session_date: TradingDate::parse(&date)
                            .map_err(|_| SimulationContractError::InvalidLedger)?,
                        value,
                    })
                })
                .collect::<Result<_, SimulationContractError>>()?,
            period_return,
            ending_nav,
            ending_cash,
            ending_holdings: ending_holdings
                .into_iter()
                .map(|(instrument_id, quantity)| Holding {
                    instrument_id,
                    quantity,
                })
                .collect(),
            cumulative_costs,
            buy_notional,
            sell_notional,
            maximum_drawdown,
            period_turnover,
            fill_count,
        };
        result.validate()?;
        Ok(result)
    }

    fn validate(&self) -> Result<(), SimulationContractError> {
        if self.maximum_drawdown.units()? < 0
            || self.maximum_drawdown.units()? > 1_000_000_000_000
            || self.period_turnover.units()? < 0
            || self.period_return.units()? < -1_000_000_000_000
            || self
                .ending_holdings
                .iter()
                .any(|holding| holding.instrument_id != TREND_INSTRUMENT || holding.quantity == 0)
            || self
                .ending_holdings
                .windows(2)
                .any(|pair| pair[0].instrument_id >= pair[1].instrument_id)
            || self
                .daily_returns
                .windows(2)
                .any(|pair| pair[0].session_date >= pair[1].session_date)
        {
            return Err(SimulationContractError::InvalidLedger);
        }
        for day in &self.daily_returns {
            if day.value.units()? <= -1_000_000_000_000 {
                return Err(SimulationContractError::InvalidLedger);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StrategyLedger {
    initial_cash: Money4,
    records: Vec<LedgerPhaseRecord>,
    metrics: LedgerMetrics,
    benchmark_return_difference: Ratio12,
}

impl StrategyLedger {
    pub fn new(
        initial_cash: Money4,
        records: Vec<LedgerPhaseRecord>,
        metrics: LedgerMetrics,
        benchmark_return_difference: Ratio12,
    ) -> Result<Self, SimulationContractError> {
        let result = Self {
            initial_cash,
            records,
            metrics,
            benchmark_return_difference,
        };
        result.metrics.validate()?;
        Ok(result)
    }

    pub fn records(&self) -> &[LedgerPhaseRecord] {
        &self.records
    }

    pub fn metrics(&self) -> &LedgerMetrics {
        &self.metrics
    }

    pub fn initial_cash(&self) -> &Money4 {
        &self.initial_cash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkLedger {
    initial_cash: Money4,
    records: Vec<LedgerPhaseRecord>,
    metrics: LedgerMetrics,
}

impl BenchmarkLedger {
    pub fn new(
        initial_cash: Money4,
        records: Vec<LedgerPhaseRecord>,
        metrics: LedgerMetrics,
    ) -> Result<Self, SimulationContractError> {
        let result = Self {
            initial_cash,
            records,
            metrics,
        };
        result.metrics.validate()?;
        Ok(result)
    }

    pub fn records(&self) -> &[LedgerPhaseRecord] {
        &self.records
    }

    pub fn metrics(&self) -> &LedgerMetrics {
        &self.metrics
    }

    pub fn initial_cash(&self) -> &Money4 {
        &self.initial_cash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimulationResult {
    contract: ContractIdentity,
    input: SimulationInputIdentity,
    engine: EngineIdentity,
    strategy: StrategyIdentity,
    schedule: SimulationSchedule,
    conventions: SimulationConventions,
    signals: Vec<SignalSnapshot>,
    strategy_ledger: StrategyLedger,
    benchmark_ledger: BenchmarkLedger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ContractIdentity {
    contract_version: String,
    contract_document_sha256: Sha256Digest,
    result_schema_version: String,
}

impl ContractIdentity {
    fn approved() -> Result<Self, SimulationContractError> {
        Ok(Self {
            contract_version: CONTRACT_VERSION.to_owned(),
            contract_document_sha256: Sha256Digest::parse(APPROVED_CONTRACT_SHA256)?,
            result_schema_version: RESULT_SCHEMA_VERSION.to_owned(),
        })
    }
}

impl SimulationResult {
    pub fn new(
        engine: EngineIdentity,
        strategy: StrategyIdentity,
        schedule: SimulationSchedule,
        cost_profile: CostProfile,
        signals: Vec<SignalSnapshot>,
        strategy_ledger: StrategyLedger,
        benchmark_ledger: BenchmarkLedger,
    ) -> Result<Self, SimulationContractError> {
        let result = Self {
            contract: ContractIdentity::approved()?,
            input: SimulationInputIdentity::approved()?,
            engine,
            strategy,
            schedule,
            conventions: SimulationConventions::approved(cost_profile)?,
            signals,
            strategy_ledger,
            benchmark_ledger,
        };
        result.validate(false)?;
        Ok(result)
    }

    /// Parses a result with duplicate-key detection, closed nested fields,
    /// fixed approved values, exact syntax, and cross-field schedule checks.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SimulationContractError> {
        let value = parse_strict_json(bytes)?;
        let wire: ResultWire =
            serde_json::from_value(value).map_err(|_| SimulationContractError::InvalidSchema)?;
        let result = Self::try_from(wire)?;
        result.validate(cfg!(test))?;
        Ok(result)
    }

    pub fn canonical_json(&self) -> Result<Vec<u8>, SimulationContractError> {
        self.validate(cfg!(test))?;
        canonical_json(self)
    }

    /// Returns the hash of the canonical result body. The hash is never
    /// inserted into that body, avoiding a circular preimage.
    pub fn result_content_sha256(&self) -> Result<Sha256Digest, SimulationContractError> {
        let bytes = self.canonical_json()?;
        Ok(Sha256Digest(ContentHash::from_bytes(&bytes)))
    }

    pub fn schedule(&self) -> &SimulationSchedule {
        &self.schedule
    }

    pub fn signals(&self) -> &[SignalSnapshot] {
        &self.signals
    }

    pub fn strategy_ledger(&self) -> &StrategyLedger {
        &self.strategy_ledger
    }

    pub fn benchmark_ledger(&self) -> &BenchmarkLedger {
        &self.benchmark_ledger
    }

    fn validate(&self, allow_synthetic_capital: bool) -> Result<(), SimulationContractError> {
        self.schedule.validate()?;
        self.conventions.validate(allow_synthetic_capital)?;
        if self.contract != ContractIdentity::approved()?
            || self.input != SimulationInputIdentity::approved()?
            || self.strategy.strategy_id != "trend_following"
            || self.strategy.version != "1.0.0"
            || self.strategy.config.benchmark_instrument != TREND_INSTRUMENT
            || self.strategy.config.fast_ma != 50
            || self.strategy.config.slow_ma != 200
            || self.strategy.factor_definition.version.is_empty()
            || !is_ascii_identifier(&self.strategy.factor_definition.version)
            || self.signals.len() != self.schedule.signal_execution.len()
        {
            return Err(SimulationContractError::InvalidSchema);
        }
        for (signal, pair) in self.signals.iter().zip(&self.schedule.signal_execution) {
            signal.validate()?;
            if signal.signal_date != pair.signal_date {
                return Err(SimulationContractError::InvalidSchedule);
            }
        }
        validate_ledger(
            self.strategy_ledger.initial_cash.as_str(),
            &self.strategy_ledger.records,
            &self.strategy_ledger.metrics,
            &self.schedule,
            &self.conventions.portfolio.initial_capital,
            self.conventions.cost.profile,
        )?;
        validate_ledger(
            self.benchmark_ledger.initial_cash.as_str(),
            &self.benchmark_ledger.records,
            &self.benchmark_ledger.metrics,
            &self.schedule,
            &self.conventions.portfolio.initial_capital,
            self.conventions.cost.profile,
        )?;
        validate_shared_market_marks(
            &self.strategy_ledger.records,
            &self.benchmark_ledger.records,
            self.conventions.cost.profile,
        )?;
        if self.strategy_ledger.metrics.daily_returns.len()
            != self.schedule.evaluation_sessions.len()
            || self.benchmark_ledger.metrics.daily_returns.len()
                != self.schedule.evaluation_sessions.len()
            || self.strategy_ledger.metrics.ending_nav
                != self
                    .strategy_ledger
                    .records
                    .last()
                    .ok_or(SimulationContractError::InvalidLedger)?
                    .nav
            || self.benchmark_ledger.metrics.ending_nav
                != self
                    .benchmark_ledger
                    .records
                    .last()
                    .ok_or(SimulationContractError::InvalidLedger)?
                    .nav
        {
            return Err(SimulationContractError::InvalidLedger);
        }
        let difference = derive_benchmark_return_difference(
            &self.strategy_ledger.initial_cash,
            &self.strategy_ledger.metrics.ending_nav,
            &self.benchmark_ledger.metrics.ending_nav,
        )?;
        if difference != self.strategy_ledger.benchmark_return_difference {
            return Err(SimulationContractError::InvalidLedger);
        }
        if self
            .schedule
            .warmup_sessions
            .first()
            .is_none_or(|date| *date < self.input.approved_start_date)
            || self
                .schedule
                .evaluation_sessions
                .last()
                .is_none_or(|date| *date > self.input.approved_end_date)
        {
            return Err(SimulationContractError::InvalidSchedule);
        }
        validate_signal_targets_against_schedule(&self.signals, &self.schedule)?;
        validate_strategy_target_path(&self.signals, &self.strategy_ledger, &self.schedule)?;
        validate_benchmark_path(&self.benchmark_ledger, &self.schedule)?;
        Ok(())
    }
}

fn validate_ledger(
    initial_cash: &str,
    records: &[LedgerPhaseRecord],
    metrics: &LedgerMetrics,
    schedule: &SimulationSchedule,
    capital: &Money4,
    cost_profile: CostProfile,
) -> Result<(), SimulationContractError> {
    if initial_cash != capital.as_str() {
        return Err(SimulationContractError::InvalidLedger);
    }
    let mut expected = Vec::with_capacity(schedule.evaluation_sessions.len() * 6 + 1);
    for (index, date) in schedule.evaluation_sessions.iter().enumerate() {
        if index == 0 {
            expected.push((*date, LedgerPhase::InitialCash));
        }
        expected.push((*date, LedgerPhase::PreOpenMark));
        expected.push((*date, LedgerPhase::Sell));
        expected.push((*date, LedgerPhase::Buy));
        expected.push((*date, LedgerPhase::CloseMark));
        if index + 1 < schedule.evaluation_sessions.len() {
            expected.push((*date, LedgerPhase::SignalForNextSession));
        }
    }
    if records.len() != expected.len()
        || records
            .iter()
            .zip(expected)
            .any(|(record, (date, phase))| record.session_date != date || record.phase != phase)
        || metrics.daily_returns.len() != schedule.evaluation_sessions.len()
        || metrics
            .daily_returns
            .iter()
            .zip(&schedule.evaluation_sessions)
            .any(|(daily, date)| daily.session_date != *date)
    {
        return Err(SimulationContractError::InvalidLedger);
    }
    let initial = records
        .first()
        .ok_or(SimulationContractError::InvalidLedger)?;
    if initial.cash != *capital || !initial.positions.is_empty() || initial.nav != *capital {
        return Err(SimulationContractError::InvalidLedger);
    }
    for record in records {
        record.validate()?;
    }
    validate_ledger_transitions(records, schedule, cost_profile)?;
    let derived = derive_performance_metrics(capital, records)?;
    let mut costs = 0i128;
    let mut buys = 0i128;
    let mut sells = 0i128;
    let mut fills = 0u64;
    for event in records.iter().flat_map(|record| &record.events) {
        let notional = event.notional.units()?;
        let expected_cost = expected_fee_units(notional, cost_profile)?;
        if event.cost.units()? != expected_cost {
            return Err(SimulationContractError::InvalidLedger);
        }
        costs = costs
            .checked_add(event.cost.units()?)
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        match event.side {
            TradeSide::Buy => {
                buys = buys
                    .checked_add(notional)
                    .ok_or(SimulationContractError::ArithmeticOverflow)?;
            }
            TradeSide::Sell => {
                sells = sells
                    .checked_add(notional)
                    .ok_or(SimulationContractError::ArithmeticOverflow)?;
            }
        }
        fills = fills
            .checked_add(1)
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
    }
    if metrics.cumulative_costs.units()? != costs
        || metrics.buy_notional.units()? != buys
        || metrics.sell_notional.units()? != sells
        || metrics.fill_count != fills
    {
        return Err(SimulationContractError::InvalidLedger);
    }
    if metrics
        .daily_returns
        .iter()
        .zip(derived.daily_returns())
        .any(|(actual, (date, value))| actual.session_date != *date || actual.value != *value)
        || metrics.period_return != *derived.period_return()
        || metrics.maximum_drawdown != *derived.maximum_drawdown()
        || metrics.period_turnover != *derived.period_turnover()
    {
        return Err(SimulationContractError::InvalidLedger);
    }
    if metrics.ending_cash
        != records
            .iter()
            .rev()
            .find(|record| record.phase == LedgerPhase::CloseMark)
            .ok_or(SimulationContractError::InvalidLedger)?
            .cash
        || metrics.ending_holdings
            != records
                .iter()
                .rev()
                .find(|record| record.phase == LedgerPhase::CloseMark)
                .ok_or(SimulationContractError::InvalidLedger)?
                .positions
                .iter()
                .map(|position| Holding {
                    instrument_id: position.instrument_id.clone(),
                    quantity: position.quantity,
                })
                .collect::<Vec<_>>()
    {
        return Err(SimulationContractError::InvalidLedger);
    }
    Ok(())
}

fn validate_ledger_transitions(
    records: &[LedgerPhaseRecord],
    schedule: &SimulationSchedule,
    cost_profile: CostProfile,
) -> Result<(), SimulationContractError> {
    let find = |date: TradingDate, phase: LedgerPhase| {
        records
            .iter()
            .find(|record| record.session_date == date && record.phase == phase)
            .ok_or(SimulationContractError::InvalidLedger)
    };
    let initial_cash = records
        .first()
        .ok_or(SimulationContractError::InvalidLedger)?
        .cash
        .units()?;
    let mut prior_close: Option<&LedgerPhaseRecord> = None;
    for (index, date) in schedule.evaluation_sessions.iter().enumerate() {
        let pre_open = find(*date, LedgerPhase::PreOpenMark)?;
        let sell = find(*date, LedgerPhase::Sell)?;
        let buy = find(*date, LedgerPhase::Buy)?;
        let close = find(*date, LedgerPhase::CloseMark)?;
        let signal = if index + 1 < schedule.evaluation_sessions.len() {
            Some(find(*date, LedgerPhase::SignalForNextSession)?)
        } else {
            None
        };
        let (opening_cash, opening_quantity) = match prior_close {
            Some(previous) => (previous.cash.units()?, marked_quantity(previous)),
            None => (initial_cash, 0),
        };
        if pre_open.cash.units()? != opening_cash || marked_quantity(pre_open) != opening_quantity {
            return Err(SimulationContractError::InvalidLedger);
        }
        let (after_sell_cash, after_sell_quantity) = validate_trade_transition(
            sell,
            opening_cash,
            opening_quantity,
            TradeSide::Sell,
            cost_profile,
        )?;
        let (after_buy_cash, after_buy_quantity) = validate_trade_transition(
            buy,
            after_sell_cash,
            after_sell_quantity,
            TradeSide::Buy,
            cost_profile,
        )?;
        if close.cash.units()? != after_buy_cash || marked_quantity(close) != after_buy_quantity {
            return Err(SimulationContractError::InvalidLedger);
        }
        if let Some(signal) = signal
            && (signal.cash != close.cash || signal.positions != close.positions)
        {
            return Err(SimulationContractError::InvalidLedger);
        }
        prior_close = Some(close);
    }
    Ok(())
}

fn marked_quantity(record: &LedgerPhaseRecord) -> u64 {
    record
        .positions
        .first()
        .map_or(0, |position| position.quantity)
}

fn validate_trade_transition(
    after: &LedgerPhaseRecord,
    before_cash: i128,
    before_quantity: u64,
    side: TradeSide,
    cost_profile: CostProfile,
) -> Result<(i128, u64), SimulationContractError> {
    if after.events.is_empty() {
        if after.cash.units()? != before_cash || marked_quantity(after) != before_quantity {
            return Err(SimulationContractError::InvalidLedger);
        }
        return Ok((before_cash, before_quantity));
    }
    let event = after
        .events
        .first()
        .ok_or(SimulationContractError::InvalidLedger)?;
    if event.side != side {
        return Err(SimulationContractError::InvalidLedger);
    }
    let (cash_delta, quantity_after) = match side {
        TradeSide::Buy => {
            if before_quantity != 0 || event.position_after != event.quantity {
                return Err(SimulationContractError::InvalidLedger);
            }
            validate_maximum_affordable_buy(before_cash, event, cost_profile)?;
            let spent = event
                .notional
                .units()?
                .checked_add(event.cost.units()?)
                .ok_or(SimulationContractError::ArithmeticOverflow)?;
            (
                before_cash
                    .checked_sub(spent)
                    .ok_or(SimulationContractError::ArithmeticOverflow)?,
                event.quantity,
            )
        }
        TradeSide::Sell => {
            if before_quantity == 0
                || event.quantity != before_quantity
                || event.position_after != 0
            {
                return Err(SimulationContractError::InvalidLedger);
            }
            let received = event
                .notional
                .units()?
                .checked_sub(event.cost.units()?)
                .ok_or(SimulationContractError::ArithmeticOverflow)?;
            (
                before_cash
                    .checked_add(received)
                    .ok_or(SimulationContractError::ArithmeticOverflow)?,
                0,
            )
        }
    };
    if after.cash.units()? != cash_delta
        || event.cash_after.units()? != cash_delta
        || marked_quantity(after) != quantity_after
    {
        return Err(SimulationContractError::InvalidLedger);
    }
    Ok((cash_delta, quantity_after))
}

fn expected_fee_units(
    notional_units: i128,
    cost_profile: CostProfile,
) -> Result<i128, SimulationContractError> {
    if notional_units < 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let (rate_numerator, rate_denominator) = cost_profile.fee_rate();
    let denominator = i128::from(rate_denominator);
    if denominator <= 0 {
        return Err(SimulationContractError::InvalidValue);
    }
    let numerator = notional_units
        .checked_mul(i128::from(rate_numerator))
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    if numerator == 0 {
        return Ok(0);
    }
    numerator
        .checked_add(denominator - 1)
        .ok_or(SimulationContractError::ArithmeticOverflow)
        .map(|rounded| rounded / denominator)
}

fn validate_maximum_affordable_buy(
    available_cash: i128,
    event: &TradeEvent,
    cost_profile: CostProfile,
) -> Result<(), SimulationContractError> {
    let quantity = event.quantity;
    if quantity == 0 {
        return Err(SimulationContractError::InvalidLedger);
    }
    let price_units = event.price.units()?;
    let notional = price_units
        .checked_mul(i128::from(quantity))
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let fee = expected_fee_units(notional, cost_profile)?;
    if event.notional.units()? != notional || event.cost.units()? != fee {
        return Err(SimulationContractError::InvalidLedger);
    }
    let spent = notional
        .checked_add(fee)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    if spent > available_cash {
        return Err(SimulationContractError::InvalidLedger);
    }

    let next_quantity = quantity
        .checked_add(1)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let next_notional = price_units
        .checked_mul(i128::from(next_quantity))
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    let next_fee = expected_fee_units(next_notional, cost_profile)?;
    let next_spent = next_notional
        .checked_add(next_fee)
        .ok_or(SimulationContractError::ArithmeticOverflow)?;
    if next_spent <= available_cash {
        return Err(SimulationContractError::InvalidLedger);
    }
    Ok(())
}

fn collect_opening_prices(
    records: &[LedgerPhaseRecord],
) -> Result<BTreeMap<(TradingDate, String), Price4>, SimulationContractError> {
    let mut prices = BTreeMap::new();
    for record in records {
        match record.phase {
            LedgerPhase::PreOpenMark => {
                for position in &record.positions {
                    insert_session_price(
                        &mut prices,
                        record.session_date,
                        &position.instrument_id,
                        &position.price,
                    )?;
                }
            }
            LedgerPhase::Sell | LedgerPhase::Buy => {
                for position in &record.positions {
                    insert_session_price(
                        &mut prices,
                        record.session_date,
                        &position.instrument_id,
                        &position.price,
                    )?;
                }
                for event in &record.events {
                    insert_session_price(
                        &mut prices,
                        record.session_date,
                        &event.instrument_id,
                        &event.price,
                    )?;
                }
            }
            LedgerPhase::InitialCash
            | LedgerPhase::CloseMark
            | LedgerPhase::SignalForNextSession => {}
        }
    }
    Ok(prices)
}

fn collect_closing_prices(
    records: &[LedgerPhaseRecord],
) -> BTreeMap<(TradingDate, String), Price4> {
    records
        .iter()
        .filter(|record| record.phase == LedgerPhase::CloseMark)
        .flat_map(|record| {
            record.positions.iter().map(|position| {
                (
                    (record.session_date, position.instrument_id.clone()),
                    position.price.clone(),
                )
            })
        })
        .collect()
}

fn insert_session_price(
    prices: &mut BTreeMap<(TradingDate, String), Price4>,
    date: TradingDate,
    instrument_id: &str,
    price: &Price4,
) -> Result<(), SimulationContractError> {
    let key = (date, instrument_id.to_owned());
    if prices.get(&key).is_some_and(|existing| existing != price) {
        return Err(SimulationContractError::InvalidLedger);
    }
    prices.entry(key).or_insert_with(|| price.clone());
    Ok(())
}

fn validate_shared_market_marks(
    strategy_records: &[LedgerPhaseRecord],
    benchmark_records: &[LedgerPhaseRecord],
    cost_profile: CostProfile,
) -> Result<(), SimulationContractError> {
    let strategy_open = collect_opening_prices(strategy_records)?;
    let benchmark_open = collect_opening_prices(benchmark_records)?;
    let mut shared_open = strategy_open.clone();
    for (key, price) in &benchmark_open {
        if shared_open
            .get(key)
            .is_some_and(|existing| existing != price)
        {
            return Err(SimulationContractError::InvalidLedger);
        }
        shared_open
            .entry(key.clone())
            .or_insert_with(|| price.clone());
    }
    validate_zero_share_claims(strategy_records, &shared_open, cost_profile)?;
    validate_zero_share_claims(benchmark_records, &shared_open, cost_profile)?;

    let strategy_close = collect_closing_prices(strategy_records);
    let benchmark_close = collect_closing_prices(benchmark_records);
    for (key, strategy_price) in &strategy_close {
        if benchmark_close
            .get(key)
            .is_some_and(|benchmark_price| benchmark_price != strategy_price)
        {
            return Err(SimulationContractError::InvalidLedger);
        }
    }
    Ok(())
}

fn validate_zero_share_claims(
    records: &[LedgerPhaseRecord],
    opening_prices: &BTreeMap<(TradingDate, String), Price4>,
    cost_profile: CostProfile,
) -> Result<(), SimulationContractError> {
    for buy in records.iter().filter(|record| {
        record.phase == LedgerPhase::Buy
            && record.events.is_empty()
            && record.unfilled_reason == UnfilledReason::ZeroShareQuantity
    }) {
        let key = (buy.session_date, TREND_INSTRUMENT.to_owned());
        let Some(price) = opening_prices.get(&key) else {
            // Cash-only records carry no quote. The q=0 claim stays
            // unverifiable until either ledger contains opening-price evidence.
            continue;
        };
        let one_share_notional = price.units()?;
        let one_share_cost = expected_fee_units(one_share_notional, cost_profile)?;
        let one_share_cash = one_share_notional
            .checked_add(one_share_cost)
            .ok_or(SimulationContractError::ArithmeticOverflow)?;
        if one_share_cash <= buy.cash.units()? {
            return Err(SimulationContractError::InvalidLedger);
        }
    }
    Ok(())
}

fn validate_signal_targets_against_schedule(
    signals: &[SignalSnapshot],
    schedule: &SimulationSchedule,
) -> Result<(), SimulationContractError> {
    if signals.len() != schedule.signal_execution.len()
        || signals
            .iter()
            .zip(&schedule.signal_execution)
            .any(|(signal, pair)| signal.signal_date != pair.signal_date)
    {
        return Err(SimulationContractError::InvalidSchedule);
    }
    Ok(())
}

fn validate_strategy_target_path(
    signals: &[SignalSnapshot],
    ledger: &StrategyLedger,
    schedule: &SimulationSchedule,
) -> Result<(), SimulationContractError> {
    let find = |date: TradingDate, phase: LedgerPhase| {
        ledger
            .records
            .iter()
            .find(|record| record.session_date == date && record.phase == phase)
            .ok_or(SimulationContractError::InvalidLedger)
    };
    let mut previous_target = 0u32;
    for (signal, date) in signals.iter().zip(&schedule.evaluation_sessions) {
        let target = signal
            .targets
            .iter()
            .find(|item| item.instrument_id == TREND_INSTRUMENT)
            .map(|item| item.weight_ppm)
            .ok_or(SimulationContractError::InvalidValue)?;
        let sell = find(*date, LedgerPhase::Sell)?;
        let buy = find(*date, LedgerPhase::Buy)?;
        match (previous_target, target) {
            (0, 0) | (1_000_000, 1_000_000) => {
                if !sell.events.is_empty()
                    || sell.unfilled_reason != UnfilledReason::NoOrderRequired
                    || !buy.events.is_empty()
                    || buy.unfilled_reason != UnfilledReason::NoOrderRequired
                {
                    return Err(SimulationContractError::InvalidLedger);
                }
            }
            (0, 1_000_000) => {
                if !sell.events.is_empty()
                    || sell.unfilled_reason != UnfilledReason::NoOrderRequired
                    || (buy.events.is_empty()
                        && buy.unfilled_reason != UnfilledReason::ZeroShareQuantity)
                    || (!buy.events.is_empty() && buy.events[0].side != TradeSide::Buy)
                {
                    return Err(SimulationContractError::InvalidLedger);
                }
            }
            (1_000_000, 0) => {
                let was_held = find(*date, LedgerPhase::PreOpenMark)?.positions.len() == 1;
                if (was_held && (sell.events.len() != 1 || sell.events[0].side != TradeSide::Sell))
                    || (!was_held
                        && (!sell.events.is_empty()
                            || sell.unfilled_reason != UnfilledReason::NoOrderRequired))
                    || !buy.events.is_empty()
                    || buy.unfilled_reason != UnfilledReason::NoOrderRequired
                {
                    return Err(SimulationContractError::InvalidLedger);
                }
            }
            _ => return Err(SimulationContractError::InvalidValue),
        }
        previous_target = target;
    }
    Ok(())
}

fn validate_benchmark_path(
    ledger: &BenchmarkLedger,
    schedule: &SimulationSchedule,
) -> Result<(), SimulationContractError> {
    for (index, date) in schedule.evaluation_sessions.iter().enumerate() {
        let sell = ledger
            .records
            .iter()
            .find(|record| record.session_date == *date && record.phase == LedgerPhase::Sell)
            .ok_or(SimulationContractError::InvalidLedger)?;
        let buy = ledger
            .records
            .iter()
            .find(|record| record.session_date == *date && record.phase == LedgerPhase::Buy)
            .ok_or(SimulationContractError::InvalidLedger)?;
        if !sell.events.is_empty()
            || sell.unfilled_reason != UnfilledReason::NoOrderRequired
            || (index == 0
                && buy.events.is_empty()
                && buy.unfilled_reason != UnfilledReason::ZeroShareQuantity)
            || (index > 0
                && (!buy.events.is_empty()
                    || buy.unfilled_reason != UnfilledReason::NoOrderRequired))
        {
            return Err(SimulationContractError::InvalidLedger);
        }
    }
    Ok(())
}

// Private deserialization DTOs. The checked public types above are only
// constructed after these closed DTOs have passed their validators.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultWire {
    contract: ContractWire,
    input: InputWire,
    engine: EngineWire,
    strategy: StrategyWire,
    schedule: ScheduleWire,
    conventions: ConventionsWire,
    signals: Vec<SignalWire>,
    strategy_ledger: StrategyLedgerWire,
    benchmark_ledger: BenchmarkLedgerWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractWire {
    contract_version: String,
    contract_document_sha256: String,
    result_schema_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputWire {
    pins: PinsWire,
    bars_sha256: String,
    approved_start_date: String,
    approved_end_date: String,
    session_count: u32,
    bar_count: u32,
    instruments: Vec<String>,
    labels: LabelsWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinsWire {
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelsWire {
    data_scope: EvidenceScope,
    return_basis: ReturnBasis,
    vendor_snapshot: bool,
    strict_pit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineWire {
    git_commit: String,
    binary_sha256: String,
    toolchain: String,
    target: String,
    profile: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrategyWire {
    strategy_id: String,
    version: String,
    config: TrendConfigWire,
    strategy_snapshot_sha256: String,
    factor_definition: FactorDefinitionWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrendConfigWire {
    benchmark_instrument: String,
    fast_ma: u16,
    slow_ma: u16,
    target_rule: TrendTargetRule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactorDefinitionWire {
    version: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleWire {
    warmup_sessions: Vec<String>,
    signal_execution: Vec<SignalExecutionWire>,
    evaluation_sessions: Vec<String>,
    phase_order: Vec<LedgerPhase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalExecutionWire {
    signal_date: String,
    execution_date: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalWire {
    signal_date: String,
    factor_snapshot_sha256: String,
    target_snapshot_sha256: String,
    targets: Vec<InstrumentTargetWire>,
    cash_weight_ppm: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentTargetWire {
    instrument_id: String,
    weight_ppm: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConventionsWire {
    precision: PrecisionWire,
    portfolio: PortfolioWire,
    pricing: PricingWire,
    cost: CostWire,
    benchmark: BenchmarkWire,
    turnover: TurnoverWire,
    metrics: MetricsConventionWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrecisionWire {
    price_scale: u8,
    money_scale: u8,
    ratio_scale: u8,
    quantity_scale: u8,
    target_weight_scale_ppm: u32,
    ratio_rounding: RoundingRule,
    negative_zero: NegativeZeroRule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioWire {
    initial_capital: String,
    currency: Currency,
    quantity_unit: QuantityUnit,
    share_sizing: ShareSizingRule,
    target_transition: TargetTransitionRule,
    residual_cash: ResidualCashRule,
    borrowing: bool,
    short_selling: bool,
    cash_interest: CashInterestRule,
    additional_cash_flows: bool,
    dividends: DividendRule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PricingWire {
    signal_price: SignalPriceRule,
    execution_price: ExecutionPriceRule,
    valuation_price: ValuationPriceRule,
    corporate_actions: CorporateActionRule,
    missing_data: MissingDataRule,
    volume: VolumeRule,
    market_impact: MarketImpactRule,
    partial_fills: bool,
    trade_order: TradeOrderRule,
    ending: EndingRule,
    warmup_returns_included: bool,
    market_timezone: String,
    timestamp_fields_included: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CostWire {
    profile: CostProfile,
    per_side_rate_numerator: u32,
    per_side_rate_denominator: u32,
    minimum_cost: String,
    separate_tax: String,
    price_slippage: String,
    rounding: CostRoundingRule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkWire {
    instrument_id: String,
    policy: BenchmarkRule,
    initial_purchase: InitialPurchaseRule,
    rebalancing: bool,
    additional_cash_purchases: bool,
    forced_final_liquidation: bool,
    same_cost_profile: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnoverWire {
    formula: TurnoverFormula,
    include_initial_trade: bool,
    fees_and_deposits_included: bool,
    annualized: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricsConventionWire {
    minimum_evaluation_sessions: u32,
    allowed_metrics: Vec<MetricName>,
    cagr_included: bool,
    annualized_volatility_included: bool,
    sharpe_included: bool,
    sortino_included: bool,
    win_rate_included: bool,
    trade_return_included: bool,
    monthly_metrics_included: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrategyLedgerWire {
    initial_cash: String,
    records: Vec<LedgerRecordWire>,
    metrics: LedgerMetricsWire,
    benchmark_return_difference: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkLedgerWire {
    initial_cash: String,
    records: Vec<LedgerRecordWire>,
    metrics: LedgerMetricsWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerRecordWire {
    session_date: String,
    phase: LedgerPhase,
    events: Vec<TradeEventWire>,
    unfilled_reason: String,
    cash: String,
    positions: Vec<PositionWire>,
    nav: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeEventWire {
    instrument_id: String,
    side: TradeSide,
    quantity: u64,
    price: String,
    notional: String,
    cost: String,
    cash_after: String,
    position_after: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionWire {
    instrument_id: String,
    quantity: u64,
    price: String,
    market_value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerMetricsWire {
    daily_returns: Vec<DailyReturnWire>,
    period_return: String,
    ending_nav: String,
    ending_cash: String,
    ending_holdings: Vec<HoldingWire>,
    cumulative_costs: String,
    buy_notional: String,
    sell_notional: String,
    maximum_drawdown: String,
    period_turnover: String,
    fill_count: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyReturnWire {
    session_date: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldingWire {
    instrument_id: String,
    quantity: u64,
}

impl TryFrom<ResultWire> for SimulationResult {
    type Error = SimulationContractError;

    fn try_from(wire: ResultWire) -> Result<Self, Self::Error> {
        let contract = ContractIdentity {
            contract_version: wire.contract.contract_version,
            contract_document_sha256: Sha256Digest::parse(&wire.contract.contract_document_sha256)?,
            result_schema_version: wire.contract.result_schema_version,
        };
        let input = SimulationInputIdentity {
            pins: ApprovedInputPins {
                candidate_content_sha256: Sha256Digest::parse(
                    &wire.input.pins.candidate_content_sha256,
                )?,
                artifact_manifest_sha256: Sha256Digest::parse(
                    &wire.input.pins.artifact_manifest_sha256,
                )?,
                stage5_manifest_sha256: Sha256Digest::parse(
                    &wire.input.pins.stage5_manifest_sha256,
                )?,
                action_manifest_sha256: Sha256Digest::parse(
                    &wire.input.pins.action_manifest_sha256,
                )?,
                approval_registry_sha256: Sha256Digest::parse(
                    &wire.input.pins.approval_registry_sha256,
                )?,
            },
            bars_sha256: Sha256Digest::parse(&wire.input.bars_sha256)?,
            approved_start_date: TradingDate::parse(&wire.input.approved_start_date)
                .map_err(|_| SimulationContractError::InvalidValue)?,
            approved_end_date: TradingDate::parse(&wire.input.approved_end_date)
                .map_err(|_| SimulationContractError::InvalidValue)?,
            session_count: wire.input.session_count,
            bar_count: wire.input.bar_count,
            instruments: wire.input.instruments,
            labels: wire.input.labels.into(),
        };
        let engine = EngineIdentity::new(
            &wire.engine.git_commit,
            &wire.engine.binary_sha256,
            &wire.engine.toolchain,
            &wire.engine.target,
            &wire.engine.profile,
        )?;
        let strategy = StrategyIdentity {
            strategy_id: wire.strategy.strategy_id,
            version: wire.strategy.version,
            config: TrendFollowingConfig {
                benchmark_instrument: wire.strategy.config.benchmark_instrument,
                fast_ma: wire.strategy.config.fast_ma,
                slow_ma: wire.strategy.config.slow_ma,
                target_rule: wire.strategy.config.target_rule,
            },
            strategy_snapshot_sha256: Sha256Digest::parse(&wire.strategy.strategy_snapshot_sha256)?,
            factor_definition: FactorDefinitionIdentity {
                version: wire.strategy.factor_definition.version,
                sha256: Sha256Digest::parse(&wire.strategy.factor_definition.sha256)?,
            },
        };
        let schedule = SimulationSchedule {
            warmup_sessions: parse_dates(wire.schedule.warmup_sessions)?,
            signal_execution: wire
                .schedule
                .signal_execution
                .into_iter()
                .map(|pair| {
                    Ok(SignalExecutionPair {
                        signal_date: TradingDate::parse(&pair.signal_date)
                            .map_err(|_| SimulationContractError::InvalidSchedule)?,
                        execution_date: TradingDate::parse(&pair.execution_date)
                            .map_err(|_| SimulationContractError::InvalidSchedule)?,
                    })
                })
                .collect::<Result<_, SimulationContractError>>()?,
            evaluation_sessions: parse_dates(wire.schedule.evaluation_sessions)?,
            phase_order: wire.schedule.phase_order,
        };
        let conventions = conventions_from_wire(wire.conventions)?;
        let signals = wire
            .signals
            .into_iter()
            .map(|signal| {
                SignalSnapshot::new(
                    &signal.signal_date,
                    &signal.factor_snapshot_sha256,
                    &signal.target_snapshot_sha256,
                    signal
                        .targets
                        .into_iter()
                        .map(|target| (target.instrument_id, target.weight_ppm))
                        .collect(),
                    signal.cash_weight_ppm,
                )
            })
            .collect::<Result<_, _>>()?;
        let strategy_ledger = StrategyLedger::new(
            Money4::parse(&wire.strategy_ledger.initial_cash)?,
            parse_records(wire.strategy_ledger.records)?,
            metrics_from_wire(wire.strategy_ledger.metrics)?,
            Ratio12::parse(&wire.strategy_ledger.benchmark_return_difference)?,
        )?;
        let benchmark_ledger = BenchmarkLedger::new(
            Money4::parse(&wire.benchmark_ledger.initial_cash)?,
            parse_records(wire.benchmark_ledger.records)?,
            metrics_from_wire(wire.benchmark_ledger.metrics)?,
        )?;
        Ok(Self {
            contract,
            input,
            engine,
            strategy,
            schedule,
            conventions,
            signals,
            strategy_ledger,
            benchmark_ledger,
        })
    }
}

impl From<LabelsWire> for EvidenceLabels {
    fn from(wire: LabelsWire) -> Self {
        Self {
            data_scope: wire.data_scope,
            return_basis: wire.return_basis,
            vendor_snapshot: wire.vendor_snapshot,
            strict_pit: wire.strict_pit,
        }
    }
}

fn parse_dates(values: Vec<String>) -> Result<Vec<TradingDate>, SimulationContractError> {
    values
        .iter()
        .map(|date| TradingDate::parse(date).map_err(|_| SimulationContractError::InvalidSchedule))
        .collect()
}

fn conventions_from_wire(
    wire: ConventionsWire,
) -> Result<SimulationConventions, SimulationContractError> {
    Ok(SimulationConventions {
        precision: PrecisionConvention {
            price_scale: wire.precision.price_scale,
            money_scale: wire.precision.money_scale,
            ratio_scale: wire.precision.ratio_scale,
            quantity_scale: wire.precision.quantity_scale,
            target_weight_scale_ppm: wire.precision.target_weight_scale_ppm,
            ratio_rounding: wire.precision.ratio_rounding,
            negative_zero: wire.precision.negative_zero,
        },
        portfolio: PortfolioConvention {
            initial_capital: Money4::parse(&wire.portfolio.initial_capital)?,
            currency: wire.portfolio.currency,
            quantity_unit: wire.portfolio.quantity_unit,
            share_sizing: wire.portfolio.share_sizing,
            target_transition: wire.portfolio.target_transition,
            residual_cash: wire.portfolio.residual_cash,
            borrowing: wire.portfolio.borrowing,
            short_selling: wire.portfolio.short_selling,
            cash_interest: wire.portfolio.cash_interest,
            additional_cash_flows: wire.portfolio.additional_cash_flows,
            dividends: wire.portfolio.dividends,
        },
        pricing: PricingConvention {
            signal_price: wire.pricing.signal_price,
            execution_price: wire.pricing.execution_price,
            valuation_price: wire.pricing.valuation_price,
            corporate_actions: wire.pricing.corporate_actions,
            missing_data: wire.pricing.missing_data,
            volume: wire.pricing.volume,
            market_impact: wire.pricing.market_impact,
            partial_fills: wire.pricing.partial_fills,
            trade_order: wire.pricing.trade_order,
            ending: wire.pricing.ending,
            warmup_returns_included: wire.pricing.warmup_returns_included,
            market_timezone: wire.pricing.market_timezone,
            timestamp_fields_included: wire.pricing.timestamp_fields_included,
        },
        cost: CostConvention {
            profile: wire.cost.profile,
            per_side_rate_numerator: wire.cost.per_side_rate_numerator,
            per_side_rate_denominator: wire.cost.per_side_rate_denominator,
            minimum_cost: Money4::parse(&wire.cost.minimum_cost)?,
            separate_tax: Money4::parse(&wire.cost.separate_tax)?,
            price_slippage: Money4::parse(&wire.cost.price_slippage)?,
            rounding: wire.cost.rounding,
        },
        benchmark: BenchmarkConvention {
            instrument_id: wire.benchmark.instrument_id,
            policy: wire.benchmark.policy,
            initial_purchase: wire.benchmark.initial_purchase,
            rebalancing: wire.benchmark.rebalancing,
            additional_cash_purchases: wire.benchmark.additional_cash_purchases,
            forced_final_liquidation: wire.benchmark.forced_final_liquidation,
            same_cost_profile: wire.benchmark.same_cost_profile,
        },
        turnover: TurnoverConvention {
            formula: wire.turnover.formula,
            include_initial_trade: wire.turnover.include_initial_trade,
            fees_and_deposits_included: wire.turnover.fees_and_deposits_included,
            annualized: wire.turnover.annualized,
        },
        metrics: MetricsConvention {
            minimum_evaluation_sessions: wire.metrics.minimum_evaluation_sessions,
            allowed_metrics: wire.metrics.allowed_metrics,
            cagr_included: wire.metrics.cagr_included,
            annualized_volatility_included: wire.metrics.annualized_volatility_included,
            sharpe_included: wire.metrics.sharpe_included,
            sortino_included: wire.metrics.sortino_included,
            win_rate_included: wire.metrics.win_rate_included,
            trade_return_included: wire.metrics.trade_return_included,
            monthly_metrics_included: wire.metrics.monthly_metrics_included,
        },
    })
}

fn parse_records(
    values: Vec<LedgerRecordWire>,
) -> Result<Vec<LedgerPhaseRecord>, SimulationContractError> {
    values
        .into_iter()
        .map(|record| {
            let events = record
                .events
                .into_iter()
                .map(|event| {
                    let price = Price4::parse(&event.price)?;
                    let notional = Money4::parse(&event.notional)?;
                    let result = TradeEvent::new(
                        &event.instrument_id,
                        event.side,
                        event.quantity,
                        price,
                        Money4::parse(&event.cost)?,
                        Money4::parse(&event.cash_after)?,
                        event.position_after,
                    )?;
                    if result.notional != notional {
                        return Err(SimulationContractError::InvalidLedger);
                    }
                    Ok(result)
                })
                .collect::<Result<_, _>>()?;
            let positions = record
                .positions
                .into_iter()
                .map(|position| {
                    let result = PositionMark::new(
                        &position.instrument_id,
                        position.quantity,
                        Price4::parse(&position.price)?,
                    )?;
                    if result.market_value.as_str() != position.market_value {
                        return Err(SimulationContractError::InvalidLedger);
                    }
                    Ok(result)
                })
                .collect::<Result<_, _>>()?;
            LedgerPhaseRecord::new(
                &record.session_date,
                record.phase,
                events,
                parse_unfilled_reason(&record.unfilled_reason)?,
                Money4::parse(&record.cash)?,
                positions,
                Money4::parse(&record.nav)?,
            )
        })
        .collect()
}

fn parse_unfilled_reason(value: &str) -> Result<UnfilledReason, SimulationContractError> {
    match value {
        "NOT_APPLICABLE" => Ok(UnfilledReason::NotApplicable),
        "NO_ORDER_REQUIRED" => Ok(UnfilledReason::NoOrderRequired),
        "ZERO_SHARE_QUANTITY" => Ok(UnfilledReason::ZeroShareQuantity),
        "FILLED" => Ok(UnfilledReason::Filled),
        _ => Err(SimulationContractError::InvalidLedger),
    }
}

fn metrics_from_wire(wire: LedgerMetricsWire) -> Result<LedgerMetrics, SimulationContractError> {
    LedgerMetrics::new(
        wire.daily_returns
            .into_iter()
            .map(|daily| Ok((daily.session_date, Ratio12::parse(&daily.value)?)))
            .collect::<Result<_, SimulationContractError>>()?,
        Ratio12::parse(&wire.period_return)?,
        Money4::parse(&wire.ending_nav)?,
        Money4::parse(&wire.ending_cash)?,
        wire.ending_holdings
            .into_iter()
            .map(|holding| (holding.instrument_id, holding.quantity))
            .collect(),
        Money4::parse(&wire.cumulative_costs)?,
        Money4::parse(&wire.buy_notional)?,
        Money4::parse(&wire.sell_notional)?,
        Ratio12::parse(&wire.maximum_drawdown)?,
        Ratio12::parse(&wire.period_turnover)?,
        wire.fill_count,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn test_dates(offset: i64) -> (Vec<String>, Vec<String>) {
        let first = TradingDate::parse("2020-01-01")
            .unwrap()
            .checked_add_days(offset)
            .unwrap();
        let mut dates = Vec::with_capacity(202);
        let mut candidate = first;
        while dates.len() < 202 {
            if !candidate.is_weekend() {
                dates.push(candidate.to_iso());
            }
            candidate = candidate.checked_add_days(1).unwrap();
        }
        (dates[..200].to_vec(), dates[200..].to_vec())
    }

    fn test_ledger(
        schedule: &SimulationSchedule,
        capital: Money4,
        benchmark: bool,
    ) -> (Vec<LedgerPhaseRecord>, LedgerMetrics) {
        let mut records = vec![
            LedgerPhaseRecord::new(
                &schedule.evaluation_sessions[0].to_string(),
                LedgerPhase::InitialCash,
                Vec::new(),
                UnfilledReason::NotApplicable,
                capital.clone(),
                Vec::new(),
                capital.clone(),
            )
            .unwrap(),
        ];
        for (index, date) in schedule.evaluation_sessions.iter().enumerate() {
            for phase in [
                LedgerPhase::PreOpenMark,
                LedgerPhase::Sell,
                LedgerPhase::Buy,
                LedgerPhase::CloseMark,
            ] {
                let reason = match (phase, index, benchmark) {
                    (LedgerPhase::PreOpenMark | LedgerPhase::CloseMark, _, _) => {
                        UnfilledReason::NotApplicable
                    }
                    (LedgerPhase::Buy, 0, true) => UnfilledReason::ZeroShareQuantity,
                    _ => UnfilledReason::NoOrderRequired,
                };
                records.push(
                    LedgerPhaseRecord::new(
                        &date.to_string(),
                        phase,
                        Vec::new(),
                        reason,
                        capital.clone(),
                        Vec::new(),
                        capital.clone(),
                    )
                    .unwrap(),
                );
            }
            if index == 0 {
                records.push(
                    LedgerPhaseRecord::new(
                        &date.to_string(),
                        LedgerPhase::SignalForNextSession,
                        Vec::new(),
                        UnfilledReason::NotApplicable,
                        capital.clone(),
                        Vec::new(),
                        capital.clone(),
                    )
                    .unwrap(),
                );
            }
        }
        let metrics = metrics_for(&capital, &records);
        (records, metrics)
    }

    fn metrics_for(initial_cash: &Money4, records: &[LedgerPhaseRecord]) -> LedgerMetrics {
        let derived = derive_performance_metrics(initial_cash, records).unwrap();
        let closing = records
            .iter()
            .rev()
            .find(|record| record.phase == LedgerPhase::CloseMark)
            .unwrap();
        let mut costs = 0i128;
        let mut buys = 0i128;
        let mut sells = 0i128;
        let mut fills = 0u64;
        for event in records.iter().flat_map(|record| &record.events) {
            costs += event.cost.units().unwrap();
            match event.side {
                TradeSide::Buy => buys += event.notional.units().unwrap(),
                TradeSide::Sell => sells += event.notional.units().unwrap(),
            }
            fills += 1;
        }
        LedgerMetrics::new(
            derived
                .daily_returns()
                .iter()
                .map(|(date, value)| (date.to_iso(), value.clone()))
                .collect(),
            derived.period_return().clone(),
            closing.nav.clone(),
            closing.cash.clone(),
            closing
                .positions
                .iter()
                .map(|position| (position.instrument_id.clone(), position.quantity))
                .collect(),
            Money4::from_units(costs).unwrap(),
            Money4::from_units(buys).unwrap(),
            Money4::from_units(sells).unwrap(),
            derived.maximum_drawdown().clone(),
            derived.period_turnover().clone(),
            fills,
        )
        .unwrap()
    }

    fn valid_result(
        offset: i64,
        engine_commit_byte: char,
        signal_hash_byte: char,
    ) -> SimulationResult {
        valid_result_with_capital(
            offset,
            engine_commit_byte,
            signal_hash_byte,
            Money4::synthetic_test_capital(),
            true,
        )
    }

    fn valid_production_result() -> SimulationResult {
        valid_result_with_capital(
            0,
            '1',
            'e',
            Money4::parse(PRODUCTION_INITIAL_CAPITAL).unwrap(),
            false,
        )
    }

    fn valid_result_with_capital(
        offset: i64,
        engine_commit_byte: char,
        signal_hash_byte: char,
        capital: Money4,
        synthetic: bool,
    ) -> SimulationResult {
        let (warmup, evaluation) = test_dates(offset);
        let schedule = SimulationSchedule::new(warmup, evaluation).unwrap();
        let targets = approved_instruments()
            .into_iter()
            .map(|instrument| (instrument, 0))
            .collect::<Vec<_>>();
        let signals = schedule
            .signal_execution
            .iter()
            .map(|pair| {
                SignalSnapshot::new(
                    &pair.signal_date.to_string(),
                    &hash('a'),
                    &hash(signal_hash_byte),
                    targets.clone(),
                    1_000_000,
                )
                .unwrap()
            })
            .collect();
        let (strategy_records, strategy_metrics) = test_ledger(&schedule, capital.clone(), false);
        let (benchmark_records, benchmark_metrics) = test_ledger(&schedule, capital.clone(), true);
        let difference = derive_benchmark_return_difference(
            &capital,
            &strategy_metrics.ending_nav,
            &benchmark_metrics.ending_nav,
        )
        .unwrap();
        let profile = CostProfile::Illustrative10BpPerSideV1;
        let conventions = if synthetic {
            SimulationConventions::synthetic_test(profile).unwrap()
        } else {
            SimulationConventions::approved(profile).unwrap()
        };
        let result = SimulationResult {
            contract: ContractIdentity::approved().unwrap(),
            input: SimulationInputIdentity::approved().unwrap(),
            engine: EngineIdentity::new(
                &engine_commit_byte.to_string().repeat(40),
                &hash('b'),
                "rustc-1.97.1",
                "x86_64-unknown-linux-gnu",
                "test",
            )
            .unwrap(),
            strategy: StrategyIdentity::trend_following_v1(&hash('c'), "price-only-v1", &hash('d'))
                .unwrap(),
            schedule,
            conventions,
            signals,
            strategy_ledger: StrategyLedger {
                initial_cash: capital.clone(),
                records: strategy_records,
                metrics: strategy_metrics,
                benchmark_return_difference: difference,
            },
            benchmark_ledger: BenchmarkLedger {
                initial_cash: capital,
                records: benchmark_records,
                metrics: benchmark_metrics,
            },
        };
        result.validate(synthetic).unwrap();
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn append_test_record(
        records: &mut Vec<LedgerPhaseRecord>,
        date: &TradingDate,
        phase: LedgerPhase,
        events: Vec<TradeEvent>,
        reason: UnfilledReason,
        cash: &str,
        positions: Vec<PositionMark>,
        nav: &str,
    ) {
        records.push(
            LedgerPhaseRecord::new(
                &date.to_string(),
                phase,
                events,
                reason,
                Money4::parse(cash).unwrap(),
                positions,
                Money4::parse(nav).unwrap(),
            )
            .unwrap(),
        );
    }

    fn valid_trading_result(first_close_price: &str) -> SimulationResult {
        let (warmup, evaluation) = test_dates(0);
        let schedule = SimulationSchedule::new(warmup, evaluation).unwrap();
        let capital = Money4::synthetic_test_capital();
        let first_date = schedule.evaluation_sessions[0];
        let second_date = schedule.evaluation_sessions[1];
        let mut signals = Vec::new();
        for (index, pair) in schedule.signal_execution.iter().enumerate() {
            let targets = approved_instruments()
                .into_iter()
                .map(|instrument| {
                    let weight = if index == 0 && instrument == TREND_INSTRUMENT {
                        1_000_000
                    } else {
                        0
                    };
                    (instrument, weight)
                })
                .collect();
            signals.push(
                SignalSnapshot::new(
                    &pair.signal_date.to_string(),
                    &hash('a'),
                    &hash('e'),
                    targets,
                    if index == 0 { 0 } else { 1_000_000 },
                )
                .unwrap(),
            );
        }

        let buy_price = Price4::parse("100.0000").unwrap();
        let close_price = Price4::parse(first_close_price).unwrap();
        let next_price = Price4::parse("120.0000").unwrap();
        let no_position = Vec::new();
        let held_at_buy = vec![PositionMark::new(TREND_INSTRUMENT, 10, buy_price.clone()).unwrap()];
        let held_at_close =
            vec![PositionMark::new(TREND_INSTRUMENT, 10, close_price.clone()).unwrap()];
        let held_next_day =
            vec![PositionMark::new(TREND_INSTRUMENT, 10, next_price.clone()).unwrap()];
        let first_close_nav =
            Money4::from_units(close_price.units().unwrap().checked_mul(10).unwrap()).unwrap();
        let first_close_nav_string = first_close_nav.as_str();
        let mut strategy_records = vec![
            LedgerPhaseRecord::new(
                &first_date.to_string(),
                LedgerPhase::InitialCash,
                Vec::new(),
                UnfilledReason::NotApplicable,
                capital.clone(),
                Vec::new(),
                capital.clone(),
            )
            .unwrap(),
        ];
        append_test_record(
            &mut strategy_records,
            &first_date,
            LedgerPhase::PreOpenMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "1000.0000",
            no_position.clone(),
            "1000.0000",
        );
        append_test_record(
            &mut strategy_records,
            &first_date,
            LedgerPhase::Sell,
            Vec::new(),
            UnfilledReason::NoOrderRequired,
            "1000.0000",
            no_position.clone(),
            "1000.0000",
        );
        let strategy_buy = TradeEvent::new(
            TREND_INSTRUMENT,
            TradeSide::Buy,
            10,
            buy_price.clone(),
            Money4::zero(),
            Money4::zero(),
            10,
        )
        .unwrap();
        append_test_record(
            &mut strategy_records,
            &first_date,
            LedgerPhase::Buy,
            vec![strategy_buy],
            UnfilledReason::Filled,
            "0.0000",
            held_at_buy.clone(),
            "1000.0000",
        );
        append_test_record(
            &mut strategy_records,
            &first_date,
            LedgerPhase::CloseMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            held_at_close.clone(),
            first_close_nav_string,
        );
        append_test_record(
            &mut strategy_records,
            &first_date,
            LedgerPhase::SignalForNextSession,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            held_at_close.clone(),
            first_close_nav_string,
        );
        append_test_record(
            &mut strategy_records,
            &second_date,
            LedgerPhase::PreOpenMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            held_next_day.clone(),
            "1200.0000",
        );
        let strategy_sell = TradeEvent::new(
            TREND_INSTRUMENT,
            TradeSide::Sell,
            10,
            next_price.clone(),
            Money4::zero(),
            Money4::parse("1200.0000").unwrap(),
            0,
        )
        .unwrap();
        append_test_record(
            &mut strategy_records,
            &second_date,
            LedgerPhase::Sell,
            vec![strategy_sell],
            UnfilledReason::Filled,
            "1200.0000",
            no_position.clone(),
            "1200.0000",
        );
        append_test_record(
            &mut strategy_records,
            &second_date,
            LedgerPhase::Buy,
            Vec::new(),
            UnfilledReason::NoOrderRequired,
            "1200.0000",
            no_position.clone(),
            "1200.0000",
        );
        append_test_record(
            &mut strategy_records,
            &second_date,
            LedgerPhase::CloseMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "1200.0000",
            no_position.clone(),
            "1200.0000",
        );

        let mut benchmark_records = vec![
            LedgerPhaseRecord::new(
                &first_date.to_string(),
                LedgerPhase::InitialCash,
                Vec::new(),
                UnfilledReason::NotApplicable,
                capital.clone(),
                Vec::new(),
                capital.clone(),
            )
            .unwrap(),
        ];
        append_test_record(
            &mut benchmark_records,
            &first_date,
            LedgerPhase::PreOpenMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "1000.0000",
            no_position.clone(),
            "1000.0000",
        );
        append_test_record(
            &mut benchmark_records,
            &first_date,
            LedgerPhase::Sell,
            Vec::new(),
            UnfilledReason::NoOrderRequired,
            "1000.0000",
            no_position.clone(),
            "1000.0000",
        );
        let benchmark_buy = TradeEvent::new(
            TREND_INSTRUMENT,
            TradeSide::Buy,
            10,
            buy_price,
            Money4::zero(),
            Money4::zero(),
            10,
        )
        .unwrap();
        append_test_record(
            &mut benchmark_records,
            &first_date,
            LedgerPhase::Buy,
            vec![benchmark_buy],
            UnfilledReason::Filled,
            "0.0000",
            held_at_buy,
            "1000.0000",
        );
        append_test_record(
            &mut benchmark_records,
            &first_date,
            LedgerPhase::CloseMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            vec![PositionMark::new(TREND_INSTRUMENT, 10, close_price.clone()).unwrap()],
            first_close_nav_string,
        );
        append_test_record(
            &mut benchmark_records,
            &first_date,
            LedgerPhase::SignalForNextSession,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            vec![PositionMark::new(TREND_INSTRUMENT, 10, close_price).unwrap()],
            first_close_nav_string,
        );
        append_test_record(
            &mut benchmark_records,
            &second_date,
            LedgerPhase::PreOpenMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            held_next_day.clone(),
            "1200.0000",
        );
        append_test_record(
            &mut benchmark_records,
            &second_date,
            LedgerPhase::Sell,
            Vec::new(),
            UnfilledReason::NoOrderRequired,
            "0.0000",
            held_next_day.clone(),
            "1200.0000",
        );
        append_test_record(
            &mut benchmark_records,
            &second_date,
            LedgerPhase::Buy,
            Vec::new(),
            UnfilledReason::NoOrderRequired,
            "0.0000",
            held_next_day.clone(),
            "1200.0000",
        );
        append_test_record(
            &mut benchmark_records,
            &second_date,
            LedgerPhase::CloseMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            "0.0000",
            held_next_day,
            "1200.0000",
        );

        let strategy_metrics = metrics_for(&capital, &strategy_records);
        let benchmark_metrics = metrics_for(&capital, &benchmark_records);
        let difference = derive_benchmark_return_difference(
            &capital,
            &strategy_metrics.ending_nav,
            &benchmark_metrics.ending_nav,
        )
        .unwrap();
        let result = SimulationResult {
            contract: ContractIdentity::approved().unwrap(),
            input: SimulationInputIdentity::approved().unwrap(),
            engine: EngineIdentity::new(
                &"1".repeat(40),
                &hash('b'),
                "rustc-1.97.1",
                "x86_64-unknown-linux-gnu",
                "test",
            )
            .unwrap(),
            strategy: StrategyIdentity::trend_following_v1(&hash('c'), "price-only-v1", &hash('d'))
                .unwrap(),
            schedule,
            conventions: SimulationConventions::synthetic_test(CostProfile::ZeroCostReferenceV1)
                .unwrap(),
            signals,
            strategy_ledger: StrategyLedger {
                initial_cash: capital.clone(),
                records: strategy_records,
                metrics: strategy_metrics,
                benchmark_return_difference: difference,
            },
            benchmark_ledger: BenchmarkLedger {
                initial_cash: capital,
                records: benchmark_records,
                metrics: benchmark_metrics,
            },
        };
        result.validate(true).unwrap();
        result
    }

    fn append_units_record(
        records: &mut Vec<LedgerPhaseRecord>,
        date: TradingDate,
        phase: LedgerPhase,
        events: Vec<TradeEvent>,
        reason: UnfilledReason,
        cash_units: i128,
        positions: Vec<PositionMark>,
    ) {
        let cash = Money4::from_units(cash_units).unwrap();
        let nav_units = positions.iter().fold(cash_units, |total, position| {
            total
                .checked_add(position.market_value.units().unwrap())
                .unwrap()
        });
        let nav = Money4::from_units(nav_units).unwrap();
        append_test_record(
            records,
            &date,
            phase,
            events,
            reason,
            cash.as_str(),
            positions,
            nav.as_str(),
        );
    }

    fn production_test_ledger(
        initial_cash: &Money4,
        quantity: u64,
        profile: CostProfile,
        liquidate_second_session: bool,
        first_date: TradingDate,
        second_date: TradingDate,
    ) -> (Vec<LedgerPhaseRecord>, LedgerMetrics) {
        let initial_cash_units = initial_cash.units().unwrap();
        let buy_price = Price4::parse("100.0000").unwrap();
        let first_close_price = Price4::parse("110.0000").unwrap();
        let second_open_price = Price4::parse("120.0000").unwrap();
        let second_close_price = Price4::parse("130.0000").unwrap();
        let buy_notional = buy_price
            .units()
            .unwrap()
            .checked_mul(i128::from(quantity))
            .unwrap();
        let buy_fee = expected_fee_units(buy_notional, profile).unwrap();
        let after_buy_cash = initial_cash_units
            .checked_sub(buy_notional)
            .unwrap()
            .checked_sub(buy_fee)
            .unwrap();
        let buy_event = TradeEvent::new(
            TREND_INSTRUMENT,
            TradeSide::Buy,
            quantity,
            buy_price.clone(),
            Money4::from_units(buy_fee).unwrap(),
            Money4::from_units(after_buy_cash).unwrap(),
            quantity,
        )
        .unwrap();
        let held_at_open = vec![PositionMark::new(TREND_INSTRUMENT, quantity, buy_price).unwrap()];
        let held_at_close =
            vec![PositionMark::new(TREND_INSTRUMENT, quantity, first_close_price.clone()).unwrap()];
        let held_at_second_open =
            vec![PositionMark::new(TREND_INSTRUMENT, quantity, second_open_price.clone()).unwrap()];
        let mut records = vec![
            LedgerPhaseRecord::new(
                &first_date.to_string(),
                LedgerPhase::InitialCash,
                Vec::new(),
                UnfilledReason::NotApplicable,
                initial_cash.clone(),
                Vec::new(),
                initial_cash.clone(),
            )
            .unwrap(),
        ];
        for (phase, reason, events, cash, positions) in [
            (
                LedgerPhase::PreOpenMark,
                UnfilledReason::NotApplicable,
                Vec::new(),
                initial_cash_units,
                Vec::new(),
            ),
            (
                LedgerPhase::Sell,
                UnfilledReason::NoOrderRequired,
                Vec::new(),
                initial_cash_units,
                Vec::new(),
            ),
            (
                LedgerPhase::Buy,
                UnfilledReason::Filled,
                vec![buy_event],
                after_buy_cash,
                held_at_open,
            ),
            (
                LedgerPhase::CloseMark,
                UnfilledReason::NotApplicable,
                Vec::new(),
                after_buy_cash,
                held_at_close.clone(),
            ),
            (
                LedgerPhase::SignalForNextSession,
                UnfilledReason::NotApplicable,
                Vec::new(),
                after_buy_cash,
                held_at_close,
            ),
        ] {
            append_units_record(
                &mut records,
                first_date,
                phase,
                events,
                reason,
                cash,
                positions,
            );
        }

        append_units_record(
            &mut records,
            second_date,
            LedgerPhase::PreOpenMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            after_buy_cash,
            held_at_second_open.clone(),
        );
        let (after_sell_cash, sell_events, sell_reason, sell_positions) =
            if liquidate_second_session {
                let sell_notional = second_open_price
                    .units()
                    .unwrap()
                    .checked_mul(i128::from(quantity))
                    .unwrap();
                let sell_fee = expected_fee_units(sell_notional, profile).unwrap();
                let cash_after_sell = after_buy_cash
                    .checked_add(sell_notional)
                    .unwrap()
                    .checked_sub(sell_fee)
                    .unwrap();
                let event = TradeEvent::new(
                    TREND_INSTRUMENT,
                    TradeSide::Sell,
                    quantity,
                    second_open_price,
                    Money4::from_units(sell_fee).unwrap(),
                    Money4::from_units(cash_after_sell).unwrap(),
                    0,
                )
                .unwrap();
                (
                    cash_after_sell,
                    vec![event],
                    UnfilledReason::Filled,
                    Vec::new(),
                )
            } else {
                (
                    after_buy_cash,
                    Vec::new(),
                    UnfilledReason::NoOrderRequired,
                    held_at_second_open.clone(),
                )
            };
        append_units_record(
            &mut records,
            second_date,
            LedgerPhase::Sell,
            sell_events,
            sell_reason,
            after_sell_cash,
            sell_positions.clone(),
        );
        append_units_record(
            &mut records,
            second_date,
            LedgerPhase::Buy,
            Vec::new(),
            UnfilledReason::NoOrderRequired,
            after_sell_cash,
            sell_positions.clone(),
        );
        let closing_positions = if liquidate_second_session {
            Vec::new()
        } else {
            vec![PositionMark::new(TREND_INSTRUMENT, quantity, second_close_price).unwrap()]
        };
        append_units_record(
            &mut records,
            second_date,
            LedgerPhase::CloseMark,
            Vec::new(),
            UnfilledReason::NotApplicable,
            after_sell_cash,
            closing_positions,
        );
        let metrics = metrics_for(initial_cash, &records);
        (records, metrics)
    }

    fn production_trading_result(
        strategy_quantity: u64,
        benchmark_quantity: u64,
    ) -> SimulationResult {
        let (warmup, evaluation) = test_dates(0);
        let schedule = SimulationSchedule::new(warmup, evaluation).unwrap();
        let capital = Money4::parse(PRODUCTION_INITIAL_CAPITAL).unwrap();
        let first_date = schedule.evaluation_sessions[0];
        let second_date = schedule.evaluation_sessions[1];
        let signals = schedule
            .signal_execution
            .iter()
            .enumerate()
            .map(|(index, pair)| {
                let targets = approved_instruments()
                    .into_iter()
                    .map(|instrument| {
                        let weight = if index == 0 && instrument == TREND_INSTRUMENT {
                            1_000_000
                        } else {
                            0
                        };
                        (instrument, weight)
                    })
                    .collect();
                SignalSnapshot::new(
                    &pair.signal_date.to_string(),
                    &hash('a'),
                    &hash('e'),
                    targets,
                    if index == 0 { 0 } else { 1_000_000 },
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let profile = CostProfile::Illustrative10BpPerSideV1;
        let (strategy_records, strategy_metrics) = production_test_ledger(
            &capital,
            strategy_quantity,
            profile,
            true,
            first_date,
            second_date,
        );
        let (benchmark_records, benchmark_metrics) = production_test_ledger(
            &capital,
            benchmark_quantity,
            profile,
            false,
            first_date,
            second_date,
        );
        let difference = derive_benchmark_return_difference(
            &capital,
            &strategy_metrics.ending_nav,
            &benchmark_metrics.ending_nav,
        )
        .unwrap();
        SimulationResult {
            contract: ContractIdentity::approved().unwrap(),
            input: SimulationInputIdentity::approved().unwrap(),
            engine: EngineIdentity::new(
                &"1".repeat(40),
                &hash('b'),
                "rustc-1.97.1",
                "x86_64-unknown-linux-gnu",
                "test",
            )
            .unwrap(),
            strategy: StrategyIdentity::trend_following_v1(&hash('c'), "price-only-v1", &hash('d'))
                .unwrap(),
            schedule,
            conventions: SimulationConventions::approved(profile).unwrap(),
            signals,
            strategy_ledger: StrategyLedger {
                initial_cash: capital.clone(),
                records: strategy_records,
                metrics: strategy_metrics,
                benchmark_return_difference: difference,
            },
            benchmark_ledger: BenchmarkLedger {
                initial_cash: capital,
                records: benchmark_records,
                metrics: benchmark_metrics,
            },
        }
    }

    fn record_mut(
        records: &mut [LedgerPhaseRecord],
        date: TradingDate,
        phase: LedgerPhase,
    ) -> &mut LedgerPhaseRecord {
        records
            .iter_mut()
            .find(|record| record.session_date == date && record.phase == phase)
            .unwrap()
    }

    fn remark_record(record: &mut LedgerPhaseRecord, price: &str) {
        let price = Price4::parse(price).unwrap();
        record.positions = record
            .positions
            .iter()
            .map(|position| {
                PositionMark::new(&position.instrument_id, position.quantity, price.clone())
                    .unwrap()
            })
            .collect();
        let nav_units =
            record
                .positions
                .iter()
                .fold(record.cash.units().unwrap(), |total, position| {
                    total
                        .checked_add(position.market_value.units().unwrap())
                        .unwrap()
                });
        record.nav = Money4::from_units(nav_units).unwrap();
    }

    fn refresh_result_metrics(result: &mut SimulationResult) {
        result.strategy_ledger.metrics = metrics_for(
            &result.strategy_ledger.initial_cash,
            &result.strategy_ledger.records,
        );
        result.benchmark_ledger.metrics = metrics_for(
            &result.benchmark_ledger.initial_cash,
            &result.benchmark_ledger.records,
        );
        result.strategy_ledger.benchmark_return_difference = derive_benchmark_return_difference(
            &result.strategy_ledger.initial_cash,
            &result.strategy_ledger.metrics.ending_nav,
            &result.benchmark_ledger.metrics.ending_nav,
        )
        .unwrap();
    }

    fn assert_result_rejected_by_all_entry_points(result: &SimulationResult) {
        assert_eq!(
            result.canonical_json(),
            Err(SimulationContractError::InvalidLedger)
        );
        let json = serde_json::to_vec(result).unwrap();
        assert_eq!(
            SimulationResult::from_json(&json),
            Err(SimulationContractError::InvalidLedger)
        );
        assert_eq!(
            SimulationResult::new(
                result.engine.clone(),
                result.strategy.clone(),
                result.schedule.clone(),
                result.conventions.cost.profile,
                result.signals.clone(),
                result.strategy_ledger.clone(),
                result.benchmark_ledger.clone(),
            ),
            Err(SimulationContractError::InvalidLedger)
        );
    }

    #[derive(Clone, Copy)]
    enum MetricTamper {
        StrategyDailyReturn,
        StrategyPeriodReturn,
        StrategyMaximumDrawdown,
        StrategyTurnover,
        BenchmarkDailyReturn,
        BenchmarkPeriodReturn,
        BenchmarkMaximumDrawdown,
        BenchmarkTurnover,
        BenchmarkDifference,
    }

    fn tamper_metric(result: &mut SimulationResult, field: MetricTamper) {
        let invalid_value = Ratio12::parse("0.000000000001").unwrap();
        match field {
            MetricTamper::StrategyDailyReturn => {
                result.strategy_ledger.metrics.daily_returns[0].value = invalid_value;
            }
            MetricTamper::StrategyPeriodReturn => {
                result.strategy_ledger.metrics.period_return = invalid_value;
            }
            MetricTamper::StrategyMaximumDrawdown => {
                result.strategy_ledger.metrics.maximum_drawdown = invalid_value;
            }
            MetricTamper::StrategyTurnover => {
                result.strategy_ledger.metrics.period_turnover = invalid_value;
            }
            MetricTamper::BenchmarkDailyReturn => {
                result.benchmark_ledger.metrics.daily_returns[0].value = invalid_value;
            }
            MetricTamper::BenchmarkPeriodReturn => {
                result.benchmark_ledger.metrics.period_return = invalid_value;
            }
            MetricTamper::BenchmarkMaximumDrawdown => {
                result.benchmark_ledger.metrics.maximum_drawdown = invalid_value;
            }
            MetricTamper::BenchmarkTurnover => {
                result.benchmark_ledger.metrics.period_turnover = invalid_value;
            }
            MetricTamper::BenchmarkDifference => {
                result.strategy_ledger.benchmark_return_difference = invalid_value;
            }
        }
    }

    #[test]
    fn decimals_reject_negative_zero_plus_exponents_leading_zero_and_bad_scales() {
        for value in [
            "-0.000000000000",
            "+1.000000000000",
            "01.000000000000",
            "1e0",
            "1.0",
            "1.0000000000000",
        ] {
            assert!(Ratio12::parse(value).is_err(), "accepted {value}");
        }
        for value in ["-0.0000", "+1.0000", "01.0000", "1e0", "1.00000", "1"] {
            assert!(Money4::parse(value).is_err(), "accepted {value}");
        }
        assert!(Price4::parse("0.0000").is_err());
    }

    #[test]
    fn canonical_bytes_and_hash_ignore_object_key_order() {
        let result = valid_result(0, '1', 'e');
        let canonical = result.canonical_json().unwrap();
        let value: Value = serde_json::from_slice(&canonical).unwrap();
        let reordered = value
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| {
                format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap(),
                    serde_json::to_string(value).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut input = format!("{{{reordered}}}").into_bytes();
        input.extend_from_slice(b" \n");
        let decoded = SimulationResult::from_json(&input).unwrap();
        assert_eq!(decoded.canonical_json().unwrap(), canonical);
        assert_eq!(
            decoded.result_content_sha256().unwrap(),
            result.result_content_sha256().unwrap()
        );
    }

    #[test]
    fn rejects_unknown_missing_null_and_duplicate_nested_fields() {
        let canonical =
            String::from_utf8(valid_result(0, '1', 'e').canonical_json().unwrap()).unwrap();
        let unknown = canonical.replacen(
            "\"profile\":\"test\"",
            "\"profile\":\"test\",\"unexpected\":1",
            1,
        );
        let root_unknown = format!(
            "{}{}",
            &canonical[..canonical.len() - 1],
            ",\"unexpected\":1}"
        );
        let missing = canonical.replacen(
            ",\"result_schema_version\":\"owner-beta-price-only-result-v1\"}",
            "}",
            1,
        );
        let null = canonical.replacen("\"strict_pit\":false", "\"strict_pit\":null", 1);
        assert!(SimulationResult::from_json(unknown.as_bytes()).is_err());
        assert!(SimulationResult::from_json(root_unknown.as_bytes()).is_err());
        assert!(SimulationResult::from_json(missing.as_bytes()).is_err());
        assert!(SimulationResult::from_json(null.as_bytes()).is_err());
        assert_eq!(
            SimulationResult::from_json(
                canonical
                    .replacen("\"signals\":", "\"signals\":[],\"signals\":", 1)
                    .as_bytes()
            ),
            Err(SimulationContractError::DuplicateJsonKey)
        );
    }

    #[test]
    fn result_construction_json_and_canonicalization_reject_each_derived_metric_tamper() {
        let fields = [
            MetricTamper::StrategyDailyReturn,
            MetricTamper::StrategyPeriodReturn,
            MetricTamper::StrategyMaximumDrawdown,
            MetricTamper::StrategyTurnover,
            MetricTamper::BenchmarkDailyReturn,
            MetricTamper::BenchmarkPeriodReturn,
            MetricTamper::BenchmarkMaximumDrawdown,
            MetricTamper::BenchmarkTurnover,
            MetricTamper::BenchmarkDifference,
        ];
        for field in fields {
            let mut tampered = valid_production_result();
            tamper_metric(&mut tampered, field);

            assert_eq!(
                tampered.canonical_json(),
                Err(SimulationContractError::InvalidLedger)
            );
            let json = serde_json::to_vec(&tampered).unwrap();
            assert_eq!(
                SimulationResult::from_json(&json),
                Err(SimulationContractError::InvalidLedger)
            );
            assert_eq!(
                SimulationResult::new(
                    tampered.engine.clone(),
                    tampered.strategy.clone(),
                    tampered.schedule.clone(),
                    tampered.conventions.cost.profile,
                    tampered.signals.clone(),
                    tampered.strategy_ledger.clone(),
                    tampered.benchmark_ledger.clone(),
                ),
                Err(SimulationContractError::InvalidLedger)
            );
        }
    }

    #[test]
    fn maximum_cost_inclusive_buy_is_checked_for_strategy_and_benchmark() {
        const MAXIMUM_QUANTITY_AT_100_KRW: u64 = 99_900;
        let valid =
            production_trading_result(MAXIMUM_QUANTITY_AT_100_KRW, MAXIMUM_QUANTITY_AT_100_KRW);
        assert!(valid.canonical_json().is_ok());

        for (strategy_quantity, benchmark_quantity) in [
            (1, MAXIMUM_QUANTITY_AT_100_KRW),
            (MAXIMUM_QUANTITY_AT_100_KRW, 1),
        ] {
            let underfilled = production_trading_result(strategy_quantity, benchmark_quantity);
            assert_result_rejected_by_all_entry_points(&underfilled);
        }
    }

    #[test]
    fn same_session_open_prices_must_match_marks_trades_and_both_ledgers() {
        const MAXIMUM_QUANTITY_AT_100_KRW: u64 = 99_900;

        // The day-two pre-open mark and turnover denominator are refreshed,
        // while the strategy's sale still uses its original raw open of 120.
        let mut changed_pre_open =
            production_trading_result(MAXIMUM_QUANTITY_AT_100_KRW, MAXIMUM_QUANTITY_AT_100_KRW);
        let second_date = changed_pre_open.schedule.evaluation_sessions[1];
        remark_record(
            record_mut(
                &mut changed_pre_open.strategy_ledger.records,
                second_date,
                LedgerPhase::PreOpenMark,
            ),
            "240.0000",
        );
        refresh_result_metrics(&mut changed_pre_open);
        assert_result_rejected_by_all_entry_points(&changed_pre_open);

        // An empty SELL/BUY phase cannot give an unchanged holding a new mark.
        let mut re_marked_empty_phase =
            production_trading_result(MAXIMUM_QUANTITY_AT_100_KRW, MAXIMUM_QUANTITY_AT_100_KRW);
        let second_date = re_marked_empty_phase.schedule.evaluation_sessions[1];
        remark_record(
            record_mut(
                &mut re_marked_empty_phase.benchmark_ledger.records,
                second_date,
                LedgerPhase::Buy,
            ),
            "240.0000",
        );
        refresh_result_metrics(&mut re_marked_empty_phase);
        assert_result_rejected_by_all_entry_points(&re_marked_empty_phase);

        // Each ledger is internally consistent at its own open; the shared
        // same-session market quote must still agree between them.
        let mut cross_ledger_open =
            production_trading_result(MAXIMUM_QUANTITY_AT_100_KRW, MAXIMUM_QUANTITY_AT_100_KRW);
        let second_date = cross_ledger_open.schedule.evaluation_sessions[1];
        for phase in [
            LedgerPhase::PreOpenMark,
            LedgerPhase::Sell,
            LedgerPhase::Buy,
        ] {
            remark_record(
                record_mut(
                    &mut cross_ledger_open.benchmark_ledger.records,
                    second_date,
                    phase,
                ),
                "240.0000",
            );
        }
        refresh_result_metrics(&mut cross_ledger_open);
        assert_result_rejected_by_all_entry_points(&cross_ledger_open);

        // Both ledgers hold the instrument on day one, so their close marks
        // must agree even though the next day's open may gap.
        let mut cross_ledger_close =
            production_trading_result(MAXIMUM_QUANTITY_AT_100_KRW, MAXIMUM_QUANTITY_AT_100_KRW);
        let first_date = cross_ledger_close.schedule.evaluation_sessions[0];
        for phase in [LedgerPhase::CloseMark, LedgerPhase::SignalForNextSession] {
            remark_record(
                record_mut(
                    &mut cross_ledger_close.benchmark_ledger.records,
                    first_date,
                    phase,
                ),
                "111.0000",
            );
        }
        refresh_result_metrics(&mut cross_ledger_close);
        assert_result_rejected_by_all_entry_points(&cross_ledger_close);
    }

    #[test]
    fn zero_share_without_any_opening_quote_remains_unverifiable_and_accepted() {
        let result = valid_result(0, '1', 'e');
        assert!(result.canonical_json().is_ok());
        assert!(
            result
                .benchmark_ledger
                .records
                .iter()
                .any(|record| record.phase == LedgerPhase::Buy
                    && record.unfilled_reason == UnfilledReason::ZeroShareQuantity)
        );
    }

    #[test]
    fn valid_content_engine_schedule_signal_and_ledger_changes_change_digest() {
        let original = valid_result(0, '1', 'e');
        let original_hash = original.result_content_sha256().unwrap();
        assert_ne!(
            original_hash,
            valid_result(0, '2', 'e').result_content_sha256().unwrap()
        );
        assert_ne!(
            original_hash,
            valid_result(1, '1', 'e').result_content_sha256().unwrap()
        );
        assert_ne!(
            original_hash,
            valid_result(0, '1', 'f').result_content_sha256().unwrap()
        );
        let mut content_changed = valid_result(0, '1', 'e');
        content_changed.strategy.strategy_snapshot_sha256 =
            Sha256Digest::parse(&hash('f')).unwrap();
        assert_ne!(
            original_hash,
            content_changed.result_content_sha256().unwrap()
        );
        let ledger_original = valid_trading_result("110.0000");
        let ledger_changed = valid_trading_result("105.0000");
        assert_ne!(
            ledger_original.result_content_sha256().unwrap(),
            ledger_changed.result_content_sha256().unwrap()
        );
        assert!(ledger_original.canonical_json().is_ok());
        assert!(ledger_changed.canonical_json().is_ok());
        let mut cost_changed = valid_result(0, '1', 'e');
        cost_changed.conventions =
            SimulationConventions::synthetic_test(CostProfile::ZeroCostReferenceV1).unwrap();
        assert_ne!(original_hash, cost_changed.result_content_sha256().unwrap());
    }
}
