//! V3 replay verifier for the immutable ETF11 KSD action-range batch.
//!
//! This is deliberately an input-evidence boundary only.  It does not
//! normalize, materialize, publish, or infer an ex-date/total-return series.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use domain::{BatchId, ContentHash, FixedPoint, TradingDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::contract::{FetchMode, ResponseKind, StoredFile};
use crate::normalize::validate_kis_action_row_fields;
use crate::providers::kis::{KIS_ACTION_SPECS, KR_ETF_CORE_SYMBOLS, KisActionSpec};
use crate::storage::{FileEntry, ManifestEntry};
use crate::{MARKET_KR, PROVIDER_KIS};

/// The one immutable Raw batch approved as V3 action input.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID: &str = "fbec8b5d-d87a-4d62-86fa-7af8ebce982b";
/// The Raw date partition of [`HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID`].
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_DATE: &str = "2016-08-29";
/// Inclusive KSD query start for the replayed historical action batch.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_RANGE_START: &str = "2016-08-29";
/// Inclusive KSD query end for the replayed historical action batch.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_RANGE_END: &str = "2026-08-28";
/// Exact expected ETF11-by-action-class matrix size.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT: usize = 77;
/// Operator pin for the original immutable `batch.json` bytes.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256: &str =
    "sha256:73a6c3e18b4cd90ea8aa2daa5a13a6c7572adc6ceed8cbe074e61bc6b5580cf2";
/// Operator pin for the exact append-only manifest JSONL record.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256: &str =
    "sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550";
/// This evidence is a vendor snapshot, not strict point-in-time evidence.
pub const HISTORICAL_PRICE_ONLY_V3_ACTION_PIT_POLICY: &str = "PRICE_RETURN_ONLY";
/// Exact treatment of validated cash-only dividend rows.
pub const HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT: &str =
    "CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1";

/// Deterministically ordered replay evidence for a single KSD response file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPriceOnlyV3ActionFileEvidence {
    symbol: String,
    logical_class: String,
    page: usize,
    file_name: String,
    content_hash: ContentHash,
    size_bytes: u64,
    endpoint: String,
    tr_id: String,
    request_continuation: String,
    response_continuation: String,
}

impl HistoricalPriceOnlyV3ActionFileEvidence {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }
    pub fn logical_class(&self) -> &str {
        &self.logical_class
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
    pub fn response_continuation(&self) -> &str {
        &self.response_continuation
    }
}

/// A content commitment for validated cash-only dividend rows.  It intentionally
/// contains neither payment output nor an invented ex-date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPriceOnlyV3CashDividendEvidence {
    treatment_id: String,
    row_count: usize,
    rows_sha256: ContentHash,
    acquired_at: domain::UtcTimestamp,
}

impl HistoricalPriceOnlyV3CashDividendEvidence {
    pub fn treatment_id(&self) -> &str {
        &self.treatment_id
    }
    pub const fn row_count(&self) -> usize {
        self.row_count
    }
    pub fn rows_sha256(&self) -> &ContentHash {
        &self.rows_sha256
    }
    pub const fn acquired_at(&self) -> domain::UtcTimestamp {
        self.acquired_at
    }
}

/// Verified V3 action evidence for the downstream price-return-only
/// materializer.  No action row becomes a total-return or ex-date claim here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPriceOnlyV3ActionEvidence {
    source_batch_id: BatchId,
    source_batch_json_sha256: ContentHash,
    source_manifest_line_sha256: ContentHash,
    vendor_snapshot: bool,
    strict_pit: bool,
    pit_policy: String,
    files: Vec<HistoricalPriceOnlyV3ActionFileEvidence>,
    cash_dividends: HistoricalPriceOnlyV3CashDividendEvidence,
    action_count: usize,
}

impl HistoricalPriceOnlyV3ActionEvidence {
    pub const fn source_batch_id(&self) -> BatchId {
        self.source_batch_id
    }
    pub fn source_batch_json_sha256(&self) -> &ContentHash {
        &self.source_batch_json_sha256
    }
    pub fn source_manifest_line_sha256(&self) -> &ContentHash {
        &self.source_manifest_line_sha256
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
    pub fn files(&self) -> &[HistoricalPriceOnlyV3ActionFileEvidence] {
        &self.files
    }
    pub fn cash_dividends(&self) -> &HistoricalPriceOnlyV3CashDividendEvidence {
        &self.cash_dividends
    }
    pub const fn action_count(&self) -> usize {
        self.action_count
    }
}

/// Typed, fail-closed replay verification errors.  No provider body or
/// diagnostic text is propagated into these errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HistoricalPriceOnlyV3ActionError {
    #[error("V3 action batch scope or immutable identity differs from the approved input")]
    InvalidSource,
    #[error("V3 action batch does not contain the exact ETF11 x seven-class matrix")]
    IncompleteMatrix,
    #[error("V3 action batch file metadata is malformed")]
    InvalidFileMetadata,
    #[error("V3 action batch page chain is incomplete or exceeds its bound")]
    InvalidPagination,
    #[error("V3 action batch stored files do not exactly match their manifest metadata")]
    StoredFileMismatch,
    #[error("V3 action response is malformed or reports a non-success result")]
    InvalidResponse,
    #[error("V3 action response contains an explicit symbol conflict")]
    SymbolConflict,
    #[error("V3 action response contains an unsupported {logical_class} action")]
    UnsupportedAction { logical_class: String },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct CashDividendRowCommitment {
    symbol: String,
    record_date: TradingDate,
    dividend_kind: String,
    face_value: FixedPoint,
    cash_dividend_amount: FixedPoint,
    cash_dividend_rate: FixedPoint,
    stock_dividend_rate: FixedPoint,
    cash_payment_date: Option<TradingDate>,
    stock_payment_date: Option<TradingDate>,
    odd_lot_payment_date: Option<TradingDate>,
    stock_kind: String,
    high_dividend_flag: String,
}

/// Replays and verifies the exact V3 ETF11 KSD Raw input batch.
///
/// The supplied files are expected to be the immutable Raw readback.  Their
/// bytes are independently re-hashed before any JSON is parsed.
pub fn verify_historical_price_only_v3_action_input(
    manifest: &ManifestEntry,
    stored: &[StoredFile],
    batch_json_hash: &ContentHash,
    manifest_line_hash: &ContentHash,
) -> Result<HistoricalPriceOnlyV3ActionEvidence, HistoricalPriceOnlyV3ActionError> {
    let batch_id = BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID)
        .expect("checked-in V3 action batch id is valid");
    let partition = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_DATE)
        .expect("checked-in V3 action partition is valid");
    let range_start = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_RANGE_START)
        .expect("checked-in V3 action range start is valid");
    let range_end = TradingDate::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_RANGE_END)
        .expect("checked-in V3 action range end is valid");
    let expected_batch_json_hash =
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256)
            .expect("checked-in V3 batch json pin is valid");
    let expected_manifest_line_hash =
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256)
            .expect("checked-in V3 manifest line pin is valid");
    if manifest.batch_id != batch_id
        || manifest.provider != PROVIDER_KIS
        || manifest.market != MARKET_KR
        || manifest.mode != FetchMode::Credentialed
        || manifest.date != partition
        || manifest.files.len() != HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        || batch_json_hash != &expected_batch_json_hash
        || manifest_line_hash != &expected_manifest_line_hash
    {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidSource);
    }

    validate_stored_files(manifest, stored)?;
    let mut groups: BTreeMap<(String, String), Vec<PageFile<'_>>> = BTreeMap::new();
    for file in &manifest.files {
        let parsed = parse_file_metadata(file, range_start, range_end)?;
        groups
            .entry((parsed.symbol.to_owned(), parsed.spec.kind.to_owned()))
            .or_default()
            .push(parsed);
    }
    let expected_groups = KIS_ACTION_SPECS
        .iter()
        .flat_map(|spec| {
            KR_ETF_CORE_SYMBOLS
                .iter()
                .map(move |symbol| ((*symbol).to_owned(), spec.kind.to_owned()))
        })
        .collect::<BTreeSet<_>>();
    if groups.len() != HISTORICAL_PRICE_ONLY_V3_ACTION_FILE_COUNT
        || groups.keys().cloned().collect::<BTreeSet<_>>() != expected_groups
    {
        return Err(HistoricalPriceOnlyV3ActionError::IncompleteMatrix);
    }

    let stored_by_name = stored
        .iter()
        .map(|file| (file.file_name.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let mut evidence = Vec::with_capacity(manifest.files.len());
    let mut cash_rows = Vec::new();
    for ((symbol, logical_class), mut pages) in groups {
        pages.sort_by_key(|page| page.page);
        validate_page_chain(&pages)?;
        for page in pages {
            let stored_file = stored_by_name
                .get(page.file.file_name.as_str())
                .expect("stored files were exactly verified before parsing");
            parse_and_classify(
                page.file,
                &stored_file.bytes,
                &symbol,
                logical_class.as_str(),
                range_start,
                range_end,
                &mut cash_rows,
            )?;
            evidence.push(HistoricalPriceOnlyV3ActionFileEvidence {
                symbol: symbol.clone(),
                logical_class: logical_class.clone(),
                page: page.page,
                file_name: page.file.file_name.clone(),
                content_hash: page.file.content_hash.clone(),
                size_bytes: page.file.size_bytes,
                endpoint: page.file.request.endpoint.clone(),
                tr_id: page.spec.tr_id.to_owned(),
                request_continuation: if page.page == 1 {
                    String::new()
                } else {
                    "N".to_owned()
                },
                response_continuation: page
                    .file
                    .response_continuation
                    .clone()
                    .expect("validated page continuation"),
            });
        }
    }
    evidence.sort_by(|left, right| {
        (&left.symbol, &left.logical_class, left.page).cmp(&(
            &right.symbol,
            &right.logical_class,
            right.page,
        ))
    });
    cash_rows.sort();
    if cash_rows.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidResponse);
    }
    let rows_sha256 = ContentHash::from_bytes(
        &serde_json::to_vec(&cash_rows).expect("cash commitment is serializable"),
    );
    Ok(HistoricalPriceOnlyV3ActionEvidence {
        source_batch_id: batch_id,
        source_batch_json_sha256: batch_json_hash.clone(),
        source_manifest_line_sha256: manifest_line_hash.clone(),
        vendor_snapshot: true,
        strict_pit: false,
        pit_policy: HISTORICAL_PRICE_ONLY_V3_ACTION_PIT_POLICY.to_owned(),
        files: evidence,
        cash_dividends: HistoricalPriceOnlyV3CashDividendEvidence {
            treatment_id: HISTORICAL_PRICE_ONLY_V3_CASH_DIVIDEND_TREATMENT.to_owned(),
            row_count: cash_rows.len(),
            rows_sha256,
            acquired_at: manifest.retrieved_at,
        },
        action_count: 0,
    })
}

struct PageFile<'a> {
    file: &'a FileEntry,
    spec: KisActionSpec,
    symbol: &'a str,
    page: usize,
}

fn validate_stored_files(
    manifest: &ManifestEntry,
    stored: &[StoredFile],
) -> Result<(), HistoricalPriceOnlyV3ActionError> {
    if stored.len() != manifest.files.len() {
        return Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch);
    }
    let mut metadata_names = BTreeSet::new();
    let mut stored_names = BTreeSet::new();
    for file in &manifest.files {
        if !metadata_names.insert(file.file_name.as_str()) {
            return Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch);
        }
    }
    for file in stored {
        if !stored_names.insert(file.file_name.as_str()) {
            return Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch);
        }
        let Some(metadata) = manifest
            .files
            .iter()
            .find(|candidate| candidate.file_name == file.file_name)
        else {
            return Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch);
        };
        if u64::try_from(file.bytes.len()).ok() != Some(metadata.size_bytes)
            || ContentHash::from_bytes(&file.bytes) != metadata.content_hash
        {
            return Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch);
        }
    }
    if metadata_names != stored_names {
        return Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch);
    }
    Ok(())
}

fn parse_file_metadata<'a>(
    file: &'a FileEntry,
    range_start: TradingDate,
    range_end: TradingDate,
) -> Result<PageFile<'a>, HistoricalPriceOnlyV3ActionError> {
    if file.kind != ResponseKind::CorporateActions || file.request.mode != FetchMode::Credentialed {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata);
    }
    let query = exact_query_map(&file.request.query)?;
    let matches = KIS_ACTION_SPECS
        .iter()
        .copied()
        .flat_map(|spec| {
            let query = query.clone();
            KR_ETF_CORE_SYMBOLS.iter().filter_map(move |symbol| {
                (file.request.endpoint == spec.path
                    && query == expected_query(spec, symbol, range_start, range_end))
                .then_some((spec, *symbol))
            })
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata);
    }
    let (spec, symbol) = matches[0];
    let prefix = format!("{}-{symbol}-page-", spec.label);
    let Some(page) = file
        .file_name
        .strip_prefix(&prefix)
        .and_then(|value| value.strip_suffix(".json"))
    else {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata);
    };
    if page.len() != 2 || !page.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata);
    }
    let page = page
        .parse::<usize>()
        .map_err(|_| HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)?;
    if !(1..=10).contains(&page) {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidPagination);
    }
    if !exact_headers(
        &file.request.headers,
        spec.tr_id,
        if page == 1 { "" } else { "N" },
    ) {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata);
    }
    Ok(PageFile {
        file,
        spec,
        symbol,
        page,
    })
}

fn validate_page_chain(pages: &[PageFile<'_>]) -> Result<(), HistoricalPriceOnlyV3ActionError> {
    if pages.is_empty() || pages.len() > 10 {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidPagination);
    }
    for (index, page) in pages.iter().enumerate() {
        if page.page != index + 1
            || !exact_headers(
                &page.file.request.headers,
                page.spec.tr_id,
                if page.page == 1 { "" } else { "N" },
            )
        {
            return Err(HistoricalPriceOnlyV3ActionError::InvalidPagination);
        }
        let terminal = index + 1 == pages.len();
        match (&page.file.response_continuation, terminal) {
            (Some(marker), false) if marker == "M" => {}
            (Some(marker), true) if marker != "M" => {}
            _ => return Err(HistoricalPriceOnlyV3ActionError::InvalidPagination),
        }
    }
    Ok(())
}

fn parse_and_classify(
    file: &FileEntry,
    bytes: &[u8],
    expected_symbol: &str,
    logical_class: &str,
    range_start: TradingDate,
    range_end: TradingDate,
    cash_rows: &mut Vec<CashDividendRowCommitment>,
) -> Result<(), HistoricalPriceOnlyV3ActionError> {
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|_| HistoricalPriceOnlyV3ActionError::InvalidResponse)?;
    let Some(object) = document.as_object() else {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidResponse);
    };
    if object.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidResponse);
    }
    let Some(rows) = object.get("output1").and_then(Value::as_array) else {
        return Err(HistoricalPriceOnlyV3ActionError::InvalidResponse);
    };
    for row in rows {
        let Some(row) = row.as_object() else {
            return Err(HistoricalPriceOnlyV3ActionError::InvalidResponse);
        };
        let validated =
            validate_kis_action_row_fields(&file.request.endpoint, &file.file_name, row)
                .map_err(|_| HistoricalPriceOnlyV3ActionError::InvalidResponse)?;
        if validated.symbol != expected_symbol {
            return Err(HistoricalPriceOnlyV3ActionError::SymbolConflict);
        }
        if validated.record_date < range_start || validated.record_date > range_end {
            return Err(HistoricalPriceOnlyV3ActionError::InvalidResponse);
        }
        if logical_class != "dividend" {
            return Err(HistoricalPriceOnlyV3ActionError::UnsupportedAction {
                logical_class: logical_class.to_owned(),
            });
        }
        let dividend = validated
            .dividend
            .ok_or(HistoricalPriceOnlyV3ActionError::InvalidResponse)?;
        if dividend.stock_dividend_rate > FixedPoint::ZERO {
            return Err(HistoricalPriceOnlyV3ActionError::UnsupportedAction {
                logical_class: logical_class.to_owned(),
            });
        }
        cash_rows.push(CashDividendRowCommitment {
            symbol: validated.symbol,
            record_date: dividend.record_date,
            dividend_kind: dividend.dividend_kind,
            face_value: dividend.face_value,
            cash_dividend_amount: dividend.cash_dividend_amount,
            cash_dividend_rate: dividend.cash_dividend_rate,
            stock_dividend_rate: dividend.stock_dividend_rate,
            cash_payment_date: dividend.cash_payment_date,
            stock_payment_date: dividend.stock_payment_date,
            odd_lot_payment_date: dividend.odd_lot_payment_date,
            stock_kind: dividend.stock_kind,
            high_dividend_flag: dividend.high_dividend_flag,
        });
    }
    Ok(())
}

fn expected_query(
    spec: KisActionSpec,
    symbol: &str,
    range_start: TradingDate,
    range_end: TradingDate,
) -> BTreeMap<String, String> {
    let mut query = BTreeMap::from([
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), range_start.to_iso().replace('-', "")),
        ("T_DT".to_owned(), range_end.to_iso().replace('-', "")),
        ("SHT_CD".to_owned(), symbol.to_owned()),
    ]);
    for (key, value) in spec.extra {
        query.insert((*key).to_owned(), (*value).to_owned());
    }
    query
}

fn exact_query_map(
    query: &[(String, String)],
) -> Result<BTreeMap<String, String>, HistoricalPriceOnlyV3ActionError> {
    let mut result = BTreeMap::new();
    for (key, value) in query {
        if result.insert(key.clone(), value.clone()).is_some() {
            return Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata);
        }
    }
    Ok(result)
}

fn exact_headers(headers: &[(String, String)], tr_id: &str, continuation: &str) -> bool {
    headers
        == [
            ("authorization".to_owned(), "[REDACTED]".to_owned()),
            ("appkey".to_owned(), "[REDACTED]".to_owned()),
            ("appsecret".to_owned(), "[REDACTED]".to_owned()),
            ("tr_id".to_owned(), tr_id.to_owned()),
            ("tr_cont".to_owned(), continuation.to_owned()),
        ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::UtcTimestamp;
    use serde_json::json;
    use std::path::PathBuf;

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn batch_id() -> BatchId {
        BatchId::from_str(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID).unwrap()
    }
    fn timestamp() -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339("2026-08-29T00:00:00Z").unwrap()
    }
    fn batch_json_hash() -> ContentHash {
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_JSON_SHA256).unwrap()
    }
    fn manifest_line_hash() -> ContentHash {
        ContentHash::parse(HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256).unwrap()
    }
    fn verify(
        manifest: &ManifestEntry,
        stored: &[StoredFile],
    ) -> Result<HistoricalPriceOnlyV3ActionEvidence, HistoricalPriceOnlyV3ActionError> {
        verify_historical_price_only_v3_action_input(
            manifest,
            stored,
            &batch_json_hash(),
            &manifest_line_hash(),
        )
    }
    fn body() -> Vec<u8> {
        br#"{"rt_cd":"0","output1":[]}"#.to_vec()
    }
    fn headers(spec: KisActionSpec, continuation: &str) -> Vec<(String, String)> {
        vec![
            ("authorization".into(), "[REDACTED]".into()),
            ("appkey".into(), "[REDACTED]".into()),
            ("appsecret".into(), "[REDACTED]".into()),
            ("tr_id".into(), spec.tr_id.into()),
            ("tr_cont".into(), continuation.into()),
        ]
    }
    fn file(
        spec: KisActionSpec,
        symbol: &str,
        page: usize,
        marker: Option<&str>,
        bytes: Vec<u8>,
    ) -> (FileEntry, StoredFile) {
        let file_name = format!("{}-{symbol}-page-{page:02}.json", spec.label);
        let entry = FileEntry {
            kind: ResponseKind::CorporateActions,
            file_name: file_name.clone(),
            content_hash: ContentHash::from_bytes(&bytes),
            size_bytes: bytes.len() as u64,
            request: crate::contract::RequestMetadata {
                endpoint: spec.path.into(),
                query: expected_query(
                    spec,
                    symbol,
                    date(HISTORICAL_PRICE_ONLY_V3_ACTION_RANGE_START),
                    date(HISTORICAL_PRICE_ONLY_V3_ACTION_RANGE_END),
                )
                .into_iter()
                .collect(),
                headers: headers(spec, if page == 1 { "" } else { "N" }),
                mode: FetchMode::Credentialed,
            },
            response_continuation: marker.map(str::to_owned),
        };
        let stored = StoredFile {
            file_name,
            bytes,
            storage_path: PathBuf::from("fixture"),
        };
        (entry, stored)
    }
    fn fixture() -> (ManifestEntry, Vec<StoredFile>) {
        let mut files = Vec::new();
        let mut stored = Vec::new();
        for spec in KIS_ACTION_SPECS {
            for symbol in KR_ETF_CORE_SYMBOLS {
                let (entry, bytes) = file(spec, symbol, 1, Some("E"), body());
                files.push(entry);
                stored.push(bytes);
            }
        }
        (
            ManifestEntry {
                batch_id: batch_id(),
                provider: PROVIDER_KIS.into(),
                market: MARKET_KR.into(),
                date: date(HISTORICAL_PRICE_ONLY_V3_ACTION_DATE),
                retrieved_at: timestamp(),
                mode: FetchMode::Credentialed,
                entitlement_reference: None,
                files,
            },
            stored,
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

    fn dividend_body(symbol: &str, stock_rate: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "rt_cd": "0",
            "output1": [{
                "sht_cd": symbol,
                "record_date": "20200829",
                "divi_kind": "cash",
                "face_val": "5000",
                "per_sto_divi_amt": "100",
                "divi_rate": "2.00",
                "stk_divi_rate": stock_rate,
                "divi_pay_dt": "20200901",
                "stk_div_pay_dt": if stock_rate == "0.00" { "" } else { "20200901" },
                "odd_pay_dt": "",
                "stk_kind": "cash",
                "high_divi_gb": "N"
            }]
        }))
        .unwrap()
    }
    #[test]
    fn accepts_the_complete_77_file_matrix_in_deterministic_order() {
        let (manifest, stored) = fixture();
        let verified = verify(&manifest, &stored).unwrap();
        assert_eq!(verified.files().len(), 77);
        assert_eq!(verified.action_count(), 0);
        assert!(verified.vendor_snapshot());
        assert!(!verified.strict_pit());
        assert_eq!(verified.cash_dividends().row_count(), 0);
        assert_eq!(
            verified.cash_dividends().acquired_at(),
            manifest.retrieved_at
        );
        assert_eq!(verified.source_batch_json_sha256(), &batch_json_hash());
        assert_eq!(
            verified.source_manifest_line_sha256(),
            &manifest_line_hash()
        );
        assert_eq!(verified.files()[0].endpoint(), KIS_ACTION_SPECS[2].path);
        assert_eq!(verified.files()[0].tr_id(), KIS_ACTION_SPECS[2].tr_id);
        assert_eq!(verified.files()[0].request_continuation(), "");
        assert_eq!(verified.files()[0].response_continuation(), "E");
        assert!(verified.files().windows(2).all(|pair| (
            pair[0].symbol(),
            pair[0].logical_class(),
            pair[0].page()
        ) <= (
            pair[1].symbol(),
            pair[1].logical_class(),
            pair[1].page()
        )));
    }
    #[test]
    fn rejects_missing_matrix_wrong_query_and_old_whole_market_shape() {
        let (mut manifest, stored) = fixture();
        manifest.files.pop();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidSource)
        );
        let (mut manifest, stored) = fixture();
        manifest.files[0].request.query[0].1 = "x".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );
        let (mut manifest, stored) = fixture();
        manifest.files.truncate(7);
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidSource)
        );
    }

    #[test]
    fn rejects_caller_hash_pins_that_do_not_match_the_approved_batch() {
        let (manifest, stored) = fixture();
        assert_eq!(
            verify_historical_price_only_v3_action_input(
                &manifest,
                &stored,
                &ContentHash::from_bytes(b"wrong-batch-json"),
                &manifest_line_hash(),
            ),
            Err(HistoricalPriceOnlyV3ActionError::InvalidSource)
        );
        assert_eq!(
            verify_historical_price_only_v3_action_input(
                &manifest,
                &stored,
                &batch_json_hash(),
                &ContentHash::from_bytes(b"wrong-manifest-line"),
            ),
            Err(HistoricalPriceOnlyV3ActionError::InvalidSource)
        );
    }

    #[test]
    fn rejects_nonproducer_header_vectors() {
        let (mut manifest, stored) = fixture();
        manifest.files[0].request.headers[0].1 = "Bearer not-redacted".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0].request.headers.pop();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0]
            .request
            .headers
            .push(("extra".into(), "value".into()));
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0]
            .request
            .headers
            .insert(3, ("tr_id".into(), KIS_ACTION_SPECS[0].tr_id.into()));
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );
    }
    #[test]
    fn rejects_hash_and_pagination_breaks() {
        let (manifest, mut stored) = fixture();
        stored[0].bytes.push(b' ');
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch)
        );
        let (mut manifest, stored) = fixture();
        manifest.files[0].response_continuation = None;
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidPagination)
        );
        let (mut manifest, stored) = fixture();
        manifest.files[0].response_continuation = Some("M".into());
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidPagination)
        );
    }

    #[test]
    fn rejects_duplicate_gap_over_bound_and_invalid_continuation_chains() {
        let (mut manifest, stored) = fixture();
        manifest.files[0].file_name = manifest.files[1].file_name.clone();
        assert!(matches!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::StoredFileMismatch)
        ));

        let (mut manifest, mut stored) = fixture();
        manifest.files[0].file_name = manifest.files[0].file_name.replace("page-01", "page-02");
        stored[0].file_name = manifest.files[0].file_name.clone();
        manifest.files[0].request.headers[4].1 = "N".into();
        assert!(matches!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::IncompleteMatrix)
                | Err(HistoricalPriceOnlyV3ActionError::InvalidPagination)
        ));

        let (mut manifest, mut stored) = fixture();
        manifest.files[0].file_name = manifest.files[0].file_name.replace("page-01", "page-11");
        stored[0].file_name = manifest.files[0].file_name.clone();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidPagination)
        );

        let (mut manifest, stored) = fixture();
        manifest.files[0].response_continuation = Some("E".into());
        manifest.files[0].request.headers[4].1 = "N".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );
    }

    #[test]
    fn accepts_m_then_terminal_page_chain_and_rejects_nonfinal_non_m() {
        let (entry_one, _stored_one) = file(
            KIS_ACTION_SPECS[0],
            KR_ETF_CORE_SYMBOLS[0],
            1,
            Some("M"),
            body(),
        );
        let (entry_two, _stored_two) = file(
            KIS_ACTION_SPECS[0],
            KR_ETF_CORE_SYMBOLS[0],
            2,
            Some("E"),
            body(),
        );
        let first = PageFile {
            file: &entry_one,
            spec: KIS_ACTION_SPECS[0],
            symbol: KR_ETF_CORE_SYMBOLS[0],
            page: 1,
        };
        let second = PageFile {
            file: &entry_two,
            spec: KIS_ACTION_SPECS[0],
            symbol: KR_ETF_CORE_SYMBOLS[0],
            page: 2,
        };
        assert!(validate_page_chain(&[first, second]).is_ok());
        let mut broken = entry_one;
        broken.response_continuation = Some("E".into());
        let first = PageFile {
            file: &broken,
            spec: KIS_ACTION_SPECS[0],
            symbol: KR_ETF_CORE_SYMBOLS[0],
            page: 1,
        };
        assert_eq!(
            validate_page_chain(&[
                first,
                PageFile {
                    file: &entry_two,
                    spec: KIS_ACTION_SPECS[0],
                    symbol: KR_ETF_CORE_SYMBOLS[0],
                    page: 2
                }
            ]),
            Err(HistoricalPriceOnlyV3ActionError::InvalidPagination)
        );
    }

    #[test]
    fn rejects_wrong_tr_nonzero_malformed_symbol_conflict_and_unsupported_actions() {
        let (mut manifest, stored) = fixture();
        manifest.files[0].request.headers[3].1 = "wrong".into();
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidFileMetadata)
        );

        let (mut manifest, mut stored) = fixture();
        replace_body(
            &mut manifest,
            &mut stored,
            0,
            br#"{"rt_cd":"1","output1":[]}"#.to_vec(),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidResponse)
        );

        let (mut manifest, mut stored) = fixture();
        replace_body(&mut manifest, &mut stored, 0, b"not-json".to_vec());
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::InvalidResponse)
        );

        let (mut manifest, mut stored) = fixture();
        let dividend = manifest
            .files
            .iter()
            .position(|file| file.file_name.contains("dividend"))
            .unwrap();
        replace_body(
            &mut manifest,
            &mut stored,
            dividend,
            dividend_body("102110", "0.00"),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::SymbolConflict)
        );

        let (mut manifest, mut stored) = fixture();
        let dividend = manifest
            .files
            .iter()
            .position(|file| file.file_name.contains("dividend"))
            .unwrap();
        let symbol = manifest.files[dividend]
            .request
            .query
            .iter()
            .find(|(key, _)| key == "SHT_CD")
            .unwrap()
            .1
            .clone();
        replace_body(
            &mut manifest,
            &mut stored,
            dividend,
            dividend_body(&symbol, "1.00"),
        );
        assert_eq!(
            verify(&manifest, &stored),
            Err(HistoricalPriceOnlyV3ActionError::UnsupportedAction {
                logical_class: "dividend".into()
            })
        );
    }

    #[test]
    fn cash_only_rows_are_order_independent_and_committed_without_total_return_data() {
        let (mut first_manifest, mut first_stored) = fixture();
        let dividends = first_manifest
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| file.file_name.contains("dividend").then_some(index))
            .collect::<Vec<_>>();
        for index in dividends {
            let symbol = first_manifest.files[index]
                .request
                .query
                .iter()
                .find(|(key, _)| key == "SHT_CD")
                .unwrap()
                .1
                .clone();
            replace_body(
                &mut first_manifest,
                &mut first_stored,
                index,
                dividend_body(&symbol, "0.00"),
            );
        }
        let first = verify(&first_manifest, &first_stored).unwrap();
        let mut second_manifest = first_manifest.clone();
        let mut second_stored = first_stored.clone();
        second_manifest.files.reverse();
        second_stored.reverse();
        let second = verify(&second_manifest, &second_stored).unwrap();
        assert_eq!(first.cash_dividends(), second.cash_dividends());
        assert_eq!(first.cash_dividends().row_count(), 11);
        assert_eq!(first.action_count(), 0);
        assert_eq!(
            first.pit_policy(),
            HISTORICAL_PRICE_ONLY_V3_ACTION_PIT_POLICY
        );
    }
}
