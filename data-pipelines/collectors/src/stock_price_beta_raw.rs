//! Isolated, operator-gated Raw capture for the fixed 30-stock price beta.
//!
//! This module is intentionally independent of the beta artifact/factor
//! modules. It owns only the KIS daily-bars request matrix and the immutable
//! Raw handoff. No response is made visible until every requested response has
//! passed validation and `RawStore` has committed/read back the complete batch.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use kis_client::MarketDataReply;
use market_data::contract::{FetchMode, MARKET_KR, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::providers::kis::KisRead;
use market_data::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};
use serde::Deserialize;
use uuid::Uuid;

pub const RAW_PROVIDER: &str = "kis-fixed-stock-price-beta-daily-bars-raw-v1";
pub const RAW_MARKET: &str = MARKET_KR;
pub const RAW_CONTRACT_VERSION: &str = "kr-stock-price-beta-raw-v1";
pub const RAW_INTERVAL: &str = "D";
pub const FIXED_RANGE_START: &str = "2025-08-04";
pub const FIXED_RANGE_END: &str = "2026-08-28";
pub const FIXED_UNIVERSE_FILE_SHA256: &str =
    "2a0d55143df0274fcfa357f2824ed752e2969469f93254ed7dfa64766a00dde1";
pub const ENTITLEMENT_FILE_SHA256: &str =
    "56bc018f748e2a1cfa78c4b94c18adccb2e0afd6a2d66fea4ecd3654db56b36e";
pub const ENTITLEMENT_ID: &str = "ent_kis_personal_owner_20260821";
pub const ENTITLEMENT_PROVIDER: &str = "kis";
pub const ENTITLEMENT_DOCUMENT_REFERENCE: &str =
    "repo://docs/decisions/0005-kis-personal-use-entitlement.md";
pub const ENTITLEMENT_CONTRACT_DOCUMENT_SHA256: &str =
    "5904a9c5ee00af734c762e877227761ab23391acf65220715f90d08b61947ea9";
pub const FIXED_SELECTION_BASIS: &str = "Owner-configured fixed observation list of established Korean equities. This is not a KOSPI 200, KOSDAQ 150, historical membership, or whole-market claim.";
pub const DAILY_BARS_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
pub const DAILY_BARS_TR_ID: &str = "FHKST03010100";
pub const FID_ORG_ADJ_PRC: &str = "1";
pub const MAX_WINDOWS_PER_SYMBOL: usize = 3;
pub const MAX_DAILY_BAR_OBSERVATIONS: usize = 100;
pub const FIXED_SYMBOL_COUNT: usize = 30;
pub const MAX_PLANNED_GETS: usize = FIXED_SYMBOL_COUNT * MAX_WINDOWS_PER_SYMBOL;
const BATCH_NAMESPACE: Uuid = Uuid::from_u128(0x7c6e3b6e6a4c5a6d8e3f2d1c0b9a8877);

/// The reviewed sequence is part of the universe contract and is used for
/// both request sequencing and deterministic validation.
pub const FIXED_STOCK_SYMBOLS: [&str; FIXED_SYMBOL_COUNT] = [
    "005930", "000660", "373220", "207940", "005380", "000270", "105560", "055550", "068270",
    "035420", "035720", "005490", "051910", "006400", "012330", "028260", "012450", "329180",
    "034020", "015760", "017670", "030200", "066570", "009150", "096770", "036570", "090430",
    "011200", "003490", "000810",
];

pub const FIXED_STOCK_NAMES: [&str; FIXED_SYMBOL_COUNT] = [
    "삼성전자",
    "SK하이닉스",
    "LG에너지솔루션",
    "삼성바이오로직스",
    "현대차",
    "기아",
    "KB금융",
    "신한지주",
    "셀트리온",
    "NAVER",
    "카카오",
    "POSCO홀딩스",
    "LG화학",
    "삼성SDI",
    "현대모비스",
    "삼성물산",
    "한화에어로스페이스",
    "HD현대중공업",
    "두산에너빌리티",
    "한국전력",
    "SK텔레콤",
    "KT",
    "LG전자",
    "삼성전기",
    "SK이노베이션",
    "엔씨소프트",
    "아모레퍼시픽",
    "HMM",
    "대한항공",
    "삼성화재",
];

/// Fixed, inclusive calendar windows. The newest window is always requested
/// first for every symbol; no provider cursor or response-derived boundary is
/// allowed to change this matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureWindow {
    pub id: &'static str,
    pub start: &'static str,
    pub end: &'static str,
}

pub const FIXED_CAPTURE_WINDOWS: [CaptureWindow; MAX_WINDOWS_PER_SYMBOL] = [
    CaptureWindow {
        id: "window-01",
        start: "2026-04-21",
        end: "2026-08-28",
    },
    CaptureWindow {
        id: "window-02",
        start: "2025-12-12",
        end: "2026-04-20",
    },
    CaptureWindow {
        id: "window-03",
        start: "2025-08-04",
        end: "2025-12-11",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockPriceBetaUniverse {
    ids: Vec<String>,
    file_sha256: ContentHash,
}

impl StockPriceBetaUniverse {
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn file_sha256(&self) -> &ContentHash {
        &self.file_sha256
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UniverseDocument {
    schema_id: String,
    schema_version: u32,
    universe_id: String,
    audience: String,
    capability: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    configured_at: String,
    effective_from: String,
    effective_until: Option<String>,
    market: String,
    venue: String,
    asset_class: String,
    selection_basis: String,
    instrument_count: usize,
    instruments: Vec<UniverseInstrument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UniverseInstrument {
    id: String,
    name: String,
}

/// Loads and validates the exact checked-in universe while hashing the source
/// bytes that were actually read. The hash, not a reconstructed JSON value,
/// participates in the Raw batch identity.
pub fn load_universe(path: &Path) -> Result<StockPriceBetaUniverse, CaptureError> {
    let bytes = fs::read(path).map_err(|_| CaptureError::InvalidConfiguration("universe"))?;
    parse_universe_bytes(&bytes)
}

pub fn parse_universe_bytes(bytes: &[u8]) -> Result<StockPriceBetaUniverse, CaptureError> {
    let file_sha256 = ContentHash::from_bytes(bytes);
    if file_sha256.as_str() != format!("sha256:{FIXED_UNIVERSE_FILE_SHA256}") {
        return Err(CaptureError::InvalidUniverse("universe bytes"));
    }
    let document: UniverseDocument = serde_json::from_slice(bytes)
        .map_err(|_| CaptureError::InvalidConfiguration("universe"))?;
    validate_universe_document(&document)?;
    let ids = document
        .instruments
        .into_iter()
        .map(|instrument| instrument.id)
        .collect();
    Ok(StockPriceBetaUniverse { ids, file_sha256 })
}

fn validate_universe_document(document: &UniverseDocument) -> Result<(), CaptureError> {
    if document.schema_id != "kr-stock-price-beta-universe"
        || document.schema_version != 1
        || document.universe_id != "kr-stock-price-beta-v1"
        || document.audience != "OWNER_ONLY"
        || document.capability != "PRICE_VOLUME_RESEARCH_ONLY"
        || !document.vendor_snapshot
        || document.strict_pit
        || document.configured_at != "2026-08-30"
        || document.effective_from != "2026-08-30"
        || document.effective_until.is_some()
        || document.market != "KR"
        || document.venue != "KRX"
        || document.asset_class != "EQUITY"
        || document.selection_basis != FIXED_SELECTION_BASIS
        || document.instrument_count != FIXED_SYMBOL_COUNT
        || document.instruments.len() != FIXED_SYMBOL_COUNT
    {
        return Err(CaptureError::InvalidUniverse("universe contract"));
    }

    let mut seen = BTreeSet::new();
    for (index, instrument) in document.instruments.iter().enumerate() {
        let Some(expected) = FIXED_STOCK_SYMBOLS.get(index) else {
            return Err(CaptureError::InvalidUniverse("universe count"));
        };
        let Some((symbol, venue)) = instrument.id.split_once('.') else {
            return Err(CaptureError::InvalidUniverse("instrument id"));
        };
        if venue != "KRX"
            || symbol != *expected
            || symbol.len() != 6
            || !symbol.bytes().all(|byte| byte.is_ascii_digit())
            || instrument.name != FIXED_STOCK_NAMES[index]
            || !seen.insert(instrument.id.as_str())
        {
            return Err(CaptureError::InvalidUniverse("instrument id"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementMetadata {
    pub file_sha256: ContentHash,
    pub entitlement_id: String,
    pub provider: String,
    pub document_reference: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitlementDocument {
    schema_version: u32,
    provider: String,
    entitlement_id: String,
    contract_document: EntitlementContractDocument,
    covered_datasets: Vec<String>,
    covered_uses: Vec<String>,
    covered_users: Vec<String>,
    effective_from: String,
    effective_until: String,
    lifecycle: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitlementContractDocument {
    document_hash: EntitlementDocumentHash,
    document_reference: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitlementDocumentHash {
    algorithm: String,
    hex: String,
}

pub fn load_entitlement(path: &Path) -> Result<EntitlementMetadata, CaptureError> {
    let bytes = fs::read(path).map_err(|_| CaptureError::InvalidConfiguration("entitlement"))?;
    parse_entitlement_bytes(&bytes)
}

pub fn parse_entitlement_bytes(bytes: &[u8]) -> Result<EntitlementMetadata, CaptureError> {
    let file_sha256 = ContentHash::from_bytes(bytes);
    if file_sha256.as_str() != format!("sha256:{ENTITLEMENT_FILE_SHA256}") {
        return Err(CaptureError::InvalidConfiguration("entitlement bytes"));
    }
    let document: EntitlementDocument = serde_json::from_slice(bytes)
        .map_err(|_| CaptureError::InvalidConfiguration("entitlement"))?;
    validate_entitlement_document(&document)?;
    Ok(EntitlementMetadata {
        file_sha256,
        entitlement_id: document.entitlement_id,
        provider: document.provider,
        document_reference: document.contract_document.document_reference,
    })
}

fn validate_entitlement_document(document: &EntitlementDocument) -> Result<(), CaptureError> {
    let has_dataset = document
        .covered_datasets
        .iter()
        .any(|dataset| dataset == "krx_eod_bars");
    let has_owner = document
        .covered_users
        .iter()
        .any(|user| user == "usr_owner");
    if document.schema_version != 1
        || document.provider != ENTITLEMENT_PROVIDER
        || document.entitlement_id != ENTITLEMENT_ID
        || document.lifecycle != "ACTIVE"
        || !has_dataset
        || !has_owner
        || document.covered_uses.is_empty()
        || document.contract_document.algorithm() != "SHA-256"
        || document.contract_document.document_hash.hex != ENTITLEMENT_CONTRACT_DOCUMENT_SHA256
        || document.contract_document.document_reference != ENTITLEMENT_DOCUMENT_REFERENCE
        || document.effective_from != "2016-08-29"
        || document.effective_until != "9999-12-31"
    {
        return Err(CaptureError::InvalidConfiguration("entitlement contract"));
    }
    Ok(())
}

impl EntitlementContractDocument {
    fn algorithm(&self) -> &str {
        &self.document_hash.algorithm
    }
}

pub fn validate_entitlement_binding(
    reference: &str,
    file_sha256: &ContentHash,
    entitlement: &EntitlementMetadata,
) -> Result<(), CaptureError> {
    if entitlement.entitlement_id != ENTITLEMENT_ID
        || entitlement.provider != ENTITLEMENT_PROVIDER
        || entitlement.document_reference != ENTITLEMENT_DOCUMENT_REFERENCE
        || reference != ENTITLEMENT_DOCUMENT_REFERENCE
        || file_sha256.as_str() != entitlement.file_sha256.as_str()
        || file_sha256.as_str() != format!("sha256:{ENTITLEMENT_FILE_SHA256}")
    {
        return Err(CaptureError::InvalidConfiguration("entitlement binding"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedCaptureWindow {
    spec: CaptureWindow,
    start: TradingDate,
    end: TradingDate,
}

fn parsed_fixed_capture_windows()
-> Result<[ParsedCaptureWindow; MAX_WINDOWS_PER_SYMBOL], CaptureError> {
    let parse = |spec: CaptureWindow| -> Result<ParsedCaptureWindow, CaptureError> {
        Ok(ParsedCaptureWindow {
            start: TradingDate::parse(spec.start)
                .map_err(|_| CaptureError::InvalidConfiguration("capture windows"))?,
            end: TradingDate::parse(spec.end)
                .map_err(|_| CaptureError::InvalidConfiguration("capture windows"))?,
            spec,
        })
    };
    let parsed = [
        parse(FIXED_CAPTURE_WINDOWS[0])?,
        parse(FIXED_CAPTURE_WINDOWS[1])?,
        parse(FIXED_CAPTURE_WINDOWS[2])?,
    ];
    for window in parsed {
        if window.start.checked_add_days(129).ok() != Some(window.end) {
            return Err(CaptureError::InvalidConfiguration("capture window length"));
        }
    }
    for pair in parsed.windows(2) {
        if pair[1].end.checked_add_days(1).ok() != Some(pair[0].start) {
            return Err(CaptureError::InvalidConfiguration("capture window overlap"));
        }
    }
    let fixed_start = TradingDate::parse(FIXED_RANGE_START)
        .map_err(|_| CaptureError::InvalidConfiguration("capture range"))?;
    let fixed_end = TradingDate::parse(FIXED_RANGE_END)
        .map_err(|_| CaptureError::InvalidConfiguration("capture range"))?;
    if parsed[0].end != fixed_end || parsed[parsed.len() - 1].start != fixed_start {
        return Err(CaptureError::InvalidConfiguration("capture range"));
    }
    Ok(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureIdentity {
    pub contract_version: String,
    pub universe_sha256: ContentHash,
    pub interval: String,
    pub start: TradingDate,
    pub end: TradingDate,
    pub entitlement_reference: String,
    pub entitlement_sha256: ContentHash,
    pub capture_commit: String,
}

impl CaptureIdentity {
    pub fn new(
        universe: &StockPriceBetaUniverse,
        start: TradingDate,
        end: TradingDate,
        entitlement_reference: String,
        entitlement_sha256: ContentHash,
        capture_commit: String,
    ) -> Result<Self, CaptureError> {
        let identity = Self {
            contract_version: RAW_CONTRACT_VERSION.to_owned(),
            universe_sha256: universe.file_sha256.clone(),
            interval: RAW_INTERVAL.to_owned(),
            start,
            end,
            entitlement_reference,
            entitlement_sha256,
            capture_commit,
        };
        identity.validate(universe)?;
        Ok(identity)
    }

    pub fn validate(&self, universe: &StockPriceBetaUniverse) -> Result<(), CaptureError> {
        let fixed_start = TradingDate::parse(FIXED_RANGE_START)
            .map_err(|_| CaptureError::InvalidConfiguration("capture range"))?;
        let fixed_end = TradingDate::parse(FIXED_RANGE_END)
            .map_err(|_| CaptureError::InvalidConfiguration("capture range"))?;
        if self.contract_version != RAW_CONTRACT_VERSION
            || self.interval != RAW_INTERVAL
            || self.universe_sha256 != universe.file_sha256
            || self.start != fixed_start
            || self.end != fixed_end
            || self.entitlement_reference != ENTITLEMENT_DOCUMENT_REFERENCE
            || self.entitlement_sha256.as_str() != format!("sha256:{ENTITLEMENT_FILE_SHA256}")
            || !is_lower_hex_commit(&self.capture_commit)
        {
            return Err(CaptureError::InvalidConfiguration("capture identity"));
        }
        Ok(())
    }

    /// UUIDv5 over a line-delimited canonical identity. All fields that can
    /// alter the meaning of the captured bytes are included; retrieval time is
    /// deliberately excluded so a retry cannot silently create a second
    /// identity for the same contract.
    pub fn batch_id(&self) -> BatchId {
        let material = format!(
            "contract_version={}\nuniverse_sha256={}\ninterval={}\nstart={}\nend={}\nentitlement_reference={}\nentitlement_sha256={}\ncapture_commit={}\n",
            self.contract_version,
            self.universe_sha256,
            self.interval,
            self.start.to_iso(),
            self.end.to_iso(),
            self.entitlement_reference,
            self.entitlement_sha256,
            self.capture_commit,
        );
        BatchId::from_uuid(Uuid::new_v5(&BATCH_NAMESPACE, material.as_bytes()))
    }
}

fn is_lower_hex_commit(value: &str) -> bool {
    value.len() == 40
        && value != "0000000000000000000000000000000000000000"
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Debug)]
pub enum CaptureError {
    InvalidConfiguration(&'static str),
    InvalidUniverse(&'static str),
    Provider(&'static str),
    Response(&'static str),
    WindowLimit,
    MissingObservation,
    RawStore(StoreError),
}

impl CaptureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidConfiguration(_) => "CONFIG_INVALID",
            Self::InvalidUniverse(_) => "UNIVERSE_INVALID",
            Self::Provider(code) => code,
            Self::Response(code) => code,
            Self::WindowLimit => "WINDOW_LIMIT",
            Self::MissingObservation => "OBSERVATION_MISSING",
            Self::RawStore(_) => "RAW_STORE_FAILURE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOutcome {
    pub batch_id: BatchId,
    pub entry: ManifestEntry,
    pub planned_gets: usize,
    pub actual_gets: usize,
}

#[derive(Debug, Clone)]
struct ValidatedPage {
    dates: Vec<TradingDate>,
}

/// Captures the fixed universe sequentially and commits exactly one Raw batch.
pub async fn capture_raw<R: KisRead>(
    store: &RawStore,
    reader: &R,
    universe: &StockPriceBetaUniverse,
    identity: &CaptureIdentity,
    retrieved_at: UtcTimestamp,
) -> Result<CaptureOutcome, CaptureError> {
    identity.validate(universe)?;
    if universe.ids.len() != FIXED_SYMBOL_COUNT
        || universe.ids.len() * FIXED_CAPTURE_WINDOWS.len() != MAX_PLANNED_GETS
    {
        return Err(CaptureError::InvalidUniverse("universe request ceiling"));
    }

    let windows = parsed_fixed_capture_windows()?;
    let batch_id = identity.batch_id();
    let mut envelopes = Vec::new();
    let mut actual_gets = 0usize;

    for symbol_id in &universe.ids {
        let symbol = symbol_id
            .strip_suffix(".KRX")
            .ok_or(CaptureError::InvalidUniverse("instrument id"))?;
        let mut seen_dates = BTreeSet::new();

        for window in windows {
            if actual_gets >= MAX_PLANNED_GETS {
                return Err(CaptureError::WindowLimit);
            }
            actual_gets += 1;

            let query = daily_bars_query(symbol, window.start, window.end);
            // `None` is the existing client's representation of the blank
            // initial continuation field. This endpoint never follows a
            // broker continuation marker.
            let reply = reader
                .get(DAILY_BARS_PATH, DAILY_BARS_TR_ID, &query, None)
                .await
                .map_err(|error| CaptureError::Provider(error.code()))?;
            let marker = reply.continuation.clone();
            if marker.as_deref().is_some_and(|value| !value.is_empty()) {
                return Err(CaptureError::Response("CONTINUATION_NONBLANK"));
            }
            let page = validate_daily_bars_response(&reply, symbol, window.start, window.end)?;
            for date in page.dates {
                if !seen_dates.insert(date) {
                    return Err(CaptureError::Response("OBSERVATION_OVERLAP"));
                }
            }

            let file_name = format!("daily-bars-{}-{symbol}-page-01.json", window.spec.id);
            let request = RequestMetadata {
                endpoint: DAILY_BARS_PATH.to_owned(),
                query,
                headers: vec![
                    ("authorization".to_owned(), "[REDACTED]".to_owned()),
                    ("appkey".to_owned(), "[REDACTED]".to_owned()),
                    ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                    ("tr_id".to_owned(), DAILY_BARS_TR_ID.to_owned()),
                    ("tr_cont".to_owned(), String::new()),
                ],
                mode: FetchMode::Credentialed,
            };
            envelopes.push(
                RawEnvelope::new(
                    batch_id,
                    ResponseKind::Bars,
                    file_name,
                    reply.body,
                    retrieved_at,
                    request,
                )
                .with_response_continuation(None),
            );
        }
        if seen_dates.is_empty() {
            return Err(CaptureError::MissingObservation);
        }
    }

    if actual_gets != MAX_PLANNED_GETS || envelopes.len() != MAX_PLANNED_GETS {
        return Err(CaptureError::MissingObservation);
    }

    let spec = BatchSpec {
        provider: RAW_PROVIDER,
        market: RAW_MARKET,
        date: &identity.start,
        batch_id,
        entitlement_reference: Some(&identity.entitlement_reference),
        mode: FetchMode::Credentialed,
    };
    let entry = store
        .store_batch(&spec, &envelopes)
        .map_err(CaptureError::RawStore)?;
    // RawStore verifies each immutable content hash during this readback. No
    // response body is returned from this helper after the commit boundary.
    store
        .read_batch_bytes(RAW_PROVIDER, RAW_MARKET, &entry)
        .map_err(CaptureError::RawStore)?;

    Ok(CaptureOutcome {
        batch_id,
        entry,
        planned_gets: MAX_PLANNED_GETS,
        actual_gets,
    })
}

fn daily_bars_query(symbol: &str, start: TradingDate, end: TradingDate) -> Vec<(String, String)> {
    vec![
        ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
        ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
        (
            "FID_INPUT_DATE_1".to_owned(),
            start.to_iso().replace('-', ""),
        ),
        ("FID_INPUT_DATE_2".to_owned(), end.to_iso().replace('-', "")),
        ("FID_PERIOD_DIV_CODE".to_owned(), RAW_INTERVAL.to_owned()),
        ("FID_ORG_ADJ_PRC".to_owned(), FID_ORG_ADJ_PRC.to_owned()),
    ]
}

fn validate_daily_bars_response(
    reply: &MarketDataReply,
    symbol: &str,
    start: TradingDate,
    end: TradingDate,
) -> Result<ValidatedPage, CaptureError> {
    let value: serde_json::Value = serde_json::from_slice(&reply.body)
        .map_err(|_| CaptureError::Response("RESPONSE_MALFORMED_JSON"))?;
    let object = value
        .as_object()
        .ok_or(CaptureError::Response("RESPONSE_SCHEMA_INVALID"))?;
    if object.get("rt_cd").and_then(serde_json::Value::as_str) != Some("0") {
        return Err(CaptureError::Response("RESPONSE_STATUS_INVALID"));
    }
    if body_has_nonblank_continuation(object) {
        return Err(CaptureError::Response("CONTINUATION_BODY_NONBLANK"));
    }
    let output1 = object
        .get("output1")
        .and_then(serde_json::Value::as_object)
        .ok_or(CaptureError::Response("RESPONSE_OUTPUT1_INVALID"))?;
    if output1
        .get("stck_shrn_iscd")
        .and_then(serde_json::Value::as_str)
        != Some(symbol)
    {
        return Err(CaptureError::Response("RESPONSE_SYMBOL_MISMATCH"));
    }
    let rows = object
        .get("output2")
        .and_then(serde_json::Value::as_array)
        .ok_or(CaptureError::Response("RESPONSE_OUTPUT2_INVALID"))?;
    if rows.is_empty() {
        return Err(CaptureError::MissingObservation);
    }
    if rows.len() >= MAX_DAILY_BAR_OBSERVATIONS {
        return Err(CaptureError::Response("RESPONSE_PAGE_TRUNCATED"));
    }

    let mut dates = Vec::with_capacity(rows.len());
    let mut previous = None;
    for row in rows {
        let row = row
            .as_object()
            .ok_or(CaptureError::Response("RESPONSE_ROW_INVALID"))?;
        let open = parse_positive_integer(row, "stck_oprc")?;
        let high = parse_positive_integer(row, "stck_hgpr")?;
        let low = parse_positive_integer(row, "stck_lwpr")?;
        let close = parse_positive_integer(row, "stck_clpr")?;
        parse_nonnegative_integer(row, "acml_vol")?;
        if low > high || open < low || open > high || close < low || close > high {
            return Err(CaptureError::Response("RESPONSE_OHLCV_RANGE"));
        }
        let date_text = row
            .get("stck_bsop_date")
            .and_then(serde_json::Value::as_str)
            .ok_or(CaptureError::Response("RESPONSE_DATE_MISSING"))?;
        let date = parse_kis_date(date_text)?;
        if date < start || date > end {
            return Err(CaptureError::Response("RESPONSE_DATE_OUT_OF_SCOPE"));
        }
        if let Some(prior) = previous {
            if date == prior {
                return Err(CaptureError::Response("OBSERVATION_OVERLAP"));
            }
            if date > prior {
                return Err(CaptureError::Response("RESPONSE_SEQUENCE_INVALID"));
            }
        }
        previous = Some(date);
        dates.push(date);
    }
    Ok(ValidatedPage { dates })
}

fn parse_positive_integer(
    row: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u64, CaptureError> {
    let value = row
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or(CaptureError::Response("RESPONSE_OHLCV_INVALID"))?
        .parse::<u64>()
        .map_err(|_| CaptureError::Response("RESPONSE_OHLCV_INVALID"))?;
    if value == 0 {
        return Err(CaptureError::Response("RESPONSE_OHLCV_NONPOSITIVE"));
    }
    Ok(value)
}

fn parse_nonnegative_integer(
    row: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u64, CaptureError> {
    row.get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or(CaptureError::Response("RESPONSE_OHLCV_INVALID"))?
        .parse::<u64>()
        .map_err(|_| CaptureError::Response("RESPONSE_OHLCV_INVALID"))
}

fn parse_kis_date(value: &str) -> Result<TradingDate, CaptureError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CaptureError::Response("RESPONSE_DATE_INVALID"));
    }
    TradingDate::parse(&format!(
        "{}-{}-{}",
        &value[..4],
        &value[4..6],
        &value[6..8]
    ))
    .map_err(|_| CaptureError::Response("RESPONSE_DATE_INVALID"))
}

fn body_has_nonblank_continuation(object: &serde_json::Map<String, serde_json::Value>) -> bool {
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
                serde_json::Value::Null => false,
                serde_json::Value::String(text) => !text.is_empty(),
                serde_json::Value::Bool(flag) => *flag,
                serde_json::Value::Number(number) => number.as_u64().is_none_or(|value| value != 0),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(map) => !map.is_empty(),
            }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use kis_client::KisError;
    use market_data::storage::RawStore;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[derive(Debug, Clone, Copy)]
    enum ResponseMode {
        OneRow,
        FullPage,
        HeaderMarker,
        BodyMarker,
        OutsideWindow,
        DuplicateRows,
        MissingOhlcv,
        ZeroOhlc,
        InvalidOhlcRange,
        NegativeVolume,
        NonNumericVolume,
        OverflowVolume,
        Malformed,
        Nonzero,
        WrongSymbol,
        Empty,
    }

    #[derive(Debug, Clone)]
    struct Call {
        symbol: String,
        path: String,
        tr_id: String,
        query: Vec<(String, String)>,
        continuation: Option<String>,
    }

    #[derive(Debug)]
    struct FixtureReader {
        calls: Arc<Mutex<Vec<Call>>>,
        mode: ResponseMode,
    }

    impl FixtureReader {
        fn new(mode: ResponseMode) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                mode,
            }
        }
    }

    impl KisRead for FixtureReader {
        async fn get(
            &self,
            path: &str,
            tr_id: &str,
            query: &[(String, String)],
            continuation: Option<&str>,
        ) -> Result<MarketDataReply, KisError> {
            let symbol = query_value(query, "FID_INPUT_ISCD").to_owned();
            let end = parse_query_date(query_value(query, "FID_INPUT_DATE_2"));
            {
                let mut calls = self.calls.lock().expect("calls lock");
                calls.push(Call {
                    symbol: symbol.clone(),
                    path: path.to_owned(),
                    tr_id: tr_id.to_owned(),
                    query: query.to_vec(),
                    continuation: continuation.map(str::to_owned),
                });
            }
            let body = match self.mode {
                ResponseMode::Malformed => b"not-json".to_vec(),
                ResponseMode::Nonzero => br#"{"rt_cd":"1","output1":{},"output2":[]}"#.to_vec(),
                ResponseMode::WrongSymbol => daily_body("999999", &[end], None),
                ResponseMode::Empty => daily_body(&symbol, &[], None),
                ResponseMode::BodyMarker => daily_body(&symbol, &[end], Some(("CTS", "cursor"))),
                ResponseMode::OutsideWindow => daily_body(
                    &symbol,
                    &[end.checked_add_days(1).expect("fixture date")],
                    None,
                ),
                ResponseMode::DuplicateRows => daily_body(&symbol, &[end, end], None),
                ResponseMode::MissingOhlcv => {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&daily_body(&symbol, &[end], None))
                            .expect("fixture body");
                    value["output2"][0]
                        .as_object_mut()
                        .expect("fixture row")
                        .remove("stck_clpr");
                    serde_json::to_vec(&value).expect("fixture JSON")
                }
                ResponseMode::ZeroOhlc => {
                    mutate_first_row(daily_body(&symbol, &[end], None), "stck_oprc", json!("0"))
                }
                ResponseMode::InvalidOhlcRange => {
                    mutate_first_row(daily_body(&symbol, &[end], None), "stck_lwpr", json!("102"))
                }
                ResponseMode::NegativeVolume => {
                    mutate_first_row(daily_body(&symbol, &[end], None), "acml_vol", json!("-1"))
                }
                ResponseMode::NonNumericVolume => mutate_first_row(
                    daily_body(&symbol, &[end], None),
                    "acml_vol",
                    json!("not-a-number"),
                ),
                ResponseMode::OverflowVolume => mutate_first_row(
                    daily_body(&symbol, &[end], None),
                    "acml_vol",
                    json!("18446744073709551616"),
                ),
                ResponseMode::FullPage => {
                    let count = 100;
                    let dates = (0..count)
                        .map(|offset| end.checked_add_days(-(offset as i64)).expect("date"))
                        .collect::<Vec<_>>();
                    daily_body(&symbol, &dates, None)
                }
                ResponseMode::OneRow | ResponseMode::HeaderMarker => {
                    daily_body(&symbol, &[end], None)
                }
            };
            let continuation = match self.mode {
                ResponseMode::HeaderMarker => Some("M".to_owned()),
                _ => None,
            };
            Ok(MarketDataReply { body, continuation })
        }
    }

    fn mutate_first_row(mut body: Vec<u8>, field: &str, value: serde_json::Value) -> Vec<u8> {
        let mut document: serde_json::Value = serde_json::from_slice(&body).expect("fixture body");
        document["output2"][0]
            .as_object_mut()
            .expect("fixture row")
            .insert(field.to_owned(), value);
        body = serde_json::to_vec(&document).expect("fixture JSON");
        body
    }

    fn query_value<'a>(query: &'a [(String, String)], name: &str) -> &'a str {
        query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .expect("query field")
    }

    fn parse_query_date(value: &str) -> TradingDate {
        TradingDate::parse(&format!(
            "{}-{}-{}",
            &value[..4],
            &value[4..6],
            &value[6..8]
        ))
        .expect("query date")
    }

    fn daily_body(symbol: &str, dates: &[TradingDate], marker: Option<(&str, &str)>) -> Vec<u8> {
        let rows = dates
            .iter()
            .map(|date| {
                json!({
                    "stck_bsop_date": date.to_iso().replace('-', ""),
                    "stck_clpr": "100",
                    "stck_oprc": "99",
                    "stck_hgpr": "101",
                    "stck_lwpr": "98",
                    "acml_vol": "100"
                })
            })
            .collect::<Vec<_>>();
        let mut value = json!({
            "rt_cd": "0",
            "output1": {"stck_shrn_iscd": symbol},
            "output2": rows
        });
        if let Some((key, value_to_insert)) = marker {
            value
                .as_object_mut()
                .expect("object")
                .insert(key.to_owned(), json!(value_to_insert));
        }
        serde_json::to_vec(&value).expect("fixture JSON")
    }

    fn checked_in_universe_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/universes/kr-stock-price-beta-v1.json")
    }

    fn checked_in_entitlement_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/data-rights/kis.entitlement.json")
    }

    fn checked_in_universe_bytes() -> Vec<u8> {
        fs::read(checked_in_universe_path()).expect("checked-in universe")
    }

    fn universe() -> StockPriceBetaUniverse {
        parse_universe_bytes(&checked_in_universe_bytes()).expect("valid universe")
    }

    fn entitlement() -> EntitlementMetadata {
        load_entitlement(&checked_in_entitlement_path()).expect("checked-in entitlement")
    }

    fn identity(universe: &StockPriceBetaUniverse) -> CaptureIdentity {
        let entitlement = entitlement();
        CaptureIdentity::new(
            universe,
            TradingDate::parse(FIXED_RANGE_START).expect("start"),
            TradingDate::parse(FIXED_RANGE_END).expect("end"),
            entitlement.document_reference,
            entitlement.file_sha256,
            COMMIT.to_owned(),
        )
        .expect("identity")
    }

    #[test]
    fn fixed_universe_has_exact_stable_thirty_symbol_sequence() {
        assert_eq!(FIXED_STOCK_SYMBOLS.len(), 30);
        assert_eq!(FIXED_STOCK_SYMBOLS[0], "005930");
        assert_eq!(FIXED_STOCK_SYMBOLS[29], "000810");
        assert!(FIXED_STOCK_SYMBOLS.iter().all(|symbol| {
            symbol.len() == 6 && symbol.bytes().all(|byte| byte.is_ascii_digit())
        }));
        let unique = FIXED_STOCK_SYMBOLS.iter().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 30);

        let document: UniverseDocument =
            serde_json::from_slice(&checked_in_universe_bytes()).expect("checked-in universe JSON");
        assert_eq!(document.selection_basis, FIXED_SELECTION_BASIS);
        assert!(document.selection_basis.contains("not a KOSPI 200"));
        assert!(document.selection_basis.contains("KOSDAQ 150"));
        assert!(document.selection_basis.contains("historical membership"));
        assert_eq!(
            document
                .instruments
                .iter()
                .map(|instrument| (instrument.id.clone(), instrument.name.clone()))
                .collect::<Vec<_>>(),
            FIXED_STOCK_SYMBOLS
                .iter()
                .zip(FIXED_STOCK_NAMES)
                .map(|(symbol, name)| (format!("{symbol}.KRX"), name))
                .map(|(id, name)| (id, name.to_owned()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn fixed_capture_windows_are_common_disjoint_and_130_calendar_days() {
        let windows = parsed_fixed_capture_windows().expect("fixed windows");
        assert_eq!(FIXED_CAPTURE_WINDOWS.len(), 3);
        assert_eq!(windows[0].spec.id, "window-01");
        assert_eq!(windows[1].spec.id, "window-02");
        assert_eq!(windows[2].spec.id, "window-03");
        for window in windows {
            assert_eq!(
                window.start.checked_add_days(129).expect("130-day window"),
                window.end
            );
        }
        for pair in windows.windows(2) {
            assert_eq!(
                pair[1].end.checked_add_days(1).expect("next day"),
                pair[0].start
            );
        }
        assert_eq!(windows[0].start.to_iso(), "2026-04-21");
        assert_eq!(windows[0].end.to_iso(), "2026-08-28");
        assert_eq!(windows[1].start.to_iso(), "2025-12-12");
        assert_eq!(windows[1].end.to_iso(), "2026-04-20");
        assert_eq!(windows[2].start.to_iso(), "2025-08-04");
        assert_eq!(windows[2].end.to_iso(), "2025-12-11");
    }

    #[test]
    fn checked_in_universe_file_matches_the_fixed_contract() {
        let loaded = load_universe(&checked_in_universe_path()).expect("checked-in fixed universe");
        assert_eq!(
            loaded.file_sha256().as_str(),
            &format!("sha256:{FIXED_UNIVERSE_FILE_SHA256}")
        );
        assert_eq!(
            loaded.ids,
            FIXED_STOCK_SYMBOLS
                .iter()
                .map(|symbol| format!("{symbol}.KRX"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn mutated_universe_bytes_fail_closed() {
        let original = checked_in_universe_bytes();
        let mut cases = Vec::new();

        let mut name = serde_json::from_slice::<serde_json::Value>(&original).expect("universe");
        name["instruments"][0]["name"] = json!("tampered");
        cases.push(serde_json::to_vec(&name).expect("name JSON"));

        let mut selection =
            serde_json::from_slice::<serde_json::Value>(&original).expect("universe");
        selection["selection_basis"] = json!("KOSPI 200 membership");
        cases.push(serde_json::to_vec(&selection).expect("selection JSON"));

        let mut reordered =
            serde_json::from_slice::<serde_json::Value>(&original).expect("universe");
        reordered["instruments"]
            .as_array_mut()
            .expect("instruments")
            .swap(0, 1);
        cases.push(serde_json::to_vec(&reordered).expect("reordered JSON"));

        let mut whitespace = original.clone();
        whitespace.insert(0, b'\n');
        cases.push(whitespace);

        let mut extra = original;
        extra.push(b' ');
        cases.push(extra);

        for bytes in cases {
            assert!(parse_universe_bytes(&bytes).is_err());
        }
    }

    #[test]
    fn invalid_and_duplicate_universes_fail_closed() {
        let mut document =
            serde_json::from_slice::<serde_json::Value>(&checked_in_universe_bytes())
                .expect("universe");
        document["instruments"].as_array_mut().expect("instruments")[1]["id"] = json!("005930.KRX");
        let bytes = serde_json::to_vec(&document).expect("universe JSON");
        assert!(matches!(
            parse_universe_bytes(&bytes),
            Err(CaptureError::InvalidUniverse(_))
        ));
    }

    #[test]
    fn checked_in_entitlement_file_is_active_and_covers_owner_eod_bars() {
        let entitlement = entitlement();
        assert_eq!(entitlement.entitlement_id, ENTITLEMENT_ID);
        assert_eq!(entitlement.provider, ENTITLEMENT_PROVIDER);
        assert_eq!(
            entitlement.file_sha256.as_str(),
            &format!("sha256:{ENTITLEMENT_FILE_SHA256}")
        );
        assert_eq!(
            entitlement.document_reference,
            ENTITLEMENT_DOCUMENT_REFERENCE
        );

        let document: EntitlementDocument =
            serde_json::from_slice(&fs::read(checked_in_entitlement_path()).expect("entitlement"))
                .expect("checked-in entitlement JSON");
        assert_eq!(document.provider, "kis");
        assert_eq!(document.entitlement_id, "ent_kis_personal_owner_20260821");
        assert_eq!(document.lifecycle, "ACTIVE");
        assert!(
            document
                .covered_datasets
                .iter()
                .any(|dataset| dataset == "krx_eod_bars")
        );
        assert!(
            document
                .covered_users
                .iter()
                .any(|user| user == "usr_owner")
        );
        assert_eq!(
            document.contract_document.document_reference,
            ENTITLEMENT_DOCUMENT_REFERENCE
        );
    }

    #[test]
    fn entitlement_bytes_and_binding_mismatches_fail_closed() {
        let original = fs::read(checked_in_entitlement_path()).expect("entitlement");
        let mut extra = original.clone();
        extra.push(b' ');
        assert!(parse_entitlement_bytes(&extra).is_err());

        let entitlement = entitlement();
        assert!(
            validate_entitlement_binding("repo://wrong", &entitlement.file_sha256, &entitlement)
                .is_err()
        );
        assert!(
            validate_entitlement_binding(
                ENTITLEMENT_DOCUMENT_REFERENCE,
                &ContentHash::from_bytes(b"wrong"),
                &entitlement
            )
            .is_err()
        );
    }

    #[test]
    fn deterministic_identity_changes_with_each_bound_input() {
        let universe = universe();
        let base = identity(&universe);
        let mut changed = base.clone();
        changed.capture_commit = "fedcba9876543210fedcba9876543210fedcba98".to_owned();
        assert_ne!(base.batch_id(), changed.batch_id());
        changed = base.clone();
        changed.entitlement_sha256 = ContentHash::from_bytes(b"other");
        assert_ne!(base.batch_id(), changed.batch_id());
        changed = base.clone();
        changed.end = TradingDate::parse("2026-08-27").expect("date");
        assert_ne!(base.batch_id(), changed.batch_id());
        changed = base.clone();
        changed.universe_sha256 = ContentHash::from_bytes(b"other-universe");
        assert_ne!(base.batch_id(), changed.batch_id());
    }

    #[tokio::test]
    async fn capture_is_sequential_exactly_shaped_and_read_back_from_raw() {
        let universe = universe();
        let identity = identity(&universe);
        let reader = FixtureReader::new(ResponseMode::OneRow);
        let calls = reader.calls.clone();
        let root = tempdir().expect("raw root");
        let store = RawStore::new(root.path());
        let outcome = capture_raw(
            &store,
            &reader,
            &universe,
            &identity,
            UtcTimestamp::parse_rfc3339("2026-08-30T00:00:00Z").expect("time"),
        )
        .await
        .expect("capture");
        assert_eq!(outcome.planned_gets, MAX_PLANNED_GETS);
        assert_eq!(outcome.actual_gets, MAX_PLANNED_GETS);
        assert_eq!(outcome.entry.provider, RAW_PROVIDER);
        assert_eq!(outcome.entry.market, RAW_MARKET);
        assert_eq!(outcome.entry.files.len(), MAX_PLANNED_GETS);

        let calls = calls.lock().expect("calls lock");
        assert_eq!(calls.len(), MAX_PLANNED_GETS);
        for (index, call) in calls.iter().enumerate() {
            let symbol_index = index / FIXED_CAPTURE_WINDOWS.len();
            let window_index = index % FIXED_CAPTURE_WINDOWS.len();
            let window = FIXED_CAPTURE_WINDOWS[window_index];
            assert_eq!(call.symbol, FIXED_STOCK_SYMBOLS[symbol_index]);
            assert_eq!(call.path, DAILY_BARS_PATH);
            assert_eq!(call.tr_id, DAILY_BARS_TR_ID);
            assert_eq!(call.continuation, None);
            assert_eq!(call.query.len(), 6);
            assert_eq!(query_value(&call.query, "FID_COND_MRKT_DIV_CODE"), "J");
            assert_eq!(query_value(&call.query, "FID_INPUT_ISCD"), call.symbol);
            assert_eq!(
                query_value(&call.query, "FID_INPUT_DATE_1"),
                window.start.replace('-', "")
            );
            assert_eq!(
                query_value(&call.query, "FID_INPUT_DATE_2"),
                window.end.replace('-', "")
            );
            assert_eq!(query_value(&call.query, "FID_PERIOD_DIV_CODE"), "D");
            assert_eq!(query_value(&call.query, "FID_ORG_ADJ_PRC"), "1");
        }

        for (index, file) in outcome.entry.files.iter().enumerate() {
            let symbol_index = index / FIXED_CAPTURE_WINDOWS.len();
            let window_index = index % FIXED_CAPTURE_WINDOWS.len();
            let window = FIXED_CAPTURE_WINDOWS[window_index];
            assert_eq!(
                file.file_name,
                format!(
                    "daily-bars-{}-{}-page-01.json",
                    window.id, FIXED_STOCK_SYMBOLS[symbol_index]
                )
            );
            assert_eq!(
                query_value(&file.request.query, "FID_INPUT_DATE_1"),
                window.start.replace('-', "")
            );
            assert_eq!(
                query_value(&file.request.query, "FID_INPUT_DATE_2"),
                window.end.replace('-', "")
            );
            assert_eq!(file.response_continuation, None);
        }

        const SECRET_SENTINEL: &str = "SENTINEL_KIS_APP_KEY_NEVER_PERSISTED";
        let batch_json = serde_json::to_vec(&outcome.entry).expect("batch metadata");
        assert!(
            !batch_json
                .windows(SECRET_SENTINEL.len())
                .any(|window| window == SECRET_SENTINEL.as_bytes())
        );
        for file in &outcome.entry.files {
            assert!(
                file.request
                    .headers
                    .iter()
                    .filter(|(name, _)| matches!(
                        name.as_str(),
                        "authorization" | "appkey" | "appsecret"
                    ))
                    .all(|(_, value)| value == "[REDACTED]")
            );
            assert!(
                !file
                    .request
                    .headers
                    .iter()
                    .any(|(_, value)| value == SECRET_SENTINEL)
            );
        }
        let manifest_path = root.path().join(format!(
            "raw/manifests/provider={RAW_PROVIDER}/market={RAW_MARKET}/manifest.jsonl"
        ));
        let manifest = fs::read(manifest_path).expect("manifest");
        assert!(
            !manifest
                .windows(SECRET_SENTINEL.len())
                .any(|window| window == SECRET_SENTINEL.as_bytes())
        );
    }

    #[tokio::test]
    async fn a_full_page_is_rejected_as_possible_truncation() {
        let universe = universe();
        let identity = identity(&universe);
        let reader = FixtureReader::new(ResponseMode::FullPage);
        let calls = reader.calls.clone();
        let root = tempdir().expect("raw root");
        let error = capture_raw(
            &RawStore::new(root.path()),
            &reader,
            &universe,
            &identity,
            UtcTimestamp::parse_rfc3339("2026-08-30T00:00:00Z").expect("time"),
        )
        .await
        .expect_err("a full page may be truncated");
        assert!(matches!(
            error,
            CaptureError::Response("RESPONSE_PAGE_TRUNCATED")
        ));
        assert_eq!(calls.lock().expect("calls lock").len(), 1);
        assert!(!root.path().join("raw").exists());
    }

    #[tokio::test]
    async fn malformed_status_symbol_empty_and_continuation_responses_fail_closed() {
        for (mode, expected_code) in [
            (ResponseMode::HeaderMarker, "CONTINUATION_NONBLANK"),
            (ResponseMode::BodyMarker, "CONTINUATION_BODY_NONBLANK"),
            (ResponseMode::OutsideWindow, "RESPONSE_DATE_OUT_OF_SCOPE"),
            (ResponseMode::DuplicateRows, "OBSERVATION_OVERLAP"),
            (ResponseMode::MissingOhlcv, "RESPONSE_OHLCV_INVALID"),
            (ResponseMode::ZeroOhlc, "RESPONSE_OHLCV_NONPOSITIVE"),
            (ResponseMode::InvalidOhlcRange, "RESPONSE_OHLCV_RANGE"),
            (ResponseMode::NegativeVolume, "RESPONSE_OHLCV_INVALID"),
            (ResponseMode::NonNumericVolume, "RESPONSE_OHLCV_INVALID"),
            (ResponseMode::OverflowVolume, "RESPONSE_OHLCV_INVALID"),
            (ResponseMode::Malformed, "RESPONSE_MALFORMED_JSON"),
            (ResponseMode::Nonzero, "RESPONSE_STATUS_INVALID"),
            (ResponseMode::WrongSymbol, "RESPONSE_SYMBOL_MISMATCH"),
            (ResponseMode::Empty, "OBSERVATION_MISSING"),
        ] {
            let universe = universe();
            let identity = identity(&universe);
            let reader = FixtureReader::new(mode);
            let root = tempdir().expect("raw root");
            let error = capture_raw(
                &RawStore::new(root.path()),
                &reader,
                &universe,
                &identity,
                UtcTimestamp::parse_rfc3339("2026-08-30T00:00:00Z").expect("time"),
            )
            .await
            .expect_err("invalid response");
            assert_eq!(error.code(), expected_code, "mode={mode:?}");
            assert!(!matches!(error, CaptureError::RawStore(_)), "mode={mode:?}");
            assert!(!root.path().join("raw").exists(), "mode={mode:?}");
        }
    }

    #[test]
    fn response_schema_price_integer_and_newest_first_guards_are_exact() {
        let start = TradingDate::parse("2026-04-21").expect("start");
        let end = TradingDate::parse("2026-08-28").expect("end");
        let base = daily_body("005930", &[end], None);

        let missing_output1 = br#"{"rt_cd":"0","output2":[]}"#.to_vec();
        let missing_output2 = br#"{"rt_cd":"0","output1":{"stck_shrn_iscd":"005930"}}"#.to_vec();
        for (body, expected) in [
            (missing_output1, "RESPONSE_OUTPUT1_INVALID"),
            (missing_output2, "RESPONSE_OUTPUT2_INVALID"),
            (
                mutate_first_row(base.clone(), "stck_oprc", json!("not-an-integer")),
                "RESPONSE_OHLCV_INVALID",
            ),
            (
                mutate_first_row(base.clone(), "stck_hgpr", json!("18446744073709551616")),
                "RESPONSE_OHLCV_INVALID",
            ),
        ] {
            let error = validate_daily_bars_response(
                &MarketDataReply {
                    body,
                    continuation: None,
                },
                "005930",
                start,
                end,
            )
            .expect_err("invalid response");
            assert_eq!(error.code(), expected);
        }

        let increasing = daily_body(
            "005930",
            &[end.checked_add_days(-1).expect("date"), end],
            None,
        );
        let error = validate_daily_bars_response(
            &MarketDataReply {
                body: increasing,
                continuation: None,
            },
            "005930",
            start,
            end,
        )
        .expect_err("increasing dates");
        assert_eq!(error.code(), "RESPONSE_SEQUENCE_INVALID");
    }
}
