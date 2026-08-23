use std::fmt;

use domain::{ContentHash, TradingDate};
use factor_engine::{
    PriceOnlyFactorSnapshot,
    price_only::{PRICE_ONLY_CAPABILITY, PRICE_ONLY_INPUT_KIND},
};
use market_data::ApprovedHistoricalPriceOnlyArtifact;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The only job type for an owner-beta price recommendation.
pub const OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE: &str = "owner_beta_price_recommendation";

/// Sealed, price-only provenance for an owner-beta recommendation job.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerBetaPriceRecommendationInput {
    run_id: Uuid,
    strategy_config_id: Uuid,
    as_of: TradingDate,
    pins: OwnerBetaPriceRecommendationPins,
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
    ) -> Self {
        let artifact_pins = artifact.pins();
        Self {
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
        }
    }

    /// Named form of [`Self::new`] for call sites that make the trust boundary
    /// explicit.
    pub fn from_approved_artifact(
        run_id: Uuid,
        strategy_config_id: Uuid,
        as_of: TradingDate,
        artifact: &ApprovedHistoricalPriceOnlyArtifact,
    ) -> Self {
        Self::new(run_id, strategy_config_id, as_of, artifact)
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
    };

    fn date() -> TradingDate {
        TradingDate::parse("2026-08-24").expect("valid test date")
    }

    fn hash(value: u8) -> ContentHash {
        ContentHash::parse(&format!("sha256:{value:064x}")).expect("valid test hash")
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
        let mut missing = encoded.clone();
        missing.as_object_mut().expect("object").remove("as_of");
        assert!(serde_json::from_value::<OwnerBetaPriceRecommendationInput>(missing).is_err());
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
