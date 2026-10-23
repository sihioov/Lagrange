//! Pure orchestration from one sealed v3 artifact to an atomic pair of
//! canonically hashed simulation results.

use std::path::Path;

use domain::TradingDate;
use uuid::Uuid;

use super::{
    CostProfile, CostProfileLedgers, EngineIdentity, Sha256Digest, SignalSnapshot,
    SimulationResult, SimulationSchedule, StrategyIdentity,
    input::{SimulationInputError, approve_and_prepare},
    ledger::simulate_ledgers,
    signals::{SimulationSignalError, compute_signals, derive_trend_factor_definition},
};

/// Owner-beta identifiers used only to build the existing immutable strategy
/// input. This creates no queue job and adds neither identifier to a result.
pub struct SimulationRunRequest {
    run_id: Uuid,
    strategy_config_id: Uuid,
    evaluation_start: Option<TradingDate>,
    evaluation_end: Option<TradingDate>,
}

impl SimulationRunRequest {
    pub fn new(run_id: Uuid, strategy_config_id: Uuid) -> Self {
        Self {
            run_id,
            strategy_config_id,
            evaluation_start: None,
            evaluation_end: None,
        }
    }

    /// Selects exact dates from the approved session range. Omitting this
    /// keeps the contract default `d[200]..=d[2451]`.
    pub fn with_evaluation_window(
        mut self,
        start: Option<TradingDate>,
        end: Option<TradingDate>,
    ) -> Self {
        self.evaluation_start = start;
        self.evaluation_end = end;
        self
    }
}

/// Engine identity embedded in the result.
///
/// `EngineIdentity` checks syntax only. The caller must bind its values to the
/// final isolated executable and private build receipt before treating a
/// result as runtime provenance. The factor definition digest is derived
/// internally from the actual trend factors and their compile-time source
/// bytes.
pub struct SimulationRunIdentity {
    engine: EngineIdentity,
}

impl SimulationRunIdentity {
    pub fn new(engine: EngineIdentity) -> Self {
        Self { engine }
    }
}

/// A checked result and the SHA-256 of its canonical JSON body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashedSimulationResult {
    result: SimulationResult,
    result_sha256: Sha256Digest,
}

impl HashedSimulationResult {
    pub fn result(&self) -> &SimulationResult {
        &self.result
    }

    pub fn result_sha256(&self) -> &Sha256Digest {
        &self.result_sha256
    }
}

/// Both cost-profile results. The pair is returned only after both results
/// pass contract validation and canonical hashing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceOnlySimulationResults {
    illustrative_10bp_per_side: HashedSimulationResult,
    zero_cost_reference: HashedSimulationResult,
}

impl PriceOnlySimulationResults {
    pub fn illustrative_10bp_per_side(&self) -> &HashedSimulationResult {
        &self.illustrative_10bp_per_side
    }

    pub fn zero_cost_reference(&self) -> &HashedSimulationResult {
        &self.zero_cost_reference
    }
}

/// Sanitized failures from simulation orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationRunError {
    #[error("approved historical artifact could not be verified")]
    ApprovalRejected,
    #[error("approved historical input or evaluation window is invalid")]
    InputInvalid,
    #[error("existing owner-beta signal computation failed")]
    SignalComputationFailed,
    #[error("existing factor definition identity is unavailable")]
    FactorIdentityUnavailable,
    #[error("simulation result contract validation failed")]
    ResultInvalid,
}

/// Approves the fixed v3 artifact once, computes the existing trend signals
/// for each prior session, simulates both approved cost profiles, then
/// validates and hashes both results before returning either.
pub fn run_approved_historical_price_only_simulation(
    artifact_root: &Path,
    request: &SimulationRunRequest,
    identity: &SimulationRunIdentity,
) -> Result<PriceOnlySimulationResults, SimulationRunError> {
    let factor_definition = derive_trend_factor_definition().map_err(map_signal_error)?;
    let prepared = approve_and_prepare(
        artifact_root,
        request.evaluation_start,
        request.evaluation_end,
    )
    .map_err(map_input_error)?;
    let computed = compute_signals(
        &prepared.artifact,
        &prepared.schedule,
        request.run_id,
        request.strategy_config_id,
        &factor_definition,
    )
    .map_err(map_signal_error)?;
    let strategy = StrategyIdentity::trend_following_v1(
        computed.strategy_snapshot_sha256.as_str(),
        &factor_definition.version,
        factor_definition.sha256.as_str(),
    )
    .map_err(|_| SimulationRunError::FactorIdentityUnavailable)?;
    let ledgers = simulate_ledgers(
        &prepared.schedule,
        &computed.signals,
        &prepared.evaluation_prices,
    )
    .map_err(|_| SimulationRunError::ResultInvalid)?;

    build_result_pair(
        &identity.engine,
        &strategy,
        &prepared.schedule,
        &computed.signals,
        ledgers.illustrative_10bp_per_side(),
        ledgers.zero_cost_reference(),
    )
}

fn build_result_pair(
    engine: &EngineIdentity,
    strategy: &StrategyIdentity,
    schedule: &SimulationSchedule,
    signals: &[SignalSnapshot],
    illustrative: &CostProfileLedgers,
    zero_cost: &CostProfileLedgers,
) -> Result<PriceOnlySimulationResults, SimulationRunError> {
    if illustrative.profile() != CostProfile::Illustrative10BpPerSideV1
        || zero_cost.profile() != CostProfile::ZeroCostReferenceV1
    {
        return Err(SimulationRunError::ResultInvalid);
    }

    let illustrative_10bp_per_side = hashed_result(
        engine,
        strategy,
        schedule,
        CostProfile::Illustrative10BpPerSideV1,
        signals,
        illustrative,
    )?;
    let zero_cost_reference = hashed_result(
        engine,
        strategy,
        schedule,
        CostProfile::ZeroCostReferenceV1,
        signals,
        zero_cost,
    )?;
    Ok(PriceOnlySimulationResults {
        illustrative_10bp_per_side,
        zero_cost_reference,
    })
}

fn hashed_result(
    engine: &EngineIdentity,
    strategy: &StrategyIdentity,
    schedule: &SimulationSchedule,
    profile: CostProfile,
    signals: &[SignalSnapshot],
    ledgers: &CostProfileLedgers,
) -> Result<HashedSimulationResult, SimulationRunError> {
    let result = SimulationResult::new(
        engine.clone(),
        strategy.clone(),
        schedule.clone(),
        profile,
        signals.to_vec(),
        ledgers.strategy().clone(),
        ledgers.benchmark().clone(),
    )
    .map_err(|_| SimulationRunError::ResultInvalid)?;
    let result_sha256 = result
        .result_content_sha256()
        .map_err(|_| SimulationRunError::ResultInvalid)?;
    Ok(HashedSimulationResult {
        result,
        result_sha256,
    })
}

fn map_input_error(error: SimulationInputError) -> SimulationRunError {
    match error {
        SimulationInputError::ApprovalRejected => SimulationRunError::ApprovalRejected,
        SimulationInputError::MetadataInvalid
        | SimulationInputError::BarsInvalid
        | SimulationInputError::InvalidWindow => SimulationRunError::InputInvalid,
    }
}

fn map_signal_error(error: SimulationSignalError) -> SimulationRunError {
    match error {
        SimulationSignalError::ComputationRejected | SimulationSignalError::SignalInvalid => {
            SimulationRunError::SignalComputationFailed
        }
        SimulationSignalError::FactorIdentityUnavailable => {
            SimulationRunError::FactorIdentityUnavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, NaiveDate, Weekday};
    use domain::TradingDate;
    use market_data::KR_ETF_CORE_SYMBOLS;

    use super::{
        CostProfile, EngineIdentity, PriceOnlySimulationResults, SimulationRunError,
        build_result_pair,
    };
    use crate::owner_beta::simulation::{
        EvaluationPriceRow, Price4, SignalSnapshot, SimulationSchedule, StrategyIdentity,
        simulate_ledgers,
    };

    fn synthetic_weekdays(count: usize) -> Vec<TradingDate> {
        let mut date = NaiveDate::from_ymd_opt(2020, 1, 1).expect("valid date");
        let mut sessions = Vec::with_capacity(count);
        while sessions.len() < count {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                sessions.push(
                    TradingDate::new(date.year(), date.month(), date.day()).expect("weekday"),
                );
            }
            date += Duration::days(1);
        }
        sessions
    }

    fn synthetic_signals(schedule: &SimulationSchedule) -> Vec<SignalSnapshot> {
        let mut targets = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| (format!("{symbol}.KRX"), 0))
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.0.cmp(&right.0));
        schedule
            .signal_execution_pairs()
            .into_iter()
            .map(|(signal_date, _)| {
                SignalSnapshot::new(
                    &signal_date.to_iso(),
                    &format!("sha256:{}", "a".repeat(64)),
                    &format!("sha256:{}", "b".repeat(64)),
                    targets.clone(),
                    1_000_000,
                )
                .expect("synthetic cash target")
            })
            .collect()
    }

    fn synthetic_identity() -> (EngineIdentity, StrategyIdentity) {
        let revision = "a".repeat(40);
        let binary = format!("sha256:{}", "c".repeat(64));
        let factor = format!("sha256:{}", "d".repeat(64));
        let engine = EngineIdentity::new(
            &revision,
            &binary,
            "rustc-test",
            "x86_64-unknown-linux-gnu",
            "test",
        )
        .expect("synthetic declarative engine identity");
        let strategy = StrategyIdentity::trend_following_v1(
            &format!("sha256:{}", "e".repeat(64)),
            "1.0.0",
            &factor,
        )
        .expect("synthetic strategy identity");
        (engine, strategy)
    }

    fn synthetic_results() -> PriceOnlySimulationResults {
        let sessions = synthetic_weekdays(202);
        let warmup = sessions[..200].iter().map(TradingDate::to_iso).collect();
        let evaluation = sessions[200..].iter().map(TradingDate::to_iso).collect();
        let schedule = SimulationSchedule::new(warmup, evaluation).expect("schedule");
        let signals = synthetic_signals(&schedule);
        let prices = schedule
            .evaluation_sessions()
            .iter()
            .map(|session| {
                EvaluationPriceRow::new(
                    &session.to_iso(),
                    Price4::parse("100.0000").expect("price"),
                    Price4::parse("101.0000").expect("price"),
                    1_000_000,
                )
                .expect("evaluation price")
            })
            .collect::<Vec<_>>();
        let ledgers = simulate_ledgers(&schedule, &signals, &prices).expect("paired ledgers");
        let (engine, strategy) = synthetic_identity();
        build_result_pair(
            &engine,
            &strategy,
            &schedule,
            &signals,
            ledgers.illustrative_10bp_per_side(),
            ledgers.zero_cost_reference(),
        )
        .expect("synthetic result pair")
    }

    #[test]
    fn synthetic_result_pair_is_canonically_hashed_and_deterministic() {
        let first = synthetic_results();
        let second = synthetic_results();
        assert_eq!(first, second);
        for result in [
            first.illustrative_10bp_per_side(),
            first.zero_cost_reference(),
        ] {
            assert_eq!(
                result.result().result_content_sha256().expect("hash"),
                *result.result_sha256()
            );
            assert!(!result.result_sha256().as_str().is_empty());
        }
        assert_eq!(
            first
                .illustrative_10bp_per_side()
                .result()
                .schedule()
                .evaluation_sessions()
                .len(),
            2
        );
    }

    #[test]
    fn profile_mismatch_returns_no_partial_comparison() {
        let sessions = synthetic_weekdays(202);
        let schedule = SimulationSchedule::new(
            sessions[..200].iter().map(TradingDate::to_iso).collect(),
            sessions[200..].iter().map(TradingDate::to_iso).collect(),
        )
        .expect("schedule");
        let signals = synthetic_signals(&schedule);
        let prices = schedule
            .evaluation_sessions()
            .iter()
            .map(|session| {
                EvaluationPriceRow::new(
                    &session.to_iso(),
                    Price4::parse("100.0000").expect("price"),
                    Price4::parse("101.0000").expect("price"),
                    1_000_000,
                )
                .expect("evaluation price")
            })
            .collect::<Vec<_>>();
        let ledgers = simulate_ledgers(&schedule, &signals, &prices).expect("paired ledgers");
        let (engine, strategy) = synthetic_identity();
        let result = build_result_pair(
            &engine,
            &strategy,
            &schedule,
            &signals,
            ledgers.illustrative_10bp_per_side(),
            ledgers.illustrative_10bp_per_side(),
        );
        assert_eq!(result, Err(SimulationRunError::ResultInvalid));
        assert_eq!(
            ledgers.zero_cost_reference().profile(),
            CostProfile::ZeroCostReferenceV1
        );
    }
}
