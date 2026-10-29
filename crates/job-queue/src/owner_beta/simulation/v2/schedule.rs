//! Checked common evaluation window and strategy-specific decision calendar.

use domain::TradingDate;
use serde::{Deserialize, Serialize};

use super::contract::{V2ContractError, V2Strategy};

/// Why the v2 engine is allowed to request a target for a date pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum V2DecisionKind {
    #[serde(rename = "INITIAL_COMMON_SESSION")]
    InitialCommonSession,
    #[serde(rename = "DAILY_TREND_TRANSITION_CHECK")]
    DailyTrendTransitionCheck,
    #[serde(rename = "MONTH_END_SIGNAL")]
    MonthEndSignal,
}

/// One T-close signal and next-approved-session raw-open decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct V2Decision {
    signal_date: TradingDate,
    execution_date: TradingDate,
    kind: V2DecisionKind,
}

impl V2Decision {
    pub const fn signal_date(&self) -> TradingDate {
        self.signal_date
    }

    pub const fn execution_date(&self) -> TradingDate {
        self.execution_date
    }

    pub const fn kind(&self) -> V2DecisionKind {
        self.kind
    }
}

/// Immutable schedule shared by all cost profiles for one strategy. Factor
/// readiness is deliberately established by WP-2 against the approved input;
/// this type accepts the resulting prepared prefix and never invents a fixed
/// 200/252-session readiness rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V2Schedule {
    strategy: V2Strategy,
    preparation_session_count: u32,
    first_signal_date: TradingDate,
    preparation_sessions: Vec<TradingDate>,
    evaluation_sessions: Vec<TradingDate>,
    decisions: Vec<V2Decision>,
}

impl V2Schedule {
    /// `preparation_sessions` must be the approved chronological prefix ending
    /// at the earliest common-ready T selected by the input/factor adapter.
    /// Evaluation sessions begin at the following approved session S.
    pub fn new(
        strategy: V2Strategy,
        preparation_sessions: Vec<String>,
        evaluation_sessions: Vec<String>,
    ) -> Result<Self, V2ContractError> {
        if preparation_sessions.is_empty() || evaluation_sessions.len() < 2 {
            return Err(V2ContractError::InvalidSchedule);
        }
        let prepared = parse_dates(preparation_sessions)?;
        let evaluation = parse_dates(evaluation_sessions)?;
        if prepared
            .iter()
            .chain(evaluation.iter())
            .any(TradingDate::is_weekend)
            || prepared.windows(2).any(|pair| pair[0] >= pair[1])
            || evaluation.windows(2).any(|pair| pair[0] >= pair[1])
            || prepared.last().is_some_and(|date| date >= &evaluation[0])
        {
            return Err(V2ContractError::InvalidSchedule);
        }

        let first_signal_date = *prepared.last().ok_or(V2ContractError::InvalidSchedule)?;
        let first_execution_date = evaluation[0];
        let mut decisions = vec![V2Decision {
            signal_date: first_signal_date,
            execution_date: first_execution_date,
            kind: V2DecisionKind::InitialCommonSession,
        }];

        for pair in evaluation.windows(2) {
            let signal_date = pair[0];
            let execution_date = pair[1];
            let kind = match strategy {
                V2Strategy::BuyAndHold => continue,
                V2Strategy::TrendFollowing => V2DecisionKind::DailyTrendTransitionCheck,
                V2Strategy::RelativeMomentum
                | V2Strategy::DualMomentum
                | V2Strategy::InverseVolatility
                    if crosses_month(signal_date, execution_date) =>
                {
                    V2DecisionKind::MonthEndSignal
                }
                V2Strategy::RelativeMomentum
                | V2Strategy::DualMomentum
                | V2Strategy::InverseVolatility => continue,
            };
            decisions.push(V2Decision {
                signal_date,
                execution_date,
                kind,
            });
        }

        Ok(Self {
            strategy,
            preparation_session_count: u32::try_from(prepared.len())
                .map_err(|_| V2ContractError::InvalidSchedule)?,
            first_signal_date,
            preparation_sessions: prepared,
            evaluation_sessions: evaluation,
            decisions,
        })
    }

    pub const fn strategy(&self) -> V2Strategy {
        self.strategy
    }

    pub const fn preparation_session_count(&self) -> u32 {
        self.preparation_session_count
    }

    pub const fn first_signal_date(&self) -> TradingDate {
        self.first_signal_date
    }

    pub fn evaluation_sessions(&self) -> &[TradingDate] {
        &self.evaluation_sessions
    }

    pub fn preparation_sessions(&self) -> &[TradingDate] {
        &self.preparation_sessions
    }

    pub fn evaluation_start(&self) -> TradingDate {
        self.evaluation_sessions[0]
    }

    pub fn evaluation_end(&self) -> TradingDate {
        *self
            .evaluation_sessions
            .last()
            .expect("schedule constructor enforces at least two evaluation sessions")
    }

    pub fn decisions(&self) -> &[V2Decision] {
        &self.decisions
    }

    pub fn decision_for_execution(&self, date: TradingDate) -> Option<&V2Decision> {
        self.decisions
            .iter()
            .find(|decision| decision.execution_date == date)
    }

    pub fn same_evaluation_window(&self, other: &Self) -> bool {
        self.evaluation_sessions == other.evaluation_sessions
    }

    pub fn same_common_period(&self, other: &Self) -> bool {
        self.preparation_sessions == other.preparation_sessions
            && self.evaluation_sessions == other.evaluation_sessions
            && self.preparation_session_count == other.preparation_session_count
            && self.first_signal_date == other.first_signal_date
    }
}

fn parse_dates(values: Vec<String>) -> Result<Vec<TradingDate>, V2ContractError> {
    values
        .iter()
        .map(|value| {
            let date = TradingDate::parse(value).map_err(|_| V2ContractError::InvalidSchedule)?;
            if date.to_iso() != *value {
                return Err(V2ContractError::InvalidSchedule);
            }
            Ok(date)
        })
        .collect()
}

fn crosses_month(signal: TradingDate, execution: TradingDate) -> bool {
    let signal = signal.to_iso();
    let execution = execution.to_iso();
    signal[..7] != execution[..7]
}
