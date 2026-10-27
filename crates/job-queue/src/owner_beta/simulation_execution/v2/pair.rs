//! Atomic v2 whole-pair validation for stored canonical result bodies.

use std::fmt;

use domain::ContentHash;
use serde::Serialize;

use crate::owner_beta::simulation::{
    EngineIdentity, Sha256Digest,
    v2::{V2InputIdentity, V2SimulationPair, V2SimulationResult},
};

use super::super::SimulationExecutionError;
use super::V2SimulationJobInput;

const MAX_CANONICAL_PAIR_BYTES: usize = 64 * 1024 * 1024;

/// Both original canonical v2 result bodies after schema, economic, path,
/// input, schedule, and engine bindings have been checked as one unit.
pub struct V2CheckedSimulationPair {
    illustrative_bytes: Vec<u8>,
    zero_cost_bytes: Vec<u8>,
    pair: V2SimulationPair,
    illustrative_sha256: Sha256Digest,
    zero_cost_sha256: Sha256Digest,
}

impl fmt::Debug for V2CheckedSimulationPair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("V2CheckedSimulationPair")
            .field("illustrative_sha256", &self.illustrative_sha256)
            .field("zero_cost_sha256", &self.zero_cost_sha256)
            .field("canonical_bytes", &"<checked>")
            .finish()
    }
}

impl V2CheckedSimulationPair {
    /// Validates both v2 bodies and their shared semantics before retaining
    /// either original byte slice. `expected_engine` is a content binding,
    /// not proof that a particular executable produced the results.
    pub fn from_canonical(
        illustrative: &[u8],
        zero_cost: &[u8],
        input: &V2SimulationJobInput,
        expected_engine: &EngineIdentity,
    ) -> Result<Self, SimulationExecutionError> {
        let combined_size = illustrative
            .len()
            .checked_add(zero_cost.len())
            .ok_or(SimulationExecutionError::SizeLimitExceeded)?;
        if combined_size > MAX_CANONICAL_PAIR_BYTES {
            return Err(SimulationExecutionError::SizeLimitExceeded);
        }

        let illustrative_result = V2SimulationResult::from_json(illustrative)
            .map_err(|_| SimulationExecutionError::PairInvalid)?;
        require_canonical_bytes(illustrative, &illustrative_result)?;
        let zero_cost_result = V2SimulationResult::from_json(zero_cost)
            .map_err(|_| SimulationExecutionError::PairInvalid)?;
        require_canonical_bytes(zero_cost, &zero_cost_result)?;

        let pair = V2SimulationPair::new(illustrative_result, zero_cost_result)
            .map_err(|_| SimulationExecutionError::PairInvalid)?;
        let expected_input = V2InputIdentity::approved_v3_etf11()
            .map_err(|_| SimulationExecutionError::PairInvalid)?;
        for result in [
            pair.illustrative_10bp_per_side(),
            pair.zero_cost_reference(),
        ] {
            if result.schedule() != input.schedule() {
                return Err(SimulationExecutionError::WindowInvalid);
            }
            let strategy = result.strategy().strategy();
            if strategy != input.schedule().strategy()
                || strategy.as_str() != input.strategy().strategy_id()
                || result.strategy().version() != input.strategy().strategy_version()
                || result.strategy().snapshot_sha256().as_str()
                    != input.strategy().config_sha256().as_str()
                || result.strategy().canonical_config_json() != strategy.default_config_json()
            {
                return Err(SimulationExecutionError::StrategyInvalid);
            }
            if !serialized_field_matches(result, "engine", expected_engine)?
                || !serialized_field_matches(result, "input", &expected_input)?
            {
                return Err(SimulationExecutionError::ProvenanceMismatch);
            }
        }

        let illustrative_sha256 = hash_original_bytes(illustrative)?;
        let zero_cost_sha256 = hash_original_bytes(zero_cost)?;
        Ok(Self {
            illustrative_bytes: illustrative.to_vec(),
            zero_cost_bytes: zero_cost.to_vec(),
            pair,
            illustrative_sha256,
            zero_cost_sha256,
        })
    }

    pub fn illustrative_bytes(&self) -> &[u8] {
        &self.illustrative_bytes
    }

    pub fn zero_cost_bytes(&self) -> &[u8] {
        &self.zero_cost_bytes
    }

    pub fn pair(&self) -> &V2SimulationPair {
        &self.pair
    }

    pub fn illustrative_result(&self) -> &V2SimulationResult {
        self.pair.illustrative_10bp_per_side()
    }

    pub fn zero_cost_result(&self) -> &V2SimulationResult {
        self.pair.zero_cost_reference()
    }

    pub fn illustrative_sha256(&self) -> &Sha256Digest {
        &self.illustrative_sha256
    }

    pub fn zero_cost_sha256(&self) -> &Sha256Digest {
        &self.zero_cost_sha256
    }
}

fn require_canonical_bytes(
    original: &[u8],
    result: &V2SimulationResult,
) -> Result<(), SimulationExecutionError> {
    let canonical = result
        .canonical_json()
        .map_err(|_| SimulationExecutionError::PairInvalid)?;
    if canonical.as_slice() != original {
        return Err(SimulationExecutionError::PairInvalid);
    }
    Ok(())
}

/// The v2 result contract currently has no typed engine/input accessors. The
/// result has already passed its duplicate-aware typed parser; compare its
/// serialized checked fields to the typed expected identities here.
fn serialized_field_matches<T: Serialize, E: Serialize>(
    result: &T,
    field: &str,
    expected: &E,
) -> Result<bool, SimulationExecutionError> {
    let result = serde_json::to_value(result).map_err(|_| SimulationExecutionError::PairInvalid)?;
    let expected =
        serde_json::to_value(expected).map_err(|_| SimulationExecutionError::PairInvalid)?;
    Ok(result.get(field) == Some(&expected))
}

fn hash_original_bytes(bytes: &[u8]) -> Result<Sha256Digest, SimulationExecutionError> {
    let content_hash = ContentHash::from_bytes(bytes);
    Sha256Digest::parse(content_hash.as_str()).map_err(|_| SimulationExecutionError::PairInvalid)
}
