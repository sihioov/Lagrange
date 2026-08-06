//! Reference stability score (design §9.6, plan Todo 21).
//!
//! The initial score is a REFERENCE indicator — it is never an absolute
//! approval criterion ("초기 점수는 참고 지표이며 절대적 승인 기준으로
//! 사용하지 않는다"). [`analyze_stability`] produces the seven documented
//! weighted components (25/20/15/15/10/10/5) with every raw input echoed in
//! the evidence payload; [`approve_investment`] STRUCTURALLY refuses to turn
//! any score into an investment approval
//! ([`RobustnessError::StabilityScoreNotApproval`]).
//!
//! Component formulas (documented; every score is clamped into `[0, weight]`):
//!   - `validation_excess_persistence` (25): 25 x (fraction of validation
//!     months with positive excess return);
//!   - `parameter_neighborhood_stability` (20): 20 x clamp(0..1,
//!     1 - dispersion / max(|mean|, 0.02));
//!   - `cost_stress_survival` (15): 15 x (stressed runs within 10 points of
//!     the parent return / total stressed runs);
//!   - `mdd_volatility` (15): 15 x average of clamp(0..1, 1 - |mdd|/0.40)
//!     and clamp(0..1, 1 - vol/0.40);
//!   - `return_concentration` (10): 10 x clamp(0..1,
//!     1 - max(top_trade_share, year_max_share));
//!   - `recent_performance` (10): 10 x clamp(0..1, recent_excess/0.05);
//!   - `tradability_turnover` (5): 5 x clamp(0..1, 1 - |turnover - 1|/4).

use domain::ReportedStat;
use serde_json::json;

use crate::robustness::RobustnessError;

/// Raw inputs of the stability assessment (all echoed in the score evidence).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StabilityEvidence {
    /// Validation-period monthly excess returns (decimal fractions).
    pub validation_monthly_excess: Vec<ReportedStat>,
    /// Total returns of the parameter-neighborhood runs.
    pub neighborhood_returns: Vec<ReportedStat>,
    /// The parent (baseline) run's total return.
    pub parent_return: ReportedStat,
    /// Total returns of the cost-stressed runs.
    pub cost_stress_final_returns: Vec<ReportedStat>,
    pub max_drawdown: ReportedStat,
    pub volatility: ReportedStat,
    pub top_trade_share: ReportedStat,
    pub year_max_share: ReportedStat,
    pub recent_excess: ReportedStat,
    pub turnover: ReportedStat,
}

/// One weighted stability component plus its raw inputs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StabilityComponent {
    pub code: String,
    pub label: String,
    pub weight: u8,
    /// Component score, always within `[0, weight]`.
    pub score: f64,
    /// The raw inputs used to compute this component.
    pub raw_evidence: serde_json::Value,
}

/// The reference stability score (design §9.6).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StabilityScore {
    /// Sum of the component scores (<= 100).
    pub total: f64,
    pub components: Vec<StabilityComponent>,
    /// The full raw evidence behind the score (never summarized away).
    pub raw_evidence: serde_json::Value,
    /// Always `true`: the score is a reference indicator only.
    pub reference_only: bool,
}

const COST_STRESS_TOLERANCE: f64 = 0.10;
const MAX_DRAWDOWN_SCALE: f64 = 0.40;
const MAX_VOLATILITY_SCALE: f64 = 0.40;
const RECENT_EXCESS_SCALE: f64 = 0.05;
const TURNOVER_IDEAL: f64 = 1.0;
const TURNOVER_TOLERANCE: f64 = 4.0;

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn component(
    code: &str,
    label: &str,
    weight: u8,
    score: f64,
    raw_evidence: serde_json::Value,
) -> StabilityComponent {
    StabilityComponent {
        code: code.to_owned(),
        label: label.to_owned(),
        weight,
        score: score.clamp(0.0, f64::from(weight)),
        raw_evidence,
    }
}

/// Computes the reference stability score from the raw evidence.
pub fn analyze_stability(
    evidence: &StabilityEvidence,
) -> Result<StabilityScore, RobustnessError> {
    let month_excess = evidence.validation_monthly_excess.clone();
    let positive_months = month_excess.iter().filter(|r| r.value() > 0.0).count();
    let validation_persistence = if month_excess.is_empty() {
        0.0
    } else {
        positive_months as f64 / month_excess.len() as f64
    };

    let neighborhood = evidence.neighborhood_returns.clone();
    let mean = if neighborhood.is_empty() {
        0.0
    } else {
        neighborhood.iter().map(|r| r.value()).sum::<f64>() / neighborhood.len() as f64
    };
    let dispersion = if neighborhood.is_empty() {
        0.0
    } else {
        let variance = neighborhood
            .iter()
            .map(|r| (r.value() - mean) * (r.value() - mean))
            .sum::<f64>()
            / neighborhood.len() as f64;
        variance.sqrt()
    };
    let neighborhood_stability = clamp01(1.0 - dispersion / mean.abs().max(0.02));

    let stressed = evidence.cost_stress_final_returns.clone();
    let parent = evidence.parent_return.value();
    let survivals = stressed
        .iter()
        .filter(|r| r.value() >= parent - COST_STRESS_TOLERANCE)
        .count();
    let survival = if stressed.is_empty() {
        0.0
    } else {
        survivals as f64 / stressed.len() as f64
    };

    let mdd_ok = clamp01(1.0 - evidence.max_drawdown.value().abs() / MAX_DRAWDOWN_SCALE);
    let vol_ok = clamp01(1.0 - evidence.volatility.value() / MAX_VOLATILITY_SCALE);
    let mdd_volatility = (mdd_ok + vol_ok) / 2.0;

    let concentration = clamp01(
        1.0 - evidence
            .top_trade_share
            .value()
            .max(evidence.year_max_share.value()),
    );
    let recent = clamp01(evidence.recent_excess.value() / RECENT_EXCESS_SCALE);
    let turnover = clamp01(
        1.0 - (evidence.turnover.value() - TURNOVER_IDEAL).abs() / TURNOVER_TOLERANCE,
    );

    let components = vec![
        component(
            "validation_excess_persistence",
            "Validation-period excess-return persistence",
            25,
            25.0 * validation_persistence,
            json!({
                "positive_months": positive_months,
                "total_months": month_excess.len(),
                "monthly_excess": month_excess.iter().map(|r| r.value()).collect::<Vec<f64>>(),
            }),
        ),
        component(
            "parameter_neighborhood_stability",
            "Parameter-neighborhood stability",
            20,
            20.0 * neighborhood_stability,
            json!({
                "mean_return": mean,
                "dispersion": dispersion,
                "returns": neighborhood.iter().map(|r| r.value()).collect::<Vec<f64>>(),
            }),
        ),
        component(
            "cost_stress_survival",
            "Cost-stress survival",
            15,
            15.0 * survival,
            json!({
                "survivals": survivals,
                "total": stressed.len(),
                "parent_return": parent,
                "stress_returns": stressed.iter().map(|r| r.value()).collect::<Vec<f64>>(),
                "tolerance": COST_STRESS_TOLERANCE,
            }),
        ),
        component(
            "mdd_volatility",
            "Drawdown and volatility",
            15,
            15.0 * mdd_volatility,
            json!({
                "max_drawdown": evidence.max_drawdown.value(),
                "volatility": evidence.volatility.value(),
                "max_drawdown_scale": MAX_DRAWDOWN_SCALE,
                "volatility_scale": MAX_VOLATILITY_SCALE,
            }),
        ),
        component(
            "return_concentration",
            "Return concentration",
            10,
            10.0 * concentration,
            json!({
                "top_trade_share": evidence.top_trade_share.value(),
                "year_max_share": evidence.year_max_share.value(),
            }),
        ),
        component(
            "recent_performance",
            "Recent-period performance",
            10,
            10.0 * recent,
            json!({
                "recent_excess": evidence.recent_excess.value(),
                "scale": RECENT_EXCESS_SCALE,
            }),
        ),
        component(
            "tradability_turnover",
            "Tradability and turnover",
            5,
            5.0 * turnover,
            json!({
                "turnover": evidence.turnover.value(),
                "ideal": TURNOVER_IDEAL,
                "tolerance": TURNOVER_TOLERANCE,
            }),
        ),
    ];

    let total: f64 = components.iter().map(|c| c.score).sum();
    if !total.is_finite() {
        return Err(RobustnessError::NonFinite {
            field: "stability total".to_owned(),
        });
    }
    Ok(StabilityScore {
        total,
        components,
        raw_evidence: json!({
            "validation_monthly_excess": month_excess.iter().map(|r| r.value()).collect::<Vec<f64>>(),
            "neighborhood_returns": neighborhood.iter().map(|r| r.value()).collect::<Vec<f64>>(),
            "parent_return": parent,
            "cost_stress_final_returns": stressed.iter().map(|r| r.value()).collect::<Vec<f64>>(),
            "max_drawdown": evidence.max_drawdown.value(),
            "volatility": evidence.volatility.value(),
            "top_trade_share": evidence.top_trade_share.value(),
            "year_max_share": evidence.year_max_share.value(),
            "recent_excess": evidence.recent_excess.value(),
            "turnover": evidence.turnover.value(),
        }),
        reference_only: true,
    })
}

/// The structural guard: the reference score can NEVER approve an investment
/// (design §9.6). Always refuses with
/// [`RobustnessError::StabilityScoreNotApproval`].
pub fn approve_investment(_score: &StabilityScore) -> Result<(), RobustnessError> {
    Err(RobustnessError::StabilityScoreNotApproval)
}
