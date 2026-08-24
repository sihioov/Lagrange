use std::{collections::BTreeMap, fmt};

use domain::{ContentHash, TradingDate};
use factor_engine::{
    PriceOnlyFactorSnapshot,
    price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND},
};
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::resolver::ResolvedConfig;

/// The only job type for an owner-beta price recommendation.
pub const OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE: &str = "owner_beta_price_recommendation";

/// Domain/schema tag included in every strategy snapshot hash. Changing the
/// wire shape or the ownership boundary requires a new tag rather than a
/// silent reinterpretation of an existing payload.
pub const OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA: &str = "owner-beta-strategy-config-v1";

/// Sealed, price-only provenance for an owner-beta recommendation job.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerBetaPriceRecommendationInput {
    run_id: Uuid,
    strategy_config_id: Uuid,
    as_of: TradingDate,
    pins: OwnerBetaPriceRecommendationPins,
    strategy: OwnerBetaStrategySnapshot,
}

/// The immutable strategy configuration snapshot carried by an owner-beta
/// job. The fields remain private so callers can only construct the hash from
/// one [`ResolvedConfig`], never by independently supplying a hash.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerBetaStrategySnapshot {
    strategy_id: String,
    strategy_version: String,
    config_json: Value,
    config_sha256: ContentHash,
}

/// The immutable approval pins carried by an owner-beta recommendation job.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerBetaPriceRecommendationPins {
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    approval_registry_sha256: ContentHash,
}

/// Fail-closed validation errors. Variants intentionally carry no values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerBetaPriceRecommendationInputError {
    #[error("owner-beta price recommendation approval pins do not match")]
    ApprovalPinsMismatch,
    #[error("owner-beta price recommendation factor snapshot does not match")]
    FactorSnapshotMismatch,
    #[error("owner-beta price recommendation strategy snapshot is invalid")]
    StrategySnapshotInvalid,
    #[error("owner-beta price recommendation strategy snapshot does not match")]
    StrategySnapshotMismatch,
}

impl fmt::Debug for OwnerBetaPriceRecommendationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaPriceRecommendationInput")
            .field("job_type", &OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
            .field("as_of", &self.as_of)
            .field("pins", &self.pins)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for OwnerBetaStrategySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaStrategySnapshot")
            .field("snapshot", &"<redacted>")
            .finish()
    }
}

impl fmt::Debug for OwnerBetaPriceRecommendationPins {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBetaPriceRecommendationPins")
            .field("approval_registry_sha256", &self.approval_registry_sha256)
            .finish_non_exhaustive()
    }
}

impl OwnerBetaPriceRecommendationInput {
    /// Constructs an input only from an owner-approved artifact; callers cannot
    /// supply the five provenance pins independently.
    pub fn new(
        run_id: Uuid,
        strategy_config_id: Uuid,
        as_of: TradingDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        resolved_config: &ResolvedConfig,
    ) -> Result<Self, OwnerBetaPriceRecommendationInputError> {
        let artifact_pins = artifact.pins();
        Ok(Self {
            run_id,
            strategy_config_id,
            as_of,
            pins: OwnerBetaPriceRecommendationPins {
                candidate_content_sha256: artifact_pins.candidate_content_sha256().clone(),
                artifact_manifest_sha256: artifact_pins.artifact_manifest_sha256().clone(),
                stage5_manifest_sha256: artifact_pins.stage5_manifest_sha256().clone(),
                action_manifest_sha256: artifact_pins.action_manifest_sha256().clone(),
                approval_registry_sha256: artifact_pins.approval_registry_sha256().clone(),
            },
            strategy: OwnerBetaStrategySnapshot::from_resolved_config(resolved_config)?,
        })
    }

    /// Named form of [`Self::new`] for call sites that make the trust boundary
    /// explicit.
    pub fn from_approved_artifact(
        run_id: Uuid,
        strategy_config_id: Uuid,
        as_of: TradingDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
        resolved_config: &ResolvedConfig,
    ) -> Result<Self, OwnerBetaPriceRecommendationInputError> {
        Self::new(run_id, strategy_config_id, as_of, artifact, resolved_config)
    }

    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub fn strategy_config_id(&self) -> Uuid {
        self.strategy_config_id
    }

    pub fn as_of(&self) -> TradingDate {
        self.as_of
    }

    pub fn pins(&self) -> &OwnerBetaPriceRecommendationPins {
        &self.pins
    }

    pub fn strategy_snapshot(&self) -> &OwnerBetaStrategySnapshot {
        &self.strategy
    }

    /// Recomputes the domain-tagged hash over the canonical strategy id,
    /// version, and JSON snapshot. This catches a payload mutation before it
    /// can be treated as a durable replay.
    pub fn validate_strategy_snapshot(&self) -> Result<(), OwnerBetaPriceRecommendationInputError> {
        let canonical_config = canonicalize_json(&self.strategy.config_json);
        if !canonical_config.is_object()
            || canonical_config != self.strategy.config_json
            || self.strategy.strategy_id.is_empty()
            || self.strategy.strategy_version.is_empty()
        {
            return Err(OwnerBetaPriceRecommendationInputError::StrategySnapshotMismatch);
        }
        let expected = strategy_config_hash(
            &self.strategy.strategy_id,
            &self.strategy.strategy_version,
            &canonical_config,
        )
        .map_err(|_| OwnerBetaPriceRecommendationInputError::StrategySnapshotMismatch)?;
        if expected != self.strategy.config_sha256 {
            return Err(OwnerBetaPriceRecommendationInputError::StrategySnapshotMismatch);
        }
        Ok(())
    }

    /// Rechecks all pins against a newly resolved approved artifact.
    pub fn validate_approved_artifact(
        &self,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
    ) -> Result<(), OwnerBetaPriceRecommendationInputError> {
        let current = artifact.pins();
        if self.pins.candidate_content_sha256 != *current.candidate_content_sha256()
            || self.pins.artifact_manifest_sha256 != *current.artifact_manifest_sha256()
            || self.pins.stage5_manifest_sha256 != *current.stage5_manifest_sha256()
            || self.pins.action_manifest_sha256 != *current.action_manifest_sha256()
            || self.pins.approval_registry_sha256 != *current.approval_registry_sha256()
        {
            return Err(OwnerBetaPriceRecommendationInputError::ApprovalPinsMismatch);
        }
        Ok(())
    }

    /// Rechecks the snapshot's complete sealed-input contract.
    pub fn validate_factor_snapshot(
        &self,
        snapshot: &PriceOnlyFactorSnapshot,
    ) -> Result<(), OwnerBetaPriceRecommendationInputError> {
        if snapshot.as_of != self.as_of
            || snapshot.input_kind != PRICE_ONLY_INPUT_KIND
            || snapshot.capability != PRICE_ONLY_CAPABILITY
            || snapshot.candidate_content_sha256 != self.pins.candidate_content_sha256.as_str()
            || snapshot.artifact_manifest_sha256 != self.pins.artifact_manifest_sha256.as_str()
            || snapshot.stage5_manifest_sha256 != self.pins.stage5_manifest_sha256.as_str()
            || snapshot.action_manifest_sha256 != self.pins.action_manifest_sha256.as_str()
            || snapshot.approval_registry_sha256 != self.pins.approval_registry_sha256.as_str()
        {
            return Err(OwnerBetaPriceRecommendationInputError::FactorSnapshotMismatch);
        }
        Ok(())
    }
}

impl OwnerBetaStrategySnapshot {
    /// Builds a canonical snapshot and computes its hash from one resolved
    /// configuration. The constructor never accepts a caller-provided hash.
    pub fn from_resolved_config(
        resolved_config: &ResolvedConfig,
    ) -> Result<Self, OwnerBetaPriceRecommendationInputError> {
        if resolved_config.strategy_id.is_empty() || resolved_config.strategy_version.is_empty() {
            return Err(OwnerBetaPriceRecommendationInputError::StrategySnapshotInvalid);
        }
        let config_json = canonicalize_json(&resolved_config.config);
        if !config_json.is_object() {
            return Err(OwnerBetaPriceRecommendationInputError::StrategySnapshotInvalid);
        }
        let config_sha256 = strategy_config_hash(
            &resolved_config.strategy_id,
            &resolved_config.strategy_version,
            &config_json,
        )
        .map_err(|_| OwnerBetaPriceRecommendationInputError::StrategySnapshotInvalid)?;
        Ok(Self {
            strategy_id: resolved_config.strategy_id.clone(),
            strategy_version: resolved_config.strategy_version.clone(),
            config_json,
            config_sha256,
        })
    }

    pub fn strategy_id(&self) -> &str {
        &self.strategy_id
    }

    pub fn strategy_version(&self) -> &str {
        &self.strategy_version
    }

    pub fn config_json(&self) -> &Value {
        &self.config_json
    }

    pub fn config_sha256(&self) -> &ContentHash {
        &self.config_sha256
    }
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut canonical = Map::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                canonical.insert(key.clone(), canonicalize_json(&object[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

fn strategy_config_hash(
    strategy_id: &str,
    strategy_version: &str,
    config_json: &Value,
) -> Result<ContentHash, serde_json::Error> {
    let mut tagged = BTreeMap::new();
    tagged.insert("config_json", canonicalize_json(config_json));
    tagged.insert(
        "schema",
        Value::String(OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA.to_owned()),
    );
    tagged.insert("strategy_id", Value::String(strategy_id.to_owned()));
    tagged.insert(
        "strategy_version",
        Value::String(strategy_version.to_owned()),
    );
    serde_json::to_vec(&tagged).map(|bytes| ContentHash::from_bytes(&bytes))
}

impl OwnerBetaPriceRecommendationPins {
    pub fn candidate_content_sha256(&self) -> &ContentHash {
        &self.candidate_content_sha256
    }

    pub fn artifact_manifest_sha256(&self) -> &ContentHash {
        &self.artifact_manifest_sha256
    }

    pub fn stage5_manifest_sha256(&self) -> &ContentHash {
        &self.stage5_manifest_sha256
    }

    pub fn action_manifest_sha256(&self) -> &ContentHash {
        &self.action_manifest_sha256
    }

    pub fn approval_registry_sha256(&self) -> &ContentHash {
        &self.approval_registry_sha256
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use domain::{ContentHash, TradingDate};
    use factor_engine::{
        PriceOnlyFactorSnapshot,
        price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND},
    };
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPriceRecommendationInput,
        OwnerBetaPriceRecommendationInputError, OwnerBetaPriceRecommendationPins,
        OwnerBetaStrategySnapshot,
    };

    fn date() -> TradingDate {
        TradingDate::parse("2026-08-24").expect("valid test date")
    }

    fn hash(value: u8) -> ContentHash {
        ContentHash::parse(&format!("sha256:{value:064x}")).expect("valid test hash")
    }

    fn strategy() -> OwnerBetaStrategySnapshot {
        OwnerBetaStrategySnapshot::from_resolved_config(&crate::resolver::ResolvedConfig {
            strategy_id: "buy_and_hold".to_owned(),
            strategy_version: "1.0.0".to_owned(),
            config: json!({"z": [2, 1], "a": 7}),
        })
        .expect("valid strategy snapshot")
    }

    // The production artifact is intentionally nonconstructible. This helper
    // exercises wire-contract validation without creating another trust path.
    fn input() -> OwnerBetaPriceRecommendationInput {
        OwnerBetaPriceRecommendationInput {
            run_id: Uuid::from_u128(1),
            strategy_config_id: Uuid::from_u128(2),
            as_of: date(),
            pins: OwnerBetaPriceRecommendationPins {
                candidate_content_sha256: hash(1),
                artifact_manifest_sha256: hash(2),
                stage5_manifest_sha256: hash(3),
                action_manifest_sha256: hash(4),
                approval_registry_sha256: hash(5),
            },
            strategy: strategy(),
        }
    }

    fn snapshot(input: &OwnerBetaPriceRecommendationInput) -> PriceOnlyFactorSnapshot {
        PriceOnlyFactorSnapshot {
            input_kind: PRICE_ONLY_INPUT_KIND.to_owned(),
            capability: PRICE_ONLY_CAPABILITY.to_owned(),
            as_of: input.as_of,
            candidate_content_sha256: input.pins.candidate_content_sha256.to_string(),
            artifact_manifest_sha256: input.pins.artifact_manifest_sha256.to_string(),
            stage5_manifest_sha256: input.pins.stage5_manifest_sha256.to_string(),
            action_manifest_sha256: input.pins.action_manifest_sha256.to_string(),
            approval_registry_sha256: input.pins.approval_registry_sha256.to_string(),
            factor_versions: Default::default(),
            normalization: factor_engine::snapshot::NormalizationMeta {
                id: "test".to_owned(),
                version: "1".to_owned(),
                params: Default::default(),
            },
            rows: Vec::new(),
            hash: ContentHash::from_bytes(b"test"),
        }
    }

    #[test]
    fn job_type_is_fixed() {
        assert_eq!(
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            "owner_beta_price_recommendation"
        );
    }

    #[test]
    fn serde_round_trip_has_exact_field_inventory() {
        let input = input();
        let encoded = serde_json::to_value(&input).expect("serialize");
        let object = encoded.as_object().expect("object");
        assert_eq!(
            object.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "as_of".to_owned(),
                "pins".to_owned(),
                "run_id".to_owned(),
                "strategy_config_id".to_owned(),
                "strategy".to_owned(),
            ])
        );
        let pins = object["pins"].as_object().expect("pins object");
        assert_eq!(
            pins.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "action_manifest_sha256".to_owned(),
                "approval_registry_sha256".to_owned(),
                "artifact_manifest_sha256".to_owned(),
                "candidate_content_sha256".to_owned(),
                "stage5_manifest_sha256".to_owned(),
            ])
        );
        assert_eq!(
            serde_json::from_value::<OwnerBetaPriceRecommendationInput>(encoded)
                .expect("deserialize"),
            input
        );
        let strategy = serde_json::to_value(input.strategy_snapshot()).expect("strategy");
        assert_eq!(
            strategy
                .as_object()
                .expect("strategy object")
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "config_json".to_owned(),
                "config_sha256".to_owned(),
                "strategy_id".to_owned(),
                "strategy_version".to_owned(),
            ])
        );
    }

    #[test]
    fn serde_rejects_unknown_missing_and_invalid_hashes() {
        let encoded = serde_json::to_value(input()).expect("serialize");
        let mut unknown = encoded.clone();
        unknown
            .as_object_mut()
            .expect("object")
            .insert("capability".to_owned(), json!("anything"));
        assert!(serde_json::from_value::<OwnerBetaPriceRecommendationInput>(unknown).is_err());
        let mut nested_unknown = encoded.clone();
        nested_unknown["pins"]["capability"] = json!("anything");
        assert!(
            serde_json::from_value::<OwnerBetaPriceRecommendationInput>(nested_unknown).is_err()
        );
        let mut strategy_unknown = encoded.clone();
        strategy_unknown["strategy"]["capability"] = json!("anything");
        assert!(
            serde_json::from_value::<OwnerBetaPriceRecommendationInput>(strategy_unknown).is_err()
        );
        let mut missing = encoded.clone();
        missing.as_object_mut().expect("object").remove("as_of");
        assert!(serde_json::from_value::<OwnerBetaPriceRecommendationInput>(missing).is_err());
        let mut missing_strategy = encoded.clone();
        missing_strategy
            .as_object_mut()
            .expect("object")
            .remove("strategy");
        assert!(
            serde_json::from_value::<OwnerBetaPriceRecommendationInput>(missing_strategy).is_err()
        );
        let mut invalid = encoded;
        invalid["pins"]["candidate_content_sha256"] = json!("not-a-hash");
        assert!(serde_json::from_value::<OwnerBetaPriceRecommendationInput>(invalid).is_err());
    }

    #[test]
    fn every_pin_is_bound_to_the_snapshot() {
        let input = input();
        for field in [
            "candidate_content_sha256",
            "artifact_manifest_sha256",
            "stage5_manifest_sha256",
            "action_manifest_sha256",
            "approval_registry_sha256",
        ] {
            let mut changed = snapshot(&input);
            match field {
                "candidate_content_sha256" => {
                    changed.candidate_content_sha256 = hash(9).to_string()
                }
                "artifact_manifest_sha256" => {
                    changed.artifact_manifest_sha256 = hash(9).to_string()
                }
                "stage5_manifest_sha256" => changed.stage5_manifest_sha256 = hash(9).to_string(),
                "action_manifest_sha256" => changed.action_manifest_sha256 = hash(9).to_string(),
                "approval_registry_sha256" => {
                    changed.approval_registry_sha256 = hash(9).to_string()
                }
                _ => unreachable!(),
            }
            assert_eq!(
                input.validate_factor_snapshot(&changed),
                Err(OwnerBetaPriceRecommendationInputError::FactorSnapshotMismatch)
            );
        }
    }

    #[test]
    fn snapshot_as_of_input_kind_and_capability_are_bound() {
        let input = input();
        let mut changed = snapshot(&input);
        changed.as_of = TradingDate::parse("2026-08-25").expect("valid date");
        assert!(input.validate_factor_snapshot(&changed).is_err());
        let mut changed = snapshot(&input);
        changed.input_kind = "other".to_owned();
        assert!(input.validate_factor_snapshot(&changed).is_err());
        let mut changed = snapshot(&input);
        changed.capability = "other".to_owned();
        assert!(input.validate_factor_snapshot(&changed).is_err());
    }

    #[test]
    fn errors_do_not_leak_values() {
        let sentinel = "TOP_SECRET_SENTINEL";
        for error in [
            OwnerBetaPriceRecommendationInputError::ApprovalPinsMismatch,
            OwnerBetaPriceRecommendationInputError::FactorSnapshotMismatch,
            OwnerBetaPriceRecommendationInputError::StrategySnapshotInvalid,
            OwnerBetaPriceRecommendationInputError::StrategySnapshotMismatch,
        ] {
            assert!(!format!("{error:?}").contains(sentinel));
            assert!(!error.to_string().contains(sentinel));
        }
    }

    #[test]
    fn payload_debug_redacts_private_identity_and_first_four_pins() {
        let input = input();
        let debug = format!("{input:?}");

        assert!(!debug.contains(&input.run_id.to_string()));
        assert!(!debug.contains(&input.strategy_config_id.to_string()));
        for private_pin in [
            &input.pins.candidate_content_sha256,
            &input.pins.artifact_manifest_sha256,
            &input.pins.stage5_manifest_sha256,
            &input.pins.action_manifest_sha256,
        ] {
            assert!(!debug.contains(private_pin.as_str()));
        }
        assert!(debug.contains(input.pins.approval_registry_sha256.as_str()));
        assert!(!debug.contains(input.strategy.strategy_id()));
        assert!(!debug.contains(input.strategy.strategy_version()));
        assert!(!debug.contains(input.strategy.config_sha256().as_str()));
        assert!(!debug.contains("TOP_SECRET_STRATEGY_CONFIG"));
    }

    #[test]
    fn strategy_snapshot_hash_is_deterministic_and_catches_tampering() {
        let input = input();
        let reordered =
            OwnerBetaStrategySnapshot::from_resolved_config(&crate::resolver::ResolvedConfig {
                strategy_id: "buy_and_hold".to_owned(),
                strategy_version: "1.0.0".to_owned(),
                config: json!({"a": 7, "z": [2, 1]}),
            })
            .expect("snapshot");
        assert_eq!(
            input.strategy_snapshot().config_sha256(),
            reordered.config_sha256()
        );

        let mut tampered = input.clone();
        tampered.strategy.config_json["a"] = json!(8);
        assert_eq!(
            tampered.validate_strategy_snapshot(),
            Err(OwnerBetaPriceRecommendationInputError::StrategySnapshotMismatch)
        );
        let mut tampered = input;
        tampered.strategy.config_sha256 = hash(99);
        assert_eq!(
            tampered.validate_strategy_snapshot(),
            Err(OwnerBetaPriceRecommendationInputError::StrategySnapshotMismatch)
        );
    }

    #[test]
    fn canonical_json_excludes_forbidden_fields() {
        let json = serde_json::to_string(&input()).expect("serialize");
        for forbidden in [
            "dataset",
            "curated",
            "path",
            "raw",
            "account",
            "paper",
            "request",
            "env",
            "capability",
        ] {
            assert!(
                !json.to_ascii_lowercase().contains(forbidden),
                "forbidden field {forbidden}"
            );
        }
    }
}
