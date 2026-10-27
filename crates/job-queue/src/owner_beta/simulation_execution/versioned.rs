//! Explicit v1/v2 queue-input dispatch while retaining the original v1 type.
//!
//! Version metadata is derived from a validated schema/contract pairing. The
//! versioned wire serializer delegates to the selected input directly, so it
//! never adds an enum tag or changes the stored JSON shape.

use std::fmt;

use domain::{ContentHash, TradingDate};
use serde::{
    Serialize, Serializer,
    de::{self, DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use uuid::Uuid;

use crate::owner_beta::{
    OwnerBetaPriceRecommendationPins, OwnerBetaStrategySnapshot,
    simulation::{
        APPROVED_CONTRACT_SHA256, CONTRACT_VERSION, EngineIdentity, RESULT_SCHEMA_VERSION,
        Sha256Digest, SimulationResult,
        v2::{
            APPROVED_CONTRACT_SHA256 as APPROVED_V2_CONTRACT_SHA256,
            CONTRACT_VERSION as V2_CONTRACT_VERSION, V2SimulationPair, V2SimulationResult,
        },
    },
};

use super::{
    OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA, SimulationExecutionError, SimulationJobInput,
    pair::CheckedSimulationPair,
    publish::SimulationPublicationError,
    v2::{V2CheckedSimulationPair, V2SimulationJobInput},
};

pub const OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2: &str =
    "owner-beta-price-only-simulation-job-v2";
pub const V1_IDEMPOTENCY_PREFIX: &str = "owner-beta:price-only-simulation:v1:";
pub const V2_IDEMPOTENCY_PREFIX: &str = "owner-beta:price-only-simulation:v2:";

const MAX_VERSIONED_JSON_NESTING: usize = 64;

/// Parses queue JSON with duplicate-key rejection before constructing a Value.
/// Unlike the v1 result parser, this preserves the exact integer-versus-float
/// distinction required by v2 defaults such as `1.0` and `0.3`.
pub(super) fn parse_versioned_json(bytes: &[u8]) -> Result<Value, SimulationExecutionError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = VersionedValueSeed { depth: 0 }
        .deserialize(&mut deserializer)
        .map_err(|_| SimulationExecutionError::InputInvalid)?;
    deserializer
        .end()
        .map_err(|_| SimulationExecutionError::InputInvalid)?;
    Ok(value)
}

struct VersionedValueSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for VersionedValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(VersionedValueVisitor { depth: self.depth })
    }
}

struct VersionedValueVisitor {
    depth: usize,
}

impl<'de> Visitor<'de> for VersionedValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with duplicate object keys rejected")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid JSON float"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth >= MAX_VERSIONED_JSON_NESTING {
            return Err(A::Error::custom("JSON nesting limit exceeded"));
        }
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(VersionedValueSeed {
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth >= MAX_VERSIONED_JSON_NESTING {
            return Err(A::Error::custom("JSON nesting limit exceeded"));
        }
        let mut values = serde_json::Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate JSON object key"));
            }
            let value = object.next_value_seed(VersionedValueSeed {
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

/// The two closed simulation contracts supported by the queue boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationContractVersion {
    V1,
    V2,
}

impl SimulationContractVersion {
    pub const fn queue_schema(self) -> &'static str {
        match self {
            Self::V1 => OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA,
            Self::V2 => OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2,
        }
    }

    pub const fn contract_version(self) -> &'static str {
        match self {
            Self::V1 => CONTRACT_VERSION,
            Self::V2 => V2_CONTRACT_VERSION,
        }
    }

    pub const fn contract_sha256(self) -> &'static str {
        match self {
            Self::V1 => APPROVED_CONTRACT_SHA256,
            Self::V2 => APPROVED_V2_CONTRACT_SHA256,
        }
    }

    pub const fn result_schema(self) -> &'static str {
        match self {
            Self::V1 => RESULT_SCHEMA_VERSION,
            Self::V2 => crate::owner_beta::simulation::v2::RESULT_SCHEMA_VERSION,
        }
    }

    pub const fn idempotency_prefix(self) -> &'static str {
        match self {
            Self::V1 => V1_IDEMPOTENCY_PREFIX,
            Self::V2 => V2_IDEMPOTENCY_PREFIX,
        }
    }
}

/// A validated v1 or v2 input. Its serializer emits the selected wire value
/// directly instead of serializing an enum wrapper.
#[derive(Clone)]
pub enum VersionedSimulationJobInput {
    V1(SimulationJobInput),
    V2(V2SimulationJobInput),
}

impl Serialize for VersionedSimulationJobInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::V1(input) => input.serialize(serializer),
            Self::V2(input) => input.serialize(serializer),
        }
    }
}

impl VersionedSimulationJobInput {
    /// Strict schema-first dispatch. Unknown schemas, mixed contracts, and
    /// duplicate keys fail closed without falling back to another version.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SimulationExecutionError> {
        let value = parse_versioned_json(bytes)?;
        let schema = value
            .as_object()
            .and_then(|object| object.get("schema"))
            .and_then(Value::as_str)
            .ok_or(SimulationExecutionError::InputInvalid)?
            .to_owned();
        match schema.as_str() {
            OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA => {
                SimulationJobInput::from_json(bytes).map(Self::V1)
            }
            OWNER_BETA_PRICE_ONLY_SIMULATION_SCHEMA_V2 => {
                V2SimulationJobInput::from_json(bytes).map(Self::V2)
            }
            _ => Err(SimulationExecutionError::InputInvalid),
        }
    }

    pub fn version(&self) -> SimulationContractVersion {
        match self {
            Self::V1(_) => SimulationContractVersion::V1,
            Self::V2(_) => SimulationContractVersion::V2,
        }
    }

    pub fn schema(&self) -> &str {
        match self {
            Self::V1(input) => input.schema(),
            Self::V2(input) => input.schema(),
        }
    }

    pub fn run_id(&self) -> Uuid {
        match self {
            Self::V1(input) => input.run_id(),
            Self::V2(input) => input.run_id(),
        }
    }

    pub fn strategy_config_id(&self) -> Uuid {
        match self {
            Self::V1(input) => input.strategy_config_id(),
            Self::V2(input) => input.strategy_config_id(),
        }
    }

    pub fn evaluation_start(&self) -> TradingDate {
        match self {
            Self::V1(input) => input.evaluation_start(),
            Self::V2(input) => input.evaluation_start(),
        }
    }

    pub fn evaluation_end(&self) -> TradingDate {
        match self {
            Self::V1(input) => input.evaluation_end(),
            Self::V2(input) => input.evaluation_end(),
        }
    }

    pub fn pins(&self) -> &OwnerBetaPriceRecommendationPins {
        match self {
            Self::V1(input) => input.pins(),
            Self::V2(input) => input.pins(),
        }
    }

    pub fn contract_document_sha256(&self) -> &ContentHash {
        match self {
            Self::V1(input) => input.contract_document_sha256(),
            Self::V2(input) => input.contract_document_sha256(),
        }
    }

    pub fn bars_sha256(&self) -> &ContentHash {
        match self {
            Self::V1(input) => input.bars_sha256(),
            Self::V2(input) => input.bars_sha256(),
        }
    }

    pub fn strategy(&self) -> &OwnerBetaStrategySnapshot {
        match self {
            Self::V1(input) => input.strategy(),
            Self::V2(input) => input.strategy(),
        }
    }

    /// Returns every date required by the selected contract: the v1 warmup
    /// and evaluation path, or the complete v2 preparation and evaluation path.
    pub fn required_dates(&self) -> Vec<TradingDate> {
        match self {
            Self::V1(input) => input
                .schedule()
                .warmup_sessions()
                .iter()
                .chain(input.schedule().evaluation_sessions())
                .copied()
                .collect(),
            Self::V2(input) => input.required_dates(),
        }
    }

    pub fn as_v1(&self) -> Option<&SimulationJobInput> {
        match self {
            Self::V1(input) => Some(input),
            Self::V2(_) => None,
        }
    }

    pub fn as_v2(&self) -> Option<&V2SimulationJobInput> {
        match self {
            Self::V1(_) => None,
            Self::V2(input) => Some(input),
        }
    }
}

enum CheckedVersionedPair {
    V1(Box<CheckedSimulationPair>),
    V2(Box<V2CheckedSimulationPair>),
}

/// The validated typed result selected by an explicit queue contract version.
#[derive(Debug, Clone, Copy)]
pub enum VersionedSimulationResultRef<'a> {
    V1(&'a SimulationResult),
    V2(&'a V2SimulationResult),
}

/// A complete checked pair without erasing the selected result type.
#[derive(Debug, Clone, Copy)]
pub enum VersionedSimulationPairRef<'a> {
    V1(&'a CheckedSimulationPair),
    V2(&'a V2SimulationPair),
}

/// Read-side result boundary that dispatches only from a validated input
/// version and retains both original canonical bodies together.
pub struct VersionedCheckedSimulationPair {
    inner: CheckedVersionedPair,
}

impl fmt::Debug for VersionedCheckedSimulationPair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionedCheckedSimulationPair")
            .field("version", &self.version())
            .field("illustrative_sha256", self.illustrative_sha256())
            .field("zero_cost_sha256", self.zero_cost_sha256())
            .field("canonical_bytes", &"<checked>")
            .finish()
    }
}

impl VersionedCheckedSimulationPair {
    /// Validates the selected contract's entire pair before making either
    /// profile available to a read caller.
    pub fn from_canonical(
        illustrative: &[u8],
        zero_cost: &[u8],
        input: &VersionedSimulationJobInput,
        expected_engine: &EngineIdentity,
    ) -> Result<Self, SimulationExecutionError> {
        let inner = match input {
            VersionedSimulationJobInput::V1(input) => {
                CheckedVersionedPair::V1(Box::new(CheckedSimulationPair::from_canonical(
                    illustrative,
                    zero_cost,
                    input,
                    expected_engine,
                )?))
            }
            VersionedSimulationJobInput::V2(input) => {
                CheckedVersionedPair::V2(Box::new(V2CheckedSimulationPair::from_canonical(
                    illustrative,
                    zero_cost,
                    input,
                    expected_engine,
                )?))
            }
        };
        Ok(Self { inner })
    }

    pub fn version(&self) -> SimulationContractVersion {
        match &self.inner {
            CheckedVersionedPair::V1(_) => SimulationContractVersion::V1,
            CheckedVersionedPair::V2(_) => SimulationContractVersion::V2,
        }
    }

    pub fn illustrative_bytes(&self) -> &[u8] {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => pair.illustrative_10bp_per_side_bytes(),
            CheckedVersionedPair::V2(pair) => pair.illustrative_bytes(),
        }
    }

    pub fn zero_cost_bytes(&self) -> &[u8] {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => pair.zero_cost_reference_bytes(),
            CheckedVersionedPair::V2(pair) => pair.zero_cost_bytes(),
        }
    }

    pub fn illustrative_sha256(&self) -> &Sha256Digest {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => pair.illustrative_10bp_per_side_sha256(),
            CheckedVersionedPair::V2(pair) => pair.illustrative_sha256(),
        }
    }

    pub fn zero_cost_sha256(&self) -> &Sha256Digest {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => pair.zero_cost_reference_sha256(),
            CheckedVersionedPair::V2(pair) => pair.zero_cost_sha256(),
        }
    }

    pub fn pair(&self) -> VersionedSimulationPairRef<'_> {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => VersionedSimulationPairRef::V1(pair),
            CheckedVersionedPair::V2(pair) => VersionedSimulationPairRef::V2(pair.pair()),
        }
    }

    pub fn illustrative_result(&self) -> VersionedSimulationResultRef<'_> {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => {
                VersionedSimulationResultRef::V1(pair.illustrative_10bp_per_side_result())
            }
            CheckedVersionedPair::V2(pair) => {
                VersionedSimulationResultRef::V2(pair.illustrative_result())
            }
        }
    }

    pub fn zero_cost_result(&self) -> VersionedSimulationResultRef<'_> {
        match &self.inner {
            CheckedVersionedPair::V1(pair) => {
                VersionedSimulationResultRef::V1(pair.zero_cost_reference_result())
            }
            CheckedVersionedPair::V2(pair) => {
                VersionedSimulationResultRef::V2(pair.zero_cost_result())
            }
        }
    }
}

/// Version-aware read validation. Both persisted hashes are checked before a
/// complete typed pair is returned.
pub fn check_stored_versioned_simulation_pair(
    illustrative: &[u8],
    illustrative_sha256: &str,
    zero_cost: &[u8],
    zero_cost_sha256: &str,
    input: &VersionedSimulationJobInput,
    expected_engine: &EngineIdentity,
) -> Result<VersionedCheckedSimulationPair, SimulationPublicationError> {
    let pair = VersionedCheckedSimulationPair::from_canonical(
        illustrative,
        zero_cost,
        input,
        expected_engine,
    )
    .map_err(map_versioned_pair_error)?;
    if pair.illustrative_sha256().as_str() != illustrative_sha256
        || pair.zero_cost_sha256().as_str() != zero_cost_sha256
    {
        return Err(SimulationPublicationError::Integrity);
    }
    Ok(pair)
}

fn map_versioned_pair_error(error: SimulationExecutionError) -> SimulationPublicationError {
    match error {
        SimulationExecutionError::InputInvalid
        | SimulationExecutionError::WindowInvalid
        | SimulationExecutionError::StrategyInvalid
        | SimulationExecutionError::PairInvalid
        | SimulationExecutionError::ProvenanceMismatch
        | SimulationExecutionError::SizeLimitExceeded => SimulationPublicationError::Integrity,
    }
}
