//! Universe manifest model, parsing, and canonical snapshot hashing (Todo 12).
//!
//! The **universe manifest** is the source of truth for a fixed universe: it
//! declares membership (exact canonical `InstrumentId = {symbol}.KRX` entries),
//! the benchmark, the base currency, the eligibility profile
//! (unleveraged / non-inverse / ETF — the only sanctioned profile), the
//! effective window, and the KRX reference snapshot the IDs were resolved from.
//!
//! Publication ([`crate::publish`]) then validates the manifest against the
//! instrument master and the entitlement gate; parsing here only performs
//! structural validation. Malformed YAML, unknown fields, invalid ids, and an
//! inverted effective window are typed [`UniverseError::MalformedManifest`]
//! errors — never panics.
//!
//! The immutable `universe_snapshot_id` is the SHA-256 content hash over the
//! **canonical form of the parsed manifest** ([`UniverseManifest::canonical_hash`]):
//! two builds of the same manifest produce identical hashes; any manifest
//! change produces a new hash.

use std::collections::BTreeSet;

use domain::{AssetClass, ContentHash, Currency, InstrumentId, TradingDate};
use serde::{Deserialize, Serialize};

/// Typed errors of the universe manifest pipeline.
///
/// Every publication-block variant names the exact instrument (or dataset)
/// and the reason — a blocked publication never substitutes another product.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UniverseError {
    /// The manifest is structurally invalid (YAML, schema, ids, window).
    #[error("universe manifest is malformed: {detail}")]
    MalformedManifest { detail: String },
    /// The manifest declares no instruments.
    #[error("universe manifest has no instruments")]
    EmptyUniverse,
    /// The same canonical id appears more than once in the manifest.
    #[error("duplicate instrument {id} in universe manifest")]
    DuplicateInstrument { id: InstrumentId },
    /// The declared benchmark is not a member of the universe.
    #[error("benchmark {benchmark} is not a member of the universe")]
    BenchmarkNotInUniverse { benchmark: InstrumentId },
    /// The instrument is not tradable on the effective date (not in the
    /// instrument master, delisted, not yet listed, or suspended).
    #[error("instrument {id} is inactive: {reason}")]
    InactiveInstrument { id: InstrumentId, reason: String },
    /// The instrument (or the universe profile) is outside the sanctioned
    /// spot-ETF / unleveraged / non-inverse scope.
    #[error("instrument {id} is unsupported: {reason}")]
    UnsupportedInstrument { id: InstrumentId, reason: String },
    /// The KRX dataset use underlying this universe is not ACTIVE (Todo 5).
    #[error("universe publication unlicensed: dataset {dataset} use {use_kind}: {reason}")]
    Unlicensed {
        dataset: String,
        use_kind: String,
        reason: String,
    },
    /// The required curated dataset is not READY (Todo 11 quality gate).
    #[error("required dataset {dataset_id} is not ready (state {state}): {blocking_issues}")]
    RequiredDataNotReady {
        dataset_id: String,
        state: String,
        blocking_issues: String,
    },
    /// Snapshot write failure.
    #[error("universe snapshot write failed ({context}): {detail}")]
    Io { context: String, detail: String },
    /// An internal invariant that cannot arise from user input.
    #[error("internal universe error: {detail}")]
    Internal { detail: String },
}

/// The universe eligibility profile. The fixed Korean ETF universe is
/// unleveraged, non-inverse, ETF-only — the only sanctioned profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Eligibility {
    /// Whether the universe contains only unleveraged products.
    pub unleveraged: bool,
    /// Whether the universe contains no inverse products.
    pub non_inverse: bool,
    /// The sanctioned asset class (etf).
    pub asset_class: AssetClass,
}

/// The KRX reference snapshot the manifest ids were resolved from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    /// The reference source (e.g. `krx-reference-2019-v1`).
    pub source: String,
    /// The reference version.
    pub version: String,
    /// The date the reference snapshot was captured.
    pub captured_at: String,
}

/// One member entry of the universe: an exact canonical instrument id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseInstrumentEntry {
    /// `{symbol}.KRX` — publication never substitutes a different product.
    pub id: InstrumentId,
}

/// The universe specification (the `universe:` YAML root).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseSpec {
    /// Stable universe id (e.g. `kr-etf-core-v1`).
    pub id: String,
    /// Base currency of the universe (KRW for the fixed Korean ETF universe).
    pub base_currency: Currency,
    /// First effective date (inclusive) of the universe.
    pub effective_from: TradingDate,
    /// Last effective date (inclusive); `None` = open-ended.
    pub effective_until: Option<TradingDate>,
    /// The benchmark instrument; must be a member of the universe.
    pub benchmark: InstrumentId,
    /// The eligibility profile (unleveraged / non-inverse / ETF).
    pub eligibility: Eligibility,
    /// The member instruments, in manifest order.
    pub instruments: Vec<UniverseInstrumentEntry>,
    /// The KRX reference snapshot metadata.
    pub source_snapshot: SourceSnapshot,
}

/// A parsed universe manifest (source of truth; the master validates at
/// publication time).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseManifest {
    pub universe: UniverseSpec,
}

/// The canonical byte form used for the immutable snapshot id. Field order is
/// fixed and every collection is sorted, so identical manifests hash
/// identically across builds.
#[derive(Debug, Clone, Serialize)]
struct CanonicalManifest<'a> {
    universe_id: &'a str,
    base_currency: &'a Currency,
    effective_from: &'a TradingDate,
    effective_until: Option<&'a TradingDate>,
    benchmark: &'a InstrumentId,
    unleveraged: bool,
    non_inverse: bool,
    asset_class: &'a AssetClass,
    instruments: BTreeSet<&'a InstrumentId>,
    source: &'a str,
    source_version: &'a str,
    captured_at: &'a str,
}

/// Parses and structurally validates a universe manifest from YAML.
///
/// Malformed YAML, unknown fields, invalid ids, and inverted effective
/// windows are typed [`UniverseError::MalformedManifest`] — never a panic.
/// Duplicate membership and eligibility are enforced at publication
/// ([`crate::publish::UniversePublisher::publish`]).
pub fn parse_manifest(yaml: &str) -> Result<UniverseManifest, UniverseError> {
    let manifest: UniverseManifest =
        serde_yaml::from_str(yaml).map_err(|e| UniverseError::MalformedManifest {
            detail: e.to_string(),
        })?;
    if manifest.universe.id.is_empty() {
        return Err(UniverseError::MalformedManifest {
            detail: "universe.id must not be empty".to_owned(),
        });
    }
    if let Some(until) = manifest.universe.effective_until
        && until < manifest.universe.effective_from
    {
        return Err(UniverseError::MalformedManifest {
            detail: format!(
                "inverted effective window: until {} precedes from {}",
                until.to_iso(),
                manifest.universe.effective_from.to_iso()
            ),
        });
    }
    Ok(manifest)
}

impl UniverseManifest {
    /// The unique member ids of the manifest, in sorted order. Returns
    /// [`UniverseError::DuplicateInstrument`] when a canonical id appears
    /// more than once.
    pub fn unique_instrument_ids(&self) -> Result<BTreeSet<InstrumentId>, UniverseError> {
        let mut seen = BTreeSet::new();
        for entry in &self.universe.instruments {
            if !seen.insert(entry.id.clone()) {
                return Err(UniverseError::DuplicateInstrument {
                    id: entry.id.clone(),
                });
            }
        }
        Ok(seen)
    }

    /// The immutable snapshot id: SHA-256 over the canonical form of the
    /// parsed manifest (excluding the hash itself). Repeated builds of the
    /// same manifest hash identically; any manifest change yields a new id.
    pub fn canonical_hash(&self) -> Result<ContentHash, UniverseError> {
        let canonical = CanonicalManifest {
            universe_id: &self.universe.id,
            base_currency: &self.universe.base_currency,
            effective_from: &self.universe.effective_from,
            effective_until: self.universe.effective_until.as_ref(),
            benchmark: &self.universe.benchmark,
            unleveraged: self.universe.eligibility.unleveraged,
            non_inverse: self.universe.eligibility.non_inverse,
            asset_class: &self.universe.eligibility.asset_class,
            instruments: self.universe.instruments.iter().map(|e| &e.id).collect(),
            source: &self.universe.source_snapshot.source,
            source_version: &self.universe.source_snapshot.version,
            captured_at: &self.universe.source_snapshot.captured_at,
        };
        let bytes = serde_json::to_vec(&canonical).map_err(|e| UniverseError::Internal {
            detail: format!("canonical manifest serialization failed: {e}"),
        })?;
        Ok(ContentHash::from_bytes(&bytes))
    }
}
