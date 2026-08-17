//! Curated zone: normalize Raw batches into partitioned, versioned Curated
//! Parquet with point-in-time corporate actions (Todo 10).
//!
//! Data flow (design §7.1 layout, §9.3 corporate-action model):
//!
//! ```text
//! data/raw/.../batch=<id>/  (immutable, hash-verified via RawStore)
//!   -> data/curated/
//!        bars/market={m}/symbol={instrument}/year={yyyy}/version={v}/
//!            bars.parquet              raw OHLCV + provenance (EXECUTION)
//!            adjusted_bars.parquet     split-adjusted (signals, price return)
//!            total_return_bars.parquet split+dividend (signals, total return)
//!        corporate_actions/market={m}/symbol={instrument}/year={yyyy}/version={v}/
//!            corporate_actions.parquet announced_at/ex_date/pay_date records
//!        datasets/{dataset_id}/version={v}/manifest.json
//! ```
//!
//! Invariants enforced here:
//! - **No look-ahead**: an action with `announced_at > now` is rejected at
//!   curation; the versioned files only ever embed announcements made by
//!   curation time; `visible_actions(as_of)` gates every later query.
//! - **Correction = new version**: a corrected batch curates under
//!   `version={v+1}`; an existing version directory is never touched, so the
//!   old version's bytes/hash are immutable (requirements §8.3).
//! - **OHLC normalization**: high/low bounds and strict positivity are
//!   enforced; NaN is structurally impossible (no float columns, see
//!   [`schema`]); negative prices/volumes are rejected.
//! - **Execution uses the raw open**: the raw bars table is never adjusted;
//!   only the `adjusted_*` tables scale prices (requirements §9.2).
//! - **Capability** (`PRICE_RETURN_ONLY | TOTAL_RETURN_CAPABLE`) is recorded
//!   per version; total-return requires complete dividend pay-date data.
//! - **Provenance per row**: `source`, `ingested_at`, `batch_id`, `raw_hash`
//!   link every curated row to its immutable Raw file.

pub mod actions;
pub mod adjust;
pub mod parse;
pub mod schema;

pub use self::schema::{read_adjusted_bars, read_bars, read_corporate_actions};

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Datelike;
use domain::{
    AssetClass, BatchId, ContentHash, Currency, DatasetId, FixedPoint, InstrumentId,
    InstrumentStatus, Price, Quantity, TradingDate, UtcTimestamp, Venue, Zone,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::calendar::{Holiday, KrCalendar, KrCalendarSpec, SessionTimes};
use crate::contract::{FetchMode, MARKET_KR, PROVIDER_KIS_NORMALIZED, PROVIDER_KRX, ResponseKind};
use crate::instrument_master::{Instrument, InstrumentMaster, MasterError};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::storage::{ManifestEntry, RawStore};

use self::actions::{CorporateAction, CorporateActionType, dataset_capability};
use self::adjust::{AdjustmentBar, adjusted_series};
use self::parse::{
    RawActionRow, RawBarsDoc, parse_actions, parse_bars, parse_date, parse_fixed,
    parse_fixed_value, parse_timestamp,
};
use self::schema::{CuratedBar, write_adjusted_bars, write_bars, write_corporate_actions};

/// The dataset capability flag (explicit per version; plan Todo 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Capability {
    /// Signals may only use price-return (split-adjusted) series.
    PriceReturnOnly,
    /// Signals may use total-return series (complete dividend pay-date data).
    TotalReturnCapable,
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PriceReturnOnly => f.write_str("PRICE_RETURN_ONLY"),
            Self::TotalReturnCapable => f.write_str("TOTAL_RETURN_CAPABLE"),
        }
    }
}

/// A typed curation failure.
#[derive(Debug, thiserror::Error)]
pub enum CurateError {
    #[error("curated store failure ({context}): {detail}")]
    StoreIo { context: String, detail: String },
    #[error("malformed dataset manifest ({context}): {detail}")]
    MalformedManifest { context: String, detail: String },
    #[error("malformed curated parquet ({context}): {detail}")]
    MalformedParquet { context: String, detail: String },
    #[error("missing curated component: {path}")]
    MissingCuratedComponent { path: String },
    #[error("raw store failure ({context}): {source}")]
    RawStore {
        context: String,
        #[source]
        source: crate::storage::StoreError,
    },
    /// Legacy string-only raw read error retained for downstream API
    /// compatibility. New RawStore reads must use [`Self::RawStore`] so the
    /// worker can classify filesystem failures without guessing from text.
    #[error("raw read failure ({context}): {detail}")]
    RawIo { context: String, detail: String },
    #[error("curation supports only {expected_scopes}, got {provider}/{market}")]
    UnsupportedScope {
        expected_scopes: &'static str,
        provider: String,
        market: String,
    },
    #[error("curation scope {provider}/{market} requires {expected} fetch mode, got {actual}")]
    UnsupportedMode {
        provider: String,
        market: String,
        expected: FetchMode,
        actual: FetchMode,
    },
    #[error("normalized curation batch has noncanonical shape: {reason}")]
    NonCanonicalNormalizedBatch { reason: String },
    #[error("batch is missing its {kind} file")]
    MissingFile { kind: ResponseKind },
    #[error("malformed bars response: {reason}")]
    MalformedBars { reason: String },
    #[error("malformed corporate-action response: {reason}")]
    MalformedAction { reason: String },
    #[error("unknown instrument {instrument}")]
    UnknownInstrument { instrument: String },
    #[error("instrument {instrument} not listed on {date}")]
    NotListed { instrument: String, date: String },
    #[error("bar on non-session date: {instrument} {date}")]
    NotASession { instrument: String, date: String },
    #[error("impossible OHLC on {instrument} {date}: {detail}")]
    ImpossibleOhlc {
        instrument: String,
        date: String,
        detail: String,
    },
    #[error("non-positive price {field} on {instrument} {date}: {value}")]
    NonPositivePrice {
        instrument: String,
        date: String,
        field: String,
        value: String,
    },
    #[error("non-finite price {field} on {instrument} {date}: {value}")]
    NonFinitePrice {
        instrument: String,
        date: String,
        field: String,
        value: String,
    },
    #[error("negative volume on {instrument} {date}: {volume}")]
    NegativeVolume {
        instrument: String,
        date: String,
        volume: String,
    },
    #[error(
        "currency conflict for {instrument}: {context} declares {declared}, expected {expected}"
    )]
    CurrencyConflict {
        instrument: String,
        expected: String,
        declared: String,
        context: String,
    },
    #[error("duplicate bar {instrument} {date}")]
    DuplicateBar { instrument: String, date: String },
    #[error("action announced in the future: {instrument} announced_at {announced_at}, now {now}")]
    FutureAnnouncedAction {
        instrument: String,
        announced_at: String,
        now: String,
    },
    #[error("invalid split for {instrument}: {detail}")]
    InvalidSplit { instrument: String, detail: String },
    #[error("invalid dividend for {instrument}: {detail}")]
    InvalidDividend { instrument: String, detail: String },
    #[error("batch {batch_id} already curated as dataset {dataset_id} version {version}")]
    BatchAlreadyCurated {
        dataset_id: String,
        version: u32,
        batch_id: String,
    },
    #[error("no bars in the batch (dataset {dataset_id})")]
    EmptyBars { dataset_id: String },
    #[error("no price for target date {target_date} (dataset {dataset_id})")]
    EodUnavailable {
        dataset_id: String,
        target_date: TradingDate,
    },
    #[error("domain arithmetic failure: {0}")]
    Domain(#[from] domain::DomainError),
}

/// One raw batch referenced by a curated dataset version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBatchRef {
    pub batch_id: BatchId,
    pub bars_file: String,
    pub bars_hash: ContentHash,
    pub actions_file: String,
    pub actions_hash: ContentHash,
}

/// The immutable per-version dataset manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub dataset_id: DatasetId,
    pub version: u32,
    pub capability: Capability,
    pub created_at: UtcTimestamp,
    pub source_batches: Vec<SourceBatchRef>,
    pub bar_count: u64,
    pub action_count: u64,
    /// SHA-256 over the canonical manifest bytes (excluding the hash itself).
    pub content_hash: ContentHash,
}

/// The curated output zone rooted at `data/`.
#[derive(Debug, Clone)]
pub struct CurateStore {
    root: PathBuf,
}

impl CurateStore {
    /// `root` is the `data/` directory; curated files live under
    /// `root/curated/...` (design §7.1 layout).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The `data/` root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `data/curated/bars/market={m}/symbol={s}/year={y}/version={v}/bars.parquet`
    pub fn bars_path(&self, market: &str, symbol: &str, year: i32, version: u32) -> PathBuf {
        self.curated_dir()
            .join("bars")
            .join(format!("market={market}"))
            .join(format!("symbol={symbol}"))
            .join(format!("year={year}"))
            .join(format!("version={version}"))
            .join("bars.parquet")
    }

    /// The split-adjusted series file next to the raw bars.
    pub fn adjusted_bars_path(
        &self,
        market: &str,
        symbol: &str,
        year: i32,
        version: u32,
    ) -> PathBuf {
        let dir = self
            .bars_path(market, symbol, year, version)
            .parent()
            .expect("bars path has a parent")
            .to_path_buf();
        dir.join("adjusted_bars.parquet")
    }

    /// The total-return series file next to the raw bars.
    pub fn total_return_bars_path(
        &self,
        market: &str,
        symbol: &str,
        year: i32,
        version: u32,
    ) -> PathBuf {
        let dir = self
            .bars_path(market, symbol, year, version)
            .parent()
            .expect("bars path has a parent")
            .to_path_buf();
        dir.join("total_return_bars.parquet")
    }

    /// `data/curated/fundamentals/market={m}/symbol={s}/version={v}/fundamentals.parquet`
    ///
    /// NOT partitioned by year, unlike bars. A fundamentals row is addressed by
    /// two dates -- the period it describes and the date it became knowable --
    /// and those disagree by design: a Q4 figure announced in March belongs to
    /// the previous year by period and to this one by knowledge. Partitioning
    /// on either would split the series a point-in-time read has to rejoin, and
    /// would invite the reader to skip a partition it actually needs.
    pub fn fundamentals_path(&self, market: &str, symbol: &str, version: u32) -> PathBuf {
        self.curated_dir()
            .join("fundamentals")
            .join(format!("market={market}"))
            .join(format!("symbol={symbol}"))
            .join(format!("version={version}"))
            .join("fundamentals.parquet")
    }

    /// `data/curated/corporate_actions/market={m}/symbol={s}/year={y}/version={v}/corporate_actions.parquet`
    pub fn corporate_actions_path(
        &self,
        market: &str,
        symbol: &str,
        year: i32,
        version: u32,
    ) -> PathBuf {
        self.curated_dir()
            .join("corporate_actions")
            .join(format!("market={market}"))
            .join(format!("symbol={symbol}"))
            .join(format!("year={year}"))
            .join(format!("version={version}"))
            .join("corporate_actions.parquet")
    }

    /// `data/curated/datasets/{dataset_id}/version={v}`
    pub fn dataset_dir(&self, dataset_id: &DatasetId, version: u32) -> PathBuf {
        self.curated_dir()
            .join("datasets")
            .join(dataset_id.as_str())
            .join(format!("version={version}"))
    }

    /// The next dataset version: max existing + 1, or 1 for a fresh dataset.
    pub fn next_version(&self, dataset_id: &DatasetId) -> Result<u32, CurateError> {
        let version_path = self.dataset_dir(dataset_id, 0);
        let dir = version_path.parent().expect("dataset dir has a parent");
        if !dir.exists() {
            return Ok(1);
        }
        let mut max = 0u32;
        for entry in fs::read_dir(dir).map_err(|e| CurateError::StoreIo {
            context: format!("list {}", dir.display()),
            detail: e.to_string(),
        })? {
            let entry = entry.map_err(|e| CurateError::StoreIo {
                context: "read_dir entry".to_owned(),
                detail: e.to_string(),
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("version=") else {
                continue;
            };
            if let Ok(v) = rest.parse::<u32>() {
                max = max.max(v);
            }
        }
        Ok(max + 1)
    }

    /// Reads a dataset manifest, if that version exists.
    pub fn read_dataset_manifest(
        &self,
        dataset_id: &DatasetId,
        version: u32,
    ) -> Result<Option<DatasetManifest>, CurateError> {
        let path = self.dataset_dir(dataset_id, version).join("manifest.json");
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err(CurateError::MalformedManifest {
                    context: format!("inspect {}", path.display()),
                    detail: "manifest path is not a regular file".to_owned(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CurateError::StoreIo {
                    context: format!("inspect {}", path.display()),
                    detail: error.to_string(),
                });
            }
        }
        let bytes = fs::read(&path).map_err(|e| CurateError::StoreIo {
            context: format!("read {}", path.display()),
            detail: e.to_string(),
        })?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| CurateError::MalformedManifest {
                context: format!("parse {}", path.display()),
                detail: e.to_string(),
            })
    }

    /// The latest manifest of a dataset, if any.
    fn latest_manifest(
        &self,
        dataset_id: &DatasetId,
    ) -> Result<Option<DatasetManifest>, CurateError> {
        let next = self.next_version(dataset_id)?;
        if next == 1 {
            return Ok(None);
        }
        self.read_dataset_manifest(dataset_id, next - 1)
    }

    /// Locate the unique immutable generation produced from a Raw batch.
    /// Duplicate provenance is treated as corruption rather than selecting a
    /// convenient generation.
    pub fn manifest_for_source_batch(
        &self,
        dataset_id: &DatasetId,
        batch_id: BatchId,
    ) -> Result<Option<DatasetManifest>, CurateError> {
        let mut found = None;
        for version in 1..self.next_version(dataset_id)? {
            let Some(manifest) = self.read_dataset_manifest(dataset_id, version)? else {
                continue;
            };
            if manifest
                .source_batches
                .iter()
                .any(|source| source.batch_id == batch_id)
            {
                if found.is_some() {
                    return Err(CurateError::MalformedManifest {
                        context: "source batch lookup".to_owned(),
                        detail: format!("batch {batch_id} appears in multiple generations"),
                    });
                }
                found = Some(manifest);
            }
        }
        Ok(found)
    }

    fn curated_dir(&self) -> PathBuf {
        self.root.join("curated")
    }

    /// The raw bytes of a curated file (immutability/QA channel).
    pub fn file_bytes(&self, path: &Path) -> Result<Vec<u8>, CurateError> {
        fs::read(path).map_err(|e| CurateError::StoreIo {
            context: format!("read {}", path.display()),
            detail: e.to_string(),
        })
    }

    /// Persists the dataset manifest for one version (JSON).
    pub fn write_dataset_manifest(&self, manifest: &DatasetManifest) -> Result<(), CurateError> {
        let dir = self.dataset_dir(&manifest.dataset_id, manifest.version);
        fs::create_dir_all(&dir).map_err(|e| CurateError::StoreIo {
            context: format!("create {}", dir.display()),
            detail: e.to_string(),
        })?;
        let path = dir.join("manifest.json");
        let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| CurateError::StoreIo {
            context: "manifest serialize".to_owned(),
            detail: e.to_string(),
        })?;
        fs::write(&path, bytes).map_err(|e| CurateError::StoreIo {
            context: format!("write {}", path.display()),
            detail: e.to_string(),
        })
    }
}

/// One curation request.
#[derive(Debug, Clone)]
pub struct CurateRequest<'a> {
    /// The canonical dataset id (e.g. `kr-etf-daily`).
    pub dataset_id: &'a DatasetId,
    /// The market partition key (e.g. `kr`).
    pub market: &'a str,
    /// Provenance source label (e.g. `krx`).
    pub source: &'a str,
    /// The curation clock: any action announced after this instant is
    /// rejected as a future announcement (point-in-time gate).
    pub now: UtcTimestamp,
}

/// The outcome of a successful curation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurateOutcome {
    pub dataset_version: u32,
    pub capability: Capability,
    pub bars_written: u64,
    pub actions_written: u64,
    pub first_session: TradingDate,
    pub last_session: TradingDate,
    pub manifest: DatasetManifest,
}

/// Exact, replayable evidence required to publish a curated price generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceCurationEvidence {
    pub curated_generation: u32,
    pub manifest_sha256: String,
    pub first_session: TradingDate,
    pub last_session: TradingDate,
    pub source_revision: String,
    pub instrument_coverage: Vec<PriceInstrumentCoverage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceInstrumentCoverage {
    pub instrument_id: String,
    pub first_session: TradingDate,
    pub last_session: TradingDate,
    pub session_count: u32,
    pub sessions: Vec<TradingDate>,
}

#[derive(Debug, Deserialize)]
struct RawReferenceDocument {
    source: String,
    instruments: Vec<RawReferenceInstrument>,
}

#[derive(Debug, Deserialize)]
struct RawReferenceInstrument {
    symbol: String,
    name: String,
    lot_size: Value,
    currency: Currency,
    kind: String,
    #[serde(default)]
    listed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCalendarDocument {
    calendar_id: String,
    schema_version: u32,
    source: String,
    timezone: String,
    session_times_local: RawSessionTimes,
    sessions: Vec<RawCalendarSession>,
    #[serde(default)]
    holidays: Vec<RawCalendarHoliday>,
}

#[derive(Debug, Deserialize)]
struct RawSessionTimes {
    open: String,
    close: String,
}

#[derive(Debug, Deserialize)]
struct RawCalendarSession {
    date: String,
}

#[derive(Debug, Deserialize)]
struct RawCalendarHoliday {
    date: String,
    #[serde(default)]
    reason: Option<String>,
}

/// Curation consumes either the legacy synthetic KRX scope or the immutable
/// canonical KIS scope.  The broker wire scope is intentionally rejected here
/// even when a caller happens to provide a subset that looks parseable: wire
/// responses are not provider-neutral evidence until normalization has
/// produced the exact four canonical documents.
fn validate_curation_scope(entry: &ManifestEntry) -> Result<(), CurateError> {
    match (entry.provider.as_str(), entry.market.as_str()) {
        (PROVIDER_KRX, MARKET_KR) => Ok(()),
        (PROVIDER_KIS_NORMALIZED, MARKET_KR) => {
            if entry.mode != FetchMode::Credentialed {
                return Err(CurateError::UnsupportedMode {
                    provider: entry.provider.clone(),
                    market: entry.market.clone(),
                    expected: FetchMode::Credentialed,
                    actual: entry.mode,
                });
            }
            const EXPECTED: [(ResponseKind, &str); 4] = [
                (ResponseKind::Bars, "bars.json"),
                (ResponseKind::Reference, "reference.json"),
                (ResponseKind::Calendar, "calendar.json"),
                (ResponseKind::CorporateActions, "corporate-actions.json"),
            ];
            if entry.files.len() != EXPECTED.len()
                || EXPECTED.iter().any(|(kind, file_name)| {
                    entry
                        .files
                        .iter()
                        .filter(|file| {
                            file.kind == *kind
                                && file.file_name == *file_name
                                && file.request.mode == FetchMode::Credentialed
                        })
                        .count()
                        != 1
                })
            {
                return Err(CurateError::NonCanonicalNormalizedBatch {
                    reason: "expected exactly one credentialed canonical bars/reference/calendar/corporate-actions file".to_owned(),
                });
            }
            Ok(())
        }
        _ => Err(CurateError::UnsupportedScope {
            expected_scopes: "krx/kr or kis-normalized/kr",
            provider: entry.provider.clone(),
            market: entry.market.clone(),
        }),
    }
}

fn validate_normalized_raw(raw: &RawStore, entry: &ManifestEntry) -> Result<(), CurateError> {
    if entry.provider == PROVIDER_KIS_NORMALIZED {
        crate::publication::PublicationBundle::from_raw(raw, entry).map_err(
            |error| match error {
                crate::publication::PublicationError::Store(source) => CurateError::RawStore {
                    context: "validate normalized raw".to_owned(),
                    source,
                },
                other => CurateError::NonCanonicalNormalizedBatch {
                    reason: other.to_string(),
                },
            },
        )?;
    }
    Ok(())
}

/// Rebuild the typed calendar and instrument master from one verified Raw
/// four-response delivery. This keeps the operating worker provider-neutral:
/// fixture and licensed transports must satisfy the same bytes contract.
pub fn curation_inputs_from_raw(
    raw: &RawStore,
    entry: &ManifestEntry,
) -> Result<(KrCalendar, InstrumentMaster), CurateError> {
    validate_curation_scope(entry)?;
    validate_normalized_raw(raw, entry)?;
    let files = raw
        .read_batch_bytes(&entry.provider, &entry.market, entry)
        .map_err(|source| CurateError::RawStore {
            context: "read curation inputs".to_owned(),
            source,
        })?;
    let bytes_for = |kind: ResponseKind| -> Result<&[u8], CurateError> {
        let metadata = entry
            .files
            .iter()
            .find(|file| file.kind == kind)
            .ok_or(CurateError::MissingFile { kind })?;
        files
            .iter()
            .find(|file| file.file_name == metadata.file_name)
            .map(|file| file.bytes.as_slice())
            .ok_or(CurateError::MissingFile { kind })
    };
    let reference: RawReferenceDocument =
        serde_json::from_slice(bytes_for(ResponseKind::Reference)?).map_err(|error| {
            CurateError::MalformedBars {
                reason: format!("invalid reference document: {error}"),
            }
        })?;
    let calendar: RawCalendarDocument = serde_json::from_slice(bytes_for(ResponseKind::Calendar)?)
        .map_err(|error| CurateError::MalformedBars {
            reason: format!("invalid calendar document: {error}"),
        })?;
    if reference.source.trim().is_empty()
        || calendar.calendar_id.trim().is_empty()
        || calendar.source.trim().is_empty()
        || calendar.schema_version == 0
        || calendar.timezone != "Asia/Seoul"
        || calendar.session_times_local.open != "09:00:00"
        || calendar.session_times_local.close != "15:30:00"
    {
        return Err(CurateError::MalformedBars {
            reason: "reference/calendar provenance is not the canonical market contract".to_owned(),
        });
    }
    let sessions = calendar
        .sessions
        .iter()
        .map(|session| {
            TradingDate::parse(&session.date).map_err(|error| CurateError::MalformedBars {
                reason: format!("invalid calendar session {}: {error}", session.date),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // A canonical KIS holiday delivery deliberately contains zero sessions
    // and one explicit holiday for the target date.  The calendar is still a
    // valid curation input; the target date is only used as a safe listing
    // fallback because no bars will be materialized for a closed session.
    let calendar_first_session = sessions.iter().copied().min().unwrap_or(entry.date);
    let holidays = calendar
        .holidays
        .into_iter()
        .map(|holiday| {
            Ok(Holiday {
                date: TradingDate::parse(&holiday.date).map_err(|error| {
                    CurateError::MalformedBars {
                        reason: format!("invalid calendar holiday {}: {error}", holiday.date),
                    }
                })?,
                reason: holiday
                    .reason
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or_else(|| "provider-declared closure".to_owned()),
            })
        })
        .collect::<Result<Vec<_>, CurateError>>()?;
    let calendar = KrCalendar::build(KrCalendarSpec {
        calendar_id: calendar.calendar_id,
        timezone: Zone::SEOUL,
        session_times: SessionTimes::krx_default(),
        sessions,
        holidays,
        source: calendar.source,
        version: calendar.schema_version,
        published_at: entry.retrieved_at,
        notes: vec!["rebuilt from immutable Raw calendar response".to_owned()],
    })
    .map_err(|error| CurateError::MalformedBars {
        reason: format!("invalid calendar: {error}"),
    })?;
    if reference.instruments.is_empty() {
        return Err(CurateError::MalformedBars {
            reason: "reference document must contain instruments".to_owned(),
        });
    }
    let mut master = InstrumentMaster::new();
    for instrument in reference.instruments {
        let listed_at = instrument
            .listed_at
            .as_deref()
            .map(TradingDate::parse)
            .transpose()
            .map_err(|error| CurateError::MalformedBars {
                reason: format!("invalid listed_at for {}: {error}", instrument.symbol),
            })?
            .unwrap_or(calendar_first_session);
        let instrument_id = InstrumentId::parse(&instrument.symbol).map_err(|error| {
            CurateError::MalformedBars {
                reason: format!("invalid reference symbol {}: {error}", instrument.symbol),
            }
        })?;
        if instrument_id.venue() != Venue::Krx || instrument.currency != Currency::KRW {
            return Err(CurateError::MalformedBars {
                reason: format!("unsupported reference instrument {}", instrument.symbol),
            });
        }
        let asset_class = match instrument.kind.as_str() {
            "equity-etf" | "etf" => AssetClass::Etf,
            "equity" => AssetClass::Equity,
            _ => {
                return Err(CurateError::MalformedBars {
                    reason: format!("unsupported instrument kind {}", instrument.kind),
                });
            }
        };
        let lot_size = match instrument.lot_size {
            Value::Number(number) => Quantity::parse(&number.to_string()),
            Value::String(value) => Quantity::parse(&value),
            value => {
                return Err(CurateError::MalformedBars {
                    reason: format!("invalid lot size {value}"),
                });
            }
        }
        .map_err(|error| CurateError::MalformedBars {
            reason: format!("invalid lot size for {}: {error}", instrument.symbol),
        })?;
        master
            .register_instrument(Instrument {
                instrument_id,
                name: instrument.name,
                asset_class,
                currency: instrument.currency,
                venue: Venue::Krx,
                listed_at,
                delisted_at: None,
                price_increment: Price::parse("1").expect("one KRW is valid"),
                size_increment: Quantity::parse("1").expect("one unit is valid"),
                lot_size,
                status: InstrumentStatus::Listed,
                reference_source: reference.source.clone(),
            })
            .map_err(map_master_error)?;
    }
    if entry.provider == PROVIDER_KIS_NORMALIZED {
        let expected = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX"))
            .collect::<BTreeSet<_>>();
        let actual = master
            .instruments()
            .map(|instrument| instrument.instrument_id.to_string())
            .collect::<BTreeSet<_>>();
        let all_etfs = master
            .instruments()
            .all(|instrument| instrument.asset_class == AssetClass::Etf);
        if actual != expected || !all_etfs {
            return Err(CurateError::NonCanonicalNormalizedBatch {
                reason: format!(
                    "normalized reference must contain exactly the fixed ETF universe (expected {} ETFs, got {} instruments; all_etfs={all_etfs})",
                    expected.len(),
                    actual.len(),
                ),
            });
        }
    }
    Ok((calendar, master))
}

/// Reconstruct price publication evidence from a verified Raw batch and its
/// immutable curated manifest. Used both after curation and after a crash
/// between filesystem publication and database publication.
pub fn price_curation_evidence(
    raw: &RawStore,
    entry: &ManifestEntry,
    manifest: &DatasetManifest,
) -> Result<PriceCurationEvidence, CurateError> {
    validate_curation_scope(entry)?;
    validate_normalized_raw(raw, entry)?;
    if manifest.version == 0 || dataset_manifest_hash(manifest)? != manifest.content_hash {
        return Err(CurateError::MalformedManifest {
            context: "price publication evidence".to_owned(),
            detail: "manifest hash or generation is invalid".to_owned(),
        });
    }
    let source = manifest
        .source_batches
        .iter()
        .find(|source| source.batch_id == entry.batch_id)
        .ok_or_else(|| CurateError::MalformedManifest {
            context: "price publication evidence".to_owned(),
            detail: "manifest does not reference the Raw batch".to_owned(),
        })?;
    let bars_meta = entry
        .files
        .iter()
        .find(|file| file.kind == ResponseKind::Bars)
        .ok_or(CurateError::MissingFile {
            kind: ResponseKind::Bars,
        })?;
    if source.bars_file != bars_meta.file_name || source.bars_hash != bars_meta.content_hash {
        return Err(CurateError::MalformedManifest {
            context: "price publication evidence".to_owned(),
            detail: "manifest does not match the Raw bars file".to_owned(),
        });
    }
    let files = raw
        .read_batch_bytes(&entry.provider, &entry.market, entry)
        .map_err(|source| CurateError::RawStore {
            context: "read price publication evidence".to_owned(),
            source,
        })?;
    let bars = files
        .iter()
        .find(|file| file.file_name == bars_meta.file_name)
        .ok_or(CurateError::MissingFile {
            kind: ResponseKind::Bars,
        })?;
    let document = parse_bars(&bars.bytes)?;
    if u64::try_from(document.bars.len()).ok() != Some(manifest.bar_count) {
        return Err(CurateError::MalformedManifest {
            context: "price publication evidence".to_owned(),
            detail: "manifest bar count does not match the verified Raw bars".to_owned(),
        });
    }
    let mut by_instrument = BTreeMap::<String, BTreeSet<TradingDate>>::new();
    let mut sessions = Vec::with_capacity(document.bars.len());
    for bar in &document.bars {
        let instrument =
            InstrumentId::parse(&bar.instrument).map_err(|error| CurateError::MalformedBars {
                reason: format!("invalid bar instrument {}: {error}", bar.instrument),
            })?;
        let session =
            TradingDate::parse(&bar.date).map_err(|error| CurateError::MalformedBars {
                reason: format!("invalid bar date {}: {error}", bar.date),
            })?;
        if !by_instrument
            .entry(instrument.to_string())
            .or_default()
            .insert(session)
        {
            return Err(CurateError::DuplicateBar {
                instrument: instrument.to_string(),
                date: session.to_iso(),
            });
        }
        sessions.push(session);
    }
    sessions.sort_unstable();
    let first_session = match sessions.first().copied() {
        Some(session) => session,
        None if entry.provider == PROVIDER_KIS_NORMALIZED => {
            return Err(CurateError::EodUnavailable {
                dataset_id: manifest.dataset_id.to_string(),
                target_date: entry.date,
            });
        }
        None => {
            return Err(CurateError::EmptyBars {
                dataset_id: manifest.dataset_id.to_string(),
            });
        }
    };
    let last_session = sessions.last().copied().expect("nonempty sessions");
    let manifest_sha256 = manifest
        .content_hash
        .as_str()
        .strip_prefix("sha256:")
        .filter(|hash| hash.len() == 64)
        .ok_or_else(|| CurateError::MalformedManifest {
            context: "price publication evidence".to_owned(),
            detail: "manifest hash is not canonical sha256".to_owned(),
        })?
        .to_owned();
    let instrument_coverage = by_instrument
        .into_iter()
        .map(|(instrument_id, sessions)| {
            let first_session = sessions.first().copied().expect("coverage is nonempty");
            let last_session = sessions.last().copied().expect("coverage is nonempty");
            let session_count =
                u32::try_from(sessions.len()).map_err(|_| CurateError::MalformedBars {
                    reason: format!("coverage count exceeds u32 for {instrument_id}"),
                })?;
            let sessions = sessions.into_iter().collect();
            Ok(PriceInstrumentCoverage {
                instrument_id,
                first_session,
                last_session,
                session_count,
                sessions,
            })
        })
        .collect::<Result<Vec<_>, CurateError>>()?;
    Ok(PriceCurationEvidence {
        curated_generation: manifest.version,
        manifest_sha256,
        first_session,
        last_session,
        source_revision: entry.batch_id.to_string(),
        instrument_coverage,
    })
}

/// Curates one immutable Raw batch into a NEW versioned Curated dataset.
///
/// All-or-nothing: on any validation error nothing is written (validation
/// happens before the first file is created).
pub fn curate_batch(
    raw: &RawStore,
    entry: &ManifestEntry,
    calendar: &KrCalendar,
    master: &InstrumentMaster,
    curated: &CurateStore,
    req: &CurateRequest<'_>,
) -> Result<CurateOutcome, CurateError> {
    validate_curation_scope(entry)?;
    validate_normalized_raw(raw, entry)?;
    if entry.provider == PROVIDER_KIS_NORMALIZED && req.source != PROVIDER_KIS_NORMALIZED {
        return Err(CurateError::NonCanonicalNormalizedBatch {
            reason: format!(
                "CurateRequest source must remain {PROVIDER_KIS_NORMALIZED}, got {}",
                req.source
            ),
        });
    }
    let dataset_id = req.dataset_id;

    // Re-curating an already-curated batch must never duplicate/overwrite.
    if let Some(latest) = curated.latest_manifest(dataset_id)?
        && latest
            .source_batches
            .iter()
            .any(|b| b.batch_id == entry.batch_id)
    {
        return Err(CurateError::BatchAlreadyCurated {
            dataset_id: dataset_id.to_string(),
            version: latest.version,
            batch_id: entry.batch_id.to_string(),
        });
    }

    let files = raw
        .read_batch_bytes(&entry.provider, &entry.market, entry)
        .map_err(|source| CurateError::RawStore {
            context: "read raw batch".to_owned(),
            source,
        })?;
    let meta = |kind: ResponseKind| -> Result<&crate::storage::FileEntry, CurateError> {
        entry
            .files
            .iter()
            .find(|f| f.kind == kind)
            .ok_or(CurateError::MissingFile { kind })
    };
    let file_bytes = |file_name: &str| -> Result<&[u8], CurateError> {
        files
            .iter()
            .find(|f| f.file_name == file_name)
            .map(|f| f.bytes.as_slice())
            .ok_or(CurateError::MissingFile {
                kind: ResponseKind::Bars,
            })
    };

    let bars_meta = meta(ResponseKind::Bars)?;
    let actions_meta = meta(ResponseKind::CorporateActions)?;
    let bars_bytes = file_bytes(&bars_meta.file_name)?;
    let actions_bytes = file_bytes(&actions_meta.file_name)?;

    // ---- parse (fixture bytes are data, never executed) ----
    let bars_doc = parse_bars(bars_bytes)?;
    let actions_doc = parse_actions(actions_bytes)?;

    // ---- validate corporate actions (point-in-time first) ----
    let actions = build_actions(&actions_doc.actions, master, req, entry, actions_meta)?;
    let capability = dataset_capability(&actions);

    // ---- validate + normalize bars ----
    let bars = build_bars(&bars_doc, master, calendar, req, entry, bars_meta)?;
    if bars.is_empty() {
        if entry.provider == PROVIDER_KIS_NORMALIZED || !calendar.is_session(entry.date) {
            return Err(CurateError::EodUnavailable {
                dataset_id: dataset_id.to_string(),
                target_date: entry.date,
            });
        }
        return Err(CurateError::EmptyBars {
            dataset_id: dataset_id.to_string(),
        });
    }

    // ---- adjusted series (signals; execution keeps the raw table) ----
    let adjusted = adjusted_series(&bars, &actions)?;
    let first_session = bars
        .iter()
        .map(|bar| bar.trading_date)
        .min()
        .expect("nonempty bars were checked");
    let last_session = bars
        .iter()
        .map(|bar| bar.trading_date)
        .max()
        .expect("nonempty bars were checked");

    // ---- partition writes (all-or-nothing per version) ----
    let version = curated.next_version(dataset_id)?;
    let version_dir = curated.dataset_dir(dataset_id, version);
    fs::create_dir_all(&version_dir).map_err(|e| CurateError::StoreIo {
        context: format!("create {}", version_dir.display()),
        detail: e.to_string(),
    })?;
    let write_guard = || -> Result<Vec<PathBuf>, CurateError> {
        let mut written: Vec<PathBuf> = Vec::new();
        let mut groups: BTreeMap<(String, i32), Vec<&CuratedBar>> = BTreeMap::new();
        for bar in &bars {
            groups
                .entry((
                    bar.instrument_id.to_string(),
                    bar.trading_date.as_naive_date().year(),
                ))
                .or_default()
                .push(bar);
        }
        for ((symbol, year), group) in &groups {
            let rows: Vec<CuratedBar> = group.iter().map(|b| (*b).clone()).collect();
            let path = curated.bars_path(req.market, symbol, *year, version);
            write_bars(&path, &rows)?;
            written.push(path);
            let split: Vec<AdjustmentBar> = adjusted
                .split
                .iter()
                .filter(|b| {
                    b.instrument_id.to_string() == *symbol
                        && b.trading_date.as_naive_date().year() == *year
                })
                .cloned()
                .collect();
            let total_return: Vec<AdjustmentBar> = adjusted
                .total_return
                .iter()
                .filter(|b| {
                    b.instrument_id.to_string() == *symbol
                        && b.trading_date.as_naive_date().year() == *year
                })
                .cloned()
                .collect();
            let path = curated.adjusted_bars_path(req.market, symbol, *year, version);
            write_adjusted_bars(&path, &split)?;
            written.push(path);
            let path = curated.total_return_bars_path(req.market, symbol, *year, version);
            write_adjusted_bars(&path, &total_return)?;
            written.push(path);
        }
        let mut action_groups: BTreeMap<(String, i32), Vec<CorporateAction>> = BTreeMap::new();
        for action in &actions {
            action_groups
                .entry((
                    action.instrument_id.to_string(),
                    action.ex_date.as_naive_date().year(),
                ))
                .or_default()
                .push(action.clone());
        }
        for ((symbol, year), rows) in &action_groups {
            let path = curated.corporate_actions_path(req.market, symbol, *year, version);
            write_corporate_actions(&path, rows)?;
            written.push(path);
        }
        Ok(written)
    };
    let written = write_guard().inspect_err(|_| {
        let _ = fs::remove_dir_all(&version_dir);
    })?;
    let cleanup_on_manifest_failure = |e: CurateError| -> CurateError {
        for path in &written {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir_all(&version_dir);
        e
    };

    // ---- dataset manifest ----
    let manifest = DatasetManifest {
        dataset_id: dataset_id.clone(),
        version,
        capability,
        created_at: req.now,
        source_batches: vec![SourceBatchRef {
            batch_id: entry.batch_id,
            bars_file: bars_meta.file_name.clone(),
            bars_hash: bars_meta.content_hash.clone(),
            actions_file: actions_meta.file_name.clone(),
            actions_hash: actions_meta.content_hash.clone(),
        }],
        bar_count: bars.len() as u64,
        action_count: actions.len() as u64,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let content_hash = dataset_manifest_hash(&manifest)?;
    let manifest = DatasetManifest {
        content_hash,
        ..manifest
    };
    curated
        .write_dataset_manifest(&manifest)
        .map_err(cleanup_on_manifest_failure)?;

    Ok(CurateOutcome {
        dataset_version: version,
        capability,
        bars_written: bars.len() as u64,
        actions_written: actions.len() as u64,
        first_session,
        last_session,
        manifest,
    })
}

/// The canonical bytes a manifest's content hash covers (everything except
/// the hash field itself). Public so consumers (Todo 11 quality gate) can
/// verify an on-disk manifest's immutability.
pub fn dataset_manifest_hash(manifest: &DatasetManifest) -> Result<ContentHash, CurateError> {
    #[derive(Serialize)]
    struct Canonical<'a> {
        dataset_id: &'a DatasetId,
        version: u32,
        capability: &'a Capability,
        created_at: &'a UtcTimestamp,
        source_batches: &'a [SourceBatchRef],
        bar_count: u64,
        action_count: u64,
    }
    let canonical = Canonical {
        dataset_id: &manifest.dataset_id,
        version: manifest.version,
        capability: &manifest.capability,
        created_at: &manifest.created_at,
        source_batches: &manifest.source_batches,
        bar_count: manifest.bar_count,
        action_count: manifest.action_count,
    };
    let bytes = serde_json::to_vec(&canonical).map_err(|e| CurateError::StoreIo {
        context: "manifest canonical form".to_owned(),
        detail: e.to_string(),
    })?;
    Ok(ContentHash::from_bytes(&bytes))
}

/// Builds the typed action records with point-in-time validation.
fn build_actions(
    raw_rows: &[RawActionRow],
    master: &InstrumentMaster,
    req: &CurateRequest<'_>,
    entry: &ManifestEntry,
    actions_meta: &crate::storage::FileEntry,
) -> Result<Vec<CorporateAction>, CurateError> {
    let mut actions = Vec::with_capacity(raw_rows.len());
    for (i, row) in raw_rows.iter().enumerate() {
        let instrument_id =
            InstrumentId::parse(&row.instrument).map_err(|_| CurateError::UnknownInstrument {
                instrument: row.instrument.clone(),
            })?;
        let ex_date = parse_date(row.ex_date.as_ref(), "ex_date", true)?.expect("ex_date required");
        let record = master
            .instrument_on(&instrument_id, ex_date)
            .map_err(map_master_error)?;
        let announced_at = parse_timestamp(row.announced_at.as_ref(), "announced_at", true)?
            .expect("announced_at required");
        if announced_at > req.now {
            return Err(CurateError::FutureAnnouncedAction {
                instrument: instrument_id.to_string(),
                announced_at: announced_at.to_rfc3339(),
                now: req.now.to_rfc3339(),
            });
        }
        if announced_at.as_datetime().date_naive() > ex_date.as_naive_date() {
            return Err(match row.event_type.as_str() {
                "split" => CurateError::InvalidSplit {
                    instrument: instrument_id.to_string(),
                    detail: format!("announced_at {} after ex_date {}", announced_at, ex_date),
                },
                _ => CurateError::InvalidDividend {
                    instrument: instrument_id.to_string(),
                    detail: format!("announced_at {} after ex_date {}", announced_at, ex_date),
                },
            });
        }
        let record_date = parse_date(row.record_date.as_ref(), "record_date", false)?;
        let pay_date = parse_date(row.pay_date.as_ref(), "pay_date", false)?;

        let event_type = CorporateActionType::parse(&row.event_type).ok_or_else(|| {
            CurateError::MalformedAction {
                reason: format!("actions[{i}]: unknown event type {:?}", row.event_type),
            }
        })?;
        let currency = match (row.currency, record.currency) {
            (Some(declared), expected) if declared != expected => {
                return Err(CurateError::CurrencyConflict {
                    instrument: instrument_id.to_string(),
                    expected: expected.to_string(),
                    declared: declared.to_string(),
                    context: "corporate action".to_owned(),
                });
            }
            (Some(declared), _) => declared,
            (None, expected) => expected,
        };

        let mut action = CorporateAction {
            instrument_id: instrument_id.clone(),
            event_type,
            ex_date,
            record_date,
            pay_date,
            ratio: row.ratio.clone(),
            split_factor: None,
            amount_per_share: None,
            tax_withholding_pct: None,
            currency,
            announced_at,
            source: req.source.to_owned(),
            batch_id: entry.batch_id,
            raw_hash: actions_meta.content_hash.clone(),
            ingested_at: entry.retrieved_at,
        };

        match event_type {
            CorporateActionType::Split => {
                let split_factor = parse_fixed_value(row.split_factor.as_ref(), "split_factor")?
                    .ok_or_else(|| CurateError::InvalidSplit {
                        instrument: instrument_id.to_string(),
                        detail: "split record without split_factor".to_owned(),
                    })?;
                if split_factor <= FixedPoint::parse("1").expect("one") {
                    return Err(CurateError::InvalidSplit {
                        instrument: instrument_id.to_string(),
                        detail: format!("split_factor must be > 1, got {split_factor}"),
                    });
                }
                if pay_date.is_some() {
                    return Err(CurateError::InvalidSplit {
                        instrument: instrument_id.to_string(),
                        detail: "splits have no pay_date".to_owned(),
                    });
                }
                action.split_factor = Some(split_factor);
            }
            CorporateActionType::CashDividend => {
                let amount = parse_fixed(row.amount_per_share.as_ref(), "amount_per_share")?
                    .ok_or_else(|| CurateError::InvalidDividend {
                        instrument: instrument_id.to_string(),
                        detail: "dividend record without amount_per_share".to_owned(),
                    })?;
                if !amount.is_positive() {
                    return Err(CurateError::InvalidDividend {
                        instrument: instrument_id.to_string(),
                        detail: format!("amount_per_share must be > 0, got {amount}"),
                    });
                }
                if pay_date.is_none() {
                    // Curated, but caps the version at PRICE_RETURN_ONLY.
                }
                action.amount_per_share = Some(domain::Money::from_fixed(amount, currency)?);
                action.tax_withholding_pct =
                    parse_fixed(row.tax_withholding_pct.as_ref(), "tax_withholding_pct")?;
            }
        }
        actions.push(action);
    }
    Ok(actions)
}

/// Builds the normalized, validated raw bar rows.
fn build_bars(
    doc: &RawBarsDoc,
    master: &InstrumentMaster,
    calendar: &KrCalendar,
    req: &CurateRequest<'_>,
    entry: &ManifestEntry,
    bars_meta: &crate::storage::FileEntry,
) -> Result<Vec<CuratedBar>, CurateError> {
    let instrument_currency: BTreeMap<String, Currency> = doc
        .instruments
        .iter()
        .filter_map(|i| i.currency.map(|c| (i.symbol.clone(), c)))
        .collect();

    let mut seen: BTreeSet<(String, TradingDate)> = BTreeSet::new();
    let mut bars = Vec::with_capacity(doc.bars.len());
    for row in &doc.bars {
        let instrument_id =
            InstrumentId::parse(&row.instrument).map_err(|_| CurateError::UnknownInstrument {
                instrument: row.instrument.clone(),
            })?;
        let date = TradingDate::parse(&row.date).map_err(|e| CurateError::MalformedBars {
            reason: format!("invalid bar date {:?}: {e}", row.date),
        })?;
        if !seen.insert((row.instrument.clone(), date)) {
            return Err(CurateError::DuplicateBar {
                instrument: row.instrument.clone(),
                date: date.to_iso(),
            });
        }
        let record = master
            .instrument_on(&instrument_id, date)
            .map_err(map_master_error)?;
        if !calendar.is_session(date) {
            return Err(CurateError::NotASession {
                instrument: row.instrument.clone(),
                date: date.to_iso(),
            });
        }
        if let Some(declared) = doc.currency
            && declared != record.currency
        {
            return Err(CurateError::CurrencyConflict {
                instrument: row.instrument.clone(),
                expected: record.currency.to_string(),
                declared: declared.to_string(),
                context: "dataset".to_owned(),
            });
        }
        if let Some(declared) = instrument_currency.get(&row.instrument)
            && *declared != record.currency
        {
            return Err(CurateError::CurrencyConflict {
                instrument: row.instrument.clone(),
                expected: record.currency.to_string(),
                declared: declared.to_string(),
                context: format!("instrument {}", row.instrument),
            });
        }

        let open = price(row, &row.open, "open")?;
        let high = price(row, &row.high, "high")?;
        let low = price(row, &row.low, "low")?;
        let close = price(row, &row.close, "close")?;
        if high < open.max(close) {
            return Err(CurateError::ImpossibleOhlc {
                instrument: row.instrument.clone(),
                date: date.to_iso(),
                detail: format!("high {high} < max(open {open}, close {close})"),
            });
        }
        if low > open.min(close) {
            return Err(CurateError::ImpossibleOhlc {
                instrument: row.instrument.clone(),
                date: date.to_iso(),
                detail: format!("low {low} > min(open {open}, close {close})"),
            });
        }
        let volume = parse_volume(&row.volume).map_err(|e| CurateError::MalformedBars {
            reason: format!("bar {} {} volume: {e}", row.instrument, date),
        })?;
        if volume < 0 {
            return Err(CurateError::NegativeVolume {
                instrument: row.instrument.clone(),
                date: date.to_iso(),
                volume: volume.to_string(),
            });
        }
        let trading_value = row.trading_value()?;

        let market_open_ts =
            calendar
                .session_open_utc(date)
                .map_err(|_| CurateError::NotASession {
                    instrument: row.instrument.clone(),
                    date: date.to_iso(),
                })?;
        let market_close_ts =
            calendar
                .session_close_utc(date)
                .map_err(|_| CurateError::NotASession {
                    instrument: row.instrument.clone(),
                    date: date.to_iso(),
                })?;

        bars.push(CuratedBar {
            instrument_id,
            trading_date: date,
            market_open_ts,
            market_close_ts,
            open: Price::from_fixed(open)?,
            high: Price::from_fixed(high)?,
            low: Price::from_fixed(low)?,
            close: Price::from_fixed(close)?,
            volume,
            trading_value,
            currency: record.currency,
            source: req.source.to_owned(),
            ingested_at: entry.retrieved_at,
            batch_id: entry.batch_id,
            raw_hash: bars_meta.content_hash.clone(),
        });
    }
    bars.sort_by_key(|b| (b.instrument_id.clone(), b.trading_date));
    Ok(bars)
}

/// Parses one OHLC number with typed positivity/finiteness checks.
fn price(row: &parse::RawBarRow, value: &Value, field: &str) -> Result<FixedPoint, CurateError> {
    let Some(number) = value.as_number() else {
        return Err(CurateError::MalformedBars {
            reason: format!(
                "bar {} {}: {field} must be a number",
                row.instrument, row.date
            ),
        });
    };
    if let Some(integer) = number.as_i64() {
        let parsed =
            FixedPoint::parse(&integer.to_string()).map_err(|e| CurateError::MalformedBars {
                reason: format!(
                    "bar {} {}: invalid {field} {integer}: {e}",
                    row.instrument, row.date
                ),
            })?;
        if !parsed.is_positive() {
            return Err(CurateError::NonPositivePrice {
                instrument: row.instrument.clone(),
                date: row.date.clone(),
                field: field.to_owned(),
                value: parsed.to_string(),
            });
        }
        return Ok(parsed);
    }
    let float = number.as_f64().ok_or_else(|| CurateError::MalformedBars {
        reason: format!("bar {} {}: {field} out of range", row.instrument, row.date),
    })?;
    if !float.is_finite() {
        return Err(CurateError::NonFinitePrice {
            instrument: row.instrument.clone(),
            date: row.date.clone(),
            field: field.to_owned(),
            value: float.to_string(),
        });
    }
    let parsed = FixedPoint::parse(&float.to_string()).map_err(|e| CurateError::MalformedBars {
        reason: format!(
            "bar {} {}: invalid {field} {float}: {e}",
            row.instrument, row.date
        ),
    })?;
    if !parsed.is_positive() {
        return Err(CurateError::NonPositivePrice {
            instrument: row.instrument.clone(),
            date: row.date.clone(),
            field: field.to_owned(),
            value: parsed.to_string(),
        });
    }
    Ok(parsed)
}

/// Parses the volume number.
fn parse_volume(value: &Value) -> Result<i64, String> {
    let Some(number) = value.as_number() else {
        return Err("volume must be a number".to_owned());
    };
    if let Some(integer) = number.as_i64() {
        return Ok(integer);
    }
    let float = number
        .as_f64()
        .ok_or_else(|| "volume out of range".to_owned())?;
    if !float.is_finite() {
        return Err("non-finite volume".to_owned());
    }
    Ok(float as i64)
}

/// Maps a master lookup failure onto the curation error surface.
fn map_master_error(e: MasterError) -> CurateError {
    match e {
        MasterError::UnknownInstrument { id } => CurateError::UnknownInstrument { instrument: id },
        MasterError::NotListed { id, date, .. } => CurateError::NotListed {
            instrument: id,
            date,
        },
        other => CurateError::UnknownInstrument {
            instrument: other.to_string(),
        },
    }
}
