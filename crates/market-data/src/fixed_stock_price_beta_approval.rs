//! Owner-only registry for the fixed stock price beta.  The checked-in file
//! starts empty; a proposal is evidence for a later owner review, never an
//! approval by itself.
use crate::{FIXED_30_INSTRUMENT_IDS, FixedStockPriceBetaArtifact};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const FIXED_STOCK_PRICE_BETA_APPROVAL_SCHEMA_ID: &str =
    "kr-stock-price-beta-approved-artifacts";
pub const FIXED_STOCK_PRICE_BETA_APPROVAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FixedStockPriceBetaApprovalError {
    #[error("invalid fixed-stock beta approval registry")]
    Invalid,
    #[error("fixed-stock beta artifact is not owner approved")]
    Unapproved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedStockPriceBetaApprovalRegistry {
    pub schema_id: String,
    pub schema_version: u32,
    pub approved_artifacts: Vec<FixedStockPriceBetaApprovedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedStockPriceBetaApprovedArtifact {
    pub status: String,
    pub audience: String,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub capability: String,
    pub selection_basis: String,
    pub index_membership: String,
    pub redistribution: String,
    pub publication_status: String,
    pub universe_sha256: String,
    pub entitlement_sha256: String,
    pub batch_id: String,
    pub source_file_count: usize,
    pub factor_version: String,
    pub capture_commit: String,
    pub batch_json_sha256: String,
    pub manifest_sha256: String,
    pub artifact_content_sha256: String,
    pub snapshot_content_sha256: String,
    pub range_start: String,
    pub range_end: String,
    pub as_of: String,
    pub instruments: Vec<String>,
    pub instrument_count: usize,
    pub session_count: usize,
    pub bar_count: usize,
    pub materialization_status: String,
    pub registration_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedStockPriceBetaVerifiedBundle {
    pub registry_sha256: String,
    pub approval: FixedStockPriceBetaApprovedArtifact,
}

pub fn parse_fixed_stock_price_beta_approval_registry(
    bytes: &[u8],
) -> Result<FixedStockPriceBetaApprovalRegistry, FixedStockPriceBetaApprovalError> {
    let registry: FixedStockPriceBetaApprovalRegistry =
        serde_json::from_slice(bytes).map_err(|_| FixedStockPriceBetaApprovalError::Invalid)?;
    if registry.schema_id != FIXED_STOCK_PRICE_BETA_APPROVAL_SCHEMA_ID
        || registry.schema_version != FIXED_STOCK_PRICE_BETA_APPROVAL_SCHEMA_VERSION
    {
        return Err(FixedStockPriceBetaApprovalError::Invalid);
    }
    let mut keys = std::collections::BTreeSet::new();
    for entry in &registry.approved_artifacts {
        validate(entry)?;
        if !keys.insert((
            &entry.artifact_content_sha256,
            &entry.snapshot_content_sha256,
        )) {
            return Err(FixedStockPriceBetaApprovalError::Invalid);
        }
    }
    Ok(registry)
}

/// Returns a typed, pinned bundle only when exactly one registry entry matches
/// the independently verified artifact and snapshot identities.
pub fn verify_fixed_stock_price_beta_approval(
    registry_bytes: &[u8],
    artifact: &FixedStockPriceBetaArtifact,
    snapshot_content_sha256: &str,
    as_of: &str,
    batch_id: &str,
) -> Result<FixedStockPriceBetaVerifiedBundle, FixedStockPriceBetaApprovalError> {
    artifact
        .verify()
        .map_err(|_| FixedStockPriceBetaApprovalError::Unapproved)?;
    let registry = parse_fixed_stock_price_beta_approval_registry(registry_bytes)?;
    let matches: Vec<_> = registry
        .approved_artifacts
        .into_iter()
        .filter(|entry| {
            entry.artifact_content_sha256 == artifact.content_sha256
                && entry.snapshot_content_sha256 == snapshot_content_sha256
                && entry.as_of == as_of
                && entry.batch_json_sha256 == artifact.evidence.batch_json_sha256
                && entry.manifest_sha256 == artifact.evidence.manifest_sha256
                && entry.capture_commit == artifact.evidence.capture_commit
                && entry.entitlement_sha256 == artifact.evidence.entitlement_sha256
                && entry.batch_id == batch_id
                && entry.source_file_count == artifact.evidence.files.len()
                && entry.factor_version == "fixed-stock-price-beta-factors-v1"
                && entry.session_count == artifact.sessions.len()
                && entry.bar_count == artifact.bars.len()
                && artifact.sessions.last().is_some_and(|date| date == as_of)
        })
        .collect();
    if matches.len() != 1 {
        return Err(FixedStockPriceBetaApprovalError::Unapproved);
    }
    Ok(FixedStockPriceBetaVerifiedBundle {
        registry_sha256: hash(registry_bytes),
        approval: matches.into_iter().next().unwrap(),
    })
}

fn validate(
    entry: &FixedStockPriceBetaApprovedArtifact,
) -> Result<(), FixedStockPriceBetaApprovalError> {
    if entry.status != "APPROVED"
        || entry.audience != "OWNER_ONLY"
        || !entry.vendor_snapshot
        || entry.strict_pit
        || entry.capability != "PRICE_VOLUME_RESEARCH_ONLY"
        || entry.selection_basis != "CONFIGURED_FIXED_LIST"
        || entry.index_membership != "NOT_EVALUATED"
        || entry.redistribution != "NO_REDISTRIBUTION"
        || entry.publication_status != "NOT_PUBLISHED"
        || entry.materialization_status != "MATERIALIZED"
        || entry.registration_status != "UNREGISTERED"
        || entry.universe_sha256 != crate::FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
        || !hex(&entry.entitlement_sha256)
        || !uuid(&entry.batch_id)
        || entry.source_file_count != 90
        || entry.factor_version != "fixed-stock-price-beta-factors-v1"
        || !hex(&entry.batch_json_sha256)
        || !hex(&entry.manifest_sha256)
        || !hex(&entry.artifact_content_sha256)
        || !hex(&entry.snapshot_content_sha256)
        || !commit(&entry.capture_commit)
        || entry.range_start != crate::FIXED_STOCK_PRICE_BETA_RANGE_START
        || entry.range_end != crate::FIXED_STOCK_PRICE_BETA_RANGE_END
        || chrono::NaiveDate::parse_from_str(&entry.as_of, "%Y-%m-%d").is_err()
        || entry.as_of < entry.range_start
        || entry.as_of > entry.range_end
        || entry.instrument_count != 30
        || entry
            .instruments
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != FIXED_30_INSTRUMENT_IDS
        || entry.session_count < 121
        || entry.bar_count != entry.session_count * 30
    {
        return Err(FixedStockPriceBetaApprovalError::Invalid);
    }
    Ok(())
}
fn hex(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn commit(v: &str) -> bool {
    v.len() == 40
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn uuid(v: &str) -> bool {
    v.len() == 36
        && v.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn hash(v: &[u8]) -> String {
    format!("{:x}", Sha256::digest(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_registry_is_valid_but_approves_nothing() {
        let b = br#"{"schema_id":"kr-stock-price-beta-approved-artifacts","schema_version":1,"approved_artifacts":[]}"#;
        assert!(parse_fixed_stock_price_beta_approval_registry(b).is_ok());
    }
    #[test]
    fn malformed_registry_is_rejected() {
        assert!(parse_fixed_stock_price_beta_approval_registry(br#"{}"#).is_err());
    }
}
