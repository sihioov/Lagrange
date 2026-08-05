//! Deterministic factor snapshots: per-date frozen cross-sectional
//! normalization plus byte/hash-stable output (requirements FR-SEL-001,
//! design §6.5 "횡단면 표준화 시 해당 날짜 후보군 스냅샷을 고정").
//!
//! The snapshot freezes, per date, the intersection of the build-time
//! universe with the instruments that have a bar on that date. Normalization
//! for a (date, factor) pair uses EXACTLY that cross-section — an instrument
//! outside the frozen universe never appears in rows nor in any statistic.
//!
//! Determinism: rows are canonically ordered (date, instrument, factor), the
//! hash is SHA-256 over canonical JSON bytes that exclude the hash itself, so
//! identical inputs produce identical bytes/hash and ANY change (data,
//! universe, factor version, normalization, as-of) produces a different hash.

use std::collections::{BTreeMap, BTreeSet};

use domain::{ContentHash, InstrumentId, TradingDate};
use market_data::CurateStore;
use serde::Serialize;

use crate::bars::Bars;
use crate::contract::{Factor, FactorContext, FactorError, FactorId};
use crate::factors::all_mvp_factors;
use crate::normalize::{NormalizePolicy, ZScorePolicy};

/// The frozen cross-sectional universe of one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenUniverse {
    snapshot_id: String,
    instruments: BTreeSet<InstrumentId>,
}

impl FrozenUniverse {
    /// A universe from canonical instrument ids.
    ///
    /// Panics on a non-canonical id — the ids are literals from the versioned
    /// universe manifest (Todo 12); the manifest path itself is validated
    /// upstream by the selector (Todo 16).
    pub fn new(snapshot_id: &str, symbols: &[&str]) -> Self {
        let instruments = symbols
            .iter()
            .map(|s| InstrumentId::parse(s).expect("canonical instrument id"))
            .collect();
        Self {
            snapshot_id: snapshot_id.to_owned(),
            instruments,
        }
    }

    /// The immutable universe snapshot id (Todo 12 manifest id).
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    /// The members, in canonical (sorted) order.
    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.instruments.iter()
    }

    /// Whether the universe contains the instrument.
    pub fn contains(&self, id: &InstrumentId) -> bool {
        self.instruments.contains(id)
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.instruments.len()
    }

    /// Whether the universe is empty.
    pub fn is_empty(&self) -> bool {
        self.instruments.is_empty()
    }
}

/// The recorded normalization policy of a snapshot (part of the hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NormalizationMeta {
    pub id: String,
    pub version: String,
    pub params: BTreeMap<String, String>,
}

/// One snapshot row: raw + normalized value of one factor for one instrument
/// on one date.
#[derive(Debug, Clone, PartialEq)]
pub struct FactorRow {
    pub date: String,
    pub instrument: String,
    pub factor: String,
    pub raw: Option<f64>,
    pub normalized: Option<f64>,
}

/// A deterministic, hash-stable factor snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct FactorSnapshot {
    pub as_of: TradingDate,
    pub universe_snapshot_id: String,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub factor_versions: BTreeMap<String, String>,
    pub normalization: NormalizationMeta,
    /// Canonically ordered rows: (date, instrument, factor).
    pub rows: Vec<FactorRow>,
    /// SHA-256 over the canonical bytes (see [`FactorSnapshot::canonical_bytes`]).
    pub hash: ContentHash,
}

impl FactorSnapshot {
    /// The canonical bytes the hash covers (everything except the hash).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, FactorError> {
        #[derive(Serialize)]
        struct CanonicalRow<'a> {
            date: &'a str,
            instrument: &'a str,
            factor: &'a str,
            raw: Option<f64>,
            normalized: Option<f64>,
        }
        #[derive(Serialize)]
        struct Canonical<'a> {
            as_of: &'a str,
            universe_snapshot_id: &'a str,
            dataset_id: &'a str,
            dataset_version: u32,
            factor_versions: &'a BTreeMap<String, String>,
            normalization: &'a NormalizationMeta,
            rows: Vec<CanonicalRow<'a>>,
        }
        let canonical = Canonical {
            as_of: &self.as_of.to_iso(),
            universe_snapshot_id: &self.universe_snapshot_id,
            dataset_id: &self.dataset_id,
            dataset_version: self.dataset_version,
            factor_versions: &self.factor_versions,
            normalization: &self.normalization,
            rows: self
                .rows
                .iter()
                .map(|r| CanonicalRow {
                    date: &r.date,
                    instrument: &r.instrument,
                    factor: &r.factor,
                    raw: r.raw,
                    normalized: r.normalized,
                })
                .collect(),
        };
        serde_json::to_vec(&canonical).map_err(|e| FactorError::Serialize {
            detail: format!("canonical snapshot: {e}"),
        })
    }

    /// The SHA-256 of the canonical bytes.
    pub fn compute_hash(&self) -> Result<ContentHash, FactorError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }
}

/// Builds one deterministic factor snapshot.
pub struct FactorSnapshotBuilder<'a> {
    as_of: TradingDate,
    universe: FrozenUniverse,
    store: &'a CurateStore,
    market: &'a str,
    dataset_id: &'a str,
    dataset_version: u32,
    factors: Vec<Box<dyn Factor>>,
    normalization: Box<dyn NormalizePolicy>,
}

impl<'a> FactorSnapshotBuilder<'a> {
    /// Defaults: the full 13-factor MVP registry, z-score normalization with
    /// a +-3 cap.
    pub fn new(
        as_of: TradingDate,
        universe: FrozenUniverse,
        store: &'a CurateStore,
        market: &'a str,
        dataset_id: &'a str,
        dataset_version: u32,
    ) -> Self {
        Self {
            as_of,
            universe,
            store,
            market,
            dataset_id,
            dataset_version,
            factors: all_mvp_factors(),
            normalization: Box::new(ZScorePolicy::default()),
        }
    }

    /// Overrides the factor registry (must be the documented set or a
    /// versioned variant; ids must be unique).
    pub fn with_factors(mut self, factors: Vec<Box<dyn Factor>>) -> Self {
        self.factors = crate::factors::registry_with(factors).expect("registry ids must be unique");
        self
    }

    /// Overrides the normalization policy.
    pub fn with_normalization(mut self, normalization: Box<dyn NormalizePolicy>) -> Self {
        self.normalization = normalization;
        self
    }

    /// Builds the snapshot: resolve bars (future-row gate), compute every
    /// factor lazily, normalize per-date over the frozen cross-section, then
    /// hash the canonical bytes.
    pub fn build(self) -> Result<FactorSnapshot, FactorError> {
        let bars = Bars::from_curated(
            self.store,
            self.market,
            self.dataset_id,
            self.dataset_version,
            &self.universe,
            self.as_of,
        )?;

        for f in &self.factors {
            for field in f.required_fields() {
                if !bars.available_fields().contains(field) {
                    return Err(FactorError::MissingField {
                        factor: f.id().to_owned(),
                        field: field.as_str().to_owned(),
                    });
                }
            }
        }

        let ctx = FactorContext {
            as_of: self.as_of,
            universe: &self.universe,
            bars: &bars,
        };
        let mut raw: BTreeMap<(String, String), BTreeMap<FactorId, Option<f64>>> = BTreeMap::new();
        for f in &self.factors {
            let frame = f.compute(&ctx)?;
            for row in frame.rows {
                raw.entry((row.date.to_iso(), row.instrument.to_string()))
                    .or_default()
                    .insert(frame.factor, row.value);
            }
        }

        // Normalize per date over that date's frozen cross-section.
        let mut factor_ids: Vec<FactorId> = self.factors.iter().map(|f| f.id()).collect();
        factor_ids.sort_unstable();
        let mut by_date: BTreeMap<String, BTreeMap<String, BTreeMap<FactorId, Option<f64>>>> =
            BTreeMap::new();
        for ((date, instrument), values) in raw {
            by_date.entry(date).or_default().insert(instrument, values);
        }
        let mut rows: Vec<FactorRow> = Vec::new();
        for (date, instruments) in &by_date {
            let members: Vec<&String> = instruments.keys().collect();
            let mut normalized: BTreeMap<FactorId, Vec<Option<f64>>> = BTreeMap::new();
            for f in &factor_ids {
                let xs: Vec<Option<f64>> = members
                    .iter()
                    .map(|inst| instruments[*inst].get(*f).copied().flatten())
                    .collect();
                normalized.insert(*f, self.normalization.apply(&xs));
            }
            for (idx, instrument) in members.iter().enumerate() {
                for f in &factor_ids {
                    rows.push(FactorRow {
                        date: date.clone(),
                        instrument: (*instrument).clone(),
                        factor: (*f).to_owned(),
                        raw: instruments[*instrument].get(*f).copied().flatten(),
                        normalized: normalized[f][idx],
                    });
                }
            }
        }

        let factor_versions: BTreeMap<String, String> = self
            .factors
            .iter()
            .map(|f| (f.id().to_owned(), f.version().to_string()))
            .collect();
        let normalization = NormalizationMeta {
            id: self.normalization.id().to_owned(),
            version: self.normalization.version().to_string(),
            params: self.normalization.params(),
        };
        let snapshot = FactorSnapshot {
            as_of: self.as_of,
            universe_snapshot_id: self.universe.snapshot_id.clone(),
            dataset_id: self.dataset_id.to_owned(),
            dataset_version: self.dataset_version,
            factor_versions,
            normalization,
            rows,
            hash: ContentHash::from_bytes(b"placeholder"),
        };
        let hash = snapshot.compute_hash()?;
        Ok(FactorSnapshot { hash, ..snapshot })
    }
}
