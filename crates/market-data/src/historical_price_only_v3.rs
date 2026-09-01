//! V3 in-memory historical price-only candidate.
//!
//! This module is the narrow seam between the verified V3 replay evidence and
//! a future artifact boundary.  It does not read Raw, write any artifact, or
//! make a historical membership or point-in-time claim.  The fixed ETF11 set
//! is a retrospective observation universe only.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use domain::{BatchId, ContentHash, FixedPoint, InstrumentId, TradingDate, UtcTimestamp, Venue};
use serde::Serialize;
use thiserror::Error;

use crate::curate::Capability;
use crate::historical_price_only::{HistoricalPriceOnlyBar, HistoricalPriceOnlyMetadata};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::range_to_canonical::RangeCanonicalBarCandidate;
use crate::range_to_canonical_v3::{
    HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID, HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
    HISTORICAL_PRICE_ONLY_V3_ACTION_PIT_POLICY, HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT,
    HistoricalPriceOnlyV3ActionEvidence,
};
use crate::range_to_canonical_v3_price::{
    HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT, HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID,
    HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
    HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT, HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT,
    HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256, HISTORICAL_PRICE_ONLY_V3_PRICE_PIT_POLICY,
    HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END, HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START,
    HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE,
    HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT, HistoricalPriceOnlyV3PriceEvidence,
};

/// Stable schema identifier for the opaque V3 candidate preimage.
pub const HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID: &str = "kis-historical-price-only-v3";
/// Version of the candidate schema.
pub const HISTORICAL_PRICE_ONLY_V3_SCHEMA_VERSION: u32 = 1;
/// Alias retaining the explicit candidate wording for callers that need it.
pub const HISTORICAL_PRICE_ONLY_V3_CANDIDATE_SCHEMA_VERSION: u32 =
    HISTORICAL_PRICE_ONLY_V3_SCHEMA_VERSION;
/// Stable contract identifier for the V3 candidate seam.
pub const HISTORICAL_PRICE_ONLY_V3_CONTRACT: &str = "kis-historical-price-only-v3";
/// Version of this non-persisted V3 materializer.
pub const HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION: &str =
    "kis-historical-price-only-materializer-v3";

/// Fixed V3 range, expressed as the typed source-contract values.
pub const HISTORICAL_PRICE_ONLY_V3_RANGE_START: &str = HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START;
/// Fixed V3 range, expressed as the typed source-contract values.
pub const HISTORICAL_PRICE_ONLY_V3_RANGE_END: &str = HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END;
/// Fixed ETF11 instrument count.
pub const HISTORICAL_PRICE_ONLY_V3_INSTRUMENT_COUNT: usize = KR_ETF_CORE_SYMBOLS.len();
/// Fixed observed session count.
pub const HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT: usize =
    HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT;
/// Fixed observed bar count.
pub const HISTORICAL_PRICE_ONLY_V3_BAR_COUNT: usize = HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT;
/// Fixed price-file count.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_FILES: usize = HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT;
/// Fixed action-file count.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_FILES: usize = HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT;
/// Fixed point-in-time policy.
pub const HISTORICAL_PRICE_ONLY_V3_PIT_POLICY: &str = HISTORICAL_PRICE_ONLY_V3_PRICE_PIT_POLICY;
/// Fixed cash-only action treatment.
pub const HISTORICAL_PRICE_ONLY_V3_CASH_TREATMENT: &str =
    HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT;
/// Exact number of validated cash-only dividend rows in the pinned action batch.
pub const HISTORICAL_PRICE_ONLY_V3_CASH_ROW_COUNT: usize = 157;
/// Canonical commitment to the validated cash-only dividend rows.
pub const HISTORICAL_PRICE_ONLY_V3_CASH_ROWS_SHA256: &str =
    "sha256:b22a5c9808a8a1a2c892aa3ff46d529672c909620a2c45c0e46d48d0538d17e8";

/// The immutable fixed metadata attached to every V3 candidate.
///
/// This is the existing V2 metadata type with the same values.  In particular,
/// `ready` remains false and `materialized` remains false at this in-memory
/// boundary; constructing a candidate does not authorize publication.
fn fixed_metadata() -> HistoricalPriceOnlyMetadata {
    HistoricalPriceOnlyMetadata {
        audience: crate::historical_price_only::HistoricalPriceOnlyAudience::OwnerOnly,
        vendor_snapshot: true,
        strict_pit: false,
        capability: Capability::PriceReturnOnly,
        materialized: false,
        in_memory: true,
        ready: false,
    }
}

/// A retained commitment for the validated cash-only KSD rows.
///
/// The rows themselves are deliberately not carried here.  The hash and
/// count are evidence commitments only; no cash amount, ex-date, or
/// point-in-time availability is inferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyV3CashDividendCommitment {
    treatment_id: String,
    row_count: usize,
    rows_sha256: ContentHash,
    acquired_at: UtcTimestamp,
}

impl HistoricalPriceOnlyV3CashDividendCommitment {
    pub fn treatment_id(&self) -> &str {
        &self.treatment_id
    }

    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    pub fn rows_sha256(&self) -> &ContentHash {
        &self.rows_sha256
    }

    pub const fn acquired_at(&self) -> UtcTimestamp {
        self.acquired_at
    }
}

/// Opaque, in-memory V3 candidate.
///
/// This type intentionally does not implement `Serialize`.  Its accessors
/// expose reviewable evidence and rows, but no artifact, database, or
/// publication API accepts it at this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalPriceOnlyV3Candidate {
    range_start: TradingDate,
    range_end: TradingDate,
    pit_policy: String,
    metadata: HistoricalPriceOnlyMetadata,
    price_batch_id: BatchId,
    price_batch_json_hash: ContentHash,
    price_manifest_line_hash: ContentHash,
    price_bars_hash: ContentHash,
    price_file_count: usize,
    price_capture_contract_commit: String,
    price_response_marker_evidence: String,
    price_acquired_at: UtcTimestamp,
    action_batch_id: BatchId,
    action_batch_json_hash: ContentHash,
    action_manifest_line_hash: ContentHash,
    action_file_count: usize,
    cash_dividends: HistoricalPriceOnlyV3CashDividendCommitment,
    action_count: usize,
    instruments: Vec<InstrumentId>,
    sessions: Vec<TradingDate>,
    bars: Vec<HistoricalPriceOnlyBar>,
    content_hash: ContentHash,
}

impl HistoricalPriceOnlyV3Candidate {
    pub const fn schema_version(&self) -> u32 {
        HISTORICAL_PRICE_ONLY_V3_SCHEMA_VERSION
    }

    pub const fn schema_id(&self) -> &'static str {
        HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID
    }

    pub const fn contract(&self) -> &'static str {
        HISTORICAL_PRICE_ONLY_V3_CONTRACT
    }

    pub const fn materializer_version(&self) -> &'static str {
        HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION
    }

    pub const fn range_start(&self) -> TradingDate {
        self.range_start
    }

    pub const fn range_end(&self) -> TradingDate {
        self.range_end
    }

    pub fn pit_policy(&self) -> &str {
        &self.pit_policy
    }

    pub const fn vendor_snapshot(&self) -> bool {
        self.metadata.vendor_snapshot
    }

    pub const fn strict_pit(&self) -> bool {
        self.metadata.strict_pit
    }

    pub const fn metadata(&self) -> HistoricalPriceOnlyMetadata {
        self.metadata
    }

    pub fn audience(&self) -> crate::historical_price_only::HistoricalPriceOnlyAudience {
        self.metadata.audience
    }

    pub const fn capability(&self) -> Capability {
        self.metadata.capability
    }

    pub const fn materialized(&self) -> bool {
        self.metadata.materialized
    }

    pub const fn in_memory(&self) -> bool {
        self.metadata.in_memory
    }

    pub const fn ready(&self) -> bool {
        self.metadata.ready
    }

    pub const fn price_batch_id(&self) -> BatchId {
        self.price_batch_id
    }

    pub fn price_batch_json_hash(&self) -> &ContentHash {
        &self.price_batch_json_hash
    }

    pub fn price_batch_json_sha256(&self) -> &ContentHash {
        self.price_batch_json_hash()
    }

    pub fn price_manifest_line_hash(&self) -> &ContentHash {
        &self.price_manifest_line_hash
    }

    pub fn price_manifest_line_sha256(&self) -> &ContentHash {
        self.price_manifest_line_hash()
    }

    pub fn price_bars_hash(&self) -> &ContentHash {
        &self.price_bars_hash
    }

    pub fn price_bars_sha256(&self) -> &ContentHash {
        self.price_bars_hash()
    }

    pub fn source_bars_sha256(&self) -> &ContentHash {
        self.price_bars_hash()
    }

    pub fn source_bars_hash(&self) -> &ContentHash {
        self.price_bars_hash()
    }

    pub const fn price_file_count(&self) -> usize {
        self.price_file_count
    }

    pub const fn source_file_count(&self) -> usize {
        self.price_file_count()
    }

    pub const fn price_source_file_count(&self) -> usize {
        self.price_file_count()
    }

    pub fn price_capture_contract_commit(&self) -> &str {
        &self.price_capture_contract_commit
    }

    pub fn capture_contract_commit(&self) -> &str {
        self.price_capture_contract_commit()
    }

    pub fn source_capture_contract_commit(&self) -> &str {
        self.price_capture_contract_commit()
    }

    pub fn price_response_marker_evidence(&self) -> &str {
        &self.price_response_marker_evidence
    }

    pub fn response_marker_evidence(&self) -> &str {
        self.price_response_marker_evidence()
    }

    pub fn source_response_marker_evidence(&self) -> &str {
        self.price_response_marker_evidence()
    }

    pub const fn price_acquired_at(&self) -> UtcTimestamp {
        self.price_acquired_at
    }

    pub const fn source_acquired_at(&self) -> UtcTimestamp {
        self.price_acquired_at()
    }

    pub const fn price_source_acquired_at(&self) -> UtcTimestamp {
        self.price_acquired_at()
    }

    pub const fn action_batch_id(&self) -> BatchId {
        self.action_batch_id
    }

    pub const fn action_source_batch_id(&self) -> BatchId {
        self.action_batch_id()
    }

    pub fn action_batch_json_hash(&self) -> &ContentHash {
        &self.action_batch_json_hash
    }

    pub fn action_source_batch_json_hash(&self) -> &ContentHash {
        self.action_batch_json_hash()
    }

    pub fn action_source_batch_json_sha256(&self) -> &ContentHash {
        self.action_batch_json_hash()
    }

    pub fn action_batch_json_sha256(&self) -> &ContentHash {
        self.action_batch_json_hash()
    }

    pub fn action_manifest_line_hash(&self) -> &ContentHash {
        &self.action_manifest_line_hash
    }

    pub fn action_source_manifest_line_hash(&self) -> &ContentHash {
        self.action_manifest_line_hash()
    }

    pub fn action_source_manifest_line_sha256(&self) -> &ContentHash {
        self.action_manifest_line_hash()
    }

    pub fn action_manifest_line_sha256(&self) -> &ContentHash {
        self.action_manifest_line_hash()
    }

    pub const fn action_file_count(&self) -> usize {
        self.action_file_count
    }

    pub const fn action_acquired_at(&self) -> UtcTimestamp {
        self.cash_dividends.acquired_at()
    }

    pub const fn action_count(&self) -> usize {
        self.action_count
    }

    pub fn cash_dividends(&self) -> &HistoricalPriceOnlyV3CashDividendCommitment {
        &self.cash_dividends
    }

    pub fn cash_treatment(&self) -> &str {
        self.cash_dividends.treatment_id()
    }

    pub fn cash_dividend_treatment(&self) -> &str {
        self.cash_treatment()
    }

    pub fn cash_dividend_treatment_id(&self) -> &str {
        self.cash_treatment()
    }

    pub const fn cash_row_count(&self) -> usize {
        self.cash_dividends.row_count()
    }

    pub const fn cash_dividend_row_count(&self) -> usize {
        self.cash_row_count()
    }

    pub fn cash_rows_hash(&self) -> &ContentHash {
        self.cash_dividends.rows_sha256()
    }

    pub fn cash_dividend_rows_hash(&self) -> &ContentHash {
        self.cash_rows_hash()
    }

    pub fn cash_rows_sha256(&self) -> &ContentHash {
        self.cash_rows_hash()
    }

    pub fn cash_dividend_rows_sha256(&self) -> &ContentHash {
        self.cash_rows_hash()
    }

    pub const fn cash_acquired_at(&self) -> UtcTimestamp {
        self.cash_dividends.acquired_at()
    }

    pub const fn cash_dividend_acquired_at(&self) -> UtcTimestamp {
        self.cash_acquired_at()
    }

    pub fn instruments(&self) -> &[InstrumentId] {
        &self.instruments
    }

    pub fn sessions(&self) -> &[TradingDate] {
        &self.sessions
    }

    pub fn trading_dates(&self) -> &[TradingDate] {
        self.sessions()
    }

    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.bars
    }

    pub const fn instrument_count(&self) -> usize {
        self.instruments.len()
    }

    pub const fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub const fn bar_count(&self) -> usize {
        self.bars.len()
    }

    pub const fn row_count(&self) -> usize {
        self.bars.len()
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    // Source-oriented aliases make the provenance distinction explicit while
    // retaining convenient names for callers that consume a single price
    // source in this candidate.
    pub const fn source_batch_id(&self) -> BatchId {
        self.price_batch_id()
    }

    pub const fn price_source_batch_id(&self) -> BatchId {
        self.price_batch_id()
    }

    pub fn source_batch_json_hash(&self) -> &ContentHash {
        self.price_batch_json_hash()
    }

    pub fn source_batch_json_sha256(&self) -> &ContentHash {
        self.price_batch_json_hash()
    }

    pub fn price_source_batch_json_hash(&self) -> &ContentHash {
        self.price_batch_json_hash()
    }

    pub fn source_manifest_line_hash(&self) -> &ContentHash {
        self.price_manifest_line_hash()
    }

    pub fn source_manifest_line_sha256(&self) -> &ContentHash {
        self.price_manifest_line_hash()
    }

    pub fn price_source_manifest_line_hash(&self) -> &ContentHash {
        self.price_manifest_line_hash()
    }
}

/// Typed, fail-closed V3 materialization errors.
///
/// Error data is limited to static reason codes and canonical instrument/date
/// keys.  No provider response body, broker prose, request metadata, or secret
/// can cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HistoricalPriceOnlyV3Error {
    #[error("V3 historical price-only input mismatch: {reason}")]
    InputMismatch { reason: &'static str },
    #[error("V3 historical price-only policies disagree")]
    PolicyMismatch,
    #[error("duplicate V3 historical price-only bar {instrument} on {session_date}")]
    DuplicateBar {
        instrument: String,
        session_date: TradingDate,
    },
    #[error("V3 historical price-only OHLC invariant violation for {instrument} on {session_date}")]
    OhlcInvariant {
        instrument: String,
        session_date: TradingDate,
    },
    #[error("V3 historical price-only canonical representation could not be serialized")]
    Serialization,
}

/// Materialize the two independently verified V3 replay evidences into an
/// opaque in-memory price-return-only candidate.
pub fn materialize_historical_price_only_v3(
    price: &HistoricalPriceOnlyV3PriceEvidence,
    action: &HistoricalPriceOnlyV3ActionEvidence,
) -> Result<HistoricalPriceOnlyV3Candidate, HistoricalPriceOnlyV3Error> {
    validate_policies(price, action)?;
    validate_price_evidence(price)?;
    validate_action_evidence(action)?;

    let (instruments, sessions, bars) = canonicalize_bars(price)?;
    let cash_dividends = HistoricalPriceOnlyV3CashDividendCommitment {
        treatment_id: action.cash_dividends().treatment_id().to_owned(),
        row_count: action.cash_dividends().row_count(),
        rows_sha256: action.cash_dividends().rows_sha256().clone(),
        acquired_at: action.cash_dividends().acquired_at(),
    };
    let metadata = fixed_metadata();
    let candidate = CandidateParts {
        range_start: price.range_start(),
        range_end: price.range_end(),
        pit_policy: price.pit_policy().to_owned(),
        metadata,
        price_batch_id: price.source_batch_id(),
        price_batch_json_hash: price.source_batch_json_sha256().clone(),
        price_manifest_line_hash: price.source_manifest_line_sha256().clone(),
        price_bars_hash: price.bars_sha256().clone(),
        price_file_count: price.files().len(),
        price_capture_contract_commit: price.capture_contract_commit().to_owned(),
        price_response_marker_evidence: price.response_marker_evidence().to_owned(),
        price_acquired_at: price.acquired_at(),
        action_batch_id: action.source_batch_id(),
        action_batch_json_hash: action.source_batch_json_sha256().clone(),
        action_manifest_line_hash: action.source_manifest_line_sha256().clone(),
        action_file_count: action.files().len(),
        cash_dividends,
        action_count: action.action_count(),
        instruments,
        sessions,
        bars,
    };
    let content_hash = canonical_content_hash(&candidate)?;

    Ok(HistoricalPriceOnlyV3Candidate {
        range_start: candidate.range_start,
        range_end: candidate.range_end,
        pit_policy: candidate.pit_policy,
        metadata: candidate.metadata,
        price_batch_id: candidate.price_batch_id,
        price_batch_json_hash: candidate.price_batch_json_hash,
        price_manifest_line_hash: candidate.price_manifest_line_hash,
        price_bars_hash: candidate.price_bars_hash,
        price_file_count: candidate.price_file_count,
        price_capture_contract_commit: candidate.price_capture_contract_commit,
        price_response_marker_evidence: candidate.price_response_marker_evidence,
        price_acquired_at: candidate.price_acquired_at,
        action_batch_id: candidate.action_batch_id,
        action_batch_json_hash: candidate.action_batch_json_hash,
        action_manifest_line_hash: candidate.action_manifest_line_hash,
        action_file_count: candidate.action_file_count,
        cash_dividends: candidate.cash_dividends,
        action_count: candidate.action_count,
        instruments: candidate.instruments,
        sessions: candidate.sessions,
        bars: candidate.bars,
        content_hash,
    })
}

fn validate_policies(
    price: &HistoricalPriceOnlyV3PriceEvidence,
    action: &HistoricalPriceOnlyV3ActionEvidence,
) -> Result<(), HistoricalPriceOnlyV3Error> {
    if price.vendor_snapshot() != action.vendor_snapshot()
        || price.strict_pit() != action.strict_pit()
        || price.pit_policy() != action.pit_policy()
    {
        return Err(HistoricalPriceOnlyV3Error::PolicyMismatch);
    }
    Ok(())
}

fn validate_price_evidence(
    price: &HistoricalPriceOnlyV3PriceEvidence,
) -> Result<(), HistoricalPriceOnlyV3Error> {
    let expected_batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID)
        .expect("checked-in V3 price batch id is valid");
    let expected_batch_hash = ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256)
        .expect("checked-in V3 price batch hash is valid");
    let expected_manifest_hash =
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256)
            .expect("checked-in V3 price manifest hash is valid");
    let expected_start = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START)
        .expect("checked-in V3 price range start is valid");
    let expected_end = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END)
        .expect("checked-in V3 price range end is valid");

    if price.source_batch_id() != expected_batch_id
        || price.source_batch_json_sha256() != &expected_batch_hash
        || price.source_manifest_line_sha256() != &expected_manifest_hash
        || price.capture_contract_commit() != HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT
        || price.response_marker_evidence()
            != HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
        || price.range_start() != expected_start
        || price.range_end() != expected_end
        || !price.vendor_snapshot()
        || price.strict_pit()
        || price.pit_policy() != HISTORICAL_PRICE_ONLY_V3_PRICE_PIT_POLICY
        || price.files().len() != HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        || price.session_count() != HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT
        || price.bar_count() != HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
    {
        return Err(input_mismatch(
            "price evidence does not match the fixed V3 contract",
        ));
    }
    Ok(())
}

fn validate_action_evidence(
    action: &HistoricalPriceOnlyV3ActionEvidence,
) -> Result<(), HistoricalPriceOnlyV3Error> {
    let expected_batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID)
        .expect("checked-in V3 action batch id is valid");
    let expected_batch_hash = ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256)
        .expect("checked-in V3 action batch hash is valid");
    let expected_manifest_hash =
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256)
            .expect("checked-in V3 action manifest hash is valid");
    let expected_cash_rows_hash = ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_CASH_ROWS_SHA256)
        .expect("checked-in V3 cash-row hash is valid");

    if action.source_batch_id() != expected_batch_id
        || action.source_batch_json_sha256() != &expected_batch_hash
        || action.source_manifest_line_sha256() != &expected_manifest_hash
        || !action.vendor_snapshot()
        || action.strict_pit()
        || action.pit_policy() != HISTORICAL_PRICE_ONLY_V3_ACTION_PIT_POLICY
        || action.files().len() != HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        || action.action_count() != 0
        || action.cash_dividends().treatment_id()
            != HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT
        || action.cash_dividends().row_count() != HISTORICAL_PRICE_ONLY_V3_CASH_ROW_COUNT
        || action.cash_dividends().rows_sha256() != &expected_cash_rows_hash
    {
        return Err(input_mismatch(
            "action evidence does not match the fixed V3 contract",
        ));
    }
    Ok(())
}

fn canonicalize_bars(
    price: &HistoricalPriceOnlyV3PriceEvidence,
) -> Result<CanonicalCoverage, HistoricalPriceOnlyV3Error> {
    let expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            InstrumentId::from_parts(symbol, Venue::Krx)
                .expect("checked-in KR ETF universe is valid")
        })
        .collect::<BTreeSet<_>>();
    let expected_start = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START)
        .expect("checked-in V3 price range start is valid");
    let expected_end = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END)
        .expect("checked-in V3 price range end is valid");

    let declared_sessions = price
        .trading_dates()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let approved_sessions =
        crate::range_normalize::ExpectedRangeSessions::approved_xkrx(expected_start, expected_end)
            .map_err(|_| input_mismatch("approved XKRX calendar is unavailable"))?
            .sessions
            .into_iter()
            .collect::<BTreeSet<_>>();
    if declared_sessions.len() != HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT
        || declared_sessions.first().copied() != Some(expected_start)
        || declared_sessions.last().copied() != Some(expected_end)
        || declared_sessions != approved_sessions
    {
        return Err(input_mismatch(
            "price evidence does not contain the exact date range",
        ));
    }

    let mut by_key = BTreeMap::<(InstrumentId, TradingDate), RangeCanonicalBarCandidate>::new();
    let mut date_instruments = BTreeMap::<TradingDate, BTreeSet<InstrumentId>>::new();
    for bar in price.bars() {
        if !expected_instruments.contains(&bar.instrument_id)
            || bar.session_date < expected_start
            || bar.session_date > expected_end
            || !declared_sessions.contains(&bar.session_date)
        {
            return Err(input_mismatch(
                "bar instrument or date is outside fixed V3 coverage",
            ));
        }
        validate_raw_bar(bar)?;
        let key = (bar.instrument_id.clone(), bar.session_date);
        if by_key.insert(key, bar.clone()).is_some() {
            return Err(HistoricalPriceOnlyV3Error::DuplicateBar {
                instrument: bar.instrument_id.to_string(),
                session_date: bar.session_date,
            });
        }
        date_instruments
            .entry(bar.session_date)
            .or_default()
            .insert(bar.instrument_id.clone());
    }
    if by_key.len() != HISTORICAL_PRICE_ONLY_V3_BAR_COUNT
        || date_instruments.len() != HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT
        || date_instruments
            .values()
            .any(|instruments| instruments != &expected_instruments)
    {
        return Err(input_mismatch(
            "price evidence does not contain exact ETF11 date coverage",
        ));
    }
    if date_instruments.keys().copied().collect::<BTreeSet<_>>() != declared_sessions {
        return Err(input_mismatch(
            "declared sessions differ from bar date coverage",
        ));
    }

    let mut canonical_candidates = by_key.into_values().collect::<Vec<_>>();
    canonical_candidates.sort_by(|left, right| {
        left.instrument_id
            .cmp(&right.instrument_id)
            .then(left.session_date.cmp(&right.session_date))
    });
    let bars = canonical_candidates
        .iter()
        .map(|raw| HistoricalPriceOnlyBar {
            instrument_id: raw.instrument_id.clone(),
            session_date: raw.session_date,
            raw_open: raw.open,
            raw_high: raw.high,
            raw_low: raw.low,
            raw_close: raw.close,
            raw_volume: raw.volume,
            raw_trading_value: raw.trading_value,
            adjusted_open: raw.open,
            adjusted_high: raw.high,
            adjusted_low: raw.low,
            adjusted_close: raw.close,
        })
        .collect::<Vec<_>>();
    let instruments = expected_instruments.into_iter().collect::<Vec<_>>();
    let sessions = declared_sessions.into_iter().collect::<Vec<_>>();
    let expected_bars_hash = ContentHash::from_bytes(
        &serde_json::to_vec(&canonical_candidates)
            .map_err(|_| HistoricalPriceOnlyV3Error::Serialization)?,
    );
    if price.bars_sha256() != &expected_bars_hash {
        return Err(input_mismatch(
            "price bar commitment differs from canonical bars",
        ));
    }
    Ok((instruments, sessions, bars))
}

fn validate_raw_bar(bar: &RangeCanonicalBarCandidate) -> Result<(), HistoricalPriceOnlyV3Error> {
    if !bar.open.is_positive()
        || !bar.high.is_positive()
        || !bar.low.is_positive()
        || !bar.close.is_positive()
        || bar.high < bar.open
        || bar.high < bar.close
        || bar.low > bar.open
        || bar.low > bar.close
        || bar.low > bar.high
        || bar
            .trading_value
            .as_ref()
            .is_some_and(FixedPoint::is_negative)
    {
        return Err(HistoricalPriceOnlyV3Error::OhlcInvariant {
            instrument: bar.instrument_id.to_string(),
            session_date: bar.session_date,
        });
    }
    Ok(())
}

fn input_mismatch(reason: &'static str) -> HistoricalPriceOnlyV3Error {
    HistoricalPriceOnlyV3Error::InputMismatch { reason }
}

struct CandidateParts {
    range_start: TradingDate,
    range_end: TradingDate,
    pit_policy: String,
    metadata: HistoricalPriceOnlyMetadata,
    price_batch_id: BatchId,
    price_batch_json_hash: ContentHash,
    price_manifest_line_hash: ContentHash,
    price_bars_hash: ContentHash,
    price_file_count: usize,
    price_capture_contract_commit: String,
    price_response_marker_evidence: String,
    price_acquired_at: UtcTimestamp,
    action_batch_id: BatchId,
    action_batch_json_hash: ContentHash,
    action_manifest_line_hash: ContentHash,
    action_file_count: usize,
    cash_dividends: HistoricalPriceOnlyV3CashDividendCommitment,
    action_count: usize,
    instruments: Vec<InstrumentId>,
    sessions: Vec<TradingDate>,
    bars: Vec<HistoricalPriceOnlyBar>,
}

type CanonicalCoverage = (
    Vec<InstrumentId>,
    Vec<TradingDate>,
    Vec<HistoricalPriceOnlyBar>,
);

#[derive(Serialize)]
struct CanonicalRepresentation<'a> {
    schema_id: &'static str,
    schema_version: u32,
    contract: &'static str,
    materializer_version: &'static str,
    range_start: TradingDate,
    range_end: TradingDate,
    audience: &'static str,
    vendor_snapshot: bool,
    strict_pit: bool,
    pit_policy: &'a str,
    capability: &'static str,
    materialized: bool,
    in_memory: bool,
    ready: bool,
    price_batch_id: BatchId,
    price_batch_json_hash: &'a ContentHash,
    price_manifest_line_hash: &'a ContentHash,
    price_bars_hash: &'a ContentHash,
    price_file_count: usize,
    price_capture_contract_commit: &'a str,
    price_response_marker_evidence: &'a str,
    price_acquired_at: UtcTimestamp,
    action_batch_id: BatchId,
    action_batch_json_hash: &'a ContentHash,
    action_manifest_line_hash: &'a ContentHash,
    action_file_count: usize,
    cash_treatment: &'a str,
    cash_row_count: usize,
    cash_rows_hash: &'a ContentHash,
    cash_acquired_at: UtcTimestamp,
    action_count: usize,
    instrument_count: usize,
    instruments: &'a [InstrumentId],
    session_count: usize,
    sessions: &'a [TradingDate],
    bar_count: usize,
    bars: Vec<CanonicalBar<'a>>,
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
    parts: &CandidateParts,
) -> Result<ContentHash, HistoricalPriceOnlyV3Error> {
    let representation = CanonicalRepresentation {
        schema_id: HISTORICAL_PRICE_ONLY_V3_SCHEMA_ID,
        schema_version: HISTORICAL_PRICE_ONLY_V3_SCHEMA_VERSION,
        contract: HISTORICAL_PRICE_ONLY_V3_CONTRACT,
        materializer_version: HISTORICAL_PRICE_ONLY_V3_MATERIALIZER_VERSION,
        range_start: parts.range_start,
        range_end: parts.range_end,
        audience: parts.metadata.audience.as_str(),
        vendor_snapshot: parts.metadata.vendor_snapshot,
        strict_pit: parts.metadata.strict_pit,
        pit_policy: &parts.pit_policy,
        capability: "PRICE_RETURN_ONLY",
        materialized: parts.metadata.materialized,
        in_memory: parts.metadata.in_memory,
        ready: parts.metadata.ready,
        price_batch_id: parts.price_batch_id,
        price_batch_json_hash: &parts.price_batch_json_hash,
        price_manifest_line_hash: &parts.price_manifest_line_hash,
        price_bars_hash: &parts.price_bars_hash,
        price_file_count: parts.price_file_count,
        price_capture_contract_commit: &parts.price_capture_contract_commit,
        price_response_marker_evidence: &parts.price_response_marker_evidence,
        price_acquired_at: parts.price_acquired_at,
        action_batch_id: parts.action_batch_id,
        action_batch_json_hash: &parts.action_batch_json_hash,
        action_manifest_line_hash: &parts.action_manifest_line_hash,
        action_file_count: parts.action_file_count,
        cash_treatment: parts.cash_dividends.treatment_id(),
        cash_row_count: parts.cash_dividends.row_count(),
        cash_rows_hash: parts.cash_dividends.rows_sha256(),
        cash_acquired_at: parts.cash_dividends.acquired_at(),
        action_count: parts.action_count,
        instrument_count: parts.instruments.len(),
        instruments: &parts.instruments,
        session_count: parts.sessions.len(),
        sessions: &parts.sessions,
        bar_count: parts.bars.len(),
        bars: parts
            .bars
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
    };
    let bytes = serde_json::to_vec(&representation)
        .map_err(|_| HistoricalPriceOnlyV3Error::Serialization)?;
    Ok(ContentHash::from_bytes(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn synthetic_evidence() -> (
        HistoricalPriceOnlyV3PriceEvidence,
        HistoricalPriceOnlyV3ActionEvidence,
    ) {
        let start = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_RANGE_START).unwrap();
        let end = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_RANGE_END).unwrap();
        let sessions = crate::range_normalize::ExpectedRangeSessions::approved_xkrx(start, end)
            .unwrap()
            .sessions;
        assert_eq!(sessions.len(), HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT);
        let mut sorted_symbols = KR_ETF_CORE_SYMBOLS.to_vec();
        sorted_symbols.sort_unstable();
        let bars = sorted_symbols
            .iter()
            .flat_map(|symbol| {
                sessions.iter().map(move |session_date| {
                    json!({
                        "instrument_id": format!("{symbol}.KRX"),
                        "session_date": session_date,
                        "open": "10.00",
                        "high": "11.00",
                        "low": "9.00",
                        "close": "10.50",
                        "volume": 1,
                        "trading_value": "1.00",
                    })
                })
            })
            .collect::<Vec<_>>();
        let parsed_bars: Vec<RangeCanonicalBarCandidate> =
            serde_json::from_value(Value::Array(bars.clone())).unwrap();
        let bar_hash = ContentHash::from_bytes(&serde_json::to_vec(&parsed_bars).unwrap());
        let price = json!({
            "source_batch_id": HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID,
            "source_batch_json_sha256": HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256,
            "source_manifest_line_sha256": HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
            "capture_contract_commit": HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT,
            "response_marker_evidence": HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE,
            "range_start": HISTORICAL_PRICE_ONLY_V3_RANGE_START,
            "range_end": HISTORICAL_PRICE_ONLY_V3_RANGE_END,
            "vendor_snapshot": true,
            "strict_pit": false,
            "pit_policy": HISTORICAL_PRICE_ONLY_V3_PIT_POLICY,
            "files": (0..HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT).map(|index| json!({
                "symbol": KR_ETF_CORE_SYMBOLS[index % KR_ETF_CORE_SYMBOLS.len()],
                "window": index + 1,
                "page": 1,
                "file_name": format!("synthetic-{index}.json"),
                "content_hash": ContentHash::from_bytes(format!("file-{index}").as_bytes()),
                "size_bytes": 1,
                "endpoint": "synthetic",
                "tr_id": "synthetic",
                "request_continuation": "",
                "response_continuation": Value::Null,
                "query_start": HISTORICAL_PRICE_ONLY_V3_RANGE_START,
                "query_end": HISTORICAL_PRICE_ONLY_V3_RANGE_END,
            })).collect::<Vec<_>>(),
            "trading_dates": sessions,
            "bars": bars,
            "bars_sha256": bar_hash,
            "acquired_at": "2026-08-29T00:00:00Z",
        });
        let action = json!({
            "source_batch_id": HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID,
            "source_batch_json_sha256": HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256,
            "source_manifest_line_sha256": HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
            "vendor_snapshot": true,
            "strict_pit": false,
            "pit_policy": HISTORICAL_PRICE_ONLY_V3_PIT_POLICY,
            "files": (0..HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT).map(|index| json!({
                "symbol": KR_ETF_CORE_SYMBOLS[index % KR_ETF_CORE_SYMBOLS.len()],
                "logical_class": "dividend",
                "page": 1,
                "file_name": format!("synthetic-action-{index}.json"),
                "content_hash": ContentHash::from_bytes(format!("action-{index}").as_bytes()),
                "size_bytes": 1,
                "endpoint": "synthetic",
                "tr_id": "synthetic",
                "request_continuation": "",
                "response_continuation": "E",
            })).collect::<Vec<_>>(),
            "cash_dividends": {
                "treatment_id": HISTORICAL_PRICE_ONLY_V3_CASH_TREATMENT,
                "row_count": HISTORICAL_PRICE_ONLY_V3_CASH_ROW_COUNT,
                "rows_sha256": HISTORICAL_PRICE_ONLY_V3_CASH_ROWS_SHA256,
                "acquired_at": "2026-08-29T00:00:00Z",
            },
            "action_count": 0,
        });
        (
            serde_json::from_value(price).unwrap(),
            serde_json::from_value(action).unwrap(),
        )
    }

    #[test]
    fn materializes_exact_shape_and_copies_raw_to_adjusted() {
        let (price, action) = synthetic_evidence();
        let candidate = materialize_historical_price_only_v3(&price, &action).unwrap();
        assert_eq!(candidate.instrument_count(), 11);
        assert_eq!(
            candidate.session_count(),
            HISTORICAL_PRICE_ONLY_V3_SESSION_COUNT
        );
        assert_eq!(candidate.bar_count(), HISTORICAL_PRICE_ONLY_V3_BAR_COUNT);
        assert_eq!(
            candidate.price_file_count(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        );
        assert_eq!(
            candidate.action_file_count(),
            HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        );
        assert_eq!(candidate.action_count(), 0);
        assert_eq!(
            candidate.cash_treatment(),
            HISTORICAL_PRICE_ONLY_V3_CASH_TREATMENT
        );
        assert_eq!(candidate.metadata().audience.as_str(), "OWNER_ONLY");
        assert!(!candidate.metadata().strict_pit);
        assert!(!candidate.metadata().materialized);
        assert!(candidate.metadata().in_memory);
        assert!(!candidate.metadata().ready);
        assert!(
            candidate
                .bars()
                .iter()
                .all(|bar| bar.raw_open == bar.adjusted_open
                    && bar.raw_high == bar.adjusted_high
                    && bar.raw_low == bar.adjusted_low
                    && bar.raw_close == bar.adjusted_close)
        );
        assert_eq!(candidate.instruments()[0].to_string(), "069500.KRX");
        assert_eq!(candidate.instruments()[10].to_string(), "229200.KRX");
    }

    #[test]
    fn canonical_hash_ignores_allowed_evidence_order_changes() {
        let (price, action) = synthetic_evidence();
        let first = materialize_historical_price_only_v3(&price, &action).unwrap();
        let mut price_json = serde_json::to_value(&price).unwrap();
        price_json["bars"].as_array_mut().unwrap().reverse();
        price_json["trading_dates"]
            .as_array_mut()
            .unwrap()
            .reverse();
        price_json["files"].as_array_mut().unwrap().reverse();
        let mut action_json = serde_json::to_value(&action).unwrap();
        action_json["files"].as_array_mut().unwrap().reverse();
        let reordered_price: HistoricalPriceOnlyV3PriceEvidence =
            serde_json::from_value(price_json).unwrap();
        let reordered_action: HistoricalPriceOnlyV3ActionEvidence =
            serde_json::from_value(action_json).unwrap();
        let second =
            materialize_historical_price_only_v3(&reordered_price, &reordered_action).unwrap();
        assert_eq!(first.content_hash(), second.content_hash());
        assert_eq!(first.bars(), second.bars());
    }

    #[test]
    fn rejects_policy_and_count_mismatch_and_retains_cash_commitment() {
        let (price, action) = synthetic_evidence();
        let mut action_json = serde_json::to_value(&action).unwrap();
        action_json["pit_policy"] = json!("WRONG");
        let wrong_policy: HistoricalPriceOnlyV3ActionEvidence =
            serde_json::from_value(action_json).unwrap();
        assert!(matches!(
            materialize_historical_price_only_v3(&price, &wrong_policy),
            Err(HistoricalPriceOnlyV3Error::PolicyMismatch)
        ));

        let mut price_json = serde_json::to_value(&price).unwrap();
        price_json["bars"].as_array_mut().unwrap().pop();
        let wrong_count: HistoricalPriceOnlyV3PriceEvidence =
            serde_json::from_value(price_json).unwrap();
        assert!(matches!(
            materialize_historical_price_only_v3(&wrong_count, &action),
            Err(HistoricalPriceOnlyV3Error::InputMismatch { .. })
        ));

        let candidate = materialize_historical_price_only_v3(&price, &action).unwrap();
        assert_eq!(
            candidate.cash_row_count(),
            HISTORICAL_PRICE_ONLY_V3_CASH_ROW_COUNT
        );
        assert_eq!(
            candidate.cash_rows_hash(),
            action.cash_dividends().rows_sha256()
        );
        assert_eq!(
            candidate.cash_acquired_at(),
            action.cash_dividends().acquired_at()
        );
        assert!(!candidate.pit_policy().is_empty());
    }
}
