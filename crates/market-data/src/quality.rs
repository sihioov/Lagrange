//! Dataset quality gate (Todo 11): validate a **Curated dataset version**
//! and classify every finding into a severity with a deterministic
//! `READY | WARNING | BLOCKED` state (FR-DATA-004, design §6.3, AT-05).
//!
//! Checks implemented here, against the Todo 10 curated layout
//! (`data/curated/bars/.../version={v}/bars.parquet`,
//! `adjusted_bars.parquet`, `total_return_bars.parquet`,
//! `corporate_actions.parquet`, and the per-version `manifest.json`):
//!
//! - **schema conformance**: every parquet file must match the documented
//!   Curated schema exactly (requirements §8.2, [`CuratedSchema`]);
//! - **duplicate trading_date rows** within an instrument;
//! - **missing bars**: against the instrument's listed calendar range (the
//!   dataset window bounded by the KRX session calendar and the instrument's
//!   listing interval) and against the required universe;
//! - **impossible OHLC**: `low <= min(open, close, high)` and
//!   `high >= max(open, close, low)` (design §6.3), all prices strictly
//!   positive;
//! - **timezone/session conformance**: every date is a KRX session and
//!   `market_open_ts`/`market_close_ts` equal the calendar's session instants;
//! - **volume sanity**: negative volume blocks, zero volume warns;
//! - **outlier detection** (documented rule): a close-to-close move at or
//!   above `QualityPolicy::outlier_threshold_pct` (default 20%) is a
//!   `SUSPICIOUS_MOVE` warning; an adjusted factor != 1 with no corporate
//!   action on record is a `SUSPICIOUS_SPLIT` warning;
//! - **currency consistency**: bar currency vs the instrument master;
//! - **freshness**: the latest close vs the policy's expected reference
//!   session — a stale close is `DATA_STALE` (stale required data blocks).
//!
//! State transitions (deterministic, exhaustive):
//!
//! | Findings | State |
//! |---|---|
//! | no issues | `READY` |
//! | only warning-severity issues | `WARNING` |
//! | any blocking-severity issue | `BLOCKED` |
//!
//! Policy (AT-05): required-universe missing bars or a stale close are
//! blocking (recommendation/backtest/Paper/Live all denied downstream via
//! [`QualityReport::permits`]); an optional symbol's missing data may exclude
//! that symbol **only** when the strategy declares the exclusion policy and
//! the reason is recorded in [`QualityReport::exclusions`] — otherwise the
//! missing optional bar also blocks.
//!
//! Immutability: re-validation of the same version is byte-identical
//! (same issues/state/`content_hash`); a correction produces a NEW version
//! via curation; the on-disk manifest's content hash is verified, so a
//! tampered manifest is `MANIFEST_HASH_MISMATCH` — and admin approval can
//! NEVER turn a structural `BLOCKED` error into `READY` without a new dataset
//! version (approval only transitions WARNING-class states).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use domain::{ContentHash, DataState, DatasetId, FixedPoint, InstrumentId, TradingDate};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};

use crate::calendar::KrCalendar;
use crate::curate::actions::CorporateAction;
use crate::curate::adjust::AdjustmentBar;
use crate::curate::schema::{CuratedBar, CuratedSchema};
use crate::curate::{
    CurateError, CurateStore, DatasetManifest, dataset_manifest_hash, read_adjusted_bars,
    read_bars, read_corporate_actions,
};
use crate::instrument_master::{InstrumentMaster, MasterError};

/// Issue severity: warnings never block; blocking issues force `BLOCKED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Warning,
    Blocking,
}

impl Severity {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "WARNING",
            Self::Blocking => "BLOCKING",
        }
    }
}

/// The typed quality-issue codes (wire names are SCREAMING_SNAKE_CASE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IssueCode {
    SchemaMismatch,
    DuplicateDate,
    MissingRequiredBar,
    MissingOptionalBar,
    ImpossibleOhlc,
    NonPositivePrice,
    NotASession,
    TimestampMismatch,
    NegativeVolume,
    ZeroVolume,
    SuspiciousMove,
    SuspiciousSplit,
    CurrencyMismatch,
    UnknownInstrument,
    DataStale,
    CorruptParquet,
    ManifestCorrupt,
    ManifestHashMismatch,
    EmptyDataset,
}

impl IssueCode {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SchemaMismatch => "SCHEMA_MISMATCH",
            Self::DuplicateDate => "DUPLICATE_DATE",
            Self::MissingRequiredBar => "MISSING_REQUIRED_BAR",
            Self::MissingOptionalBar => "MISSING_OPTIONAL_BAR",
            Self::ImpossibleOhlc => "IMPOSSIBLE_OHLC",
            Self::NonPositivePrice => "NON_POSITIVE_PRICE",
            Self::NotASession => "NOT_A_SESSION",
            Self::TimestampMismatch => "TIMESTAMP_MISMATCH",
            Self::NegativeVolume => "NEGATIVE_VOLUME",
            Self::ZeroVolume => "ZERO_VOLUME",
            Self::SuspiciousMove => "SUSPICIOUS_MOVE",
            Self::SuspiciousSplit => "SUSPICIOUS_SPLIT",
            Self::CurrencyMismatch => "CURRENCY_MISMATCH",
            Self::UnknownInstrument => "UNKNOWN_INSTRUMENT",
            Self::DataStale => "DATA_STALE",
            Self::CorruptParquet => "CORRUPT_PARQUET",
            Self::ManifestCorrupt => "MANIFEST_CORRUPT",
            Self::ManifestHashMismatch => "MANIFEST_HASH_MISMATCH",
            Self::EmptyDataset => "EMPTY_DATASET",
        }
    }
}

/// One classified finding of a quality validation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct QualityIssue {
    pub code: IssueCode,
    pub severity: Severity,
    pub instrument: Option<InstrumentId>,
    pub date: Option<TradingDate>,
    pub detail: String,
}

/// A recorded optional-symbol exclusion (AT-05): the strategy-declared reason
/// and the exact dates excluded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExclusionRecord {
    pub instrument: InstrumentId,
    pub reason: String,
    pub missing_dates: Vec<TradingDate>,
}

/// The strategy-declared optional-symbol exclusion policy (AT-05). Without
/// it, missing optional bars fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalExclusion {
    /// The recorded reason the strategy can run without those symbols.
    pub reason: String,
}

/// The freshness contract: the latest close must be within
/// `max_stale_sessions` sessions of the expected latest session at or before
/// `reference_date`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessPolicy {
    /// The expected reference date (typically "today").
    pub reference_date: TradingDate,
    /// Tolerated stale sessions before `DATA_STALE` (0 = any delay is stale).
    pub max_stale_sessions: u32,
}

/// The caller-supplied quality policy (strategy/backtest/Paper/Live layer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityPolicy {
    /// Missing bars here are blocking (design §16 fail-closed policy).
    pub required_universe: BTreeSet<InstrumentId>,
    /// Declared optional-exclusion policy; `None` fails closed on optional
    /// missing data.
    pub optional_exclusion: Option<OptionalExclusion>,
    /// Freshness expectation (latest close vs reference date).
    pub freshness: FreshnessPolicy,
    /// Documented outlier rule: a close-to-close relative move at or above
    /// this threshold (0.20 = 20%) is flagged `SUSPICIOUS_MOVE`.
    pub outlier_threshold_pct: f64,
}

/// A downstream consumer of the dataset (all four are denied on `BLOCKED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataUse {
    Recommendation,
    Backtest,
    Paper,
    Live,
}

/// Why a downstream use was denied (fail-closed on `BLOCKED`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataUseDenial {
    pub use_case: DataUse,
    pub state: DataState,
    pub blocking_issues: Vec<IssueCode>,
}

/// The deterministic outcome of validating one dataset version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityReport {
    pub dataset_id: DatasetId,
    pub version: u32,
    pub state: DataState,
    pub issues: Vec<QualityIssue>,
    pub exclusions: Vec<ExclusionRecord>,
    /// SHA-256 over the canonical report bytes (excluding the hash itself).
    /// Re-validation of the same version yields the same hash.
    pub content_hash: ContentHash,
}

impl QualityReport {
    /// Fail-closed downstream gate: `BLOCKED` denies every use case with the
    /// blocking codes; `READY`/`WARNING` permit research use.
    pub fn permits(&self, use_case: DataUse) -> Result<(), DataUseDenial> {
        if self.state == DataState::Blocked {
            let blocking_issues: Vec<IssueCode> = self
                .issues
                .iter()
                .filter(|i| i.severity == Severity::Blocking)
                .map(|i| i.code)
                .collect();
            Err(DataUseDenial {
                use_case,
                state: self.state,
                blocking_issues,
            })
        } else {
            Ok(())
        }
    }
}

/// A typed gate failure (filesystem trouble); data-quality findings are
/// always classified into the report, never an error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QualityError {
    #[error("quality store failure ({context}): {detail}")]
    Io { context: String, detail: String },
}

/// Validates a Curated dataset version against the policy.
///
/// Owns its calendar/master/curate-store (all cheap clones) so callers can
/// construct one per validation without lifetime plumbing.
pub struct QualityGate {
    curated: CurateStore,
    calendar: KrCalendar,
    master: InstrumentMaster,
    market: String,
    policy: QualityPolicy,
}

/// Why reading one partition failed (value/type/file level).
enum PartitionFailure {
    Corrupt(String),
    SchemaMismatch(arrow::datatypes::Schema),
    NonPositivePrice(String),
}

impl QualityGate {
    /// `market` is the curated partition key (e.g. `kr`).
    pub fn new(
        curated: CurateStore,
        calendar: KrCalendar,
        master: InstrumentMaster,
        market: impl Into<String>,
        policy: QualityPolicy,
    ) -> Self {
        Self {
            curated,
            calendar,
            master,
            market: market.into(),
            policy,
        }
    }

    /// The policy this gate enforces.
    pub fn policy(&self) -> &QualityPolicy {
        &self.policy
    }

    /// Validates one immutable dataset version. Deterministic: same bytes +
    /// same policy => same report (same issues/state/content hash).
    pub fn validate_dataset(
        &self,
        dataset_id: &DatasetId,
        version: u32,
    ) -> Result<QualityReport, QualityError> {
        let mut issues: Vec<QualityIssue> = Vec::new();
        let mut exclusions: Vec<ExclusionRecord> = Vec::new();

        let manifest = self.read_manifest(dataset_id, version, &mut issues);

        let mut bars: Vec<CuratedBar> = Vec::new();
        let mut adjusted: Vec<AdjustmentBar> = Vec::new();
        let mut actions: Vec<CorporateAction> = Vec::new();
        let mut failed: BTreeSet<InstrumentId> = BTreeSet::new();
        for (symbol, year) in self.discover_partitions(version)? {
            self.read_partition(
                version,
                &symbol,
                year,
                &mut issues,
                &mut bars,
                &mut adjusted,
                &mut actions,
                &mut failed,
            );
        }

        if let Some(manifest) = &manifest
            && manifest.bar_count != bars.len() as u64
        {
            issues.push(issue(
                IssueCode::ManifestCorrupt,
                Severity::Blocking,
                None,
                None,
                format!(
                    "manifest declares bar_count {} but {} bars are on disk",
                    manifest.bar_count,
                    bars.len()
                ),
            ));
        }

        self.check_bars(&bars, &mut issues);
        if bars.is_empty() {
            issues.push(issue(
                IssueCode::EmptyDataset,
                Severity::Blocking,
                None,
                None,
                format!("no bars found for dataset {dataset_id} version {version}"),
            ));
        } else {
            self.check_missing(&bars, &failed, &mut issues, &mut exclusions);
            self.check_outliers(&bars, &mut issues);
            self.check_splits(&adjusted, &actions, &mut issues);
            self.check_freshness(&bars, &failed, &mut issues, &mut exclusions);
        }

        Ok(build_report(
            dataset_id.clone(),
            version,
            issues,
            exclusions,
        ))
    }

    /// Reads and verifies the version's manifest; findings go into `issues`.
    fn read_manifest(
        &self,
        dataset_id: &DatasetId,
        version: u32,
        issues: &mut Vec<QualityIssue>,
    ) -> Option<DatasetManifest> {
        let path = self
            .curated
            .dataset_dir(dataset_id, version)
            .join("manifest.json");
        if !path.exists() {
            issues.push(issue(
                IssueCode::ManifestCorrupt,
                Severity::Blocking,
                None,
                None,
                format!("manifest.json is missing for dataset {dataset_id} version {version}"),
            ));
            return None;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                issues.push(issue(
                    IssueCode::ManifestCorrupt,
                    Severity::Blocking,
                    None,
                    None,
                    format!("manifest.json is unreadable: {e}"),
                ));
                return None;
            }
        };
        let manifest: DatasetManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(e) => {
                issues.push(issue(
                    IssueCode::ManifestCorrupt,
                    Severity::Blocking,
                    None,
                    None,
                    format!("manifest.json does not parse: {e}"),
                ));
                return None;
            }
        };
        if manifest.dataset_id != *dataset_id || manifest.version != version {
            issues.push(issue(
                IssueCode::ManifestCorrupt,
                Severity::Blocking,
                None,
                None,
                format!(
                    "manifest declares {} version {} but version {version} of {dataset_id} was requested",
                    manifest.dataset_id, manifest.version
                ),
            ));
            return None;
        }
        match dataset_manifest_hash(&manifest) {
            Ok(expected) if expected == manifest.content_hash => Some(manifest),
            Ok(expected) => {
                issues.push(issue(
                    IssueCode::ManifestHashMismatch,
                    Severity::Blocking,
                    None,
                    None,
                    format!(
                        "manifest content hash {} does not match its recomputed hash {expected}",
                        manifest.content_hash
                    ),
                ));
                None
            }
            Err(e) => {
                issues.push(issue(
                    IssueCode::ManifestCorrupt,
                    Severity::Blocking,
                    None,
                    None,
                    format!("manifest cannot be re-hashed: {e}"),
                ));
                None
            }
        }
    }

    /// The `(symbol, year)` partitions of `version` under `market`, sorted.
    fn discover_partitions(&self, version: u32) -> Result<Vec<(String, i32)>, QualityError> {
        let root = self
            .curated
            .root()
            .join("curated")
            .join("bars")
            .join(format!("market={}", self.market));
        let mut partitions: Vec<(String, i32)> = Vec::new();
        if !root.exists() {
            return Ok(partitions);
        }
        let read_dir = |path: &Path| -> Result<Vec<String>, QualityError> {
            let mut names: Vec<String> = fs::read_dir(path)
                .map_err(|e| QualityError::Io {
                    context: format!("list {}", path.display()),
                    detail: e.to_string(),
                })?
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    entry
                        .file_type()
                        .ok()?
                        .is_dir()
                        .then(|| entry.file_name().to_string_lossy().into_owned())
                })
                .collect();
            names.sort();
            Ok(names)
        };
        for symbol_name in read_dir(&root)? {
            let Some(symbol) = symbol_name.strip_prefix("symbol=") else {
                continue;
            };
            let year_root = root.join(&symbol_name);
            for year_name in read_dir(&year_root)? {
                let Some(year) = year_name
                    .strip_prefix("year=")
                    .and_then(|y| y.parse::<i32>().ok())
                else {
                    continue;
                };
                if year_root
                    .join(format!("year={year}"))
                    .join(format!("version={version}"))
                    .is_dir()
                {
                    partitions.push((symbol.to_owned(), year));
                }
            }
        }
        Ok(partitions)
    }

    /// Reads one partition's four parquet files, classifying failures.
    #[allow(clippy::too_many_arguments)]
    fn read_partition(
        &self,
        version: u32,
        symbol: &str,
        year: i32,
        issues: &mut Vec<QualityIssue>,
        bars: &mut Vec<CuratedBar>,
        adjusted: &mut Vec<AdjustmentBar>,
        actions: &mut Vec<CorporateAction>,
        failed: &mut BTreeSet<InstrumentId>,
    ) {
        let symbol_id = InstrumentId::parse(symbol).ok();
        let bars_path = self.curated.bars_path(&self.market, symbol, year, version);
        match self.read_bars_safe(&bars_path) {
            Ok(rows) => bars.extend(rows),
            Err(failure) => {
                issues.push(partition_issue(symbol_id.clone(), "bars.parquet", failure));
                if let Some(id) = symbol_id.clone() {
                    failed.insert(id);
                }
            }
        }
        let adjusted_path = self
            .curated
            .adjusted_bars_path(&self.market, symbol, year, version);
        match self.read_adjusted_safe(&adjusted_path) {
            Ok(rows) => adjusted.extend(rows),
            Err(failure) => issues.push(partition_issue(
                symbol_id.clone(),
                "adjusted_bars.parquet",
                failure,
            )),
        }
        let tr_path = self
            .curated
            .total_return_bars_path(&self.market, symbol, year, version);
        if let Err(failure) = self.read_adjusted_safe(&tr_path) {
            issues.push(partition_issue(
                symbol_id.clone(),
                "total_return_bars.parquet",
                failure,
            ));
        }
        let actions_path = self
            .curated
            .corporate_actions_path(&self.market, symbol, year, version);
        if actions_path.exists() {
            match self.read_actions_safe(&actions_path) {
                Ok(rows) => actions.extend(rows),
                Err(failure) => issues.push(partition_issue(
                    symbol_id,
                    "corporate_actions.parquet",
                    failure,
                )),
            }
        }
    }

    /// Schema-gated, value-safe bars read (never panics on on-disk data).
    fn read_bars_safe(&self, path: &Path) -> Result<Vec<CuratedBar>, PartitionFailure> {
        let schema = self.parquet_schema(path)?;
        if schema != CuratedSchema::bars() {
            return Err(PartitionFailure::SchemaMismatch(schema));
        }
        read_bars(path).map_err(|e| match e {
            CurateError::NonPositivePrice { .. } => {
                PartitionFailure::NonPositivePrice(e.to_string())
            }
            other => PartitionFailure::Corrupt(other.to_string()),
        })
    }

    fn read_adjusted_safe(&self, path: &Path) -> Result<Vec<AdjustmentBar>, PartitionFailure> {
        let schema = self.parquet_schema(path)?;
        if schema != CuratedSchema::adjusted_bars() {
            return Err(PartitionFailure::SchemaMismatch(schema));
        }
        read_adjusted_bars(path).map_err(|e| match e {
            CurateError::NonPositivePrice { .. } => {
                PartitionFailure::NonPositivePrice(e.to_string())
            }
            other => PartitionFailure::Corrupt(other.to_string()),
        })
    }

    fn read_actions_safe(&self, path: &Path) -> Result<Vec<CorporateAction>, PartitionFailure> {
        let schema = self.parquet_schema(path)?;
        if schema != CuratedSchema::corporate_actions() {
            return Err(PartitionFailure::SchemaMismatch(schema));
        }
        read_corporate_actions(path).map_err(|e| match e {
            CurateError::NonPositivePrice { .. } => {
                PartitionFailure::NonPositivePrice(e.to_string())
            }
            other => PartitionFailure::Corrupt(other.to_string()),
        })
    }

    fn parquet_schema(&self, path: &Path) -> Result<arrow::datatypes::Schema, PartitionFailure> {
        let file = fs::File::open(path).map_err(|e| {
            PartitionFailure::Corrupt(format!("cannot open {}: {e}", path.display()))
        })?;
        ParquetRecordBatchReaderBuilder::try_new(file)
            .map(|builder| builder.schema().as_ref().clone())
            .map_err(|e| {
                PartitionFailure::Corrupt(format!(
                    "{} is not a readable parquet file: {e}",
                    path.display()
                ))
            })
    }

    /// Schema/duplicate/OHLC/timezone/volume/currency row checks.
    fn check_bars(&self, bars: &[CuratedBar], issues: &mut Vec<QualityIssue>) {
        let mut seen: BTreeSet<(InstrumentId, TradingDate)> = BTreeSet::new();
        for bar in bars {
            let key = (bar.instrument_id.clone(), bar.trading_date);
            if !seen.insert(key) {
                issues.push(issue(
                    IssueCode::DuplicateDate,
                    Severity::Blocking,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    "duplicate trading_date row",
                ));
            }
            if !self.calendar.is_session(bar.trading_date) {
                issues.push(issue(
                    IssueCode::NotASession,
                    Severity::Blocking,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    format!(
                        "{} is not a session of {}",
                        bar.trading_date.to_iso(),
                        self.calendar.provenance().calendar_id
                    ),
                ));
                continue;
            }
            let open_ts = self.calendar.session_open_utc(bar.trading_date);
            let close_ts = self.calendar.session_close_utc(bar.trading_date);
            if let (Ok(open_ts), Ok(close_ts)) = (open_ts, close_ts) {
                if bar.market_open_ts != open_ts {
                    issues.push(issue(
                        IssueCode::TimestampMismatch,
                        Severity::Blocking,
                        Some(bar.instrument_id.clone()),
                        Some(bar.trading_date),
                        format!(
                            "market_open_ts {} != calendar session open {open_ts}",
                            bar.market_open_ts
                        ),
                    ));
                }
                if bar.market_close_ts != close_ts {
                    issues.push(issue(
                        IssueCode::TimestampMismatch,
                        Severity::Blocking,
                        Some(bar.instrument_id.clone()),
                        Some(bar.trading_date),
                        format!(
                            "market_close_ts {} != calendar session close {close_ts}",
                            bar.market_close_ts
                        ),
                    ));
                }
            }
            let instrument = bar.instrument_id.to_string();
            for (field, price) in [
                ("open", &bar.open),
                ("high", &bar.high),
                ("low", &bar.low),
                ("close", &bar.close),
            ] {
                if !price.amount().is_positive() {
                    issues.push(issue(
                        IssueCode::NonPositivePrice,
                        Severity::Blocking,
                        Some(bar.instrument_id.clone()),
                        Some(bar.trading_date),
                        format!("{field} is not strictly positive"),
                    ));
                }
            }
            let low = bar.low.amount();
            let high = bar.high.amount();
            let min_oc = bar.open.amount().min(bar.close.amount());
            let max_oc = bar.open.amount().max(bar.close.amount());
            if low > high || low > min_oc || high < max_oc {
                issues.push(issue(
                    IssueCode::ImpossibleOhlc,
                    Severity::Blocking,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    format!(
                        "low {low} must be <= min(open, close, high) and high {high} must be >= max(open, close, low) (open {}, close {})",
                        bar.open.amount(),
                        bar.close.amount()
                    ),
                ));
            }
            if bar.volume < 0 {
                issues.push(issue(
                    IssueCode::NegativeVolume,
                    Severity::Blocking,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    format!("negative volume {}", bar.volume),
                ));
            } else if bar.volume == 0 {
                issues.push(issue(
                    IssueCode::ZeroVolume,
                    Severity::Warning,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    "zero volume on a session is suspicious",
                ));
            }
            match self
                .master
                .instrument_on(&bar.instrument_id, bar.trading_date)
            {
                Ok(record) if record.currency != bar.currency => issues.push(issue(
                    IssueCode::CurrencyMismatch,
                    Severity::Blocking,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    format!(
                        "bar currency {} conflicts with master currency {}",
                        bar.currency, record.currency
                    ),
                )),
                Ok(_) => {}
                Err(MasterError::UnknownInstrument { .. }) => issues.push(issue(
                    IssueCode::UnknownInstrument,
                    Severity::Blocking,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    format!("instrument {instrument} is not in the instrument master"),
                )),
                Err(_) => {}
            }
        }
    }

    /// Missing bars: every calendar session in the dataset window where the
    /// instrument is listed must have a bar (AT-05 policy).
    fn check_missing(
        &self,
        bars: &[CuratedBar],
        failed: &BTreeSet<InstrumentId>,
        issues: &mut Vec<QualityIssue>,
        exclusions: &mut Vec<ExclusionRecord>,
    ) {
        let (window_start, window_end) = bars
            .iter()
            .map(|b| b.trading_date)
            .minmax_checked()
            .expect("bars are non-empty");
        let mut present: BTreeMap<InstrumentId, BTreeSet<TradingDate>> = BTreeMap::new();
        for bar in bars {
            present
                .entry(bar.instrument_id.clone())
                .or_default()
                .insert(bar.trading_date);
        }
        let mut instruments: BTreeSet<InstrumentId> = self.policy.required_universe.clone();
        instruments.extend(present.keys().cloned());
        for instrument in instruments {
            if failed.contains(&instrument) {
                continue;
            }
            let mut missing: Vec<TradingDate> = Vec::new();
            for date in self.calendar.sessions() {
                if date < window_start || date > window_end {
                    continue;
                }
                if present
                    .get(&instrument)
                    .is_some_and(|dates| dates.contains(&date))
                {
                    continue;
                }
                match self.master.instrument_on(&instrument, date) {
                    Ok(_) => missing.push(date),
                    Err(MasterError::NotListed { .. }) => {}
                    Err(MasterError::UnknownInstrument { .. }) => {
                        if self.policy.required_universe.contains(&instrument) {
                            issues.push(issue(
                                IssueCode::UnknownInstrument,
                                Severity::Blocking,
                                Some(instrument.clone()),
                                Some(date),
                                "required universe references an unknown instrument",
                            ));
                        }
                    }
                    Err(_) => {}
                }
            }
            if missing.is_empty() {
                continue;
            }
            if self.policy.required_universe.contains(&instrument) {
                for date in &missing {
                    issues.push(issue(
                        IssueCode::MissingRequiredBar,
                        Severity::Blocking,
                        Some(instrument.clone()),
                        Some(*date),
                        "required-universe instrument has no bar for this session",
                    ));
                }
            } else if let Some(exclusion) = &self.policy.optional_exclusion {
                issues.push(issue(
                    IssueCode::MissingOptionalBar,
                    Severity::Warning,
                    Some(instrument.clone()),
                    missing.first().copied(),
                    format!(
                        "optional symbol excluded per declared policy ({} sessions missing)",
                        missing.len()
                    ),
                ));
                exclusions.push(ExclusionRecord {
                    instrument,
                    reason: exclusion.reason.clone(),
                    missing_dates: missing,
                });
            } else {
                for date in &missing {
                    issues.push(issue(
                        IssueCode::MissingOptionalBar,
                        Severity::Blocking,
                        Some(instrument.clone()),
                        Some(*date),
                        "optional symbol missing without a declared exclusion policy (fail closed)",
                    ));
                }
            }
        }
    }

    /// Documented outlier rule: close-to-close moves at/above the threshold.
    fn check_outliers(&self, bars: &[CuratedBar], issues: &mut Vec<QualityIssue>) {
        let mut by_instrument: BTreeMap<InstrumentId, Vec<&CuratedBar>> = BTreeMap::new();
        for bar in bars {
            by_instrument
                .entry(bar.instrument_id.clone())
                .or_default()
                .push(bar);
        }
        for (instrument, mut rows) in by_instrument {
            rows.sort_by_key(|b| b.trading_date);
            for pair in rows.windows(2) {
                let prev = &pair[0];
                let cur = &pair[1];
                let Ok(delta) = cur.close.amount().checked_sub(&prev.close.amount()) else {
                    continue;
                };
                let prev_value = prev.close.amount().to_f64();
                let move_pct = delta.abs().to_f64() / prev_value;
                if !move_pct.is_finite() || move_pct < self.policy.outlier_threshold_pct {
                    continue;
                }
                issues.push(issue(
                    IssueCode::SuspiciousMove,
                    Severity::Warning,
                    Some(instrument.clone()),
                    Some(cur.trading_date),
                    format!(
                        "close-to-close move {:.2}% from {} to {} meets the documented {:.0}% outlier threshold",
                        move_pct * 100.0,
                        prev.trading_date.to_iso(),
                        cur.trading_date.to_iso(),
                        self.policy.outlier_threshold_pct * 100.0
                    ),
                ));
            }
        }
    }

    /// A split-adjusted factor != 1 with no action on record is suspicious.
    fn check_splits(
        &self,
        adjusted: &[AdjustmentBar],
        actions: &[CorporateAction],
        issues: &mut Vec<QualityIssue>,
    ) {
        let mut actions_by_instrument: BTreeMap<InstrumentId, Vec<&CorporateAction>> =
            BTreeMap::new();
        for action in actions {
            actions_by_instrument
                .entry(action.instrument_id.clone())
                .or_default()
                .push(action);
        }
        let one = FixedPoint::parse("1").expect("one");
        for bar in adjusted {
            if bar.adjustment_factor == one {
                continue;
            }
            let covered = actions_by_instrument
                .get(&bar.instrument_id)
                .is_some_and(|acts| acts.iter().any(|a| a.ex_date <= bar.trading_date));
            if !covered {
                issues.push(issue(
                    IssueCode::SuspiciousSplit,
                    Severity::Warning,
                    Some(bar.instrument_id.clone()),
                    Some(bar.trading_date),
                    format!(
                        "adjusted factor {} on {} has no corporate action on record",
                        bar.adjustment_factor,
                        bar.trading_date.to_iso()
                    ),
                ));
            }
        }
    }

    /// Freshness: the latest close must reach the expected latest session.
    fn check_freshness(
        &self,
        bars: &[CuratedBar],
        failed: &BTreeSet<InstrumentId>,
        issues: &mut Vec<QualityIssue>,
        exclusions: &mut Vec<ExclusionRecord>,
    ) {
        let reference = self.policy.freshness.reference_date;
        let expected = if self.calendar.is_session(reference) {
            reference
        } else {
            let Ok(prev) = self.calendar.previous_trading_day(reference) else {
                return;
            };
            prev
        };
        let mut by_instrument: BTreeMap<InstrumentId, Vec<&CuratedBar>> = BTreeMap::new();
        for bar in bars {
            by_instrument
                .entry(bar.instrument_id.clone())
                .or_default()
                .push(bar);
        }
        for (instrument, rows) in by_instrument {
            if failed.contains(&instrument) {
                continue;
            }
            let latest = rows
                .iter()
                .map(|b| b.trading_date)
                .max()
                .expect("rows are non-empty");
            let stale_sessions = self
                .calendar
                .sessions()
                .filter(|d| *d > latest && *d <= expected)
                .count() as u32;
            if stale_sessions <= self.policy.freshness.max_stale_sessions {
                continue;
            }
            let detail = format!(
                "latest close {latest}, expected {expected} ({stale_sessions} stale session(s) beyond the {}-session grace)",
                self.policy.freshness.max_stale_sessions
            );
            if self.policy.required_universe.contains(&instrument) {
                issues.push(issue(
                    IssueCode::DataStale,
                    Severity::Blocking,
                    Some(instrument.clone()),
                    Some(latest),
                    detail,
                ));
            } else if let Some(exclusion) = &self.policy.optional_exclusion {
                issues.push(issue(
                    IssueCode::DataStale,
                    Severity::Warning,
                    Some(instrument.clone()),
                    Some(latest),
                    format!("{detail}; excluded per declared policy"),
                ));
                exclusions.push(ExclusionRecord {
                    instrument,
                    reason: exclusion.reason.clone(),
                    missing_dates: vec![expected],
                });
            } else {
                issues.push(issue(
                    IssueCode::DataStale,
                    Severity::Blocking,
                    Some(instrument.clone()),
                    Some(latest),
                    format!("{detail}; no declared exclusion policy (fail closed)"),
                ));
            }
        }
    }
}

/// Builds the deterministic report (sorted issues, computed state, hash).
fn build_report(
    dataset_id: DatasetId,
    version: u32,
    mut issues: Vec<QualityIssue>,
    mut exclusions: Vec<ExclusionRecord>,
) -> QualityReport {
    issues.sort_by(|a, b| {
        (
            a.code.as_str(),
            a.instrument.as_ref().map(ToString::to_string),
            a.date.map(|d| d.to_iso()),
            a.detail.as_str(),
        )
            .cmp(&(
                b.code.as_str(),
                b.instrument.as_ref().map(ToString::to_string),
                b.date.map(|d| d.to_iso()),
                b.detail.as_str(),
            ))
    });
    exclusions.sort_by(|a, b| {
        (
            a.instrument.to_string(),
            a.reason.as_str(),
            a.missing_dates
                .iter()
                .map(|d| d.to_iso())
                .collect::<Vec<String>>(),
        )
            .cmp(&(
                b.instrument.to_string(),
                b.reason.as_str(),
                b.missing_dates
                    .iter()
                    .map(|d| d.to_iso())
                    .collect::<Vec<String>>(),
            ))
    });
    let state = if issues.iter().any(|i| i.severity == Severity::Blocking) {
        DataState::Blocked
    } else if !issues.is_empty() {
        DataState::Warning
    } else {
        DataState::Ready
    };
    let content_hash = report_content_hash(&dataset_id, version, state, &issues, &exclusions);
    QualityReport {
        dataset_id,
        version,
        state,
        issues,
        exclusions,
        content_hash,
    }
}

/// The canonical bytes a report's content hash covers (everything except the
/// hash field itself).
fn report_content_hash(
    dataset_id: &DatasetId,
    version: u32,
    state: DataState,
    issues: &[QualityIssue],
    exclusions: &[ExclusionRecord],
) -> ContentHash {
    #[derive(Serialize)]
    struct Canonical<'a> {
        dataset_id: &'a DatasetId,
        version: u32,
        state: &'a DataState,
        issues: &'a [QualityIssue],
        exclusions: &'a [ExclusionRecord],
    }
    let canonical = Canonical {
        dataset_id,
        version,
        state: &state,
        issues,
        exclusions,
    };
    let bytes = serde_json::to_vec(&canonical).expect("canonical report serializes");
    ContentHash::from_bytes(&bytes)
}

fn issue(
    code: IssueCode,
    severity: Severity,
    instrument: Option<InstrumentId>,
    date: Option<TradingDate>,
    detail: impl Into<String>,
) -> QualityIssue {
    QualityIssue {
        code,
        severity,
        instrument,
        date,
        detail: detail.into(),
    }
}

fn partition_issue(
    instrument: Option<InstrumentId>,
    file: &str,
    failure: PartitionFailure,
) -> QualityIssue {
    match failure {
        PartitionFailure::Corrupt(detail) => issue(
            IssueCode::CorruptParquet,
            Severity::Blocking,
            instrument,
            None,
            format!("{file}: {detail}"),
        ),
        PartitionFailure::SchemaMismatch(schema) => issue(
            IssueCode::SchemaMismatch,
            Severity::Blocking,
            instrument,
            None,
            format!(
                "{file}: schema does not match the documented Curated schema ({})",
                schema
            ),
        ),
        PartitionFailure::NonPositivePrice(detail) => issue(
            IssueCode::NonPositivePrice,
            Severity::Blocking,
            instrument,
            None,
            format!("{file}: {detail}"),
        ),
    }
}

// minmax via std (avoid itertools) — local helper trait.
trait MinMaxExt: Iterator + Sized {
    fn minmax_checked(self) -> Option<(Self::Item, Self::Item)>
    where
        Self::Item: Ord + Copy,
    {
        let mut min = None;
        let mut max = None;
        for item in self {
            min = Some(min.map_or(item, |m: Self::Item| m.min(item)));
            max = Some(max.map_or(item, |m: Self::Item| m.max(item)));
        }
        min.zip(max)
    }
}

impl<I: Iterator> MinMaxExt for I {}
