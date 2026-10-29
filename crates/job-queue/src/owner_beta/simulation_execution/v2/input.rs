//! Version-2 queue input reconstructed against the fixed approved calendar.
//!
//! Wire reconstruction binds dates, pins, and exact strategy defaults. It does
//! not prove actual factor readiness; admission and execution use the retained
//! prepared-input seam to establish that separately.

use domain::{ContentHash, TradingDate};
use market_data::range_normalize::ExpectedRangeSessions;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    owner_beta::{
        OwnerBetaPriceRecommendationInput, OwnerBetaPriceRecommendationPins,
        OwnerBetaStrategySnapshot,
        simulation::{
            APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256,
            APPROVED_BARS_SHA256, APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_END_DATE,
            APPROVED_REGISTRY_SHA256, APPROVED_SESSION_COUNT, APPROVED_STAGE5_MANIFEST_SHA256,
            APPROVED_START_DATE,
            v2::{
                APPROVED_CONTRACT_SHA256 as APPROVED_V2_CONTRACT_SHA256, V2InputError, V2Schedule,
                V2SimulationRunError, V2Strategy, prepare_v2_simulation_schedule,
            },
        },
    },
    resolver::ResolvedConfig,
};

use super::super::{OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2, SimulationExecutionError};

/// Closed v2 input for the exact adopted strategy defaults and one retained
/// sealed artifact. Deserialization is intentionally available only through
/// `from_json`, which performs duplicate-key-aware parsing first.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct V2SimulationJobInput {
    schema: String,
    run_id: Uuid,
    strategy_config_id: Uuid,
    evaluation_start: TradingDate,
    evaluation_end: TradingDate,
    pins: OwnerBetaPriceRecommendationPins,
    contract_document_sha256: ContentHash,
    bars_sha256: ContentHash,
    strategy: OwnerBetaStrategySnapshot,
    #[serde(skip)]
    schedule: V2Schedule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V2SimulationJobInputWire {
    schema: String,
    run_id: Uuid,
    strategy_config_id: Uuid,
    evaluation_start: TradingDate,
    evaluation_end: TradingDate,
    pins: OwnerBetaPriceRecommendationPins,
    contract_document_sha256: ContentHash,
    bars_sha256: ContentHash,
    strategy: OwnerBetaStrategySnapshot,
}

impl V2SimulationJobInput {
    /// Creates the queue input from the already approved and retained artifact.
    /// The shared WP-2 seam establishes real common readiness without running
    /// either result ledger.
    pub fn from_prepared(
        prepared: &crate::owner_beta::simulation::v2::V2PreparedInput,
        run_id: Uuid,
        strategy_config_id: Uuid,
        resolved: &ResolvedConfig,
        evaluation_start: Option<TradingDate>,
        evaluation_end: Option<TradingDate>,
    ) -> Result<Self, SimulationExecutionError> {
        let strategy = OwnerBetaStrategySnapshot::from_resolved_config(resolved)
            .map_err(|_| SimulationExecutionError::StrategyInvalid)?;
        let v2_strategy = validate_v2_strategy_snapshot(&strategy)?;
        let schedule = prepare_v2_simulation_schedule(
            prepared,
            run_id,
            strategy_config_id,
            resolved,
            evaluation_start,
            evaluation_end,
        )
        .map_err(map_v2_run_error)?;
        if schedule.strategy() != v2_strategy {
            return Err(SimulationExecutionError::StrategyInvalid);
        }

        // This existing adapter copies only the already approved pins and a
        // strategy snapshot from the retained artifact. It does not reopen it.
        let recommendation = OwnerBetaPriceRecommendationInput::from_approved_artifact(
            run_id,
            strategy_config_id,
            schedule.evaluation_start(),
            &prepared.approved,
            resolved,
        )
        .map_err(|_| SimulationExecutionError::StrategyInvalid)?;
        if recommendation.strategy_snapshot() != &strategy
            || !pins_are_approved(recommendation.pins())
        {
            return Err(SimulationExecutionError::ProvenanceMismatch);
        }

        let input = Self {
            schema: OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2.to_owned(),
            run_id,
            strategy_config_id,
            evaluation_start: schedule.evaluation_start(),
            evaluation_end: schedule.evaluation_end(),
            pins: recommendation.pins().clone(),
            contract_document_sha256: parse_hash(APPROVED_V2_CONTRACT_SHA256)?,
            bars_sha256: parse_hash(APPROVED_BARS_SHA256)?,
            strategy,
            schedule,
        };
        input.validate_fixed_bindings()?;
        Ok(input)
    }

    /// Parses strict JSON, rejecting duplicate keys before serde constructs a
    /// map, then accepts only the exact v2 schema and closed wire fields.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SimulationExecutionError> {
        let value = super::super::versioned::parse_versioned_json(bytes)?;
        Self::from_value(value)
    }

    fn from_value(value: Value) -> Result<Self, SimulationExecutionError> {
        let wire = serde_json::from_value::<V2SimulationJobInputWire>(value)
            .map_err(|_| SimulationExecutionError::InputInvalid)?;
        if wire.schema != OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2 {
            return Err(SimulationExecutionError::InputInvalid);
        }
        let v2_strategy = validate_v2_strategy_snapshot(&wire.strategy)?;
        let schedule =
            reconstruct_v2_schedule(v2_strategy, wire.evaluation_start, wire.evaluation_end)?;
        let input = Self {
            schema: wire.schema,
            run_id: wire.run_id,
            strategy_config_id: wire.strategy_config_id,
            evaluation_start: wire.evaluation_start,
            evaluation_end: wire.evaluation_end,
            pins: wire.pins,
            contract_document_sha256: wire.contract_document_sha256,
            bars_sha256: wire.bars_sha256,
            strategy: wire.strategy,
            schedule,
        };
        input.validate_fixed_bindings()?;
        Ok(input)
    }

    fn validate_fixed_bindings(&self) -> Result<(), SimulationExecutionError> {
        if self.schema != OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2 {
            return Err(SimulationExecutionError::InputInvalid);
        }
        if !pins_are_approved(&self.pins)
            || self.contract_document_sha256.as_str() != APPROVED_V2_CONTRACT_SHA256
            || self.bars_sha256.as_str() != APPROVED_BARS_SHA256
        {
            return Err(SimulationExecutionError::ProvenanceMismatch);
        }
        let strategy = validate_v2_strategy_snapshot(&self.strategy)?;
        if self.schedule.strategy() != strategy
            || self.schedule.evaluation_start() != self.evaluation_start
            || self.schedule.evaluation_end() != self.evaluation_end
        {
            return Err(SimulationExecutionError::WindowInvalid);
        }
        let reconstructed =
            reconstruct_v2_schedule(strategy, self.evaluation_start, self.evaluation_end)?;
        if reconstructed != self.schedule {
            return Err(SimulationExecutionError::WindowInvalid);
        }
        Ok(())
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub fn strategy_config_id(&self) -> Uuid {
        self.strategy_config_id
    }

    pub fn evaluation_start(&self) -> TradingDate {
        self.evaluation_start
    }

    pub fn evaluation_end(&self) -> TradingDate {
        self.evaluation_end
    }

    pub fn pins(&self) -> &OwnerBetaPriceRecommendationPins {
        &self.pins
    }

    pub fn contract_document_sha256(&self) -> &ContentHash {
        &self.contract_document_sha256
    }

    pub fn bars_sha256(&self) -> &ContentHash {
        &self.bars_sha256
    }

    pub fn strategy(&self) -> &OwnerBetaStrategySnapshot {
        &self.strategy
    }

    pub fn schedule(&self) -> &V2Schedule {
        &self.schedule
    }

    /// Returns the whole fixed preparation prefix followed by evaluation
    /// sessions. A decoded wire proves only calendar binding; production
    /// admission and execution must re-establish actual factor readiness.
    pub fn required_dates(&self) -> Vec<TradingDate> {
        self.schedule
            .preparation_sessions()
            .iter()
            .chain(self.schedule.evaluation_sessions())
            .copied()
            .collect()
    }
}

fn validate_v2_strategy_snapshot(
    snapshot: &OwnerBetaStrategySnapshot,
) -> Result<V2Strategy, SimulationExecutionError> {
    let strategy = V2Strategy::ALL
        .into_iter()
        .find(|strategy| strategy.as_str() == snapshot.strategy_id())
        .ok_or(SimulationExecutionError::StrategyInvalid)?;
    if snapshot.strategy_version() != "1.0.0" {
        return Err(SimulationExecutionError::StrategyInvalid);
    }
    let config = serde_json::from_str::<Value>(strategy.default_config_json())
        .map_err(|_| SimulationExecutionError::StrategyInvalid)?;
    let expected = OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
        strategy_id: strategy.as_str().to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config,
    })
    .map_err(|_| SimulationExecutionError::StrategyInvalid)?;
    if snapshot != &expected {
        return Err(SimulationExecutionError::StrategyInvalid);
    }
    Ok(strategy)
}

fn reconstruct_v2_schedule(
    strategy: V2Strategy,
    evaluation_start: TradingDate,
    evaluation_end: TradingDate,
) -> Result<V2Schedule, SimulationExecutionError> {
    let start = TradingDate::parse(APPROVED_START_DATE)
        .map_err(|_| SimulationExecutionError::InputInvalid)?;
    let end = TradingDate::parse(APPROVED_END_DATE)
        .map_err(|_| SimulationExecutionError::InputInvalid)?;
    let expected = ExpectedRangeSessions::approved_xkrx(start, end)
        .map_err(|_| SimulationExecutionError::InputInvalid)?;
    let sessions = expected.sessions;
    if sessions.len() != APPROVED_SESSION_COUNT as usize
        || sessions.first() != Some(&start)
        || sessions.last() != Some(&end)
        || sessions.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(SimulationExecutionError::InputInvalid);
    }
    let start_index = sessions
        .iter()
        .position(|date| *date == evaluation_start)
        .ok_or(SimulationExecutionError::WindowInvalid)?;
    let end_index = sessions
        .iter()
        .position(|date| *date == evaluation_end)
        .ok_or(SimulationExecutionError::WindowInvalid)?;
    // The 200-close check is only the fixed contract's necessary lower bound.
    // This calendar reconstruction does not prove any strategy is ready.
    if start_index < 200 || end_index <= start_index {
        return Err(SimulationExecutionError::WindowInvalid);
    }
    let preparation = sessions[..start_index]
        .iter()
        .map(TradingDate::to_iso)
        .collect::<Vec<_>>();
    let evaluation = sessions[start_index..=end_index]
        .iter()
        .map(TradingDate::to_iso)
        .collect::<Vec<_>>();
    V2Schedule::new(strategy, preparation, evaluation)
        .map_err(|_| SimulationExecutionError::WindowInvalid)
}

fn pins_are_approved(pins: &OwnerBetaPriceRecommendationPins) -> bool {
    pins.candidate_content_sha256().as_str() == APPROVED_CANDIDATE_CONTENT_SHA256
        && pins.artifact_manifest_sha256().as_str() == APPROVED_ARTIFACT_MANIFEST_SHA256
        && pins.stage5_manifest_sha256().as_str() == APPROVED_STAGE5_MANIFEST_SHA256
        && pins.action_manifest_sha256().as_str() == APPROVED_ACTION_MANIFEST_SHA256
        && pins.approval_registry_sha256().as_str() == APPROVED_REGISTRY_SHA256
}

fn parse_hash(value: &str) -> Result<ContentHash, SimulationExecutionError> {
    ContentHash::parse(value).map_err(|_| SimulationExecutionError::ProvenanceMismatch)
}

fn map_v2_run_error(error: V2SimulationRunError) -> SimulationExecutionError {
    match error {
        V2SimulationRunError::InvalidRequest => SimulationExecutionError::StrategyInvalid,
        V2SimulationRunError::Input(V2InputError::InvalidWindow) => {
            SimulationExecutionError::WindowInvalid
        }
        _ => SimulationExecutionError::InputInvalid,
    }
}
