//! V3 replay verifier for the immutable ETF11 KIS daily-price range batch.
//!
//! This module is an input-evidence boundary only.  It authenticates the
//! committed Raw bytes and parses the original-price OHLCV rows needed by a
//! later range normalizer.  The fixed ETF11 universe is a retrospective
//! basket for this verifier; the evidence makes no claim about historical
//! index membership or listing status.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use domain::{BatchId, ContentHash, FixedPoint, InstrumentId, TradingDate, UtcTimestamp, Venue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::contract::{FetchMode, ResponseKind, StoredFile};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::range_to_canonical::RangeCanonicalBarCandidate;
use crate::storage::{FileEntry, ManifestEntry};
use crate::{MARKET_KR, PROVIDER_KIS_DAILY_RANGE};

/// The one immutable Raw batch approved as V3 daily-price input.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID: &str = "d746ef9f-7eed-5333-97db-cb064331bd06";
/// The Raw date partition of [`HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID`].
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_DATE: &str = "2016-08-29";
/// Inclusive first date in the replayed historical price range.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START: &str = "2016-08-29";
/// Inclusive final date in the replayed historical price range.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END: &str = "2026-08-28";
/// Exact number of daily price response files in the approved batch.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT: usize = 275;
/// Exact number of range windows per fixed ETF11 symbol.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_WINDOW_COUNT: usize = 25;
/// Exact number of observed dates in the approved range.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT: usize = 2_452;
/// Exact number of ETF11 price bars in the approved range.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT: usize = 26_972;
/// Operator pin for the original immutable `batch.json` bytes.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256: &str =
    "sha256:1673cdc3f29ecd13cc5117ce15d1d2e26a22db4328fc8b49926608721a67d5e6";
/// Operator pin for the exact append-only manifest JSONL record, including
/// its terminating newline.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256: &str =
    "sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a";
/// Exact collector commit that captured the approved batch.  Its daily-range
/// contract rejects every non-empty response header marker and body cursor,
/// but predates persistence of the response marker in `FileEntry`.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT: &str =
    "23a01b49114943f93b3c8b240843d360c7485e94";
/// Honest marker provenance for this one pinned legacy batch.  No blank
/// response marker is fabricated during replay.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE: &str =
    "UNRECORDED_CAPTURE_CONTRACT_REJECTED_NONEMPTY_V1";
/// This evidence is a vendor snapshot, not strict point-in-time evidence.
pub const HISTORICAL_PRICE_ONLY_V3_PRICE_PIT_POLICY: &str = "PRICE_RETURN_ONLY";

const DAILY_BARS_ENDPOINT: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_BARS_TR_ID: &str = "FHKST03010100";
const MAX_DAILY_PAGE_ROWS: usize = 100;

/// Deterministically ordered replay evidence for one daily-price response
/// file.  The request continuation is represented as the blank header value
/// because that is the only page retained by this strict single-page source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPriceOnlyV3PriceFileEvidence {
    symbol: String,
    window: usize,
    page: usize,
    file_name: String,
    content_hash: ContentHash,
    size_bytes: u64,
    endpoint: String,
    tr_id: String,
    request_continuation: String,
    response_continuation: Option<String>,
    query_start: TradingDate,
    query_end: TradingDate,
}

impl HistoricalPriceOnlyV3PriceFileEvidence {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub const fn window(&self) -> usize {
        self.window
    }

    pub const fn page(&self) -> usize {
        self.page
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }

    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn tr_id(&self) -> &str {
        &self.tr_id
    }

    pub fn request_continuation(&self) -> &str {
        &self.request_continuation
    }

    /// The underlying provider call used no continuation argument.  The
    /// manifest stores that call as a blank `tr_cont` header.
    pub fn request_continuation_is_none(&self) -> bool {
        self.request_continuation.is_empty()
    }

    pub fn response_continuation(&self) -> Option<&str> {
        self.response_continuation.as_deref()
    }

    pub const fn query_start(&self) -> TradingDate {
        self.query_start
    }

    pub const fn query_end(&self) -> TradingDate {
        self.query_end
    }
}

/// Verified V3 daily-price evidence for the downstream range normalizer.
/// `trading_dates` are dates observed in the pinned KIS responses; they are
/// not a historical ETF membership or listing assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPriceOnlyV3PriceEvidence {
    source_batch_id: BatchId,
    source_batch_json_sha256: ContentHash,
    source_manifest_line_sha256: ContentHash,
    capture_contract_commit: String,
    response_marker_evidence: String,
    range_start: TradingDate,
    range_end: TradingDate,
    vendor_snapshot: bool,
    strict_pit: bool,
    pit_policy: String,
    files: Vec<HistoricalPriceOnlyV3PriceFileEvidence>,
    trading_dates: Vec<TradingDate>,
    bars: Vec<RangeCanonicalBarCandidate>,
    bars_sha256: ContentHash,
    acquired_at: UtcTimestamp,
}

impl HistoricalPriceOnlyV3PriceEvidence {
    pub const fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }

    pub fn source_batch_json_sha256(&self) -> &ContentHash {
        &self.source_batch_json_sha256
    }

    pub fn source_manifest_line_sha256(&self) -> &ContentHash {
        &self.source_manifest_line_sha256
    }

    pub fn capture_contract_commit(&self) -> &str {
        &self.capture_contract_commit
    }

    pub fn response_marker_evidence(&self) -> &str {
        &self.response_marker_evidence
    }

    /// Alias for callers that name the pins after their verifier arguments.
    pub fn batch_json_hash(&self) -> &ContentHash {
        &self.source_batch_json_sha256
    }

    /// Alias for callers that name the pins after their verifier arguments.
    pub fn manifest_line_hash(&self) -> &ContentHash {
        &self.source_manifest_line_sha256
    }

    pub const fn range_start(&self) -> TradingDate {
        self.range_start
    }

    pub const fn range_end(&self) -> TradingDate {
        self.range_end
    }

    pub const fn vendor_snapshot(&self) -> bool {
        self.vendor_snapshot
    }

    pub const fn strict_pit(&self) -> bool {
        self.strict_pit
    }

    pub fn pit_policy(&self) -> &str {
        &self.pit_policy
    }

    pub fn files(&self) -> &[HistoricalPriceOnlyV3PriceFileEvidence] {
        &self.files
    }

    pub fn trading_dates(&self) -> &[TradingDate] {
        &self.trading_dates
    }

    /// Alias emphasizing that this is observed session-date coverage only.
    pub fn sessions(&self) -> &[TradingDate] {
        &self.trading_dates
    }

    pub fn bars(&self) -> &[RangeCanonicalBarCandidate] {
        &self.bars
    }

    pub fn bars_sha256(&self) -> &ContentHash {
        &self.bars_sha256
    }

    pub const fn session_count(&self) -> usize {
        self.trading_dates.len()
    }

    pub const fn bar_count(&self) -> usize {
        self.bars.len()
    }

    pub const fn acquired_at(&self) -> UtcTimestamp {
        self.acquired_at
    }
}

/// Typed, fail-closed replay verification errors.  No variant carries a KIS
/// response body, broker message, or other free-form provider diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HistoricalPriceOnlyV3PriceError {
    #[error("V3 price batch scope or immutable identity differs from the approved input")]
    InvalidSource,
    #[error("V3 price batch does not contain the exact ETF11 x 25-window matrix")]
    IncompleteMatrix,
    #[error("V3 price batch file metadata is malformed")]
    InvalidFileMetadata,
    #[error("V3 price batch uses unsupported continuation metadata")]
    InvalidPagination,
    #[error("V3 price batch stored files do not exactly match their manifest metadata")]
    StoredFileMismatch,
    #[error("V3 price response is malformed or reports a non-success result")]
    InvalidResponse,
    #[error("V3 price response contains an explicit symbol conflict")]
    SymbolConflict,
    #[error("V3 price response contains a duplicate symbol/date observation")]
    DuplicateObservation,
    #[error("V3 price response does not provide the exact ETF11 date coverage")]
    InvalidCoverage,
}

/// Replays and verifies the exact V3 ETF11 KIS daily-price Raw input batch.
///
/// The supplied files are expected to be immutable Raw readback.  Their bytes
/// are independently re-hashed before any JSON is parsed.
pub fn verify_historical_price_only_v3_price_input(
    manifest: &ManifestEntry,
    stored: &[StoredFile],
    batch_json_hash: &ContentHash,
    manifest_line_hash: &ContentHash,
) -> Result<HistoricalPriceOnlyV3PriceEvidence, HistoricalPriceOnlyV3PriceError> {
    let batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID)
        .expect("checked-in V3 price batch id is valid");
    let partition = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_DATE)
        .expect("checked-in V3 price partition is valid");
    let range_start = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START)
        .expect("checked-in V3 price range start is valid");
    let range_end = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END)
        .expect("checked-in V3 price range end is valid");
    let expected_batch_json_hash =
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256)
            .expect("checked-in V3 price batch pin is valid");
    let expected_manifest_line_hash =
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256)
            .expect("checked-in V3 price manifest pin is valid");

    if manifest.batch_id != batch_id
        || manifest.provider != PROVIDER_KIS_DAILY_RANGE
        || manifest.market != MARKET_KR
        || manifest.mode != FetchMode::Credentialed
        || manifest.date != partition
        || manifest.files.len() != HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        || batch_json_hash != &expected_batch_json_hash
        || manifest_line_hash != &expected_manifest_line_hash
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidSource);
    }

    validate_stored_files(manifest, stored)?;

    let mut files = BTreeMap::<(String, usize), ParsedFile<'_>>::new();
    for file in &manifest.files {
        let parsed = parse_file_metadata(file, range_start, range_end)?;
        if files
            .insert((parsed.symbol.clone(), parsed.window), parsed)
            .is_some()
        {
            return Err(HistoricalPriceOnlyV3PriceError::IncompleteMatrix);
        }
    }

    let expected_groups = KR_ETF_CORE_SYMBOLS
        .iter()
        .flat_map(|symbol| {
            (1..=HISTORICAL_PRICE_ONLY_V3_PRICE_WINDOW_COUNT)
                .map(move |window| ((*symbol).to_owned(), window))
        })
        .collect::<BTreeSet<_>>();
    if files.keys().cloned().collect::<BTreeSet<_>>() != expected_groups {
        return Err(HistoricalPriceOnlyV3PriceError::IncompleteMatrix);
    }

    let stored_by_name = stored
        .iter()
        .map(|file| (file.file_name.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let mut file_evidence = Vec::with_capacity(manifest.files.len());
    let mut bars_by_key = BTreeMap::<(String, TradingDate), RangeCanonicalBarCandidate>::new();

    for symbol in KR_ETF_CORE_SYMBOLS {
        let mut previous_oldest: Option<TradingDate> = None;
        let mut previous_window_rows: Option<usize> = None;
        for window in 1..=HISTORICAL_PRICE_ONLY_V3_PRICE_WINDOW_COUNT {
            let parsed = files
                .get(&(symbol.to_owned(), window))
                .expect("the exact ETF11 x 25 matrix was checked above");
            let stored_file = stored_by_name
                .get(parsed.file.file_name.as_str())
                .expect("stored files were exactly verified before parsing");
            let parsed_rows = parse_daily_response(
                &stored_file.bytes,
                parsed.symbol.as_str(),
                parsed.query_start,
                parsed.query_end,
            )?;
            if let Some(previous_rows) = previous_window_rows {
                // The producer stops after a short page, so any nonfinal
                // window in an exact 25-window batch must be full.
                if previous_rows < MAX_DAILY_PAGE_ROWS {
                    return Err(HistoricalPriceOnlyV3PriceError::InvalidCoverage);
                }
            }
            if window == 1 {
                if parsed.query_end != range_end {
                    return Err(HistoricalPriceOnlyV3PriceError::InvalidCoverage);
                }
            } else if parsed.query_end
                != previous_oldest
                    .expect("every earlier window has parsed rows")
                    .previous_day()
            {
                return Err(HistoricalPriceOnlyV3PriceError::InvalidCoverage);
            }
            previous_oldest = Some(parsed_rows.oldest);
            previous_window_rows = Some(parsed_rows.rows.len());

            for bar in parsed_rows.rows {
                let key = (parsed.symbol.clone(), bar.session_date);
                if bars_by_key.insert(key, bar).is_some() {
                    return Err(HistoricalPriceOnlyV3PriceError::DuplicateObservation);
                }
            }

            file_evidence.push(HistoricalPriceOnlyV3PriceFileEvidence {
                symbol: parsed.symbol.clone(),
                window: parsed.window,
                page: parsed.page,
                file_name: parsed.file.file_name.clone(),
                content_hash: parsed.file.content_hash.clone(),
                size_bytes: parsed.file.size_bytes,
                endpoint: parsed.file.request.endpoint.clone(),
                tr_id: DAILY_BARS_TR_ID.to_owned(),
                request_continuation: String::new(),
                response_continuation: parsed.file.response_continuation.clone(),
                query_start: parsed.query_start,
                query_end: parsed.query_end,
            });
        }
        if previous_window_rows == Some(MAX_DAILY_PAGE_ROWS) && previous_oldest != Some(range_start)
        {
            return Err(HistoricalPriceOnlyV3PriceError::InvalidCoverage);
        }
    }

    let mut trading_dates = BTreeMap::<TradingDate, BTreeSet<String>>::new();
    for key in bars_by_key.keys() {
        trading_dates
            .entry(key.1)
            .or_default()
            .insert(key.0.clone());
    }
    let date_set = trading_dates.keys().copied().collect::<Vec<_>>();
    if date_set.len() != HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT
        || date_set.first().copied() != Some(range_start)
        || date_set.last().copied() != Some(range_end)
        || trading_dates
            .values()
            .any(|symbols| symbols.len() != KR_ETF_CORE_SYMBOLS.len())
        || bars_by_key.len() != HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidCoverage);
    }
    let expected_symbols = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| (*symbol).to_owned())
        .collect::<BTreeSet<_>>();
    if trading_dates
        .values()
        .any(|symbols| symbols != &expected_symbols)
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidCoverage);
    }

    let mut bars = bars_by_key.into_values().collect::<Vec<_>>();
    bars.sort_by(|left, right| {
        left.instrument_id
            .cmp(&right.instrument_id)
            .then(left.session_date.cmp(&right.session_date))
    });
    file_evidence.sort_by(|left, right| {
        (&left.symbol, left.window, left.page).cmp(&(&right.symbol, right.window, right.page))
    });
    let bars_sha256 = ContentHash::from_bytes(
        &serde_json::to_vec(&bars).expect("verified price bars are serializable"),
    );

    Ok(HistoricalPriceOnlyV3PriceEvidence {
        source_batch_id: batch_id,
        source_batch_json_sha256: batch_json_hash.clone(),
        source_manifest_line_sha256: manifest_line_hash.clone(),
        capture_contract_commit: HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT.to_owned(),
        response_marker_evidence: HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
            .to_owned(),
        range_start,
        range_end,
        vendor_snapshot: true,
        strict_pit: false,
        pit_policy: HISTORICAL_PRICE_ONLY_V3_PRICE_PIT_POLICY.to_owned(),
        files: file_evidence,
        trading_dates: date_set,
        bars,
        bars_sha256,
        acquired_at: manifest.retrieved_at,
    })
}

struct ParsedFile<'a> {
    file: &'a FileEntry,
    symbol: String,
    window: usize,
    page: usize,
    query_start: TradingDate,
    query_end: TradingDate,
}

struct ParsedRows {
    rows: Vec<RangeCanonicalBarCandidate>,
    oldest: TradingDate,
}

fn validate_stored_files(
    manifest: &ManifestEntry,
    stored: &[StoredFile],
) -> Result<(), HistoricalPriceOnlyV3PriceError> {
    if stored.len() != manifest.files.len() {
        return Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch);
    }
    let mut metadata_names = BTreeSet::new();
    let mut stored_names = BTreeSet::new();
    for file in &manifest.files {
        if !metadata_names.insert(file.file_name.as_str()) {
            return Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch);
        }
    }
    for file in stored {
        if !stored_names.insert(file.file_name.as_str()) {
            return Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch);
        }
        let Some(metadata) = manifest
            .files
            .iter()
            .find(|candidate| candidate.file_name == file.file_name)
        else {
            return Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch);
        };
        if u64::try_from(file.bytes.len()).ok() != Some(metadata.size_bytes)
            || ContentHash::from_bytes(&file.bytes) != metadata.content_hash
        {
            return Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch);
        }
    }
    if metadata_names != stored_names {
        return Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch);
    }
    Ok(())
}

fn parse_file_metadata<'a>(
    file: &'a FileEntry,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<ParsedFile<'a>, HistoricalPriceOnlyV3PriceError> {
    if file.kind != ResponseKind::Bars || file.request.mode != FetchMode::Credentialed {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    if file.request.endpoint != DAILY_BARS_ENDPOINT {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    // The exact approved d746... batch predates FileEntry marker persistence.
    // Its capture commit rejected every non-empty header/body cursor before
    // Raw visibility.  Require the legacy absence here and never rewrite it
    // into a claimed blank response marker.
    if file.response_continuation.is_some() {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidPagination);
    }
    if file.request.headers
        != [
            ("authorization".to_owned(), "[REDACTED]".to_owned()),
            ("appkey".to_owned(), "[REDACTED]".to_owned()),
            ("appsecret".to_owned(), "[REDACTED]".to_owned()),
            ("tr_id".to_owned(), DAILY_BARS_TR_ID.to_owned()),
            ("tr_cont".to_owned(), String::new()),
        ]
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }

    let keys = [
        "FID_COND_MRKT_DIV_CODE",
        "FID_INPUT_ISCD",
        "FID_INPUT_DATE_1",
        "FID_INPUT_DATE_2",
        "FID_PERIOD_DIV_CODE",
        "FID_ORG_ADJ_PRC",
    ];
    if file.request.query.len() != keys.len()
        || file
            .request
            .query
            .iter()
            .map(|(key, _)| key.as_str())
            .ne(keys)
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    if file.request.query[0].1 != "J"
        || file.request.query[4].1 != "D"
        || file.request.query[5].1 != "1"
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    let symbol = file.request.query[1].1.as_str();
    if !KR_ETF_CORE_SYMBOLS.contains(&symbol) {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    let query_start = parse_compact_date(&file.request.query[2].1)?;
    let query_end = parse_compact_date(&file.request.query[3].1)?;
    if query_start != range_start || query_end < query_start || query_end > range_end {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }

    let prefix = "daily-bars-range-window-";
    let suffix = format!("-{symbol}-page-01.json");
    let Some(window_text) = file
        .file_name
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(&suffix))
    else {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    };
    if window_text.is_empty()
        || !window_text.bytes().all(|byte| byte.is_ascii_digit())
        || (window_text.len() > 1 && window_text.starts_with('0'))
    {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    let window = window_text
        .parse::<usize>()
        .map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidFileMetadata)?;
    if !(1..=HISTORICAL_PRICE_ONLY_V3_PRICE_WINDOW_COUNT).contains(&window) {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    Ok(ParsedFile {
        file,
        symbol: symbol.to_owned(),
        window,
        page: 1,
        query_start,
        query_end,
    })
}

fn parse_compact_date(value: &str) -> Result<TradingDate, HistoricalPriceOnlyV3PriceError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata);
    }
    TradingDate::parse(&format!(
        "{}-{}-{}",
        &value[..4],
        &value[4..6],
        &value[6..8]
    ))
    .map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidFileMetadata)
}

fn parse_daily_response(
    bytes: &[u8],
    expected_symbol: &str,
    query_start: TradingDate,
    query_end: TradingDate,
) -> Result<ParsedRows, HistoricalPriceOnlyV3PriceError> {
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    let object = document
        .as_object()
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    if body_has_continuation_marker(bytes) {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidPagination);
    }
    if object.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidResponse);
    }
    let output1 = object
        .get("output1")
        .and_then(Value::as_object)
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    let returned_symbol = output1
        .get("stck_shrn_iscd")
        .and_then(Value::as_str)
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    if returned_symbol != expected_symbol {
        return Err(HistoricalPriceOnlyV3PriceError::SymbolConflict);
    }
    let output2 = object
        .get("output2")
        .and_then(Value::as_array)
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    if output2.is_empty() || output2.len() > MAX_DAILY_PAGE_ROWS {
        return Err(HistoricalPriceOnlyV3PriceError::InvalidResponse);
    }

    let mut rows = Vec::with_capacity(output2.len());
    let mut previous_date = None;
    for row in output2 {
        let row = row
            .as_object()
            .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
        validate_optional_row_symbol(row, expected_symbol)?;
        let date = parse_row_date(row)?;
        if date < query_start || date > query_end {
            return Err(HistoricalPriceOnlyV3PriceError::InvalidResponse);
        }
        if previous_date.is_some_and(|previous| date >= previous) {
            return Err(HistoricalPriceOnlyV3PriceError::InvalidResponse);
        }
        previous_date = Some(date);

        let open = parse_decimal(row, "stck_oprc")?;
        let high = parse_decimal(row, "stck_hgpr")?;
        let low = parse_decimal(row, "stck_lwpr")?;
        let close = parse_decimal(row, "stck_clpr")?;
        if !open.is_positive()
            || !high.is_positive()
            || !low.is_positive()
            || !close.is_positive()
            || high < open
            || high < close
            || low > open
            || low > close
            || low > high
        {
            return Err(HistoricalPriceOnlyV3PriceError::InvalidResponse);
        }
        let volume = parse_nonnegative_integer(row, "acml_vol")?;
        let trading_value = row
            .get("acml_tr_pbmn")
            .map(|_| parse_decimal(row, "acml_tr_pbmn"))
            .transpose()?
            .map(|value| {
                if value.is_negative() {
                    Err(HistoricalPriceOnlyV3PriceError::InvalidResponse)
                } else {
                    Ok(value)
                }
            })
            .transpose()?;
        let instrument_id = InstrumentId::from_parts(expected_symbol, Venue::Krx)
            .map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
        rows.push(RangeCanonicalBarCandidate {
            instrument_id,
            session_date: date,
            open,
            high,
            low,
            close,
            volume,
            trading_value,
        });
    }
    Ok(ParsedRows {
        oldest: previous_date.ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?,
        rows,
    })
}

fn validate_optional_row_symbol(
    row: &Map<String, Value>,
    expected_symbol: &str,
) -> Result<(), HistoricalPriceOnlyV3PriceError> {
    for key in ["stck_shrn_iscd", "sht_cd", "isu_srt_cd"] {
        if let Some(value) = row.get(key)
            && value.as_str() != Some(expected_symbol)
        {
            return Err(HistoricalPriceOnlyV3PriceError::SymbolConflict);
        }
    }
    Ok(())
}

fn parse_row_date(
    row: &Map<String, Value>,
) -> Result<TradingDate, HistoricalPriceOnlyV3PriceError> {
    let value = row
        .get("stck_bsop_date")
        .and_then(Value::as_str)
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    parse_compact_date(value).map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidResponse)
}

fn parse_decimal(
    row: &Map<String, Value>,
    field: &str,
) -> Result<FixedPoint, HistoricalPriceOnlyV3PriceError> {
    let raw = row
        .get(field)
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    let text = match raw {
        Value::String(value) if !value.is_empty() && value.trim() == value => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return Err(HistoricalPriceOnlyV3PriceError::InvalidResponse),
    };
    FixedPoint::parse(&text).map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidResponse)
}

fn parse_nonnegative_integer(
    row: &Map<String, Value>,
    field: &str,
) -> Result<u64, HistoricalPriceOnlyV3PriceError> {
    let raw = row
        .get(field)
        .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse)?;
    match raw {
        Value::String(value) if !value.is_empty() && value.trim() == value => value
            .parse::<u64>()
            .map_err(|_| HistoricalPriceOnlyV3PriceError::InvalidResponse),
        Value::Number(value) => value
            .as_u64()
            .ok_or(HistoricalPriceOnlyV3PriceError::InvalidResponse),
        _ => Err(HistoricalPriceOnlyV3PriceError::InvalidResponse),
    }
}

/// Mirror the producer's strict-single-page body check.  Missing markers and
/// explicit null/blank markers are terminal; a nonempty cursor-like field is
/// rejected before any response row is admitted.
fn body_has_continuation_marker(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return true;
    };
    let Some(object) = value.as_object() else {
        return true;
    };
    object.iter().any(|(key, value)| {
        let normalized = key.to_ascii_lowercase();
        let looks_like_cursor = normalized.contains("ctx")
            || normalized.contains("cts")
            || normalized.contains("continu")
            || normalized == "next"
            || normalized == "has_more"
            || normalized == "more";
        looks_like_cursor
            && match value {
                Value::Null => false,
                Value::String(value) => !value.is_empty(),
                _ => true,
            }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::RequestMetadata;
    use domain::BatchId;
    use serde_json::json;
    use std::path::PathBuf;

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("valid fixture date")
    }

    fn timestamp() -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339("2026-08-29T00:00:00Z").expect("valid timestamp")
    }

    fn batch_id() -> BatchId {
        BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID).expect("valid batch id")
    }

    fn batch_json_hash() -> ContentHash {
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256).expect("valid pin")
    }

    fn manifest_line_hash() -> ContentHash {
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256).expect("valid pin")
    }

    fn approved_sessions() -> Vec<TradingDate> {
        crate::range_normalize::ExpectedRangeSessions::approved_xkrx(
            date(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START),
            date(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END),
        )
        .expect("embedded approved XKRX dates")
        .sessions
    }

    fn compact(value: TradingDate) -> String {
        value.to_iso().replace('-', "")
    }

    fn query(symbol: &str, end: TradingDate) -> Vec<(String, String)> {
        vec![
            ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
            ("FID_INPUT_ISCD".into(), symbol.into()),
            (
                "FID_INPUT_DATE_1".into(),
                HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_START.replace('-', ""),
            ),
            ("FID_INPUT_DATE_2".into(), compact(end)),
            ("FID_PERIOD_DIV_CODE".into(), "D".into()),
            ("FID_ORG_ADJ_PRC".into(), "1".into()),
        ]
    }

    fn headers() -> Vec<(String, String)> {
        vec![
            ("authorization".into(), "[REDACTED]".into()),
            ("appkey".into(), "[REDACTED]".into()),
            ("appsecret".into(), "[REDACTED]".into()),
            ("tr_id".into(), DAILY_BARS_TR_ID.into()),
            ("tr_cont".into(), String::new()),
        ]
    }

    fn body(symbol: &str, rows: &[TradingDate]) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "rt_cd": "0",
            "msg_cd": "MCA00000",
            "msg1": "",
            "output1": {"stck_shrn_iscd": symbol},
            "output2": rows.iter().map(|date| json!({
                "stck_bsop_date": compact(*date),
                "stck_oprc": "99",
                "stck_hgpr": "101",
                "stck_lwpr": "98",
                "stck_clpr": "100",
                "acml_vol": "100",
                "acml_tr_pbmn": "10000"
            })).collect::<Vec<_>>()
        }))
        .expect("fixture JSON")
    }

    fn file(
        symbol: &str,
        window: usize,
        query_end: TradingDate,
        rows: &[TradingDate],
    ) -> (FileEntry, StoredFile) {
        let file_name = format!("daily-bars-range-window-{window}-{symbol}-page-01.json");
        let bytes = body(symbol, rows);
        let entry = FileEntry {
            kind: ResponseKind::Bars,
            file_name: file_name.clone(),
            content_hash: ContentHash::from_bytes(&bytes),
            size_bytes: bytes.len() as u64,
            request: RequestMetadata {
                endpoint: DAILY_BARS_ENDPOINT.into(),
                query: query(symbol, query_end),
                headers: headers(),
                mode: FetchMode::Credentialed,
            },
            response_continuation: None,
        };
        let stored = StoredFile {
            file_name,
            bytes,
            storage_path: PathBuf::from("fixture"),
        };
        (entry, stored)
    }

    fn fixture() -> (ManifestEntry, Vec<StoredFile>) {
        let sessions = approved_sessions();
        assert_eq!(sessions.len(), HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT);
        let batch = batch_id();
        let mut files = Vec::new();
        let mut stored = Vec::new();
        for symbol in KR_ETF_CORE_SYMBOLS {
            for window in 1..=HISTORICAL_PRICE_ONLY_V3_PRICE_WINDOW_COUNT {
                let end = sessions.len() - (window - 1) * MAX_DAILY_PAGE_ROWS;
                let start = end.saturating_sub(MAX_DAILY_PAGE_ROWS);
                let slice = &sessions[start..end];
                let rows = slice.iter().rev().copied().collect::<Vec<_>>();
                let query_end = if window == 1 {
                    date(HISTORICAL_PRICE_ONLY_V3_PRICE_RANGE_END)
                } else {
                    sessions
                        .get(end)
                        .expect("every non-final synthetic window has a next date")
                        .previous_day()
                };
                let (entry, raw) = file(symbol, window, query_end, &rows);
                files.push(entry);
                stored.push(raw);
            }
        }
        (
            ManifestEntry {
                batch_id: batch,
                provider: PROVIDER_KIS_DAILY_RANGE.into(),
                market: MARKET_KR.into(),
                date: date(HISTORICAL_PRICE_ONLY_V3_PRICE_DATE),
                retrieved_at: timestamp(),
                mode: FetchMode::Credentialed,
                entitlement_reference: None,
                files,
            },
            stored,
        )
    }

    fn verify(
        manifest: &ManifestEntry,
        stored: &[StoredFile],
    ) -> Result<HistoricalPriceOnlyV3PriceEvidence, HistoricalPriceOnlyV3PriceError> {
        verify_historical_price_only_v3_price_input(
            manifest,
            stored,
            &batch_json_hash(),
            &manifest_line_hash(),
        )
    }

    fn replace_body(
        manifest: &mut ManifestEntry,
        stored: &mut [StoredFile],
        index: usize,
        bytes: Vec<u8>,
    ) {
        manifest.files[index].content_hash = ContentHash::from_bytes(&bytes);
        manifest.files[index].size_bytes = bytes.len() as u64;
        stored[index].bytes = bytes;
    }

    #[test]
    fn accepts_exact_etf11_twenty_five_window_matrix_and_sorts_evidence() {
        let (manifest, stored) = fixture();
        let evidence = verify(&manifest, &stored).expect("valid exact fixture");
        assert_eq!(
            evidence.files().len(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_FILE_COUNT
        );
        assert_eq!(
            evidence.session_count(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_SESSION_COUNT
        );
        assert_eq!(
            evidence.bar_count(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_BAR_COUNT
        );
        assert_eq!(evidence.range_start(), date("2016-08-29"));
        assert_eq!(evidence.range_end(), date("2026-08-28"));
        assert!(evidence.vendor_snapshot());
        assert!(!evidence.strict_pit());
        assert_eq!(
            evidence.pit_policy(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_PIT_POLICY
        );
        assert_eq!(evidence.acquired_at(), manifest.retrieved_at);
        assert_eq!(
            evidence.capture_contract_commit(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_CAPTURE_COMMIT
        );
        assert_eq!(
            evidence.response_marker_evidence(),
            HISTORICAL_PRICE_ONLY_V3_PRICE_RESPONSE_MARKER_EVIDENCE
        );
        assert_eq!(evidence.trading_dates().first(), Some(&date("2016-08-29")));
        assert_eq!(evidence.trading_dates().last(), Some(&date("2026-08-28")));
        assert!(evidence.bars().windows(2).all(|pair| {
            (pair[0].instrument_id.clone(), pair[0].session_date)
                <= (pair[1].instrument_id.clone(), pair[1].session_date)
        }));
        assert!(
            evidence
                .files()
                .iter()
                .all(|file| file.request_continuation_is_none())
        );
        assert!(
            evidence
                .files()
                .iter()
                .all(|file| file.response_continuation().is_none())
        );
    }

    #[test]
    fn rejects_pin_scope_and_metadata_mismatches() {
        let (manifest, stored) = fixture();
        assert_eq!(
            verify_historical_price_only_v3_price_input(
                &manifest,
                &stored,
                &ContentHash::from_bytes(b"wrong"),
                &manifest_line_hash(),
            ),
            Err(HistoricalPriceOnlyV3PriceError::InvalidSource)
        );
        let mut manifest = manifest;
        manifest.provider = "kis".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidSource)
        );
    }

    #[test]
    fn rejects_headers_endpoint_tr_and_continuation() {
        let (mut manifest, stored) = fixture();
        manifest.files[0].request.endpoint = "/wrong".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0].request.headers[3].1 = "wrong".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0].request.headers[4].1 = "N".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0].response_continuation = Some(String::new());
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidPagination)
        );
    }

    #[test]
    fn rejects_hash_size_filename_and_matrix_breaks() {
        let (manifest, mut stored) = fixture();
        stored[0].bytes.push(b' ');
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0].size_bytes += 1;
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::StoredFileMismatch)
        );

        let (mut manifest, mut stored) = fixture();
        manifest.files[0].file_name = manifest.files[0].file_name.replace("page-01", "page-02");
        stored[0].file_name = manifest.files[0].file_name.clone();
        assert!(matches!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidFileMetadata
                | HistoricalPriceOnlyV3PriceError::IncompleteMatrix)
        ));

        let (mut manifest, stored) = fixture();
        manifest.files.pop();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidSource)
        );
    }

    #[test]
    fn rejects_duplicate_gap_symbol_date_schema_and_rt_cd() {
        let (mut manifest, mut stored) = fixture();
        let duplicate_date = approved_sessions()[100];
        let bytes = body("069500", &[duplicate_date]);
        replace_body(&mut manifest, &mut stored, 1, bytes);
        assert!(matches!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::DuplicateObservation
                | HistoricalPriceOnlyV3PriceError::InvalidCoverage
                | HistoricalPriceOnlyV3PriceError::InvalidResponse)
        ));

        let (mut manifest, mut stored) = fixture();
        let symbol = "102110";
        let bytes = body(symbol, &[date("2016-08-29")]);
        replace_body(&mut manifest, &mut stored, 0, bytes);
        assert!(matches!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::SymbolConflict
                | HistoricalPriceOnlyV3PriceError::InvalidCoverage)
        ));

        let (mut manifest, mut stored) = fixture();
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            br#"{"rt_cd":"1","output1":{},"output2":[]}"#.to_vec(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidResponse)
        );

        let (mut manifest, mut stored) = fixture();
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            br#"{"rt_cd":"0","output1":{},"output2":[]}"#.to_vec(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidResponse)
        );

        let (mut manifest, mut stored) = fixture();
        let mut value: Value = serde_json::from_slice(&stored[0].bytes).expect("fixture body");
        value["output2"][0]["stck_bsop_date"] = json!("20160828");
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            serde_json::to_vec(&value).unwrap(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidResponse)
        );
    }

    #[test]
    fn rejects_nonempty_body_cursor_and_numeric_or_ohlc_invalidity() {
        let (mut manifest, mut stored) = fixture();
        let mut value: Value = serde_json::from_slice(&stored[0].bytes).expect("fixture body");
        value["next"] = json!("cursor");
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            serde_json::to_vec(&value).unwrap(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidPagination)
        );

        let (mut manifest, mut stored) = fixture();
        let mut value: Value = serde_json::from_slice(&stored[0].bytes).expect("fixture body");
        value["output2"][0]["acml_vol"] = json!("1.5");
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            serde_json::to_vec(&value).unwrap(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidResponse)
        );

        let (mut manifest, mut stored) = fixture();
        let mut value: Value = serde_json::from_slice(&stored[0].bytes).expect("fixture body");
        value["output2"][0]["stck_hgpr"] = json!("90");
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            serde_json::to_vec(&value).unwrap(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3PriceError::InvalidResponse)
        );
    }
}
