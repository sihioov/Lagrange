//! Missing-data policy (AT-05, plan Todo 21).
//!
//! AT-05: "특정 종목 일봉 누락 → 추천·백테스트가 경고 또는 차단 정책대로
//! 동작". Two declared policies:
//!   - [`MissingDataPolicy::RequiredUniverse`] — missing bars of a required
//!     universe instrument BLOCK the run (mirrors the T11 quality policy and
//!     the queue's `DataBlocked` error class: never retried);
//!   - [`MissingDataPolicy::OptionalExclude`] — the strategy declared that
//!     optional symbols may be excluded; the run proceeds with a recorded
//!     exclusion reason (T11: "optional-symbol missing data may exclude only
//!     when the strategy declares that policy and records the reason").

use serde_json::json;

use crate::Warning;
use crate::backtest::BacktestResult;
use crate::robustness::RobustnessError;

/// How missing data is handled for a run (strategy-declared policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MissingDataPolicy {
    /// Missing required-universe bars block the run.
    RequiredUniverse,
    /// Missing optional-symbol bars exclude the symbol with a recorded reason.
    OptionalExclude,
}

/// One missing-instrument record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MissingInstrument {
    pub instrument: String,
    pub missing_sessions: u64,
    pub last_observed: Option<String>,
}

/// The recorded exclusion of one instrument.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExclusionRecord {
    pub instrument: String,
    pub reason: String,
}

/// The policy outcome for a set of missing instruments.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MissingDataOutcome {
    /// No missing data; the policy is satisfied.
    Passed,
    /// Required-universe policy: the run is blocked (typed detail).
    Blocked { detail: String },
    /// Optional-exclusion policy: the run proceeds with recorded exclusions.
    Warning { exclusions: Vec<ExclusionRecord> },
}

fn exclusions(missing: &[MissingInstrument]) -> Vec<ExclusionRecord> {
    missing
        .iter()
        .map(|m| ExclusionRecord {
            instrument: m.instrument.clone(),
            reason: format!(
                "excluded by strategy-declared optional policy: {} sessions missing{}",
                m.missing_sessions,
                m.last_observed
                    .as_ref()
                    .map(|d| format!(" (last observed {d})"))
                    .unwrap_or_default()
            ),
        })
        .collect()
}

/// Applies the declared policy to the missing instruments (AT-05).
pub fn apply_missing_data_policy(
    missing: &[MissingInstrument],
    policy: MissingDataPolicy,
) -> Result<MissingDataOutcome, RobustnessError> {
    if missing.is_empty() {
        return Ok(MissingDataOutcome::Passed);
    }
    match policy {
        MissingDataPolicy::RequiredUniverse => {
            let names: Vec<&str> = missing.iter().map(|m| m.instrument.as_str()).collect();
            Err(RobustnessError::DataBlocked {
                detail: format!(
                    "required-universe instruments missing bars: {}",
                    names.join(", ")
                ),
            })
        }
        MissingDataPolicy::OptionalExclude => Ok(MissingDataOutcome::Warning {
            exclusions: exclusions(missing),
        }),
    }
}

/// Enforces the policy against a result: required-universe blocks the run
/// entirely; optional exclusion returns the result with the recorded
/// `missing_data_excluded` warning attached.
pub fn enforce_missing_data_policy(
    result: &BacktestResult,
    missing: &[MissingInstrument],
    policy: MissingDataPolicy,
) -> Result<BacktestResult, RobustnessError> {
    match apply_missing_data_policy(missing, policy)? {
        MissingDataOutcome::Passed => Ok(result.clone()),
        MissingDataOutcome::Blocked { detail } => Err(RobustnessError::DataBlocked { detail }),
        MissingDataOutcome::Warning { exclusions } => {
            let mut warned = result.clone();
            for exclusion in &exclusions {
                let record = missing
                    .iter()
                    .find(|m| m.instrument == exclusion.instrument)
                    .expect("exclusion references a missing instrument");
                warned.warnings.push(
                    Warning::warn(
                        "missing_data_excluded",
                        format!("{}: {}", exclusion.instrument, exclusion.reason),
                    )
                    .with_details(json!({
                        "instrument": exclusion.instrument,
                        "missing_sessions": record.missing_sessions,
                        "last_observed": record.last_observed,
                        "reason": exclusion.reason,
                    })),
                );
            }
            Ok(warned)
        }
    }
}
