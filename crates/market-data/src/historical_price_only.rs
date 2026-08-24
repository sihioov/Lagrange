//! Bounded, in-memory historical ETF11 price-only materialization.
//!
//! The public entry point accepts only the RawStore-authenticated
//! [`HistoricalPriceOnlyBetaInput`].  This module deliberately stops at an
//! opaque in-memory candidate: it has no serializer, filesystem writer,
//! database type, or publication path.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use domain::{BatchId, ContentHash, FixedPoint, InstrumentId, TradingDate, UtcTimestamp, Venue};
use serde::Serialize;
use thiserror::Error;

use crate::contract::{RequestMetadata, ResponseKind};
use crate::curate::Capability;
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::range_to_canonical::{
    HISTORICAL_PRICE_ONLY_BETA_CONTRACT, HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT,
    HistoricalPriceOnlyBetaInput, HistoricalPriceOnlyIgnoredCashDividendEvidence,
    HistoricalPriceOnlySessionWitness, REQUIRED_ACTION_KINDS, RangeAction,
    RangeCanonicalBarCandidate,
};
use crate::storage::FileEntry;

/// Version of the explicit, non-persisted candidate representation.
pub const HISTORICAL_PRICE_ONLY_MATERIALIZER_VERSION: &str =
    "kis-historical-price-only-materializer-v2";
/// Cumulative reciprocal split-factor scale.
pub const HISTORICAL_PRICE_ONLY_FACTOR_SCALE: u8 = 8;
/// Canonical adjusted-price scale.
pub const HISTORICAL_PRICE_ONLY_PRICE_SCALE: u8 = 4;

/// Closed audience scope encoded by every in-memory historical candidate.
///
/// This layer records the owner-only scope but does not authorize an identity:
/// it has no publication or access path. Downstream publication must enforce
/// owner identity independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalPriceOnlyAudience {
    OwnerOnly,
}

impl HistoricalPriceOnlyAudience {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnerOnly => "OWNER_ONLY",
        }
    }
}

/// Fixed metadata attached to every historical price-only candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoricalPriceOnlyMetadata {
    pub audience: HistoricalPriceOnlyAudience,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub capability: Capability,
    pub materialized: bool,
    pub in_memory: bool,
    pub ready: bool,
}

impl HistoricalPriceOnlyMetadata {
    const FIXED: Self = Self {
        audience: HistoricalPriceOnlyAudience::OwnerOnly,
        vendor_snapshot: true,
        strict_pit: false,
        capability: Capability::PriceReturnOnly,
        materialized: false,
        in_memory: true,
        ready: false,
    };
}

/// One date-only raw/adjusted price row.
///
/// The raw OHLCV/trading-value fields are retained independently of the
/// split-adjusted OHLC fields. Volume and trading value are never adjusted.
/// No market-open or market-close timestamp is represented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyBar {
    pub instrument_id: InstrumentId,
    pub session_date: TradingDate,
    pub raw_open: FixedPoint,
    pub raw_high: FixedPoint,
    pub raw_low: FixedPoint,
    pub raw_close: FixedPoint,
    pub raw_volume: u64,
    pub raw_trading_value: Option<FixedPoint>,
    pub adjusted_open: FixedPoint,
    pub adjusted_high: FixedPoint,
    pub adjusted_low: FixedPoint,
    pub adjusted_close: FixedPoint,
}

/// Deterministic provenance copied from one verified Stage5 session witness.
///
/// `acquired_at` is source acquisition provenance only; it is not a session
/// open/close time and is never used as a bar timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlySessionProvenance {
    pub session_date: TradingDate,
    pub normalized_batch_id: BatchId,
    pub normalized_entry_hash: ContentHash,
    pub normalized_bars_hash: ContentHash,
    pub acquired_at: UtcTimestamp,
}

impl From<&HistoricalPriceOnlySessionWitness> for HistoricalPriceOnlySessionProvenance {
    fn from(witness: &HistoricalPriceOnlySessionWitness) -> Self {
        Self {
            session_date: witness.session_date(),
            normalized_batch_id: witness.normalized_batch_id(),
            normalized_entry_hash: witness.normalized_entry_hash().clone(),
            normalized_bars_hash: witness.normalized_bars_hash().clone(),
            acquired_at: witness.acquired_at(),
        }
    }
}

/// Verified bonus-issue evidence retained by the in-memory candidate.
///
/// This is deliberately not serializable and names the timestamp `acquired_at`:
/// it is retrieval provenance copied from authenticated KSD evidence, never a
/// point-in-time availability claim. There is no `available_at` field here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyBonusEvidence {
    pub instrument_id: InstrumentId,
    pub record_date: TradingDate,
    pub ex_date: TradingDate,
    pub split_factor: FixedPoint,
    pub acquired_at: UtcTimestamp,
}

/// Opaque in-memory candidate for the owner-beta price-only path.
///
/// Fields are private by design. The accessors expose evidence and rows for
/// review/tests, but this type cannot be serialized or written by this
/// module, and no downstream database/publication API accepts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyCandidate {
    range_start: TradingDate,
    range_end: TradingDate,
    source_batch_id: BatchId,
    source_manifest_hash: ContentHash,
    source_files: Vec<FileEntry>,
    action_batch_id: BatchId,
    action_manifest_hash: ContentHash,
    action_file_count: usize,
    ignored_cash_dividends: HistoricalPriceOnlyIgnoredCashDividendEvidence,
    sessions: Vec<HistoricalPriceOnlySessionProvenance>,
    bonus_evidence: Vec<HistoricalPriceOnlyBonusEvidence>,
    bars: Vec<HistoricalPriceOnlyBar>,
    metadata: HistoricalPriceOnlyMetadata,
    content_hash: ContentHash,
}

impl HistoricalPriceOnlyCandidate {
    pub fn range_start(&self) -> TradingDate {
        self.range_start
    }

    pub fn range_end(&self) -> TradingDate {
        self.range_end
    }

    pub fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }

    pub fn source_manifest_hash(&self) -> &ContentHash {
        &self.source_manifest_hash
    }

    pub fn source_files(&self) -> &[FileEntry] {
        &self.source_files
    }

    pub fn action_batch_id(&self) -> BatchId {
        self.action_batch_id
    }

    pub fn action_manifest_hash(&self) -> &ContentHash {
        &self.action_manifest_hash
    }

    pub fn action_file_count(&self) -> usize {
        self.action_file_count
    }

    pub fn ignored_cash_dividends(&self) -> &HistoricalPriceOnlyIgnoredCashDividendEvidence {
        &self.ignored_cash_dividends
    }

    pub fn session_provenance(&self) -> &[HistoricalPriceOnlySessionProvenance] {
        &self.sessions
    }

    pub fn bonus_evidence(&self) -> &[HistoricalPriceOnlyBonusEvidence] {
        &self.bonus_evidence
    }

    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.bars
    }

    pub fn row_count(&self) -> usize {
        self.bars.len()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn source_file_count(&self) -> usize {
        self.source_files.len()
    }

    pub fn metadata(&self) -> HistoricalPriceOnlyMetadata {
        self.metadata
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }
}

/// Typed fail-closed errors for the in-memory materializer.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HistoricalPriceOnlyError {
    #[error("historical price-only input mismatch: {reason}")]
    InputMismatch { reason: String },
    #[error("duplicate historical price-only bar {instrument} on {session_date}")]
    DuplicateBar {
        instrument: String,
        session_date: TradingDate,
    },
    #[error("duplicate historical price-only action for {instrument} on {ex_date}")]
    DuplicateAction {
        instrument: String,
        ex_date: TradingDate,
    },
    #[error("conflicting historical price-only action for {instrument} on {ex_date}")]
    ConflictingAction {
        instrument: String,
        ex_date: TradingDate,
    },
    #[error("noncanonical historical price-only {kind} ordering")]
    NonCanonicalOrdering { kind: &'static str },
    #[error("unsupported corporate action {kind}")]
    UnsupportedAction { kind: String },
    #[error("invalid split factor {factor} for {instrument} on {ex_date}")]
    InvalidSplitFactor {
        instrument: String,
        ex_date: TradingDate,
        factor: String,
    },
    #[error("OHLC invariant violation for {instrument} on {session_date} ({stage})")]
    OhlcInvariant {
        instrument: String,
        session_date: TradingDate,
        stage: &'static str,
    },
    #[error("arithmetic overflow during historical price-only {operation}")]
    ArithmeticOverflow { operation: &'static str },
    #[error("historical price-only canonical representation could not be serialized")]
    Serialization,
}

/// Materialize one verified historical ETF11 input into an in-memory,
/// date-only, split-adjusted price-return candidate.
pub fn materialize_historical_price_only_beta(
    input: &HistoricalPriceOnlyBetaInput,
) -> Result<HistoricalPriceOnlyCandidate, HistoricalPriceOnlyError> {
    let sessions = input
        .sessions()
        .iter()
        .map(HistoricalPriceOnlySessionProvenance::from)
        .collect();
    materialize_parts(MaterializationParts {
        range_start: input.range_start(),
        range_end: input.range_end(),
        source_batch_id: input.source_batch_id(),
        source_manifest_hash: input.source_manifest_hash().clone(),
        source_files: input.source_files().to_vec(),
        action_batch_id: input.action_batch_id(),
        action_manifest_hash: input.action_manifest_hash().clone(),
        action_file_count: input.action_file_count(),
        ignored_cash_dividends: input.ignored_cash_dividends().clone(),
        sessions,
        bars: input.bars().to_vec(),
        actions: input.actions().to_vec(),
    })
}

/// Private input view shared by the public opaque seam and focused unit tests.
/// Tests construct this type directly instead of adding a public constructor
/// that could bypass RawStore authentication.
struct MaterializationParts {
    range_start: TradingDate,
    range_end: TradingDate,
    source_batch_id: BatchId,
    source_manifest_hash: ContentHash,
    source_files: Vec<FileEntry>,
    action_batch_id: BatchId,
    action_manifest_hash: ContentHash,
    action_file_count: usize,
    ignored_cash_dividends: HistoricalPriceOnlyIgnoredCashDividendEvidence,
    sessions: Vec<HistoricalPriceOnlySessionProvenance>,
    bars: Vec<RangeCanonicalBarCandidate>,
    actions: Vec<RangeAction>,
}

fn materialize_parts(
    parts: MaterializationParts,
) -> Result<HistoricalPriceOnlyCandidate, HistoricalPriceOnlyError> {
    validate_parts(&parts)?;
    let source_files = canonical_source_files(parts.source_files.clone())?;
    let actions = canonical_actions(&parts.actions, parts.range_start, parts.range_end)?;
    let bars = materialize_bars(&parts.bars, &actions)?;
    let bonus_evidence = actions.iter().map(bonus_evidence).collect::<Vec<_>>();
    let content_hash = canonical_content_hash(&parts, &source_files, &bonus_evidence, &bars)?;

    Ok(HistoricalPriceOnlyCandidate {
        range_start: parts.range_start,
        range_end: parts.range_end,
        source_batch_id: parts.source_batch_id,
        source_manifest_hash: parts.source_manifest_hash,
        source_files,
        action_batch_id: parts.action_batch_id,
        action_manifest_hash: parts.action_manifest_hash,
        action_file_count: parts.action_file_count,
        ignored_cash_dividends: parts.ignored_cash_dividends,
        sessions: parts.sessions,
        bonus_evidence,
        bars,
        metadata: HistoricalPriceOnlyMetadata::FIXED,
        content_hash,
    })
}

fn validate_parts(parts: &MaterializationParts) -> Result<(), HistoricalPriceOnlyError> {
    if parts.range_start > parts.range_end {
        return Err(input_mismatch("range_start is after range_end"));
    }
    if parts.sessions.is_empty() {
        return Err(input_mismatch("session provenance is empty"));
    }
    if parts.sessions.first().map(|session| session.session_date) != Some(parts.range_start)
        || parts.sessions.last().map(|session| session.session_date) != Some(parts.range_end)
    {
        return Err(input_mismatch(
            "session provenance does not cover the exact input range",
        ));
    }
    if parts
        .sessions
        .windows(2)
        .any(|pair| pair[0].session_date >= pair[1].session_date)
    {
        return Err(HistoricalPriceOnlyError::NonCanonicalOrdering { kind: "sessions" });
    }
    if parts.action_file_count != REQUIRED_ACTION_KINDS.len() {
        return Err(input_mismatch(
            "action evidence does not contain exactly seven verified files",
        ));
    }
    if parts.ignored_cash_dividends.treatment_id() != HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT
        || parts.ignored_cash_dividends.row_count() == 0
    {
        return Err(input_mismatch(
            "cash-dividend treatment evidence is missing or invalid",
        ));
    }
    let expected_rows = parts
        .sessions
        .len()
        .checked_mul(KR_ETF_CORE_SYMBOLS.len())
        .ok_or(HistoricalPriceOnlyError::ArithmeticOverflow {
            operation: "row-count calculation",
        })?;
    if parts.bars.len() != expected_rows {
        return Err(input_mismatch(
            "bar count does not equal session count times ETF11",
        ));
    }

    let session_dates = parts
        .sessions
        .iter()
        .map(|session| session.session_date)
        .collect::<BTreeSet<_>>();
    let mut bar_counts = BTreeMap::<TradingDate, usize>::new();
    let mut seen_bars = BTreeSet::<(TradingDate, InstrumentId)>::new();
    let mut previous_key: Option<(TradingDate, InstrumentId)> = None;
    for bar in &parts.bars {
        if !session_dates.contains(&bar.session_date)
            || bar.session_date < parts.range_start
            || bar.session_date > parts.range_end
        {
            return Err(input_mismatch(
                "bar date is outside the verified session set",
            ));
        }
        if !is_fixed_etf(&bar.instrument_id) {
            return Err(input_mismatch("bar instrument is outside fixed ETF11"));
        }
        let key = (bar.session_date, bar.instrument_id.clone());
        if !seen_bars.insert(key.clone()) {
            return Err(HistoricalPriceOnlyError::DuplicateBar {
                instrument: bar.instrument_id.to_string(),
                session_date: bar.session_date,
            });
        }
        if let Some(previous) = previous_key.as_ref()
            && key.cmp(previous) == Ordering::Less
        {
            return Err(HistoricalPriceOnlyError::NonCanonicalOrdering { kind: "bars" });
        }
        previous_key = Some(key);
        *bar_counts.entry(bar.session_date).or_default() += 1;
        validate_raw_bar(bar)?;
    }
    if parts.sessions.iter().any(|session| {
        bar_counts.get(&session.session_date).copied() != Some(KR_ETF_CORE_SYMBOLS.len())
    }) {
        return Err(input_mismatch(
            "every verified session must contain exactly ETF11 bars",
        ));
    }
    Ok(())
}

/// Source files are canonical in the exact Stage5 producer order: fixed ETF11
/// instrument order, then numeric range-window order for that instrument. The
/// caller must provide that order; the materializer never sorts authenticated
/// input or mistakes lexical `window-10` ordering for numeric ordering.
fn canonical_source_files(
    files: Vec<FileEntry>,
) -> Result<Vec<FileEntry>, HistoricalPriceOnlyError> {
    if files.is_empty() {
        return Err(input_mismatch("source manifest contains no files"));
    }
    let mut names = BTreeSet::new();
    let mut positions = BTreeSet::new();
    let mut previous_position = None;
    for file in &files {
        if file.file_name.trim().is_empty() || file.size_bytes == 0 {
            return Err(input_mismatch(
                "source manifest contains an invalid file entry",
            ));
        }
        if !names.insert(file.file_name.clone()) {
            return Err(input_mismatch(
                "source manifest contains duplicate file names",
            ));
        }
        let position = source_file_position(file)?;
        if !positions.insert(position) {
            return Err(input_mismatch(
                "source manifest contains duplicate instrument/window positions",
            ));
        }
        if previous_position.is_some_and(|previous| previous >= position) {
            return Err(HistoricalPriceOnlyError::NonCanonicalOrdering {
                kind: "source files",
            });
        }
        previous_position = Some(position);
    }
    Ok(files)
}

fn source_file_position(file: &FileEntry) -> Result<(usize, usize), HistoricalPriceOnlyError> {
    const BETA_RANGE_WINDOWS: usize = 17;

    if file.kind != ResponseKind::Bars {
        return Err(input_mismatch("source manifest contains a non-bars file"));
    }
    let symbols = file
        .request
        .query
        .iter()
        .filter(|(key, _)| key == "FID_INPUT_ISCD")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    let [symbol] = symbols.as_slice() else {
        return Err(input_mismatch(
            "source file request does not contain one canonical instrument",
        ));
    };
    let instrument_position = KR_ETF_CORE_SYMBOLS
        .iter()
        .position(|expected| expected == symbol)
        .ok_or_else(|| input_mismatch("source file instrument is outside fixed ETF11"))?;
    let prefix = "daily-bars-range-window-";
    let suffix = format!("-{symbol}-page-01.json");
    let window_text = file
        .file_name
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(&suffix))
        .ok_or_else(|| input_mismatch("source file name has an invalid numeric range window"))?;
    let window = window_text
        .parse::<usize>()
        .ok()
        .filter(|window| window_text == window.to_string())
        .filter(|window| (1..=BETA_RANGE_WINDOWS).contains(window))
        .ok_or_else(|| input_mismatch("source file name has an invalid numeric range window"))?;
    Ok((instrument_position, window))
}

fn canonical_actions(
    actions: &[RangeAction],
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<Vec<RangeAction>, HistoricalPriceOnlyError> {
    let mut canonical = Vec::with_capacity(actions.len());
    let mut seen_identity = BTreeMap::<(InstrumentId, TradingDate), RangeAction>::new();
    for action in actions {
        match action {
            RangeAction::Unsupported { kind, .. } => {
                return Err(HistoricalPriceOnlyError::UnsupportedAction { kind: kind.clone() });
            }
            RangeAction::BonusIssue {
                instrument_id,
                record_date,
                ex_date,
                split_factor,
                ..
            } => {
                if !is_fixed_etf(instrument_id)
                    || *record_date < range_start
                    || *record_date > range_end
                    || *ex_date < range_start
                    || *ex_date > range_end
                {
                    return Err(input_mismatch(
                        "bonus action instrument or date is outside the verified range",
                    ));
                }
                let one = FixedPoint::parse("1").expect("one is a valid fixed point");
                if !split_factor.is_positive() || *split_factor <= one {
                    return Err(HistoricalPriceOnlyError::InvalidSplitFactor {
                        instrument: instrument_id.to_string(),
                        ex_date: *ex_date,
                        factor: split_factor.to_string(),
                    });
                }
                let identity = (instrument_id.clone(), *ex_date);
                if let Some(previous) = seen_identity.get(&identity) {
                    let error = if previous == action {
                        HistoricalPriceOnlyError::DuplicateAction {
                            instrument: instrument_id.to_string(),
                            ex_date: *ex_date,
                        }
                    } else {
                        HistoricalPriceOnlyError::ConflictingAction {
                            instrument: instrument_id.to_string(),
                            ex_date: *ex_date,
                        }
                    };
                    return Err(error);
                }
                seen_identity.insert(identity, action.clone());
                canonical.push(action.clone());
            }
        }
    }
    if canonical
        .windows(2)
        .any(|pair| compare_actions(&pair[0], &pair[1]) == Ordering::Greater)
    {
        return Err(HistoricalPriceOnlyError::NonCanonicalOrdering {
            kind: "bonus actions",
        });
    }
    Ok(canonical)
}

fn bonus_evidence(action: &RangeAction) -> HistoricalPriceOnlyBonusEvidence {
    match action {
        RangeAction::BonusIssue {
            instrument_id,
            record_date,
            ex_date,
            split_factor,
            available_at,
        } => HistoricalPriceOnlyBonusEvidence {
            instrument_id: instrument_id.clone(),
            record_date: *record_date,
            ex_date: *ex_date,
            split_factor: *split_factor,
            acquired_at: *available_at,
        },
        RangeAction::Unsupported { .. } => unreachable!("unsupported actions are rejected"),
    }
}

fn compare_actions(left: &RangeAction, right: &RangeAction) -> Ordering {
    match (left, right) {
        (
            RangeAction::BonusIssue {
                instrument_id: left_instrument,
                ex_date: left_ex,
                record_date: left_record,
                split_factor: left_factor,
                available_at: left_available,
            },
            RangeAction::BonusIssue {
                instrument_id: right_instrument,
                ex_date: right_ex,
                record_date: right_record,
                split_factor: right_factor,
                available_at: right_available,
            },
        ) => left_instrument
            .cmp(right_instrument)
            .then(left_ex.cmp(right_ex))
            .then(left_record.cmp(right_record))
            .then(left_factor.cmp(right_factor))
            .then(left_available.cmp(right_available)),
        (
            RangeAction::Unsupported {
                kind: left_kind, ..
            },
            RangeAction::Unsupported {
                kind: right_kind, ..
            },
        ) => left_kind.cmp(right_kind),
        (RangeAction::Unsupported { .. }, RangeAction::BonusIssue { .. }) => Ordering::Less,
        (RangeAction::BonusIssue { .. }, RangeAction::Unsupported { .. }) => Ordering::Greater,
    }
}

fn materialize_bars(
    raw_bars: &[RangeCanonicalBarCandidate],
    actions: &[RangeAction],
) -> Result<Vec<HistoricalPriceOnlyBar>, HistoricalPriceOnlyError> {
    let mut bars = raw_bars
        .iter()
        .map(|raw| {
            let factor = cumulative_factor(raw, actions)?;
            let adjusted_open = adjust_price(&raw.open, &factor)?;
            let adjusted_high = adjust_price(&raw.high, &factor)?;
            let adjusted_low = adjust_price(&raw.low, &factor)?;
            let adjusted_close = adjust_price(&raw.close, &factor)?;
            if !adjusted_open.is_positive()
                || !adjusted_high.is_positive()
                || !adjusted_low.is_positive()
                || !adjusted_close.is_positive()
                || adjusted_high < adjusted_open
                || adjusted_high < adjusted_close
                || adjusted_low > adjusted_open
                || adjusted_low > adjusted_close
                || adjusted_low > adjusted_high
            {
                return Err(HistoricalPriceOnlyError::OhlcInvariant {
                    instrument: raw.instrument_id.to_string(),
                    session_date: raw.session_date,
                    stage: "adjusted",
                });
            }
            Ok(HistoricalPriceOnlyBar {
                instrument_id: raw.instrument_id.clone(),
                session_date: raw.session_date,
                raw_open: raw.open,
                raw_high: raw.high,
                raw_low: raw.low,
                raw_close: raw.close,
                raw_volume: raw.volume,
                raw_trading_value: raw.trading_value,
                adjusted_open,
                adjusted_high,
                adjusted_low,
                adjusted_close,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    bars.sort_by(|left, right| {
        left.instrument_id
            .cmp(&right.instrument_id)
            .then(left.session_date.cmp(&right.session_date))
    });
    Ok(bars)
}

fn cumulative_factor(
    bar: &RangeCanonicalBarCandidate,
    actions: &[RangeAction],
) -> Result<FixedPoint, HistoricalPriceOnlyError> {
    let one = FixedPoint::parse("1")
        .expect("one is a valid fixed point")
        .with_scale(HISTORICAL_PRICE_ONLY_FACTOR_SCALE)
        .map_err(|_| HistoricalPriceOnlyError::ArithmeticOverflow {
            operation: "factor initialization",
        })?;
    let mut cumulative = one;
    for action in actions {
        let RangeAction::BonusIssue {
            instrument_id,
            ex_date,
            split_factor,
            ..
        } = action
        else {
            return Err(HistoricalPriceOnlyError::UnsupportedAction {
                kind: "unsupported".to_owned(),
            });
        };
        if instrument_id == &bar.instrument_id && bar.session_date < *ex_date {
            let reciprocal = one
                .checked_div(split_factor, HISTORICAL_PRICE_ONLY_FACTOR_SCALE)
                .map_err(|_| HistoricalPriceOnlyError::ArithmeticOverflow {
                    operation: "reciprocal split factor",
                })?;
            cumulative = cumulative
                .checked_mul(&reciprocal)
                .map_err(|_| HistoricalPriceOnlyError::ArithmeticOverflow {
                    operation: "cumulative split factor multiplication",
                })?
                .with_scale(HISTORICAL_PRICE_ONLY_FACTOR_SCALE)
                .map_err(|_| HistoricalPriceOnlyError::ArithmeticOverflow {
                    operation: "cumulative split factor rounding",
                })?;
        }
    }
    Ok(cumulative)
}

fn adjust_price(
    raw: &FixedPoint,
    factor: &FixedPoint,
) -> Result<FixedPoint, HistoricalPriceOnlyError> {
    raw.checked_mul(factor)
        .map_err(|_| HistoricalPriceOnlyError::ArithmeticOverflow {
            operation: "price-factor multiplication",
        })?
        .with_scale(HISTORICAL_PRICE_ONLY_PRICE_SCALE)
        .map_err(|_| HistoricalPriceOnlyError::ArithmeticOverflow {
            operation: "adjusted price rounding",
        })
}

fn validate_raw_bar(bar: &RangeCanonicalBarCandidate) -> Result<(), HistoricalPriceOnlyError> {
    if !bar.open.is_positive()
        || !bar.high.is_positive()
        || !bar.low.is_positive()
        || !bar.close.is_positive()
    {
        return Err(HistoricalPriceOnlyError::OhlcInvariant {
            instrument: bar.instrument_id.to_string(),
            session_date: bar.session_date,
            stage: "raw-positive",
        });
    }
    if bar.high < bar.open
        || bar.high < bar.close
        || bar.low > bar.open
        || bar.low > bar.close
        || bar.low > bar.high
    {
        return Err(HistoricalPriceOnlyError::OhlcInvariant {
            instrument: bar.instrument_id.to_string(),
            session_date: bar.session_date,
            stage: "raw",
        });
    }
    if bar
        .trading_value
        .as_ref()
        .is_some_and(FixedPoint::is_negative)
    {
        return Err(input_mismatch("trading value is negative"));
    }
    Ok(())
}

fn is_fixed_etf(instrument: &InstrumentId) -> bool {
    instrument.venue() == Venue::Krx
        && KR_ETF_CORE_SYMBOLS
            .iter()
            .any(|symbol| instrument.symbol() == *symbol)
}

fn input_mismatch(reason: &str) -> HistoricalPriceOnlyError {
    HistoricalPriceOnlyError::InputMismatch {
        reason: reason.to_owned(),
    }
}

#[derive(Serialize)]
struct CanonicalRepresentation<'a> {
    schema: &'static str,
    contract: &'static str,
    audience: &'static str,
    range_start: TradingDate,
    range_end: TradingDate,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: &'static str,
    materialized: bool,
    in_memory: bool,
    ready: bool,
    source_batch_id: BatchId,
    source_manifest_hash: &'a ContentHash,
    source_file_count: usize,
    source_files: Vec<CanonicalFile<'a>>,
    action_batch_id: BatchId,
    action_manifest_hash: &'a ContentHash,
    action_file_count: usize,
    cash_dividend_treatment_id: &'a str,
    ignored_cash_dividend_row_count: usize,
    ignored_cash_dividend_rows_sha256: &'a ContentHash,
    ignored_cash_dividend_source_file_sha256: &'a ContentHash,
    ignored_cash_dividend_acquired_at: UtcTimestamp,
    session_count: usize,
    row_count: usize,
    sessions: Vec<CanonicalSession<'a>>,
    bonus_evidence: Vec<CanonicalBonusEvidence<'a>>,
    bars: Vec<CanonicalBar<'a>>,
}

#[derive(Serialize)]
struct CanonicalFile<'a> {
    kind: ResponseKind,
    file_name: &'a str,
    content_hash: &'a ContentHash,
    size_bytes: u64,
    request: &'a RequestMetadata,
}

#[derive(Serialize)]
struct CanonicalSession<'a> {
    session_date: TradingDate,
    normalized_batch_id: BatchId,
    normalized_entry_hash: &'a ContentHash,
    normalized_bars_hash: &'a ContentHash,
    acquired_at: UtcTimestamp,
}

#[derive(Serialize)]
struct CanonicalBonusEvidence<'a> {
    instrument_id: &'a InstrumentId,
    record_date: TradingDate,
    ex_date: TradingDate,
    split_factor: &'a FixedPoint,
    acquired_at: UtcTimestamp,
}

#[derive(Serialize)]
struct CanonicalBar<'a> {
    instrument_id: &'a InstrumentId,
    session_date: TradingDate,
    raw_open: &'a FixedPoint,
    raw_high: &'a FixedPoint,
    raw_low: &'a FixedPoint,
    raw_close: &'a FixedPoint,
    raw_volume: u64,
    raw_trading_value: &'a Option<FixedPoint>,
    adjusted_open: &'a FixedPoint,
    adjusted_high: &'a FixedPoint,
    adjusted_low: &'a FixedPoint,
    adjusted_close: &'a FixedPoint,
}

fn canonical_content_hash(
    parts: &MaterializationParts,
    source_files: &[FileEntry],
    bonus_evidence: &[HistoricalPriceOnlyBonusEvidence],
    bars: &[HistoricalPriceOnlyBar],
) -> Result<ContentHash, HistoricalPriceOnlyError> {
    let representation = canonical_representation(parts, source_files, bonus_evidence, bars);
    let bytes =
        serde_json::to_vec(&representation).map_err(|_| HistoricalPriceOnlyError::Serialization)?;
    Ok(ContentHash::from_bytes(&bytes))
}

fn canonical_representation<'a>(
    parts: &'a MaterializationParts,
    source_files: &'a [FileEntry],
    bonus_evidence: &'a [HistoricalPriceOnlyBonusEvidence],
    bars: &'a [HistoricalPriceOnlyBar],
) -> CanonicalRepresentation<'a> {
    CanonicalRepresentation {
        schema: HISTORICAL_PRICE_ONLY_MATERIALIZER_VERSION,
        contract: HISTORICAL_PRICE_ONLY_BETA_CONTRACT,
        audience: HistoricalPriceOnlyAudience::OwnerOnly.as_str(),
        range_start: parts.range_start,
        range_end: parts.range_end,
        vendor_snapshot: HistoricalPriceOnlyMetadata::FIXED.vendor_snapshot,
        strict_pit: HistoricalPriceOnlyMetadata::FIXED.strict_pit,
        capability: "PRICE_RETURN_ONLY",
        materialized: HistoricalPriceOnlyMetadata::FIXED.materialized,
        in_memory: HistoricalPriceOnlyMetadata::FIXED.in_memory,
        ready: HistoricalPriceOnlyMetadata::FIXED.ready,
        source_batch_id: parts.source_batch_id,
        source_manifest_hash: &parts.source_manifest_hash,
        source_file_count: source_files.len(),
        source_files: source_files
            .iter()
            .map(|file| CanonicalFile {
                kind: file.kind,
                file_name: &file.file_name,
                content_hash: &file.content_hash,
                size_bytes: file.size_bytes,
                request: &file.request,
            })
            .collect(),
        action_batch_id: parts.action_batch_id,
        action_manifest_hash: &parts.action_manifest_hash,
        action_file_count: parts.action_file_count,
        cash_dividend_treatment_id: parts.ignored_cash_dividends.treatment_id(),
        ignored_cash_dividend_row_count: parts.ignored_cash_dividends.row_count(),
        ignored_cash_dividend_rows_sha256: parts.ignored_cash_dividends.rows_sha256(),
        ignored_cash_dividend_source_file_sha256: parts.ignored_cash_dividends.source_file_sha256(),
        ignored_cash_dividend_acquired_at: parts.ignored_cash_dividends.acquired_at(),
        session_count: parts.sessions.len(),
        row_count: bars.len(),
        sessions: parts
            .sessions
            .iter()
            .map(|session| CanonicalSession {
                session_date: session.session_date,
                normalized_batch_id: session.normalized_batch_id,
                normalized_entry_hash: &session.normalized_entry_hash,
                normalized_bars_hash: &session.normalized_bars_hash,
                acquired_at: session.acquired_at,
            })
            .collect(),
        bonus_evidence: bonus_evidence
            .iter()
            .map(|evidence| CanonicalBonusEvidence {
                instrument_id: &evidence.instrument_id,
                record_date: evidence.record_date,
                ex_date: evidence.ex_date,
                split_factor: &evidence.split_factor,
                acquired_at: evidence.acquired_at,
            })
            .collect(),
        bars: bars
            .iter()
            .map(|bar| CanonicalBar {
                instrument_id: &bar.instrument_id,
                session_date: bar.session_date,
                raw_open: &bar.raw_open,
                raw_high: &bar.raw_high,
                raw_low: &bar.raw_low,
                raw_close: &bar.raw_close,
                raw_volume: bar.raw_volume,
                raw_trading_value: &bar.raw_trading_value,
                adjusted_open: &bar.adjusted_open,
                adjusted_high: &bar.adjusted_high,
                adjusted_low: &bar.adjusted_low,
                adjusted_close: &bar.adjusted_close,
            })
            .collect(),
    }
}

#[cfg(test)]
pub(crate) fn artifact_test_candidate() -> HistoricalPriceOnlyCandidate {
    use uuid::Uuid;

    let start = TradingDate::parse("2020-01-31").unwrap();
    let end = TradingDate::parse("2026-08-19").unwrap();
    let session_dates = crate::range_normalize::ExpectedRangeSessions::approved_xkrx(start, end)
        .expect("embedded approved XKRX sessions")
        .sessions;
    let h = |value: &str| ContentHash::from_bytes(value.as_bytes());
    let id = |value| BatchId::from_uuid(Uuid::from_u128(value));
    let timestamp = UtcTimestamp::parse_rfc3339("2026-08-19T00:00:00Z").unwrap();
    let sessions = session_dates
        .iter()
        .enumerate()
        .map(|(i, date)| HistoricalPriceOnlySessionProvenance {
            session_date: *date,
            normalized_batch_id: id(i as u128 + 100),
            normalized_entry_hash: h(&format!("entry-{i}")),
            normalized_bars_hash: h(&format!("bars-{i}")),
            acquired_at: timestamp,
        })
        .collect::<Vec<_>>();
    let source_files = KR_ETF_CORE_SYMBOLS
        .iter()
        .flat_map(|symbol| {
            (1..=17).map(move |window| {
                let name = format!("daily-bars-range-window-{window}-{symbol}-page-01.json");
                FileEntry {
                    kind: ResponseKind::Bars,
                    file_name: name.clone(),
                    content_hash: h(&name),
                    size_bytes: 1,
                    request: RequestMetadata {
                        endpoint: "sentinel-request-secret".into(),
                        query: vec![("sentinel-query".into(), "sentinel-value".into())],
                        headers: vec![("sentinel-header".into(), "sentinel-secret".into())],
                        mode: crate::contract::FetchMode::Credentialed,
                    },
                }
            })
        })
        .collect::<Vec<_>>();
    let bars = KR_ETF_CORE_SYMBOLS
        .iter()
        .flat_map(|symbol| {
            session_dates
                .iter()
                .map(move |date| HistoricalPriceOnlyBar {
                    instrument_id: InstrumentId::from_parts(symbol, Venue::Krx).unwrap(),
                    session_date: *date,
                    raw_open: FixedPoint::parse("10.00").unwrap(),
                    raw_high: FixedPoint::parse("11.00").unwrap(),
                    raw_low: FixedPoint::parse("9.00").unwrap(),
                    raw_close: FixedPoint::parse("10.50").unwrap(),
                    raw_volume: 1,
                    raw_trading_value: Some(FixedPoint::parse("1.00").unwrap()),
                    adjusted_open: FixedPoint::parse("10.0000").unwrap(),
                    adjusted_high: FixedPoint::parse("11.0000").unwrap(),
                    adjusted_low: FixedPoint::parse("9.0000").unwrap(),
                    adjusted_close: FixedPoint::parse("10.5000").unwrap(),
                })
        })
        .collect::<Vec<_>>();
    HistoricalPriceOnlyCandidate {
        range_start: start,
        range_end: end,
        source_batch_id: id(1),
        source_manifest_hash: h("source"),
        source_files,
        action_batch_id: id(2),
        action_manifest_hash: h("actions"),
        action_file_count: 7,
        ignored_cash_dividends: HistoricalPriceOnlyIgnoredCashDividendEvidence::new(
            1,
            h("ignored-cash-dividend-rows"),
            h("dividend-source-file"),
            timestamp,
        ),
        sessions,
        bonus_evidence: Vec::new(),
        bars,
        metadata: HistoricalPriceOnlyMetadata::FIXED,
        content_hash: h("opaque-candidate-lineage"),
    }
}

#[cfg(test)]
#[path = "historical_price_only_tests.rs"]
mod tests;
