//! Universe publication: resolve the manifest against the instrument master
//! and the entitlement gate, and write an immutable versioned snapshot
//! (Todo 12).
//!
//! Publication is **fail closed**: it blocks with a typed error naming the
//! exact instrument + reason when a member id is
//!
//! - **inactive** — not in the instrument master, not listed on the effective
//!   date (delisted / not yet listed), or not `listed` status;
//! - **unsupported** — wrong asset class / currency / venue, a leveraged or
//!   inverse product (KRX product reference), or an eligibility profile that
//!   permits leverage/inverse/non-ETF products;
//! - **duplicated** — the same canonical id listed twice;
//! - **unlicensed** — the KRX dataset use is not `ACTIVE` (Todo 5 gate).
//!
//! A blocked publication NEVER substitutes a different product: the error
//! names the offending id, and no snapshot is produced.
//!
//! The published snapshot carries an immutable `universe_snapshot_id`
//! (content hash over the canonical manifest, [`crate::universe::UniverseManifest::canonical_hash`]):
//! repeated builds hash identically, and a changed manifest produces a new
//! snapshot id.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use auth::entitlement::{AccessRequest, Actor, CalendarDate, DatasetId, EntitlementService, KrUse};
use domain::{
    AssetClass, ContentHash, Currency, InstrumentId, InstrumentStatus, TradingDate, Venue,
};
use market_data::{DataUse, InstrumentMaster, MasterError, QualityReport};

use crate::universe::{Eligibility, SourceSnapshot, UniverseError, UniverseManifest};

/// The product kind of an instrument, resolved from the KRX product
/// reference. Only spot ETFs are supported in the fixed Korean ETF universe;
/// leveraged and inverse products are unsupported platform-wide (scope: no
/// leverage / shorting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProductKind {
    /// A plain (unleveraged, non-inverse) ETF — the only sanctioned kind.
    SpotEtf,
    /// A leveraged ETF product.
    Leveraged,
    /// An inverse ETF product.
    Inverse,
    /// Any other unsupported product kind.
    Other,
}

impl ProductKind {
    /// The stable wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SpotEtf => "spot_etf",
            Self::Leveraged => "leveraged",
            Self::Inverse => "inverse",
            Self::Other => "other",
        }
    }
}

/// The published immutable universe snapshot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PublishedSnapshot {
    /// The stable universe id (e.g. `kr-etf-core-v1`).
    pub universe_id: String,
    /// Base currency of the universe (KRW).
    pub base_currency: Currency,
    /// First effective date (inclusive).
    pub effective_from: TradingDate,
    /// Last effective date (inclusive); `None` = open-ended.
    pub effective_until: Option<TradingDate>,
    /// The benchmark instrument (e.g. `069500.KRX`).
    pub benchmark: InstrumentId,
    /// The eligibility profile (unleveraged / non-inverse / ETF).
    pub eligibility: Eligibility,
    /// The resolved unique member instruments (sorted).
    pub instruments: std::collections::BTreeSet<InstrumentId>,
    /// The KRX reference snapshot metadata.
    pub source_snapshot: SourceSnapshot,
    /// Immutable content hash over the canonical manifest.
    pub universe_snapshot_id: ContentHash,
}

impl PublishedSnapshot {
    /// Writes the snapshot to `dir` as `{universe_id}-{hash12}.json`. The
    /// filename is derived from the content hash, so the same manifest always
    /// writes the same file (immutable naming) and a changed manifest writes
    /// a new file.
    pub fn write_snapshot(&self, dir: &Path) -> Result<PathBuf, UniverseError> {
        let hex = self.universe_snapshot_id.as_str();
        let short: String = hex.chars().skip("sha256:".len()).take(12).collect();
        let path = dir.join(format!("{}-{short}.json", self.universe_id));
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| UniverseError::Internal {
            detail: format!("snapshot serialization failed: {e}"),
        })?;
        fs::write(&path, bytes).map_err(|e| UniverseError::Io {
            context: format!("write {}", path.display()),
            detail: e.to_string(),
        })?;
        Ok(path)
    }
}

/// Publishes a universe manifest into an immutable snapshot.
///
/// Owns the instrument master, the entitlement service, the KRX product
/// reference, and the optional required-dataset quality report (all cheap
/// clones) so callers can construct one per publication without lifetime
/// plumbing.
pub struct UniversePublisher {
    master: InstrumentMaster,
    entitlement: EntitlementService,
    /// Per-instrument product-kind overrides from the KRX product reference.
    /// An instrument with no record is governed by the manifest eligibility
    /// profile; a record declaring a leveraged / inverse / other product
    /// blocks that instrument by name.
    product_kinds: BTreeMap<InstrumentId, ProductKind>,
    /// The required curated dataset quality report (Todo 11). When present,
    /// a non-READY report blocks publication (fail closed on missing required
    /// bars / stale data).
    required_data: Option<QualityReport>,
}

impl UniversePublisher {
    /// A publisher with no product-kind overrides and no required-data gate.
    pub fn new(master: InstrumentMaster, entitlement: EntitlementService) -> Self {
        Self {
            master,
            entitlement,
            product_kinds: BTreeMap::new(),
            required_data: None,
        }
    }

    /// A publisher with an explicit KRX product reference.
    pub fn with_product_kinds(
        master: InstrumentMaster,
        entitlement: EntitlementService,
        product_kinds: BTreeMap<InstrumentId, ProductKind>,
    ) -> Self {
        Self {
            master,
            entitlement,
            product_kinds,
            required_data: None,
        }
    }

    /// Requires the curated dataset quality report to permit research use.
    pub fn with_required_data(mut self, report: QualityReport) -> Self {
        self.required_data = Some(report);
        self
    }

    /// Publishes `manifest`, blocking (typed error naming the exact
    /// instrument + reason) on any inactive, unsupported, duplicated, or
    /// unlicensed member — never substituting a different product.
    pub fn publish(&self, manifest: &UniverseManifest) -> Result<PublishedSnapshot, UniverseError> {
        let spec = &manifest.universe;

        // --- membership shape -------------------------------------------------
        let instruments = manifest.unique_instrument_ids()?;
        if instruments.is_empty() {
            return Err(UniverseError::EmptyUniverse);
        }
        if !instruments.contains(&spec.benchmark) {
            return Err(UniverseError::BenchmarkNotInUniverse {
                benchmark: spec.benchmark.clone(),
            });
        }

        // --- Todo 5 entitlement gate (dataset-level, fail closed) -------------
        let as_of = CalendarDate::parse(&spec.effective_from.to_iso()).map_err(|e| {
            UniverseError::Internal {
                detail: format!("invalid entitlement as-of date: {e}"),
            }
        })?;
        let request = AccessRequest {
            actor: Actor::owner("universe_publisher"),
            dataset: DatasetId::krx_eod_bars(),
            as_of,
        };
        if let Err(denied) = self.entitlement.authorize_use(KrUse::Dataset, &request) {
            return Err(UniverseError::Unlicensed {
                dataset: denied.dataset.0.clone(),
                use_kind: denied.use_kind.as_str().to_owned(),
                reason: denied.to_string(),
            });
        }

        // --- Todo 11 required-data gate (fail closed on BLOCKED) --------------
        if let Some(report) = &self.required_data
            && let Err(denial) = report.permits(DataUse::Recommendation)
        {
            let blocking: Vec<&str> = denial
                .blocking_issues
                .iter()
                .map(|code| code.as_str())
                .collect();
            return Err(UniverseError::RequiredDataNotReady {
                dataset_id: report.dataset_id.to_string(),
                state: report.state.to_string(),
                blocking_issues: blocking.join(", "),
            });
        }

        // --- per-instrument gates (manifest order, deterministic) -------------
        for entry in &spec.instruments {
            let id = &entry.id;
            self.check_instrument(manifest, id, spec.effective_from)?;
        }

        Ok(PublishedSnapshot {
            universe_id: spec.id.clone(),
            base_currency: spec.base_currency,
            effective_from: spec.effective_from,
            effective_until: spec.effective_until,
            benchmark: spec.benchmark.clone(),
            eligibility: spec.eligibility,
            instruments,
            source_snapshot: spec.source_snapshot.clone(),
            universe_snapshot_id: manifest.canonical_hash()?,
        })
    }

    /// One instrument gate: eligibility profile, master resolution (inactive),
    /// record conformance (unsupported), and product kind (unsupported).
    fn check_instrument(
        &self,
        manifest: &UniverseManifest,
        id: &InstrumentId,
        effective_from: TradingDate,
    ) -> Result<(), UniverseError> {
        let eligibility = &manifest.universe.eligibility;
        if !eligibility.unleveraged {
            return Err(UniverseError::UnsupportedInstrument {
            id: id.clone(),
            reason: "eligibility unleveraged=false (leveraged products are not supported in the fixed Korean ETF universe)".to_owned(),
        });
        }
        if !eligibility.non_inverse {
            return Err(UniverseError::UnsupportedInstrument {
            id: id.clone(),
            reason: "eligibility non_inverse=false (inverse products are not supported in the fixed Korean ETF universe)".to_owned(),
        });
        }
        if eligibility.asset_class != AssetClass::Etf {
            return Err(UniverseError::UnsupportedInstrument {
                id: id.clone(),
                reason: format!(
                    "eligibility asset_class={} (only etf is supported in the fixed Korean ETF universe)",
                    asset_class_name(eligibility.asset_class)
                ),
            });
        }

        let record = match self.master.instrument_on(id, effective_from) {
            Ok(record) => record,
            Err(MasterError::UnknownInstrument { .. }) => {
                return Err(UniverseError::InactiveInstrument {
                    id: id.clone(),
                    reason: format!("not in instrument master on {effective_from}"),
                });
            }
            Err(MasterError::NotListed { .. }) => {
                return Err(UniverseError::InactiveInstrument {
                    id: id.clone(),
                    reason: "not listed on the effective date".to_owned(),
                });
            }
            Err(other) => {
                return Err(UniverseError::Internal {
                    detail: format!("instrument master lookup failed for {id}: {other}"),
                });
            }
        };

        if record.status != InstrumentStatus::Listed {
            return Err(UniverseError::InactiveInstrument {
                id: id.clone(),
                reason: format!("status {}", record.status),
            });
        }
        if record.asset_class != AssetClass::Etf {
            return Err(UniverseError::UnsupportedInstrument {
                id: id.clone(),
                reason: format!("asset class {}", asset_class_name(record.asset_class)),
            });
        }
        if record.currency != Currency::KRW {
            return Err(UniverseError::UnsupportedInstrument {
                id: id.clone(),
                reason: format!("currency {} (expected KRW)", record.currency),
            });
        }
        if record.venue != Venue::Krx {
            return Err(UniverseError::UnsupportedInstrument {
                id: id.clone(),
                reason: format!("venue {} (expected KRX)", record.venue),
            });
        }

        if let Some(kind) = self.product_kinds.get(id)
            && *kind != ProductKind::SpotEtf
        {
            return Err(UniverseError::UnsupportedInstrument {
                id: id.clone(),
                reason: format!("{} product (KRX product reference)", kind.as_str()),
            });
        }
        Ok(())
    }
}

fn asset_class_name(asset_class: AssetClass) -> &'static str {
    match asset_class {
        AssetClass::Etf => "etf",
        AssetClass::Equity => "equity",
        AssetClass::Bond => "bond",
        AssetClass::Cash => "cash",
        AssetClass::Index => "index",
    }
}
