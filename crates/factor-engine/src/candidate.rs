//! Deterministic cross-sectional scoring for the stock-research vertical.
//!
//! This module is intentionally independent from the ETF recommendation
//! selector. It consumes already point-in-time source rows, derives exact
//! 5/20/60-session features, and normalizes each factor against a frozen
//! eligible universe. Missing factors reduce coverage and are never
//! reweighted. Scenarios are evidence/trigger records, not forecasts with
//! invented probabilities or target prices.

use std::collections::{BTreeMap, BTreeSet};

use domain::{InstrumentId, TradingDate};
use serde::{Deserialize, Serialize};

const MIN_SECTOR_SAMPLE: usize = 8;
const MIN_AXIS_COVERAGE: f64 = 0.60;
const STRONG_AXIS_COVERAGE: f64 = 0.80;
const Z_CAP: f64 = 3.0;
const SCORE_SCALE: f64 = 15.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSession {
    pub date: TradingDate,
    pub close: f64,
    pub trading_value: f64,
    pub foreign_net_amount: Option<f64>,
    pub institution_net_amount: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateFlags {
    pub suspended: bool,
    pub administrative: bool,
    pub liquidation: bool,
    pub inactive: bool,
    pub disqualifying_audit_opinion: bool,
    pub complete_capital_impairment: bool,
    pub data_stale: bool,
    pub entitlement_active: bool,
    pub fundamental_profile_supported: bool,
}

impl CandidateFlags {
    pub fn eligible_defaults() -> Self {
        Self {
            entitlement_active: true,
            fundamental_profile_supported: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateInstrumentInput {
    pub instrument: InstrumentId,
    pub sector_code: String,
    pub is_financial: bool,
    pub sessions: Vec<CandidateSession>,
    /// Point-in-time fundamental values keyed by the scoring contract's
    /// stable metric ids. Absent values stay absent.
    pub fundamentals: BTreeMap<String, f64>,
    pub flags: CandidateFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandidateAxis {
    Flow,
    Fundamental,
    Technical,
}

impl CandidateAxis {
    pub const ALL: [Self; 3] = [Self::Flow, Self::Fundamental, Self::Technical];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::Fundamental => "fundamental",
            Self::Technical => "technical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandidateExclusion {
    Suspended,
    Administrative,
    Liquidation,
    Inactive,
    InsufficientPriceHistory,
    RequiredFlowMissing,
    DataStale,
    Illiquid,
    DisqualifyingAuditOpinion,
    CompleteCapitalImpairment,
    UnsupportedFundamentalProfile,
    EntitlementInactive,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NormalizationScope {
    Sector,
    UniverseFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceStrength {
    Strong,
    Moderate,
    Weak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateFactorValue {
    pub raw: Option<f64>,
    pub normalized: Option<f64>,
    pub weight: f64,
    pub normalization_scope: NormalizationScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAxisScore {
    /// Centered 0..100 score. It is unavailable when coverage is below 60%.
    pub score: Option<f64>,
    pub coverage: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateScenario {
    pub label: String,
    pub title: String,
    pub trigger_expression: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAnalysis {
    pub instrument: InstrumentId,
    pub sector_code: String,
    pub exclusions: Vec<CandidateExclusion>,
    pub factors: BTreeMap<String, CandidateFactorValue>,
    pub axes: BTreeMap<String, CandidateAxisScore>,
    pub composite_score: Option<f64>,
    pub evidence_strength: EvidenceStrength,
    pub scenarios: BTreeMap<String, CandidateScenario>,
}

impl CandidateAnalysis {
    pub fn is_top_five_eligible(&self) -> bool {
        self.exclusions.is_empty()
            && self.composite_score.is_some()
            && matches!(
                self.evidence_strength,
                EvidenceStrength::Strong | EvidenceStrength::Moderate
            )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateScoringConfig {
    pub version: String,
    pub flow_weight: f64,
    pub fundamental_weight: f64,
    pub technical_weight: f64,
    pub min_average_trading_value_20: f64,
    pub winsor_lower: f64,
    pub winsor_upper: f64,
}

impl Default for CandidateScoringConfig {
    fn default() -> Self {
        Self {
            version: "candidate-score-v1".to_owned(),
            flow_weight: 0.35,
            fundamental_weight: 0.30,
            technical_weight: 0.35,
            min_average_trading_value_20: 1_000_000_000.0,
            winsor_lower: 0.01,
            winsor_upper: 0.99,
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CandidateScoreError {
    #[error("candidate scoring configuration is invalid: {detail}")]
    InvalidConfig { detail: String },
    #[error("candidate input for {instrument} is invalid: {detail}")]
    InvalidInput { instrument: String, detail: String },
    #[error("candidate universe contains duplicate instrument {instrument}")]
    DuplicateInstrument { instrument: String },
}

#[derive(Clone, Copy)]
struct FactorRule {
    id: &'static str,
    axis: CandidateAxis,
    weight: f64,
    direction: f64,
}

const FLOW_RULES: [FactorRule; 9] = [
    rule("foreign_intensity_5", CandidateAxis::Flow, 0.12, 1.0),
    rule("foreign_intensity_20", CandidateAxis::Flow, 0.18, 1.0),
    rule("foreign_intensity_60", CandidateAxis::Flow, 0.10, 1.0),
    rule("institution_intensity_5", CandidateAxis::Flow, 0.10, 1.0),
    rule("institution_intensity_20", CandidateAxis::Flow, 0.15, 1.0),
    rule("institution_intensity_60", CandidateAxis::Flow, 0.10, 1.0),
    rule("flow_acceleration", CandidateAxis::Flow, 0.10, 1.0),
    rule("joint_accumulation", CandidateAxis::Flow, 0.10, 1.0),
    rule("price_flow_divergence", CandidateAxis::Flow, 0.05, 1.0),
];

const FUNDAMENTAL_RULES: [FactorRule; 6] = [
    rule("revenue_growth", CandidateAxis::Fundamental, 0.18, 1.0),
    rule("operating_margin", CandidateAxis::Fundamental, 0.18, 1.0),
    rule("roe", CandidateAxis::Fundamental, 0.20, 1.0),
    rule("debt_ratio", CandidateAxis::Fundamental, 0.14, -1.0),
    rule("cash_conversion", CandidateAxis::Fundamental, 0.15, 1.0),
    rule("earnings_yield", CandidateAxis::Fundamental, 0.15, 1.0),
];

const FINANCIAL_RULES: [FactorRule; 6] = [
    rule("earnings_growth", CandidateAxis::Fundamental, 0.15, 1.0),
    rule("roe", CandidateAxis::Fundamental, 0.20, 1.0),
    rule("net_interest_margin", CandidateAxis::Fundamental, 0.15, 1.0),
    rule("capital_adequacy", CandidateAxis::Fundamental, 0.20, 1.0),
    rule(
        "nonperforming_asset_ratio",
        CandidateAxis::Fundamental,
        0.15,
        -1.0,
    ),
    rule("book_yield", CandidateAxis::Fundamental, 0.15, 1.0),
];

const TECHNICAL_RULES: [FactorRule; 8] = [
    rule("return_5", CandidateAxis::Technical, 0.12, 1.0),
    rule("return_20", CandidateAxis::Technical, 0.18, 1.0),
    rule("return_60", CandidateAxis::Technical, 0.12, 1.0),
    rule("distance_high_20", CandidateAxis::Technical, 0.12, 1.0),
    rule("above_sma_20", CandidateAxis::Technical, 0.12, 1.0),
    rule("volatility_20", CandidateAxis::Technical, 0.12, -1.0),
    rule(
        "average_trading_value_20",
        CandidateAxis::Technical,
        0.10,
        1.0,
    ),
    rule("drawdown_60", CandidateAxis::Technical, 0.12, 1.0),
];

const fn rule(id: &'static str, axis: CandidateAxis, weight: f64, direction: f64) -> FactorRule {
    FactorRule {
        id,
        axis,
        weight,
        direction,
    }
}

struct Prepared {
    instrument: InstrumentId,
    sector_code: String,
    exclusions: Vec<CandidateExclusion>,
    raw: BTreeMap<&'static str, Option<f64>>,
    financial: bool,
}

/// Scores a frozen candidate universe and returns rows in canonical instrument
/// order. The caller can obtain the feed by filtering
/// [`CandidateAnalysis::is_top_five_eligible`], sorting by descending score,
/// then by instrument, and taking five.
pub fn score_candidates(
    config: &CandidateScoringConfig,
    inputs: &[CandidateInstrumentInput],
) -> Result<Vec<CandidateAnalysis>, CandidateScoreError> {
    validate_config(config)?;
    let mut seen = BTreeSet::new();
    let mut prepared = Vec::with_capacity(inputs.len());
    for input in inputs {
        if !seen.insert(input.instrument.clone()) {
            return Err(CandidateScoreError::DuplicateInstrument {
                instrument: input.instrument.to_string(),
            });
        }
        prepared.push(prepare(config, input)?);
    }
    prepared.sort_by(|left, right| left.instrument.cmp(&right.instrument));

    let all_rules: Vec<FactorRule> = FLOW_RULES
        .into_iter()
        .chain(FUNDAMENTAL_RULES)
        .chain(FINANCIAL_RULES)
        .chain(TECHNICAL_RULES)
        .collect();
    let unique_rules: BTreeMap<&'static str, FactorRule> =
        all_rules.into_iter().map(|rule| (rule.id, rule)).collect();
    let normalized = normalize(config, &prepared, &unique_rules);

    Ok(prepared
        .iter()
        .enumerate()
        .map(|(index, row)| build_analysis(config, row, index, &normalized))
        .collect())
}

/// Returns feed candidates in deterministic rank order. A common feed is only
/// publishable when at least five instruments satisfy the evidence gate.
pub fn top_five(analyses: &[CandidateAnalysis]) -> Vec<&CandidateAnalysis> {
    let mut eligible: Vec<&CandidateAnalysis> = analyses
        .iter()
        .filter(|analysis| analysis.is_top_five_eligible())
        .collect();
    eligible.sort_by(|left, right| {
        right
            .composite_score
            .unwrap_or_default()
            .total_cmp(&left.composite_score.unwrap_or_default())
            .then_with(|| left.instrument.cmp(&right.instrument))
    });
    eligible.truncate(5);
    eligible
}

fn validate_config(config: &CandidateScoringConfig) -> Result<(), CandidateScoreError> {
    let weights = [
        config.flow_weight,
        config.fundamental_weight,
        config.technical_weight,
    ];
    if config.version.trim().is_empty()
        || weights
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || (weights.iter().sum::<f64>() - 1.0).abs() > 1e-12
        || !config.min_average_trading_value_20.is_finite()
        || config.min_average_trading_value_20 < 0.0
        || !(0.0..config.winsor_upper).contains(&config.winsor_lower)
        || !(config.winsor_lower..=1.0).contains(&config.winsor_upper)
    {
        return Err(CandidateScoreError::InvalidConfig {
            detail: "weights must be positive and sum to one; winsor bounds and liquidity floor must be valid"
                .to_owned(),
        });
    }
    Ok(())
}

fn prepare(
    config: &CandidateScoringConfig,
    input: &CandidateInstrumentInput,
) -> Result<Prepared, CandidateScoreError> {
    if input.sector_code.trim().is_empty() {
        return Err(invalid_input(input, "sector_code must not be empty"));
    }
    for pair in input.sessions.windows(2) {
        if pair[0].date >= pair[1].date {
            return Err(invalid_input(
                input,
                "sessions must be strictly increasing and unique",
            ));
        }
    }
    for session in &input.sessions {
        if !session.close.is_finite()
            || session.close <= 0.0
            || !session.trading_value.is_finite()
            || session.trading_value < 0.0
            || session
                .foreign_net_amount
                .is_some_and(|value| !value.is_finite())
            || session
                .institution_net_amount
                .is_some_and(|value| !value.is_finite())
        {
            return Err(invalid_input(
                input,
                "session values must be finite and valid",
            ));
        }
    }
    if input.fundamentals.values().any(|value| !value.is_finite()) {
        return Err(invalid_input(input, "fundamental values must be finite"));
    }

    let mut exclusions = BTreeSet::new();
    let flags = input.flags;
    for (active, reason) in [
        (flags.suspended, CandidateExclusion::Suspended),
        (flags.administrative, CandidateExclusion::Administrative),
        (flags.liquidation, CandidateExclusion::Liquidation),
        (flags.inactive, CandidateExclusion::Inactive),
        (flags.data_stale, CandidateExclusion::DataStale),
        (
            flags.disqualifying_audit_opinion,
            CandidateExclusion::DisqualifyingAuditOpinion,
        ),
        (
            flags.complete_capital_impairment,
            CandidateExclusion::CompleteCapitalImpairment,
        ),
        (
            !flags.fundamental_profile_supported,
            CandidateExclusion::UnsupportedFundamentalProfile,
        ),
        (
            !flags.entitlement_active,
            CandidateExclusion::EntitlementInactive,
        ),
    ] {
        if active {
            exclusions.insert(reason);
        }
    }
    if input.sessions.len() < 60 {
        exclusions.insert(CandidateExclusion::InsufficientPriceHistory);
    }
    let last_twenty = tail(&input.sessions, 20);
    if last_twenty.len() < 20
        || last_twenty.iter().any(|session| {
            session.foreign_net_amount.is_none() || session.institution_net_amount.is_none()
        })
    {
        exclusions.insert(CandidateExclusion::RequiredFlowMissing);
    }
    let average_trading_value_20 = mean(last_twenty.iter().map(|row| row.trading_value));
    if average_trading_value_20.is_none_or(|value| value < config.min_average_trading_value_20) {
        exclusions.insert(CandidateExclusion::Illiquid);
    }

    let mut raw = BTreeMap::new();
    raw.insert(
        "foreign_intensity_5",
        flow_intensity(&input.sessions, 5, true),
    );
    raw.insert(
        "foreign_intensity_20",
        flow_intensity(&input.sessions, 20, true),
    );
    raw.insert(
        "foreign_intensity_60",
        flow_intensity(&input.sessions, 60, true),
    );
    raw.insert(
        "institution_intensity_5",
        flow_intensity(&input.sessions, 5, false),
    );
    raw.insert(
        "institution_intensity_20",
        flow_intensity(&input.sessions, 20, false),
    );
    raw.insert(
        "institution_intensity_60",
        flow_intensity(&input.sessions, 60, false),
    );
    let joint_20 = paired(
        raw["foreign_intensity_20"],
        raw["institution_intensity_20"],
        |foreign, institution| foreign.min(institution),
    );
    raw.insert(
        "flow_acceleration",
        paired(
            raw["foreign_intensity_20"],
            raw["foreign_intensity_60"],
            |twenty, sixty| twenty - sixty,
        ),
    );
    raw.insert("joint_accumulation", joint_20);

    raw.insert("return_5", session_return(&input.sessions, 5));
    raw.insert("return_20", session_return(&input.sessions, 20));
    raw.insert("return_60", session_return(&input.sessions, 60));
    raw.insert(
        "price_flow_divergence",
        triple(
            raw["foreign_intensity_20"],
            raw["institution_intensity_20"],
            raw["return_20"],
            |foreign, institution, price| foreign + institution - price,
        ),
    );
    raw.insert("distance_high_20", distance_from_high(&input.sessions, 20));
    raw.insert("above_sma_20", above_average(&input.sessions, 20));
    raw.insert("volatility_20", volatility(&input.sessions, 20));
    raw.insert(
        "average_trading_value_20",
        average_trading_value_20
            .filter(|value| *value > 0.0)
            .map(f64::ln),
    );
    raw.insert("drawdown_60", drawdown(&input.sessions, 60));

    let fundamental_rules = if input.is_financial {
        &FINANCIAL_RULES[..]
    } else {
        &FUNDAMENTAL_RULES[..]
    };
    for rule in fundamental_rules {
        raw.insert(rule.id, input.fundamentals.get(rule.id).copied());
    }

    Ok(Prepared {
        instrument: input.instrument.clone(),
        sector_code: input.sector_code.clone(),
        exclusions: exclusions.into_iter().collect(),
        raw,
        financial: input.is_financial,
    })
}

type Normalized = BTreeMap<(&'static str, usize), (Option<f64>, NormalizationScope)>;

fn normalize(
    config: &CandidateScoringConfig,
    rows: &[Prepared],
    rules: &BTreeMap<&'static str, FactorRule>,
) -> Normalized {
    let eligible: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| row.exclusions.is_empty().then_some(index))
        .collect();
    let mut output = BTreeMap::new();
    for factor_id in rules.keys().copied() {
        for &index in &eligible {
            let sector_peers: Vec<usize> = eligible
                .iter()
                .copied()
                .filter(|peer| {
                    rows[*peer].sector_code == rows[index].sector_code
                        && rows[*peer].raw.get(factor_id).copied().flatten().is_some()
                })
                .collect();
            let (scope, peers) = if sector_peers.len() >= MIN_SECTOR_SAMPLE {
                (NormalizationScope::Sector, sector_peers)
            } else {
                (
                    NormalizationScope::UniverseFallback,
                    eligible
                        .iter()
                        .copied()
                        .filter(|peer| rows[*peer].raw.get(factor_id).copied().flatten().is_some())
                        .collect(),
                )
            };
            let value = rows[index].raw.get(factor_id).copied().flatten();
            let values: Vec<f64> = peers
                .iter()
                .filter_map(|peer| rows[*peer].raw.get(factor_id).copied().flatten())
                .collect();
            output.insert(
                (factor_id, index),
                (
                    value.and_then(|raw| normalize_value(config, raw, &values)),
                    scope,
                ),
            );
        }
    }
    output
}

fn normalize_value(config: &CandidateScoringConfig, raw: f64, values: &[f64]) -> Option<f64> {
    if values.len() < 3 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len() as f64;
    let lower = sorted[((n - 1.0) * config.winsor_lower).floor() as usize];
    let upper = sorted[((n - 1.0) * config.winsor_upper).ceil() as usize];
    let winsorized: Vec<f64> = sorted
        .into_iter()
        .map(|value| value.clamp(lower, upper))
        .collect();
    let mean = winsorized.iter().sum::<f64>() / n;
    let variance = winsorized
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / n;
    if variance <= f64::EPSILON {
        return None;
    }
    Some(((raw.clamp(lower, upper) - mean) / variance.sqrt()).clamp(-Z_CAP, Z_CAP))
}

fn build_analysis(
    config: &CandidateScoringConfig,
    row: &Prepared,
    index: usize,
    normalized: &Normalized,
) -> CandidateAnalysis {
    let fundamental = if row.financial {
        &FINANCIAL_RULES[..]
    } else {
        &FUNDAMENTAL_RULES[..]
    };
    let rules: Vec<FactorRule> = FLOW_RULES
        .into_iter()
        .chain(fundamental.iter().copied())
        .chain(TECHNICAL_RULES)
        .collect();
    let mut factors = BTreeMap::new();
    for rule in &rules {
        let raw = row.raw.get(rule.id).copied().flatten();
        let (value, scope) = normalized
            .get(&(rule.id, index))
            .copied()
            .unwrap_or((None, NormalizationScope::UniverseFallback));
        factors.insert(
            rule.id.to_owned(),
            CandidateFactorValue {
                raw,
                normalized: value.map(|normalized| normalized * rule.direction),
                weight: rule.weight,
                normalization_scope: scope,
            },
        );
    }

    let mut axes = BTreeMap::new();
    for axis in CandidateAxis::ALL {
        let axis_rules: Vec<&FactorRule> = rules.iter().filter(|rule| rule.axis == axis).collect();
        let present_weight: f64 = axis_rules
            .iter()
            .filter(|rule| factors[rule.id].normalized.is_some())
            .map(|rule| rule.weight)
            .sum();
        let coverage = present_weight.clamp(0.0, 1.0);
        let centered: f64 = axis_rules
            .iter()
            .filter_map(|rule| factors[rule.id].normalized.map(|value| value * rule.weight))
            .sum();
        let score = (coverage + 1e-12 >= MIN_AXIS_COVERAGE)
            .then_some((50.0 + SCORE_SCALE * centered).clamp(0.0, 100.0));
        axes.insert(
            axis.as_str().to_owned(),
            CandidateAxisScore { score, coverage },
        );
    }

    let composite_score = if row.exclusions.is_empty()
        && CandidateAxis::ALL
            .iter()
            .all(|axis| axes[axis.as_str()].score.is_some())
    {
        Some(
            config.flow_weight * axes[CandidateAxis::Flow.as_str()].score.unwrap_or_default()
                + config.fundamental_weight
                    * axes[CandidateAxis::Fundamental.as_str()]
                        .score
                        .unwrap_or_default()
                + config.technical_weight
                    * axes[CandidateAxis::Technical.as_str()]
                        .score
                        .unwrap_or_default(),
        )
    } else {
        None
    };
    let strength = evidence_strength(&axes);
    let evidence_refs = strongest_evidence(&factors);
    let mut exclusions = row.exclusions.clone();
    if exclusions.is_empty() && (composite_score.is_none() || strength == EvidenceStrength::Weak) {
        exclusions.push(CandidateExclusion::InsufficientEvidence);
    }

    CandidateAnalysis {
        instrument: row.instrument.clone(),
        sector_code: row.sector_code.clone(),
        exclusions,
        factors,
        axes,
        composite_score,
        evidence_strength: strength,
        scenarios: scenarios(evidence_refs),
    }
}

fn evidence_strength(axes: &BTreeMap<String, CandidateAxisScore>) -> EvidenceStrength {
    let rows: Vec<&CandidateAxisScore> = CandidateAxis::ALL
        .iter()
        .map(|axis| &axes[axis.as_str()])
        .collect();
    if rows
        .iter()
        .any(|axis| axis.coverage + 1e-12 < MIN_AXIS_COVERAGE)
    {
        return EvidenceStrength::Weak;
    }
    let signs: Vec<i8> = rows
        .iter()
        .filter_map(|axis| axis.score)
        .map(|score| {
            if score > 50.0 + 1e-12 {
                1
            } else if score < 50.0 - 1e-12 {
                -1
            } else {
                0
            }
        })
        .collect();
    let agree_three =
        signs.len() == 3 && signs[0] != 0 && signs.iter().all(|sign| *sign == signs[0]);
    if rows
        .iter()
        .all(|axis| axis.coverage + 1e-12 >= STRONG_AXIS_COVERAGE)
        && agree_three
    {
        EvidenceStrength::Strong
    } else if [-1, 1]
        .into_iter()
        .any(|sign| signs.iter().filter(|value| **value == sign).count() >= 2)
    {
        EvidenceStrength::Moderate
    } else {
        EvidenceStrength::Weak
    }
}

fn strongest_evidence(factors: &BTreeMap<String, CandidateFactorValue>) -> Vec<String> {
    let mut values: Vec<(&str, f64)> = factors
        .iter()
        .filter_map(|(id, factor)| factor.normalized.map(|value| (id.as_str(), value.abs())))
        .collect();
    values.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    values
        .into_iter()
        .take(5)
        .map(|(id, _)| id.to_owned())
        .collect()
}

fn scenarios(evidence_refs: Vec<String>) -> BTreeMap<String, CandidateScenario> {
    [
        (
            "bullish",
            "상승 경로",
            "flow_score > 55 AND technical_score > 55",
        ),
        ("neutral", "중립 경로", "ABS(composite_score - 50) <= 5"),
        (
            "bearish",
            "하락 경로",
            "technical_score < 45 OR flow_score < 45",
        ),
    ]
    .into_iter()
    .map(|(label, title, expression)| {
        (
            label.to_owned(),
            CandidateScenario {
                label: label.to_ascii_uppercase(),
                title: title.to_owned(),
                trigger_expression: expression.to_owned(),
                evidence_refs: evidence_refs.clone(),
            },
        )
    })
    .collect()
}

fn invalid_input(input: &CandidateInstrumentInput, detail: &str) -> CandidateScoreError {
    CandidateScoreError::InvalidInput {
        instrument: input.instrument.to_string(),
        detail: detail.to_owned(),
    }
}

fn tail<T>(values: &[T], count: usize) -> &[T] {
    &values[values.len().saturating_sub(count)..]
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values: Vec<f64> = values.collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn flow_intensity(sessions: &[CandidateSession], window: usize, foreign: bool) -> Option<f64> {
    let rows = tail(sessions, window);
    if rows.len() < window {
        return None;
    }
    let denominator = rows.iter().map(|row| row.trading_value).sum::<f64>();
    if denominator <= 0.0 {
        return None;
    }
    let numerator = rows.iter().try_fold(0.0, |sum, row| {
        let value = if foreign {
            row.foreign_net_amount
        } else {
            row.institution_net_amount
        }?;
        Some(sum + value)
    })?;
    Some(numerator / denominator)
}

fn session_return(sessions: &[CandidateSession], window: usize) -> Option<f64> {
    if sessions.len() <= window {
        return None;
    }
    let end = sessions.last()?.close;
    let start = sessions.get(sessions.len() - 1 - window)?.close;
    Some(end / start - 1.0)
}

fn distance_from_high(sessions: &[CandidateSession], window: usize) -> Option<f64> {
    let rows = tail(sessions, window);
    if rows.len() < window {
        return None;
    }
    let high = rows.iter().map(|row| row.close).max_by(f64::total_cmp)?;
    Some(rows.last()?.close / high - 1.0)
}

fn above_average(sessions: &[CandidateSession], window: usize) -> Option<f64> {
    let rows = tail(sessions, window);
    let average = mean(rows.iter().map(|row| row.close))?;
    (rows.len() == window).then(|| rows.last().expect("non-empty window").close / average - 1.0)
}

fn volatility(sessions: &[CandidateSession], window: usize) -> Option<f64> {
    if sessions.len() <= window {
        return None;
    }
    let rows = tail(sessions, window + 1);
    let returns: Vec<f64> = rows
        .windows(2)
        .map(|pair| pair[1].close / pair[0].close - 1.0)
        .collect();
    let average = mean(returns.iter().copied())?;
    Some(
        (returns
            .iter()
            .map(|value| (value - average).powi(2))
            .sum::<f64>()
            / returns.len() as f64)
            .sqrt(),
    )
}

fn drawdown(sessions: &[CandidateSession], window: usize) -> Option<f64> {
    let rows = tail(sessions, window);
    if rows.len() < window {
        return None;
    }
    let high = rows.iter().map(|row| row.close).max_by(f64::total_cmp)?;
    Some(rows.last()?.close / high - 1.0)
}

fn paired(left: Option<f64>, right: Option<f64>, f: impl FnOnce(f64, f64) -> f64) -> Option<f64> {
    Some(f(left?, right?))
}

fn triple(
    first: Option<f64>,
    second: Option<f64>,
    third: Option<f64>,
    f: impl FnOnce(f64, f64, f64) -> f64,
) -> Option<f64> {
    Some(f(first?, second?, third?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(day: usize) -> TradingDate {
        TradingDate::parse(&format!("2026-05-{:02}", (day % 28) + 1)).unwrap()
    }

    fn input(index: usize, sector: &str, missing_fundamental: bool) -> CandidateInstrumentInput {
        let sessions = (0..61)
            .map(|day| CandidateSession {
                date: TradingDate::parse(
                    &chrono::NaiveDate::from_ymd_opt(2026, 1, 2)
                        .unwrap()
                        .checked_add_days(chrono::Days::new(day as u64))
                        .unwrap()
                        .format("%Y-%m-%d")
                        .to_string(),
                )
                .unwrap(),
                close: 100.0 + index as f64 * 3.0 + day as f64 * (0.2 + index as f64 / 100.0),
                trading_value: 2_000_000_000.0 + index as f64 * 50_000_000.0,
                foreign_net_amount: Some(1_000_000.0 * index as f64 + day as f64),
                institution_net_amount: Some(500_000.0 * index as f64 + day as f64),
            })
            .collect();
        let mut fundamentals = BTreeMap::from([
            ("revenue_growth".to_owned(), index as f64 + 0.1),
            ("operating_margin".to_owned(), index as f64 + 0.2),
            ("roe".to_owned(), index as f64 + 0.3),
            ("debt_ratio".to_owned(), 20.0 - index as f64),
            ("cash_conversion".to_owned(), index as f64 + 0.4),
            ("earnings_yield".to_owned(), index as f64 + 0.5),
        ]);
        if missing_fundamental {
            fundamentals.remove("earnings_yield");
        }
        CandidateInstrumentInput {
            instrument: InstrumentId::parse(&format!("{:06}.KRX", index + 1)).unwrap(),
            sector_code: sector.to_owned(),
            is_financial: false,
            sessions,
            fundamentals,
            flags: CandidateFlags::eligible_defaults(),
        }
    }

    #[test]
    fn sector_under_eight_uses_universe_fallback() {
        let inputs: Vec<_> = (0..10)
            .map(|index| input(index, if index < 7 { "A" } else { "B" }, false))
            .collect();
        let scored = score_candidates(&CandidateScoringConfig::default(), &inputs).unwrap();
        assert!(scored.iter().all(|analysis| {
            analysis
                .factors
                .values()
                .all(|factor| factor.normalization_scope == NormalizationScope::UniverseFallback)
        }));
    }

    #[test]
    fn missing_factor_reduces_coverage_without_reweighting() {
        let inputs: Vec<_> = (0..10).map(|index| input(index, "A", index == 9)).collect();
        let scored = score_candidates(&CandidateScoringConfig::default(), &inputs).unwrap();
        let incomplete = scored
            .iter()
            .find(|analysis| analysis.instrument.to_string() == "000010.KRX")
            .unwrap();
        assert!((incomplete.axes["fundamental"].coverage - 0.85).abs() < 1e-12);
        assert!(incomplete.axes["fundamental"].score.is_some());
        assert_eq!(incomplete.factors["earnings_yield"].normalized, None);
    }

    #[test]
    fn hard_exclusions_make_score_and_feed_ineligible() {
        let mut inputs: Vec<_> = (0..10).map(|index| input(index, "A", false)).collect();
        inputs[9].flags.suspended = true;
        let scored = score_candidates(&CandidateScoringConfig::default(), &inputs).unwrap();
        let excluded = scored
            .iter()
            .find(|analysis| analysis.instrument.to_string() == "000010.KRX")
            .unwrap();
        assert!(excluded.exclusions.contains(&CandidateExclusion::Suspended));
        assert_eq!(excluded.composite_score, None);
        assert!(
            !top_five(&scored)
                .iter()
                .any(|analysis| analysis.instrument == excluded.instrument)
        );
    }

    #[test]
    fn rank_and_scenarios_are_deterministic_without_probability_copy() {
        let inputs: Vec<_> = (0..12).map(|index| input(index, "A", false)).collect();
        let first = score_candidates(&CandidateScoringConfig::default(), &inputs).unwrap();
        let second = score_candidates(&CandidateScoringConfig::default(), &inputs).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            top_five(&first)
                .iter()
                .map(|analysis| analysis.instrument.to_string())
                .collect::<Vec<_>>(),
            top_five(&second)
                .iter()
                .map(|analysis| analysis.instrument.to_string())
                .collect::<Vec<_>>()
        );
        let encoded = serde_json::to_string(&first[0].scenarios).unwrap();
        let lower = encoded.to_ascii_lowercase();
        assert!(!lower.contains("probability"));
        assert!(!lower.contains("target_price"));
        assert!(!encoded.contains('%'));
    }

    #[test]
    fn non_financial_and_financial_profiles_do_not_mix() {
        let mut financial = input(0, "FIN", false);
        financial.is_financial = true;
        financial.fundamentals = BTreeMap::from([
            ("earnings_growth".to_owned(), 1.0),
            ("roe".to_owned(), 1.0),
            ("net_interest_margin".to_owned(), 1.0),
            ("capital_adequacy".to_owned(), 1.0),
            ("nonperforming_asset_ratio".to_owned(), 1.0),
            ("book_yield".to_owned(), 1.0),
        ]);
        let mut inputs = vec![financial];
        inputs.extend((1..10).map(|index| input(index, "FIN", false)));
        let scored = score_candidates(&CandidateScoringConfig::default(), &inputs).unwrap();
        assert!(scored[0].factors.contains_key("capital_adequacy"));
        assert!(!scored[0].factors.contains_key("debt_ratio"));
    }

    #[test]
    fn test_date_helper_stays_valid() {
        assert_eq!(date(0).to_string(), "2026-05-01");
    }
}
