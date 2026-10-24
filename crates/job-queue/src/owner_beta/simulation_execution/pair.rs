use std::fmt;

use domain::ContentHash;

use crate::owner_beta::simulation::{
    CostProfile, EngineIdentity, Sha256Digest, SimulationResult, canonical_json,
};

use super::{SimulationExecutionError, SimulationJobInput};

const MAX_CANONICAL_PAIR_BYTES: usize = 64 * 1024 * 1024;

/// Two independently parsed, canonical result bodies bound to one queue
/// input. This proves content consistency only; executable attestation belongs
/// to the isolated runner and is not established by `expected_engine`.
pub struct CheckedSimulationPair {
    illustrative_10bp_per_side_bytes: Vec<u8>,
    zero_cost_reference_bytes: Vec<u8>,
    illustrative_10bp_per_side: SimulationResult,
    zero_cost_reference: SimulationResult,
    illustrative_10bp_per_side_sha256: Sha256Digest,
    zero_cost_reference_sha256: Sha256Digest,
}

impl fmt::Debug for CheckedSimulationPair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CheckedSimulationPair")
            .field(
                "illustrative_10bp_per_side_sha256",
                &self.illustrative_10bp_per_side_sha256,
            )
            .field(
                "zero_cost_reference_sha256",
                &self.zero_cost_reference_sha256,
            )
            .field("canonical_bytes", &"<checked>")
            .finish()
    }
}

impl CheckedSimulationPair {
    /// Validates the whole pair before retaining either canonical body.
    /// `expected_engine` is a declared content binding, not runtime proof.
    pub fn from_canonical(
        illustrative: &[u8],
        zero_cost: &[u8],
        input: &SimulationJobInput,
        expected_engine: &EngineIdentity,
    ) -> Result<Self, SimulationExecutionError> {
        let combined_size = illustrative
            .len()
            .checked_add(zero_cost.len())
            .ok_or(SimulationExecutionError::SizeLimitExceeded)?;
        if combined_size > MAX_CANONICAL_PAIR_BYTES {
            return Err(SimulationExecutionError::SizeLimitExceeded);
        }

        let illustrative_result = SimulationResult::from_json(illustrative)
            .map_err(|_| SimulationExecutionError::PairInvalid)?;
        require_canonical_bytes(illustrative, &illustrative_result)?;
        let zero_cost_result = SimulationResult::from_json(zero_cost)
            .map_err(|_| SimulationExecutionError::PairInvalid)?;
        require_canonical_bytes(zero_cost, &zero_cost_result)?;

        if illustrative_result.conventions().cost_profile()
            != CostProfile::Illustrative10BpPerSideV1
            || zero_cost_result.conventions().cost_profile() != CostProfile::ZeroCostReferenceV1
        {
            return Err(SimulationExecutionError::PairInvalid);
        }
        if illustrative_result.engine() != expected_engine
            || zero_cost_result.engine() != expected_engine
            || illustrative_result.engine() != zero_cost_result.engine()
        {
            return Err(SimulationExecutionError::ProvenanceMismatch);
        }
        if illustrative_result.input() != zero_cost_result.input()
            || illustrative_result.strategy() != zero_cost_result.strategy()
            || illustrative_result.schedule() != zero_cost_result.schedule()
            || illustrative_result.signals() != zero_cost_result.signals()
        {
            return Err(SimulationExecutionError::PairInvalid);
        }

        validate_input_binding(&illustrative_result, input)?;
        validate_input_binding(&zero_cost_result, input)?;

        let illustrative_sha256 = hash_original_bytes(illustrative)?;
        let zero_cost_sha256 = hash_original_bytes(zero_cost)?;
        Ok(Self {
            illustrative_10bp_per_side_bytes: illustrative.to_vec(),
            zero_cost_reference_bytes: zero_cost.to_vec(),
            illustrative_10bp_per_side: illustrative_result,
            zero_cost_reference: zero_cost_result,
            illustrative_10bp_per_side_sha256: illustrative_sha256,
            zero_cost_reference_sha256: zero_cost_sha256,
        })
    }

    pub fn illustrative_10bp_per_side_bytes(&self) -> &[u8] {
        &self.illustrative_10bp_per_side_bytes
    }

    pub fn zero_cost_reference_bytes(&self) -> &[u8] {
        &self.zero_cost_reference_bytes
    }

    pub fn illustrative_10bp_per_side_result(&self) -> &SimulationResult {
        &self.illustrative_10bp_per_side
    }

    pub fn zero_cost_reference_result(&self) -> &SimulationResult {
        &self.zero_cost_reference
    }

    pub fn illustrative_10bp_per_side_sha256(&self) -> &Sha256Digest {
        &self.illustrative_10bp_per_side_sha256
    }

    pub fn zero_cost_reference_sha256(&self) -> &Sha256Digest {
        &self.zero_cost_reference_sha256
    }
}

fn require_canonical_bytes(
    original: &[u8],
    result: &SimulationResult,
) -> Result<(), SimulationExecutionError> {
    let canonical = canonical_json(result).map_err(|_| SimulationExecutionError::PairInvalid)?;
    if canonical.as_slice() != original {
        return Err(SimulationExecutionError::PairInvalid);
    }
    Ok(())
}

fn validate_input_binding(
    result: &SimulationResult,
    input: &SimulationJobInput,
) -> Result<(), SimulationExecutionError> {
    let identity = result.input();
    let pins = input.pins();
    if identity.candidate_content_sha256().as_str() != pins.candidate_content_sha256().as_str()
        || identity.artifact_manifest_sha256().as_str() != pins.artifact_manifest_sha256().as_str()
        || identity.stage5_manifest_sha256().as_str() != pins.stage5_manifest_sha256().as_str()
        || identity.action_manifest_sha256().as_str() != pins.action_manifest_sha256().as_str()
        || identity.approval_registry_sha256().as_str() != pins.approval_registry_sha256().as_str()
        || identity.bars_sha256().as_str() != input.bars_sha256().as_str()
    {
        return Err(SimulationExecutionError::ProvenanceMismatch);
    }
    if result.schedule() != input.schedule() {
        return Err(SimulationExecutionError::WindowInvalid);
    }
    if result.strategy().strategy_snapshot_sha256().as_str()
        != input.strategy().config_sha256().as_str()
    {
        return Err(SimulationExecutionError::ProvenanceMismatch);
    }
    Ok(())
}

fn hash_original_bytes(bytes: &[u8]) -> Result<Sha256Digest, SimulationExecutionError> {
    let content_hash = ContentHash::from_bytes(bytes);
    Sha256Digest::parse(content_hash.as_str()).map_err(|_| SimulationExecutionError::PairInvalid)
}
