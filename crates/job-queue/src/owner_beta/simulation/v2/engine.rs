//! Pure v2 orchestration from one retained sealed artifact to a checked pair.

use std::{collections::BTreeMap, path::Path};

use domain::{ContentHash, TradingDate};
use uuid::Uuid;

use crate::{
    owner_beta::simulation::{CostProfile, EngineIdentity, Sha256Digest},
    resolver::ResolvedConfig,
};

use super::{
    V2ContractError, V2Ledger, V2LedgerRun, V2MarketSession, V2Schedule, V2SimulationPair,
    V2SimulationResult, V2Strategy, V2TargetAllocation,
    input::{V2InputError, V2PreparedInput, select_window},
    signals::{
        V2SignalError, compute_target_at, default_config, first_common_ready_start,
        strategy_for_config, strategy_identity,
    },
};

/// Sanitized failures from one pure v2 run. Variants contain no request,
/// path, artifact, factor, or provider data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum V2SimulationRunError {
    #[error("v2 simulation request is invalid")]
    InvalidRequest,
    #[error(transparent)]
    Input(#[from] V2InputError),
    #[error(transparent)]
    Signal(#[from] V2SignalError),
    #[error(transparent)]
    Contract(#[from] V2ContractError),
    #[error("v2 result pair validation or hashing failed")]
    ResultInvalid,
}

/// One immutable request for a fixed default strategy.
///
/// Run and strategy-config identifiers are passed only to the existing
/// computation input. They are not copied into the financial result.
pub struct V2SimulationRunRequest {
    run_id: Uuid,
    strategy_config_id: Uuid,
    resolved_config: ResolvedConfig,
    engine: EngineIdentity,
    evaluation_start: Option<TradingDate>,
    evaluation_end: Option<TradingDate>,
}

impl V2SimulationRunRequest {
    pub fn new(
        run_id: Uuid,
        strategy_config_id: Uuid,
        resolved_config: ResolvedConfig,
        engine: EngineIdentity,
    ) -> Result<Self, V2SimulationRunError> {
        strategy_for_config(&resolved_config).map_err(|_| V2SimulationRunError::InvalidRequest)?;
        Ok(Self {
            run_id,
            strategy_config_id,
            resolved_config,
            engine,
            evaluation_start: None,
            evaluation_end: None,
        })
    }

    /// Applies optional dates after the common readiness start is established.
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

/// A pair plus the SHA-256 of each canonical financial result body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2HashedSimulationPair {
    pair: V2SimulationPair,
    illustrative_sha256: Sha256Digest,
    zero_cost_sha256: Sha256Digest,
}

impl V2HashedSimulationPair {
    pub fn pair(&self) -> &V2SimulationPair {
        &self.pair
    }

    pub fn illustrative_sha256(&self) -> &Sha256Digest {
        &self.illustrative_sha256
    }

    pub fn zero_cost_sha256(&self) -> &Sha256Digest {
        &self.zero_cost_sha256
    }
}

/// Approves the fixed artifact exactly once and immediately retains its
/// validated object for signal calculation.
pub fn prepare_v2_simulation_input(artifact_root: &Path) -> Result<V2PreparedInput, V2InputError> {
    V2PreparedInput::approve(artifact_root)
}

/// Production entrypoint: prepare the fixed v3 input and return only after
/// both cost profiles, both benchmarks, canonical bodies, and hashes validate.
pub fn run_v2_simulation(
    artifact_root: &Path,
    request: &V2SimulationRunRequest,
) -> Result<V2HashedSimulationPair, V2SimulationRunError> {
    let prepared = prepare_v2_simulation_input(artifact_root)?;
    run_prepared_v2_simulation(&prepared, request)
}

/// Runs against a retained sealed object. This seam allows later admission to
/// prepare once and reuse the same verified bytes without filesystem access.
pub fn run_prepared_v2_simulation(
    prepared: &V2PreparedInput,
    request: &V2SimulationRunRequest,
) -> Result<V2HashedSimulationPair, V2SimulationRunError> {
    let schedule = prepare_v2_simulation_schedule(
        prepared,
        request.run_id,
        request.strategy_config_id,
        &request.resolved_config,
        request.evaluation_start,
        request.evaluation_end,
    )?;
    let strategy = schedule.strategy();
    let benchmark_schedule = V2Schedule::new(
        V2Strategy::BuyAndHold,
        schedule
            .preparation_sessions()
            .iter()
            .map(TradingDate::to_iso)
            .collect(),
        schedule
            .evaluation_sessions()
            .iter()
            .map(TradingDate::to_iso)
            .collect(),
    )?;
    let strategy_identity = strategy_identity(&request.resolved_config, strategy)?;

    let strategy_targets = targets_for_decisions(
        prepared,
        request.run_id,
        request.strategy_config_id,
        &request.resolved_config,
        &schedule,
    )?;
    let benchmark_resolved = default_config(V2Strategy::BuyAndHold)?;
    let benchmark_signal_date = schedule
        .decisions()
        .first()
        .ok_or(V2SimulationRunError::ResultInvalid)?
        .signal_date();
    let benchmark_target = compute_target_at(
        &prepared.approved,
        request.run_id,
        request.strategy_config_id,
        &benchmark_resolved,
        V2Strategy::BuyAndHold,
        benchmark_signal_date,
    )?
    .ok_or(V2SignalError::RequiredSignalUnavailable)?;
    let benchmark_targets = BTreeMap::from([(benchmark_signal_date, benchmark_target)]);

    let market_sessions = schedule
        .evaluation_sessions()
        .iter()
        .map(|date| prepared.market_session(*date))
        .collect::<Result<Vec<_>, _>>()?;
    let illustrative_ledger = run_ledger(
        CostProfile::Illustrative10BpPerSideV1,
        &schedule,
        &strategy_targets,
        &market_sessions,
    )?;
    let zero_cost_ledger = run_ledger(
        CostProfile::ZeroCostReferenceV1,
        &schedule,
        &strategy_targets,
        &market_sessions,
    )?;
    let illustrative_benchmark = run_ledger(
        CostProfile::Illustrative10BpPerSideV1,
        &benchmark_schedule,
        &benchmark_targets,
        &market_sessions,
    )?;
    let zero_cost_benchmark = run_ledger(
        CostProfile::ZeroCostReferenceV1,
        &benchmark_schedule,
        &benchmark_targets,
        &market_sessions,
    )?;

    let illustrative = V2SimulationResult::new(
        strategy_identity.clone(),
        request.engine.clone(),
        schedule.clone(),
        CostProfile::Illustrative10BpPerSideV1,
        illustrative_ledger,
        illustrative_benchmark,
    )?;
    let zero_cost = V2SimulationResult::new(
        strategy_identity,
        request.engine.clone(),
        schedule,
        CostProfile::ZeroCostReferenceV1,
        zero_cost_ledger,
        zero_cost_benchmark,
    )?;
    let pair = V2SimulationPair::new(illustrative, zero_cost)?;
    let (illustrative_body, zero_cost_body) = pair.canonical_json()?;
    let illustrative_sha256 = digest(&illustrative_body)?;
    let zero_cost_sha256 = digest(&zero_cost_body)?;
    Ok(V2HashedSimulationPair {
        pair,
        illustrative_sha256,
        zero_cost_sha256,
    })
}

/// Establishes the real common-ready schedule from an already sealed input.
/// Admission and execution share this path; reconstructing queue dates alone
/// does not prove factor readiness. No ledger or result is computed here.
pub fn prepare_v2_simulation_schedule(
    prepared: &V2PreparedInput,
    run_id: Uuid,
    strategy_config_id: Uuid,
    resolved: &ResolvedConfig,
    evaluation_start: Option<TradingDate>,
    evaluation_end: Option<TradingDate>,
) -> Result<V2Schedule, V2SimulationRunError> {
    let strategy =
        strategy_for_config(resolved).map_err(|_| V2SimulationRunError::InvalidRequest)?;
    let first_ready_start = first_common_ready_start(prepared, run_id, strategy_config_id)?;
    let (start_index, end_index) = select_window(
        prepared.sessions(),
        first_ready_start,
        evaluation_start,
        evaluation_end,
    )?;
    if start_index > first_ready_start {
        validate_common_readiness(
            prepared,
            run_id,
            strategy_config_id,
            prepared.sessions()[start_index - 1],
        )?;
    }

    let preparation_dates = prepared.sessions()[..start_index]
        .iter()
        .map(TradingDate::to_iso)
        .collect::<Vec<_>>();
    let evaluation_dates = prepared.sessions()[start_index..=end_index]
        .iter()
        .map(TradingDate::to_iso)
        .collect::<Vec<_>>();
    V2Schedule::new(strategy, preparation_dates, evaluation_dates).map_err(Into::into)
}

fn validate_common_readiness(
    prepared: &V2PreparedInput,
    run_id: Uuid,
    strategy_config_id: Uuid,
    signal_date: TradingDate,
) -> Result<(), V2SignalError> {
    for strategy in V2Strategy::ALL {
        let resolved = default_config(strategy)?;
        if compute_target_at(
            &prepared.approved,
            run_id,
            strategy_config_id,
            &resolved,
            strategy,
            signal_date,
        )?
        .is_none()
        {
            return Err(V2SignalError::InsufficientReadiness);
        }
    }
    Ok(())
}

fn targets_for_decisions(
    prepared: &V2PreparedInput,
    run_id: Uuid,
    strategy_config_id: Uuid,
    resolved: &ResolvedConfig,
    schedule: &V2Schedule,
) -> Result<BTreeMap<TradingDate, V2TargetAllocation>, V2SimulationRunError> {
    let mut targets = BTreeMap::new();
    for decision in schedule.decisions() {
        let target = compute_target_at(
            &prepared.approved,
            run_id,
            strategy_config_id,
            resolved,
            schedule.strategy(),
            decision.signal_date(),
        )?
        .ok_or(V2SignalError::RequiredSignalUnavailable)?;
        if targets.insert(decision.signal_date(), target).is_some() {
            return Err(V2SimulationRunError::ResultInvalid);
        }
    }
    Ok(targets)
}

fn run_ledger(
    profile: CostProfile,
    schedule: &V2Schedule,
    targets: &BTreeMap<TradingDate, V2TargetAllocation>,
    market_sessions: &[V2MarketSession],
) -> Result<V2LedgerRun, V2SimulationRunError> {
    if market_sessions
        .iter()
        .map(V2MarketSession::date)
        .ne(schedule.evaluation_sessions().iter().copied())
    {
        return Err(V2SimulationRunError::ResultInvalid);
    }
    let mut ledger = V2Ledger::new(profile)?;
    for market_session in market_sessions {
        let execution_date = market_session.date();
        ledger.open_session(market_session.clone())?;
        if let Some(decision) = schedule.decision_for_execution(execution_date) {
            let target = targets
                .get(&decision.signal_date())
                .cloned()
                .ok_or(V2SimulationRunError::ResultInvalid)?;
            if schedule.strategy() == V2Strategy::TrendFollowing {
                ledger.observe_trend_target(schedule, decision, target)?;
            } else {
                ledger.rebalance(decision, target)?;
            }
        }
        ledger.close_session()?;
    }
    Ok(ledger.finish(schedule)?)
}

fn digest(body: &[u8]) -> Result<Sha256Digest, V2SimulationRunError> {
    let hash = ContentHash::from_bytes(body);
    Sha256Digest::parse(hash.as_str()).map_err(|_| V2SimulationRunError::ResultInvalid)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use domain::{ContentHash, TradingDate};

    use crate::{
        owner_beta::simulation::{CostProfile, EngineIdentity},
        resolver::ResolvedConfig,
    };

    use super::{V2SimulationRunError, V2SimulationRunRequest, run_ledger, strategy_identity};
    use crate::owner_beta::simulation::v2::{
        V2MarketSession, V2Schedule, V2SimulationPair, V2SimulationResult, V2Strategy,
        V2TargetAllocation, contract::approved_instruments,
    };

    fn engine_identity() -> EngineIdentity {
        EngineIdentity::new(
            "0123456789abcdef0123456789abcdef01234567",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "rustc-1.97.1",
            "x86_64-unknown-linux-gnu",
            "release",
        )
        .expect("syntactically valid caller-attested identity")
    }

    #[test]
    fn run_request_accepts_exact_defaults_and_rejects_custom_values() {
        let exact = ResolvedConfig {
            strategy_id: V2Strategy::TrendFollowing.as_str().to_owned(),
            strategy_version: "1.0.0".to_owned(),
            config: serde_json::from_str(V2Strategy::TrendFollowing.default_config_json())
                .expect("config"),
        };
        assert!(
            V2SimulationRunRequest::new(
                uuid::Uuid::nil(),
                uuid::Uuid::nil(),
                exact.clone(),
                engine_identity(),
            )
            .is_ok()
        );

        let mut custom = exact;
        custom.config["slow_ma"] = serde_json::json!(201);
        assert!(matches!(
            V2SimulationRunRequest::new(
                uuid::Uuid::nil(),
                uuid::Uuid::nil(),
                custom,
                engine_identity(),
            ),
            Err(V2SimulationRunError::InvalidRequest)
        ));
    }

    #[test]
    fn synthetic_checked_buy_hold_pair_reuses_full_market_and_target_paths() {
        let strategy = V2Strategy::BuyAndHold;
        let resolved = ResolvedConfig {
            strategy_id: strategy.as_str().to_owned(),
            strategy_version: "1.0.0".to_owned(),
            config: serde_json::from_str(strategy.default_config_json()).expect("config"),
        };
        let identity = strategy_identity(&resolved, strategy).expect("identity");
        let schedule = V2Schedule::new(
            strategy,
            vec!["2020-01-02".to_owned()],
            vec!["2020-01-03".to_owned(), "2020-01-06".to_owned()],
        )
        .expect("schedule");
        let synthetic_target_hash = ContentHash::from_bytes(b"synthetic checked-ledger target");
        let target = V2TargetAllocation::new(
            synthetic_target_hash.as_str(),
            approved_instruments()
                .into_iter()
                .map(|instrument| {
                    let weight = if instrument == "069500.KRX" {
                        1_000_000
                    } else {
                        0
                    };
                    (instrument, weight)
                })
                .collect(),
            0,
        )
        .expect("target");
        let signal_date = TradingDate::new(2020, 1, 2).expect("date");
        let targets = BTreeMap::from([(signal_date, target)]);
        let market = ["2020-01-03", "2020-01-06"]
            .into_iter()
            .map(|date| {
                V2MarketSession::new(
                    date,
                    approved_instruments()
                        .into_iter()
                        .map(|instrument| {
                            (
                                instrument,
                                "100.0000".to_owned(),
                                "100.0000".to_owned(),
                                1_000,
                            )
                        })
                        .collect(),
                )
                .expect("full market session")
            })
            .collect::<Vec<_>>();
        let benchmark_schedule = V2Schedule::new(
            V2Strategy::BuyAndHold,
            schedule
                .preparation_sessions()
                .iter()
                .map(TradingDate::to_iso)
                .collect(),
            schedule
                .evaluation_sessions()
                .iter()
                .map(TradingDate::to_iso)
                .collect(),
        )
        .expect("benchmark schedule");
        let illustrative = run_ledger(
            CostProfile::Illustrative10BpPerSideV1,
            &schedule,
            &targets,
            &market,
        )
        .expect("illustrative ledger");
        let illustrative_benchmark = run_ledger(
            CostProfile::Illustrative10BpPerSideV1,
            &benchmark_schedule,
            &targets,
            &market,
        )
        .expect("illustrative benchmark");
        let zero = run_ledger(
            CostProfile::ZeroCostReferenceV1,
            &schedule,
            &targets,
            &market,
        )
        .expect("zero-cost ledger");
        let zero_benchmark = run_ledger(
            CostProfile::ZeroCostReferenceV1,
            &benchmark_schedule,
            &targets,
            &market,
        )
        .expect("zero-cost benchmark");
        let illustrative_result = V2SimulationResult::new(
            identity.clone(),
            engine_identity(),
            schedule.clone(),
            CostProfile::Illustrative10BpPerSideV1,
            illustrative,
            illustrative_benchmark,
        )
        .expect("checked illustrative result");
        let zero_result = V2SimulationResult::new(
            identity,
            engine_identity(),
            schedule,
            CostProfile::ZeroCostReferenceV1,
            zero,
            zero_benchmark,
        )
        .expect("checked zero-cost result");
        let pair =
            V2SimulationPair::new(illustrative_result, zero_result).expect("checked atomic pair");

        for result in [
            pair.illustrative_10bp_per_side(),
            pair.zero_cost_reference(),
        ] {
            assert_eq!(
                result.ledger().market_sessions(),
                result.benchmark().market_sessions()
            );
            assert_eq!(
                result.ledger().target_observations(),
                result.benchmark().target_observations()
            );
        }
        assert_eq!(
            pair.illustrative_10bp_per_side().ledger().ending_nav(),
            pair.illustrative_10bp_per_side().benchmark().ending_nav()
        );
        assert!(
            pair.canonical_json()
                .expect("canonical pair bodies")
                .0
                .starts_with(b"{")
        );
    }
}
