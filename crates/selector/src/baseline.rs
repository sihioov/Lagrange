//! The five versioned baseline strategy packages (design §6.7, FR-STR-004).
//!
//! Metadata mirror of the Python packages under `nt/strategies/<id>/`
//! (`package.py` + `schema.json`): identical strategy ids, SemVer, parameter
//! schemas/defaults, markets, cadences, required factors, lookbacks, risk
//! descriptions, and golden fixtures.  The Rust registry stores and governs
//! this metadata; the Python packages carry the same metadata plus the
//! engine-independent target generators and the NT adapters.
//!
//! Baseline semantics (design §6.7 + §8.2):
//! - `buy_and_hold`: hold the benchmark ETF (069500.KRX) at the target
//!   weight; no market timing, no factors required.
//! - `trend_following`: long the benchmark when the fast moving average
//!   exceeds the slow one, cash otherwise; consumes `trend_50`/`trend_200`.
//! - `relative_momentum`: rank the fixed universe by 12-minus-1 momentum and
//!   hold the top N equally weighted; consumes `momentum_12_1`.
//! - `dual_momentum` (design §8.2): invest fully in the strongest 12-month
//!   performer when its return beats the absolute threshold, else defensive
//!   cash; consumes `return_12m`.
//! - `inverse_volatility`: weight each eligible ETF inversely to its realized
//!   volatility (default 60 sessions); consumes `vol_60`.

use domain::StrategyVersion;
use serde_json::{Value as Json, json};

use crate::registry::{AssetClass, Cadence, Market, StrategyPackage, StrategyState};

/// The stable baseline strategy ids in canonical order.
pub const BASELINE_STRATEGY_IDS: [&str; 5] = [
    "buy_and_hold",
    "trend_following",
    "relative_momentum",
    "dual_momentum",
    "inverse_volatility",
];

/// The definition fields of one baseline package (bundled to keep the
/// constructors readable).
struct Spec {
    strategy_id: &'static str,
    version: &'static str,
    name: &'static str,
    description: &'static str,
    risk_description: &'static str,
    parameter_schema: Json,
    default_parameters: Json,
    required_factors: &'static [&'static str],
    minimum_lookback_sessions: u64,
    golden_fixture: &'static str,
}

impl Spec {
    fn build(self) -> StrategyPackage {
        StrategyPackage {
            strategy_id: self.strategy_id.to_owned(),
            version: StrategyVersion::parse(self.version).expect("static baseline semver"),
            name: self.name.to_owned(),
            description: self.description.to_owned(),
            risk_description: self.risk_description.to_owned(),
            parameter_schema: self.parameter_schema,
            default_parameters: self.default_parameters,
            markets: vec![Market::Krx],
            asset_classes: vec![AssetClass::Etf],
            cadences: vec![Cadence::Daily],
            required_factors: self
                .required_factors
                .iter()
                .map(|f| f.to_string())
                .collect(),
            minimum_lookback_sessions: self.minimum_lookback_sessions,
            target_generator_ref: format!(
                "nt.strategies.{}.target:generate_target",
                self.strategy_id
            ),
            nt_adapter_ref: format!(
                "nt.strategies.{}.adapter:{}Adapter",
                self.strategy_id, self.name
            ),
            golden_fixture_refs: vec![self.golden_fixture.to_owned()],
            state: StrategyState::Draft,
            canonical_hash: String::new(),
        }
    }
}

fn buy_and_hold() -> StrategyPackage {
    Spec {
        strategy_id: "buy_and_hold",
        version: "1.0.0",
        name: "BuyAndHold",
        description: "Benchmark buy-and-hold: hold the benchmark ETF \
                      (069500.KRX) at the declared target weight from the \
                      first session open; no market timing.",
        risk_description: "Full market exposure with no drawdown control; \
                           single-instrument concentration in the benchmark; \
                           no exit mechanism beyond the optional monthly \
                           rebalance.",
        parameter_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "benchmark_instrument": {
                    "type": "string",
                    "pattern": "^[0-9]{6}\\.KRX$",
                    "description": "The canonical KRX ETF id to hold."
                },
                "target_weight": {
                    "type": "number",
                    "exclusiveMinimum": 0.0,
                    "maximum": 1.0,
                    "description": "Fraction of equity held in the benchmark."
                },
                "rebalance_cadence": {
                    "type": "string",
                    "enum": ["none", "monthly"],
                    "description": "When to re-apply the target (none = hold)."
                }
            },
            "required": ["benchmark_instrument", "target_weight"],
            "additionalProperties": false
        }),
        default_parameters: json!({
            "benchmark_instrument": "069500.KRX",
            "target_weight": 1.0,
            "rebalance_cadence": "none"
        }),
        required_factors: &[],
        minimum_lookback_sessions: 0,
        golden_fixture: "nt/strategies/buy_and_hold/golden.json",
    }
    .build()
}

fn trend_following() -> StrategyPackage {
    Spec {
        strategy_id: "trend_following",
        version: "1.0.0",
        name: "TrendFollowing",
        description: "Moving-average trend following on the benchmark ETF: \
                      long the benchmark while the fast average exceeds the \
                      slow average, cash otherwise; signals at T-close \
                      execute at T+1 open.",
        risk_description: "Whipsaw losses in choppy sideways regimes; \
                           concentrated single-position exposure; cash drag \
                           when out of the market; one-session signal lag \
                           (T-close -> T+1 open) by construction.",
        parameter_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "benchmark_instrument": {
                    "type": "string",
                    "pattern": "^[0-9]{6}\\.KRX$"
                },
                "fast_ma": {
                    "type": "integer",
                    "minimum": 5,
                    "maximum": 250,
                    "description": "Fast moving-average window (sessions)."
                },
                "slow_ma": {
                    "type": "integer",
                    "minimum": 10,
                    "maximum": 500,
                    "description": "Slow moving-average window (sessions)."
                }
            },
            "required": ["benchmark_instrument", "fast_ma", "slow_ma"],
            "additionalProperties": false
        }),
        default_parameters: json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 50,
            "slow_ma": 200
        }),
        required_factors: &["trend_50", "trend_200"],
        minimum_lookback_sessions: 200,
        golden_fixture: "nt/strategies/trend_following/golden.json",
    }
    .build()
}

fn relative_momentum() -> StrategyPackage {
    Spec {
        strategy_id: "relative_momentum",
        version: "1.0.0",
        name: "RelativeMomentum",
        description: "Relative momentum: rank the fixed Korean ETF universe \
                      by 12-minus-1 month momentum and hold the top N equally \
                      weighted, rebalanced at the monthly close and executed \
                      at the next open.",
        risk_description: "Momentum crash and reversal risk; concentration in \
                           the top-N names; monthly rotation turnover and \
                           cost drag; the lookback excludes the most recent \
                           month (12-minus-1) by design.",
        parameter_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "top_n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "description": "Number of highest-momentum ETFs held."
                },
                "lookback_months": {
                    "type": "integer",
                    "enum": [6, 12],
                    "description": "Momentum lookback in months."
                }
            },
            "required": ["top_n", "lookback_months"],
            "additionalProperties": false
        }),
        default_parameters: json!({ "top_n": 3, "lookback_months": 12 }),
        required_factors: &["momentum_12_1"],
        minimum_lookback_sessions: 252,
        golden_fixture: "nt/strategies/relative_momentum/golden.json",
    }
    .build()
}

fn dual_momentum() -> StrategyPackage {
    Spec {
        strategy_id: "dual_momentum",
        version: "1.0.0",
        name: "DualMomentum",
        description: "Dual momentum (design §8.2): invest fully in the \
                      strongest 12-month performer of the universe when its \
                      return exceeds the absolute threshold; otherwise hold \
                      defensive cash.  Rebalanced monthly, executed at the \
                      next open.",
        risk_description: "All-or-nothing switching between a single risky \
                           asset and cash; threshold flips can trigger large \
                           full repositions; no diversification; single-name \
                           failure risk while invested.",
        parameter_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "absolute_threshold": {
                    "type": "number",
                    "description": "Minimum 12-month return (decimal) of the \
                                    top risky asset to stay invested."
                },
                "lookback_months": {
                    "type": "integer",
                    "enum": [6, 12],
                    "description": "Return lookback in months."
                }
            },
            "required": ["absolute_threshold", "lookback_months"],
            "additionalProperties": false
        }),
        default_parameters: json!({ "absolute_threshold": 0.0, "lookback_months": 12 }),
        required_factors: &["return_12m"],
        minimum_lookback_sessions: 252,
        golden_fixture: "nt/strategies/dual_momentum/golden.json",
    }
    .build()
}

fn inverse_volatility() -> StrategyPackage {
    Spec {
        strategy_id: "inverse_volatility",
        version: "1.0.0",
        name: "InverseVolatility",
        description: "Inverse volatility weighting: each eligible ETF \
                      receives weight inversely proportional to its realized \
                      volatility (default 60 sessions), so calmer names \
                      receive more capital; rebalanced monthly at the next \
                      open.",
        risk_description: "Low-volatility concentration during calm markets; \
                           underweights high-volatility momentum winners; \
                           monthly rebalancing turnover; volatility \
                           estimation lag.",
        parameter_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "vol_window": {
                    "type": "integer",
                    "enum": [20, 60, 120],
                    "description": "Realized-volatility window in sessions."
                },
                "max_weight": {
                    "type": "number",
                    "exclusiveMinimum": 0.0,
                    "maximum": 1.0,
                    "description": "Per-instrument weight ceiling."
                }
            },
            "required": ["vol_window", "max_weight"],
            "additionalProperties": false
        }),
        default_parameters: json!({ "vol_window": 60, "max_weight": 0.3 }),
        required_factors: &["vol_60"],
        minimum_lookback_sessions: 60,
        golden_fixture: "nt/strategies/inverse_volatility/golden.json",
    }
    .build()
}

/// The five baseline packages at their initial Draft state, in canonical
/// order.
pub fn baseline_packages() -> Vec<StrategyPackage> {
    vec![
        buy_and_hold(),
        trend_following(),
        relative_momentum(),
        dual_momentum(),
        inverse_volatility(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_ids_match_documented_set() {
        let packages = baseline_packages();
        let ids: Vec<&str> = packages.iter().map(|p| p.strategy_id.as_str()).collect();
        assert_eq!(ids, BASELINE_STRATEGY_IDS);
    }

    #[test]
    fn baseline_versions_are_semver() {
        for package in baseline_packages() {
            assert!(StrategyVersion::parse(&package.version.to_string()).is_ok());
        }
    }
}
