//! Explicit historical/daily job decoding. Decoded pins are claims, not approval.

use std::{collections::BTreeSet, fmt};

use domain::{ContentHash, TradingDate};
use factor_engine::daily_price_only::{
    DAILY_FACTOR_SNAPSHOT_SCHEMA, DAILY_PRICE_ONLY_INPUT_KIND, DailyPriceOnlyFactorSnapshot,
};
use factor_engine::price_only::PRICE_ONLY_CAPABILITY;
use market_data::daily_price_only::input::AdmittedDailyPriceInputV1;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;
use uuid::Uuid;

use super::{
    OwnerBetaPriceRecommendationInput, OwnerBetaPriceRecommendationPins, OwnerBetaStrategySnapshot,
};
use crate::resolver::ResolvedConfig;

pub const OWNER_BETA_DAILY_RECOMMENDATION_SCHEMA: &str = "owner-beta-daily-recommendation-v1";
pub const OWNER_BETA_DAILY_TRIGGER_VERSION: &str = "owner-beta-daily-trigger-v1";
const MAX_JOB_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaDailyInputError {
    #[error("owner-beta execution input is invalid")]
    InvalidWire,
    #[error("owner-beta daily strategy is invalid")]
    InvalidStrategy,
    #[error("owner-beta daily trigger is invalid")]
    InvalidTrigger,
    #[error("owner-beta daily admission does not match")]
    AdmissionMismatch,
    #[error("owner-beta daily factor snapshot does not match")]
    FactorMismatch,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DailySchema {
    #[serde(rename = "owner-beta-daily-recommendation-v1")]
    V1,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DailyKind {
    #[serde(rename = "owner_beta_daily_etf11_price_only_v1")]
    V1,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TriggerKind {
    #[serde(rename = "DAILY")]
    Daily,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TriggerVersion {
    #[serde(rename = "owner-beta-daily-trigger-v1")]
    V1,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerBetaDailyRecommendationPins {
    kind: DailyKind,
    content_sha256: ContentHash,
    manifest_sha256: ContentHash,
    admission_sha256: ContentHash,
    policy_sha256: ContentHash,
}
impl OwnerBetaDailyRecommendationPins {
    pub fn content_sha256(&self) -> &ContentHash {
        &self.content_sha256
    }
    pub fn manifest_sha256(&self) -> &ContentHash {
        &self.manifest_sha256
    }
    pub fn admission_sha256(&self) -> &ContentHash {
        &self.admission_sha256
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy_sha256
    }
}
impl fmt::Debug for OwnerBetaDailyRecommendationPins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerBetaDailyRecommendationPins")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyTrigger {
    kind: TriggerKind,
    version: TriggerVersion,
    config_epoch: i64,
    activation_epoch: i64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyPayload {
    schema: DailySchema,
    run_id: Uuid,
    strategy_config_id: Uuid,
    as_of: TradingDate,
    input: OwnerBetaDailyRecommendationPins,
    strategy: OwnerBetaStrategySnapshot,
    trigger: DailyTrigger,
}

/// An exact daily job payload. Deserialization never grants filesystem or DB
/// authority; compute must compare it with a freshly reopened sealed input.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OwnerBetaDailyRecommendationInputV1 {
    inner: DailyPayload,
}

impl fmt::Debug for OwnerBetaDailyRecommendationInputV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerBetaDailyRecommendationInputV1")
            .field("as_of", &self.as_of())
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for OwnerBetaDailyRecommendationInputV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = StrictValue::deserialize(deserializer)?.0;
        let inner = serde_json::from_value(value)
            .map_err(|_| de::Error::custom("invalid daily input shape"))?;
        let input = Self { inner };
        input.validate().map_err(de::Error::custom)?;
        Ok(input)
    }
}

impl OwnerBetaDailyRecommendationInputV1 {
    pub fn from_admitted(
        run_id: Uuid,
        strategy_config_id: Uuid,
        admitted: &AdmittedDailyPriceInputV1<'_, '_>,
        resolved: &ResolvedConfig,
        config_epoch: i64,
        activation_epoch: i64,
    ) -> Result<Self, OwnerBetaDailyInputError> {
        admitted
            .revalidate()
            .map_err(|_| OwnerBetaDailyInputError::AdmissionMismatch)?;
        let pins = admitted.pins();
        let input = Self {
            inner: DailyPayload {
                schema: DailySchema::V1,
                run_id,
                strategy_config_id,
                as_of: admitted.as_of(),
                input: OwnerBetaDailyRecommendationPins {
                    kind: DailyKind::V1,
                    content_sha256: pins.content_sha256.clone(),
                    manifest_sha256: pins.manifest_sha256.clone(),
                    admission_sha256: pins.admission_sha256.clone(),
                    policy_sha256: pins.policy_sha256.clone(),
                },
                strategy: OwnerBetaStrategySnapshot::from_resolved_config(resolved)
                    .map_err(|_| OwnerBetaDailyInputError::InvalidStrategy)?,
                trigger: DailyTrigger {
                    kind: TriggerKind::Daily,
                    version: TriggerVersion::V1,
                    config_epoch,
                    activation_epoch,
                },
            },
        };
        input.validate()?;
        Ok(input)
    }

    pub fn run_id(&self) -> Uuid {
        self.inner.run_id
    }
    pub fn strategy_config_id(&self) -> Uuid {
        self.inner.strategy_config_id
    }
    pub fn as_of(&self) -> TradingDate {
        self.inner.as_of
    }
    pub fn pins(&self) -> &OwnerBetaDailyRecommendationPins {
        &self.inner.input
    }
    pub fn strategy_snapshot(&self) -> &OwnerBetaStrategySnapshot {
        &self.inner.strategy
    }
    pub fn config_epoch(&self) -> i64 {
        self.inner.trigger.config_epoch
    }
    pub fn activation_epoch(&self) -> i64 {
        self.inner.trigger.activation_epoch
    }

    pub fn validate(&self) -> Result<(), OwnerBetaDailyInputError> {
        if self.config_epoch() < 0 || self.activation_epoch() < 0 {
            return Err(OwnerBetaDailyInputError::InvalidTrigger);
        }
        let strategy = self.strategy_snapshot();
        let expected = OwnerBetaStrategySnapshot::from_resolved_config(&ResolvedConfig {
            strategy_id: strategy.strategy_id().to_owned(),
            strategy_version: strategy.strategy_version().to_owned(),
            config: strategy.config_json().clone(),
        })
        .map_err(|_| OwnerBetaDailyInputError::InvalidStrategy)?;
        if &expected != strategy {
            return Err(OwnerBetaDailyInputError::InvalidStrategy);
        }
        Ok(())
    }

    pub fn validate_admitted(
        &self,
        admitted: &AdmittedDailyPriceInputV1<'_, '_>,
    ) -> Result<(), OwnerBetaDailyInputError> {
        self.validate()?;
        admitted
            .revalidate()
            .map_err(|_| OwnerBetaDailyInputError::AdmissionMismatch)?;
        let pins = admitted.pins();
        if self.as_of() != admitted.as_of()
            || self.pins().content_sha256 != pins.content_sha256
            || self.pins().manifest_sha256 != pins.manifest_sha256
            || self.pins().admission_sha256 != pins.admission_sha256
            || self.pins().policy_sha256 != pins.policy_sha256
        {
            return Err(OwnerBetaDailyInputError::AdmissionMismatch);
        }
        Ok(())
    }

    pub fn validate_factor_snapshot(
        &self,
        snapshot: &DailyPriceOnlyFactorSnapshot,
    ) -> Result<(), OwnerBetaDailyInputError> {
        if snapshot.schema != DAILY_FACTOR_SNAPSHOT_SCHEMA
            || snapshot.input_kind != DAILY_PRICE_ONLY_INPUT_KIND
            || snapshot.capability != PRICE_ONLY_CAPABILITY
            || snapshot.as_of != self.as_of()
            || snapshot.content_sha256 != self.pins().content_sha256.as_str()
            || snapshot.manifest_sha256 != self.pins().manifest_sha256.as_str()
            || snapshot.admission_sha256 != self.pins().admission_sha256.as_str()
            || snapshot.policy_sha256 != self.pins().policy_sha256.as_str()
            || snapshot
                .compute_hash()
                .map_err(|_| OwnerBetaDailyInputError::FactorMismatch)?
                != snapshot.hash
        {
            return Err(OwnerBetaDailyInputError::FactorMismatch);
        }
        Ok(())
    }
}

/// The historical variant serializes as the original untagged five-key object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerBetaExecutionInput {
    Historical(OwnerBetaPriceRecommendationInput),
    Daily(OwnerBetaDailyRecommendationInputV1),
}

pub enum OwnerBetaExecutionProvenance<'a> {
    Historical(&'a OwnerBetaPriceRecommendationPins),
    Daily(&'a OwnerBetaDailyRecommendationPins),
}

impl OwnerBetaExecutionInput {
    pub fn run_id(&self) -> Uuid {
        match self {
            Self::Historical(v) => v.run_id(),
            Self::Daily(v) => v.run_id(),
        }
    }
    pub fn strategy_config_id(&self) -> Uuid {
        match self {
            Self::Historical(v) => v.strategy_config_id(),
            Self::Daily(v) => v.strategy_config_id(),
        }
    }
    pub fn as_of(&self) -> TradingDate {
        match self {
            Self::Historical(v) => v.as_of(),
            Self::Daily(v) => v.as_of(),
        }
    }
    pub fn strategy_snapshot(&self) -> &OwnerBetaStrategySnapshot {
        match self {
            Self::Historical(v) => v.strategy_snapshot(),
            Self::Daily(v) => v.strategy_snapshot(),
        }
    }
    pub fn provenance(&self) -> OwnerBetaExecutionProvenance<'_> {
        match self {
            Self::Historical(v) => OwnerBetaExecutionProvenance::Historical(v.pins()),
            Self::Daily(v) => OwnerBetaExecutionProvenance::Daily(v.pins()),
        }
    }
}

impl Serialize for OwnerBetaExecutionInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Historical(input) => input.serialize(serializer),
            Self::Daily(input) => input.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for OwnerBetaExecutionInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = StrictValue::deserialize(deserializer)?.0;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("execution input must be an object"))?;
        match object.get("schema") {
            None => {
                let input: OwnerBetaPriceRecommendationInput = serde_json::from_value(value)
                    .map_err(|_| de::Error::custom("invalid historical input shape"))?;
                input
                    .validate_strategy_snapshot()
                    .map_err(|_| de::Error::custom("invalid historical strategy"))?;
                Ok(Self::Historical(input))
            }
            Some(Value::String(schema)) if schema == OWNER_BETA_DAILY_RECOMMENDATION_SCHEMA => {
                serde_json::from_value(value)
                    .map(Self::Daily)
                    .map_err(|_| de::Error::custom("invalid daily input"))
            }
            _ => Err(de::Error::custom("unknown execution input schema")),
        }
    }
}

pub fn decode_owner_beta_execution_input(
    bytes: &[u8],
) -> Result<OwnerBetaExecutionInput, OwnerBetaDailyInputError> {
    if bytes.len() > MAX_JOB_INPUT_BYTES {
        return Err(OwnerBetaDailyInputError::InvalidWire);
    }
    serde_json::from_slice(bytes).map_err(|_| OwnerBetaDailyInputError::InvalidWire)
}

/// Reject duplicate keys before converting to a Value, including nested config
/// objects. A Value-first schema dispatch would silently overwrite duplicates.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(v.into())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(v.into())))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .map(StrictValue)
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v.to_owned())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = seq.next_element::<StrictValue>()? {
                    values.push(v.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    values.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
