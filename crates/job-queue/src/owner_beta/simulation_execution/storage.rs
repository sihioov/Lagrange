//! Read-side validation for immutable two-profile result storage.
//!
//! This module never queries the database. The HTTP/API layer owns actor
//! lookup; it passes both stored bodies and their recorded hashes here.

use crate::owner_beta::simulation::EngineIdentity;

use super::{
    CheckedSimulationPair, SimulationExecutionError, SimulationJobInput,
    publish::SimulationPublicationError,
};

/// Reconstructs the closed pair from both original bodies and rejects either
/// stored hash before a caller can expose one profile on its own.
pub fn check_stored_simulation_pair(
    illustrative: &[u8],
    illustrative_sha256: &str,
    zero_cost: &[u8],
    zero_cost_sha256: &str,
    input: &SimulationJobInput,
    expected_engine: &EngineIdentity,
) -> Result<CheckedSimulationPair, SimulationPublicationError> {
    let pair =
        CheckedSimulationPair::from_canonical(illustrative, zero_cost, input, expected_engine)
            .map_err(map_pair_error)?;
    if pair.illustrative_10bp_per_side_sha256().as_str() != illustrative_sha256
        || pair.zero_cost_reference_sha256().as_str() != zero_cost_sha256
    {
        return Err(SimulationPublicationError::Integrity);
    }
    Ok(pair)
}

fn map_pair_error(error: SimulationExecutionError) -> SimulationPublicationError {
    match error {
        SimulationExecutionError::InputInvalid
        | SimulationExecutionError::WindowInvalid
        | SimulationExecutionError::StrategyInvalid
        | SimulationExecutionError::PairInvalid
        | SimulationExecutionError::ProvenanceMismatch
        | SimulationExecutionError::SizeLimitExceeded => SimulationPublicationError::Integrity,
    }
}
