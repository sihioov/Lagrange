use std::fmt;

use domain::{ContentHash, TradingDate};
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::json;
use uuid::Uuid;

use crate::{
    owner_beta::{
        OwnerBetaPriceRecommendationInput, OwnerBetaPriceRecommendationPins,
        OwnerBetaStrategySnapshot,
        simulation::{
            APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256,
            APPROVED_BARS_SHA256, APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_CONTRACT_SHA256,
            APPROVED_REGISTRY_SHA256, APPROVED_STAGE5_MANIFEST_SHA256, SimulationInputError,
            SimulationSchedule, parse_strict_json, prepare_approved_simulation_window,
            reconstruct_approved_simulation_schedule,
        },
    },
    resolver::ResolvedConfig,
};

use super::{OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA, SimulationExecutionError};

/// Closed, validated input for one owner-beta price-only simulation job.
///
/// The cached schedule is reconstructed from the approved calendar and is
/// intentionally omitted from the serialized payload. This payload binds
/// dates and content pins; execution must still reapprove artifact bytes.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationJobInput {
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
    schedule: SimulationSchedule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationJobInputWire {
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

impl fmt::Debug for SimulationJobInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SimulationJobInput")
            .field(
                "job_type",
                &super::OWNER_BETA_PRICE_ONLY_SIMULATION_JOB_TYPE,
            )
            .field("evaluation_start", &self.evaluation_start)
            .field("evaluation_end", &self.evaluation_end)
            .field("strategy", &"<validated>")
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for SimulationJobInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SimulationJobInputWire::deserialize(deserializer)?;
        Self::from_wire(wire).map_err(D::Error::custom)
    }
}

impl SimulationJobInput {
    /// Builds a queue input from an already approved artifact. The shared
    /// helper validates the artifact envelope, bars, calendar, and date window.
    pub fn from_approved_artifact(
        run_id: Uuid,
        strategy_config_id: Uuid,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        evaluation_start: Option<TradingDate>,
        evaluation_end: Option<TradingDate>,
        resolved: &ResolvedConfig,
    ) -> Result<Self, SimulationExecutionError> {
        let schedule =
            prepare_approved_simulation_window(artifact, evaluation_start, evaluation_end)
                .map_err(map_input_error)?;
        let evaluation_start = *schedule
            .evaluation_sessions()
            .first()
            .ok_or(SimulationExecutionError::WindowInvalid)?;
        let evaluation_end = *schedule
            .evaluation_sessions()
            .last()
            .ok_or(SimulationExecutionError::WindowInvalid)?;

        let expected_strategy = expected_strategy_snapshot()?;
        let resolved_strategy = OwnerBetaStrategySnapshot::from_resolved_config(resolved)
            .map_err(|_| SimulationExecutionError::StrategyInvalid)?;
        if resolved_strategy != expected_strategy {
            return Err(SimulationExecutionError::StrategyInvalid);
        }

        let recommendation = OwnerBetaPriceRecommendationInput::from_approved_artifact(
            run_id,
            strategy_config_id,
            evaluation_start,
            artifact,
            resolved,
        )
        .map_err(|_| SimulationExecutionError::StrategyInvalid)?;

        let input = Self {
            schema: OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA.to_owned(),
            run_id,
            strategy_config_id,
            evaluation_start,
            evaluation_end,
            pins: recommendation.pins().clone(),
            contract_document_sha256: parse_hash(APPROVED_CONTRACT_SHA256)?,
            bars_sha256: parse_hash(APPROVED_BARS_SHA256)?,
            strategy: recommendation.strategy_snapshot().clone(),
            schedule,
        };
        input.validate_fixed_bindings()?;
        Ok(input)
    }

    /// Parses with the existing duplicate-key-aware JSON parser before serde
    /// constructs Values or typed fields.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SimulationExecutionError> {
        let value = parse_strict_json(bytes).map_err(|_| SimulationExecutionError::InputInvalid)?;
        let wire = serde_json::from_value::<SimulationJobInputWire>(value)
            .map_err(|_| SimulationExecutionError::InputInvalid)?;
        Self::from_wire(wire)
    }

    fn from_wire(wire: SimulationJobInputWire) -> Result<Self, SimulationExecutionError> {
        let schedule =
            reconstruct_approved_simulation_schedule(wire.evaluation_start, wire.evaluation_end)
                .map_err(map_input_error)?;
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
        if self.schema != OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA {
            return Err(SimulationExecutionError::InputInvalid);
        }
        if !pins_are_approved(&self.pins)
            || self.contract_document_sha256.as_str() != APPROVED_CONTRACT_SHA256
            || self.bars_sha256.as_str() != APPROVED_BARS_SHA256
        {
            return Err(SimulationExecutionError::ProvenanceMismatch);
        }
        let expected_strategy = expected_strategy_snapshot()?;
        if self.strategy != expected_strategy {
            return Err(SimulationExecutionError::StrategyInvalid);
        }
        if self.schedule.evaluation_sessions().first() != Some(&self.evaluation_start)
            || self.schedule.evaluation_sessions().last() != Some(&self.evaluation_end)
        {
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

    /// Returns the immutable calendar schedule reconstructed during
    /// construction or deserialization.
    pub fn schedule(&self) -> &SimulationSchedule {
        &self.schedule
    }
}

fn expected_strategy_snapshot() -> Result<OwnerBetaStrategySnapshot, SimulationExecutionError> {
    OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
        strategy_id: "trend_following".to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config: json!({
            "benchmark_instrument": "069500.KRX",
            "fast_ma": 50,
            "slow_ma": 200
        }),
    })
    .map_err(|_| SimulationExecutionError::StrategyInvalid)
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

fn map_input_error(error: SimulationInputError) -> SimulationExecutionError {
    match error {
        SimulationInputError::InvalidWindow => SimulationExecutionError::WindowInvalid,
        SimulationInputError::ApprovalRejected
        | SimulationInputError::MetadataInvalid
        | SimulationInputError::BarsInvalid => SimulationExecutionError::InputInvalid,
    }
}
