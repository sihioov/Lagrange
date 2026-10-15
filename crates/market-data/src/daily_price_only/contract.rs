//! Unapproved daily ETF11 JSON wire values and their canonical commitments.
//!
//! This module deliberately contains no Raw reader, registry, filesystem,
//! database, provider, or approval path. A self-consistent value is still only
//! a claim until an independent verifier checks its source bytes and lineage.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, Write};
use std::str::FromStr;

use chrono::SecondsFormat;
use domain::{BatchId, CodeCommit, ContentHash, InstrumentId, TradingDate, UtcTimestamp};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

const ETF11_SYMBOLS: [&str; 11] = [
    "069500", "102110", "114260", "132030", "133690", "143850", "148070", "153130", "192090",
    "195930", "229200",
];
const ETF11_INSTRUMENTS: [&str; 11] = [
    "069500.KRX",
    "102110.KRX",
    "114260.KRX",
    "132030.KRX",
    "133690.KRX",
    "143850.KRX",
    "148070.KRX",
    "153130.KRX",
    "192090.KRX",
    "195930.KRX",
    "229200.KRX",
];
const BASE_START: &str = "2016-08-29";
const BASE_FRONTIER: &str = "2026-08-28";
const POLICY_SCHEMA: &str = "owner-beta-daily-etf11-policy-v1";
const MANIFEST_SCHEMA: &str = "owner-beta-daily-etf11-input-v1";
const EVIDENCE_SCHEMA: &str = "owner-beta-daily-etf11-evidence-v1";
const NORMALIZATION_SCHEMA: &str = "owner-beta-daily-etf11-normalization-v1";
const NORMALIZED_SOURCE_SCHEMA: &str = "owner-beta-daily-normalized-source-v1";
const ADMISSION_SCHEMA: &str = "owner-beta-daily-etf11-admission-v1";
const NORMALIZER_VERSION: &str = "daily-raw-reconstruction-v1";
const MATERIALIZER_VERSION: &str = "daily-price-only-v1";
const BASE_KIND: &str = "historical_price_only_v3";
const DAILY_ENDPOINT_PREFIX: &str = "/uapi/domestic-stock/v1/";

type FileIdentityParts = (String, String, String, String);
type FileKeySortParts = (String, String, String, String, String);
type CashRowSortKey = (String, TradingDate, String, FileKeySortParts, Vec<u8>);
type ActionSortKey = (
    &'static str,
    &'static str,
    Option<String>,
    TradingDate,
    TradingDate,
    Option<FileKeySortParts>,
);

/// Finite, payload-free failures for the unapproved daily wire contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyWireErrorV1 {
    InvalidJson,
    DuplicateKey,
    UnknownField,
    MissingField,
    InvalidType,
    InvalidLiteral,
    InvalidHash,
    InvalidDate,
    InvalidTimestamp,
    InvalidUuid,
    InvalidCommit,
    InvalidDecimal,
    InvalidName,
    DuplicateEntry,
    InvalidOrder,
    LimitExceeded,
    InconsistentFields,
    HashMismatch,
}

impl fmt::Display for DailyWireErrorV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidJson => "InvalidJson",
            Self::DuplicateKey => "DuplicateKey",
            Self::UnknownField => "UnknownField",
            Self::MissingField => "MissingField",
            Self::InvalidType => "InvalidType",
            Self::InvalidLiteral => "InvalidLiteral",
            Self::InvalidHash => "InvalidHash",
            Self::InvalidDate => "InvalidDate",
            Self::InvalidTimestamp => "InvalidTimestamp",
            Self::InvalidUuid => "InvalidUuid",
            Self::InvalidCommit => "InvalidCommit",
            Self::InvalidDecimal => "InvalidDecimal",
            Self::InvalidName => "InvalidName",
            Self::DuplicateEntry => "DuplicateEntry",
            Self::InvalidOrder => "InvalidOrder",
            Self::LimitExceeded => "LimitExceeded",
            Self::InconsistentFields => "InconsistentFields",
            Self::HashMismatch => "HashMismatch",
        })
    }
}

impl std::error::Error for DailyWireErrorV1 {}

/// Nine-digit UTC timestamp scalar used only by this wire format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyWireTimestampV1(UtcTimestamp);

impl DailyWireTimestampV1 {
    /// Parses exactly `YYYY-MM-DDTHH:MM:SS.dddddddddZ` without normalization.
    pub fn parse(value: &str) -> Result<Self, DailyWireErrorV1> {
        let b = value.as_bytes();
        if b.len() != 30
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || b[19] != b'.'
            || b[29] != b'Z'
            || !b[..4].iter().all(u8::is_ascii_digit)
            || !b[5..7].iter().all(u8::is_ascii_digit)
            || !b[8..10].iter().all(u8::is_ascii_digit)
            || !b[11..13].iter().all(u8::is_ascii_digit)
            || !b[14..16].iter().all(u8::is_ascii_digit)
            || !b[17..19].iter().all(u8::is_ascii_digit)
            || !b[20..29].iter().all(u8::is_ascii_digit)
        {
            return Err(DailyWireErrorV1::InvalidTimestamp);
        }
        let year = value[..4]
            .parse::<u32>()
            .map_err(|_| DailyWireErrorV1::InvalidTimestamp)?;
        let second = value[17..19]
            .parse::<u8>()
            .map_err(|_| DailyWireErrorV1::InvalidTimestamp)?;
        if !(1..=9999).contains(&year) || second > 59 {
            return Err(DailyWireErrorV1::InvalidTimestamp);
        }
        let timestamp =
            UtcTimestamp::parse_rfc3339(value).map_err(|_| DailyWireErrorV1::InvalidTimestamp)?;
        Ok(Self(timestamp))
    }

    /// Returns the exact instant, retaining all nanoseconds internally.
    pub fn as_timestamp(&self) -> UtcTimestamp {
        self.0
    }
}

impl Serialize for DailyWireTimestampV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(
            &self
                .0
                .as_datetime()
                .to_rfc3339_opts(SecondsFormat::Nanos, true),
        )
    }
}

impl<'de> Deserialize<'de> for DailyWireTimestampV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $literal:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $literal)] $variant),+
        }
        impl $name {
            fn literal(self) -> &'static str {
                match self { $(Self::$variant => $literal),+ }
            }
        }
    };
}

wire_enum!(DailyEvidenceKindV1 {
    Bars => "bars",
    Reference => "reference",
    Calendar => "calendar",
    CorporateActions => "corporate_actions",
});
wire_enum!(DailyActionClassV1 {
    PaidinSubscription => "paidin-subscription",
    PaidinRecord => "paidin-record",
    BonusIssue => "bonus-issue",
    Dividend => "dividend",
    MergerSplit => "merger-split",
    ReverseSplit => "reverse-split",
    CapitalDecrease => "capital-decrease",
});
wire_enum!(DailyActionScopeV1 {
    WholeMarket => "whole_market",
    Etf11Symbol => "etf11_symbol",
});
wire_enum!(DailySessionTypeV1 {
    Trading => "TRADING",
    Closed => "CLOSED",
});

macro_rules! wire_struct {
    ($(#[$meta:meta])* $name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
    };
}

wire_struct!(DailyBasePinsV1Wire {
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    approval_registry_sha256: ContentHash,
});
wire_struct!(DailyBaseV1Wire {
    kind: String,
    pins: DailyBasePinsV1Wire,
});
wire_struct!(DailyPricePolicyV1Wire {
    schema: String,
    capability: String,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    base: DailyBaseV1Wire,
    instruments: Vec<InstrumentId>,
    price_semantics: String,
    cash_action_policy: String,
    normalizer_version: String,
    materializer_version: String,
    source_kinds: Vec<DailySourceKindV1Wire>,
    limits: DailyResourceLimitsV1Wire,
});
wire_struct!(DailySourceKindV1Wire {
    provider: String,
    market: String,
    kind: DailyEvidenceKindV1,
});
wire_struct!(DailyResourceLimitsV1Wire {
    max_policy_bytes: u64,
    max_manifest_bytes: u64,
    max_admission_bytes: u64,
    max_normalization_bytes: u64,
    max_bars_bytes: u64,
    max_sessions: u64,
    max_civil_dates_per_addition: u64,
    max_action_chains_per_session: u64,
    max_file_evidence_per_session: u64,
    max_source_file_bytes: u64,
    max_normalized_document_bytes: u64,
    max_copy_depth: u64,
    max_pages_per_action_chain: u64,
    max_cash_rows_per_chain: u64,
    max_json_depth: u64,
});
wire_struct!(DailyFileOriginV1Wire {
    provider: String,
    market: String,
    batch_id: BatchId,
    batch_date: TradingDate,
    batch_json_sha256: ContentHash,
    manifest_line_sha256: ContentHash,
    file_name: String,
    kind: DailyEvidenceKindV1,
    wire_sha256: ContentHash,
    size_bytes: u64,
    request: DailyRequestMetadataV1Wire,
    response_continuation: Option<String>,
    retrieved_at: DailyWireTimestampV1,
    retrieval_precision: String,
});
wire_struct!(DailyFileEvidenceV1Wire {
    provider: String,
    market: String,
    batch_id: BatchId,
    batch_date: TradingDate,
    batch_json_sha256: ContentHash,
    manifest_line_sha256: ContentHash,
    file_name: String,
    kind: DailyEvidenceKindV1,
    wire_sha256: ContentHash,
    size_bytes: u64,
    request: DailyRequestMetadataV1Wire,
    response_continuation: Option<String>,
    retrieved_at: DailyWireTimestampV1,
    retrieval_precision: String,
    original_retrieved_at: DailyWireTimestampV1,
    original_retrieval_precision: String,
    copied_from: Vec<DailyFileOriginV1Wire>,
});
wire_struct!(DailyFileKeyV1Wire {
    provider: String,
    market: String,
    batch_id: BatchId,
    file_name: String,
    wire_sha256: ContentHash,
});
wire_struct!(DailyRequestMetadataV1Wire {
    endpoint: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    mode: String,
});
wire_struct!(DailyActionChainV1Wire {
    logical_class: DailyActionClassV1,
    scope: DailyActionScopeV1,
    symbol: Option<String>,
    range_start: TradingDate,
    range_end: TradingDate,
    pages: Vec<DailyFileEvidenceV1Wire>,
    cash_dividends: Vec<DailyCashDividendRowV1Wire>,
    cash_dividend_rows_sha256: ContentHash,
});
wire_struct!(DailyCashDividendRowV1Wire {
    symbol: String,
    record_date: TradingDate,
    dividend_kind: String,
    face_value: String,
    cash_dividend_amount: String,
    cash_dividend_rate: String,
    stock_dividend_rate: String,
    cash_payment_date: Option<TradingDate>,
    stock_payment_date: Option<TradingDate>,
    odd_lot_payment_date: Option<TradingDate>,
    stock_kind: String,
    high_dividend_flag: String,
    source_file: DailyFileKeyV1Wire,
});
wire_struct!(DailyCalendarBindingV1Wire {
    civil_date: TradingDate,
    session_type: DailySessionTypeV1,
    exchange: String,
    timezone: String,
    source_file: DailyFileEvidenceV1Wire,
    normalized_document_sha256: ContentHash,
    legacy_projection: Option<DailyLegacyCalendarProjectionV1Wire>,
});
wire_struct!(DailyLegacyCalendarProjectionV1Wire {
    source: String,
    source_version: String,
    source_batch_id: BatchId,
    source_file_name: String,
    document_sha256: ContentHash,
    retrieved_at: DailyWireTimestampV1,
});
wire_struct!(DailyNormalizedDocumentV1Wire {
    kind: DailyEvidenceKindV1,
    session_date: TradingDate,
    normalizer_version: String,
    sha256: ContentHash,
    size_bytes: u64,
    row_count: u64,
    source_files: Vec<DailyFileKeyV1Wire>,
    source_evidence_sha256: ContentHash,
});
wire_struct!(DailyNormalizedSourceCommitmentV1Wire {
    schema: String,
    kind: DailyEvidenceKindV1,
    session_date: TradingDate,
    source_files: Vec<DailyFileKeyV1Wire>,
});
wire_struct!(DailySessionEvidenceV1Wire {
    session_date: TradingDate,
    previous_frontier: TradingDate,
    instruments: Vec<InstrumentId>,
    bars: Vec<DailyFileEvidenceV1Wire>,
    reference: Vec<DailyFileEvidenceV1Wire>,
    calendar: Vec<DailyCalendarBindingV1Wire>,
    actions: Vec<DailyActionChainV1Wire>,
    normalized_documents: Vec<DailyNormalizedDocumentV1Wire>,
});
wire_struct!(DailyBarsDescriptorV1Wire {
    file: String,
    sha256: ContentHash,
    size_bytes: u64,
    row_count: u64,
});
wire_struct!(DailyNormalizationDescriptorV1Wire {
    file: String,
    sha256: ContentHash,
    size_bytes: u64,
    row_count: u64,
});
wire_struct!(DailyPriceManifestV1Wire {
    schema: String,
    capability: String,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    base: DailyBaseV1Wire,
    parent_admission_sha256: Option<ContentHash>,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<InstrumentId>,
    sessions: Vec<TradingDate>,
    daily_sources: Vec<DailySessionEvidenceV1Wire>,
    bars: DailyBarsDescriptorV1Wire,
    normalization: DailyNormalizationDescriptorV1Wire,
    policy_sha256: ContentHash,
    materializer_version: String,
    code_commit: CodeCommit,
    content_sha256: ContentHash,
});
wire_struct!(DailyPriceManifestDraftV1Wire {
    schema: String,
    capability: String,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    base: DailyBaseV1Wire,
    parent_admission_sha256: Option<ContentHash>,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<InstrumentId>,
    sessions: Vec<TradingDate>,
    daily_sources: Vec<DailySessionEvidenceV1Wire>,
    bars: DailyBarsDescriptorV1Wire,
    normalization: DailyNormalizationDescriptorV1Wire,
    policy_sha256: ContentHash,
    materializer_version: String,
    code_commit: CodeCommit,
});
wire_struct!(DailyEvidenceCommitmentV1Wire {
    schema: String,
    daily_sources: Vec<DailySessionEvidenceV1Wire>,
});
wire_struct!(DailyNormalizationProofV1Wire {
    schema: String,
    normalizer_version: String,
    evidence_sha256: ContentHash,
    documents: Vec<DailyNormalizedDocumentV1Wire>,
});
wire_struct!(DailyAdmissionCountsV1Wire {
    instrument_count: u64,
    session_count: u64,
    bar_count: u64,
    added_session_count: u64,
    source_file_count: u64,
    calendar_row_count: u64,
    action_chain_count: u64,
    action_page_count: u64,
    cash_dividend_row_count: u64,
    normalized_document_count: u64,
});
wire_struct!(DailyPriceAdmissionV1Wire {
    schema: String,
    content_sha256: ContentHash,
    manifest_sha256: ContentHash,
    policy_sha256: ContentHash,
    parent_admission_sha256: Option<ContentHash>,
    base: DailyBaseV1Wire,
    verifier_code_commit: CodeCommit,
    normalizer_version: String,
    evidence_sha256: ContentHash,
    counts: DailyAdmissionCountsV1Wire,
});
wire_struct!(DailyPricePinsV1Wire {
    content_sha256: ContentHash,
    manifest_sha256: ContentHash,
    admission_sha256: ContentHash,
    policy_sha256: ContentHash,
});

// The scanner runs before serde_json::Value exists. It detects decoded key
// collisions (including escaped aliases), enforces JSON depth and canonical
// unsigned numeric tokens, and lets serde perform typed decoding only after
// the full byte stream has been accepted as unambiguous JSON.
struct JsonScanner<'a> {
    bytes: &'a [u8],
    cursor: usize,
    duplicate_key: bool,
    invalid_number: bool,
    overlarge_array: bool,
}

impl<'a> JsonScanner<'a> {
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|b| matches!(*b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.cursor += 1;
        }
    }

    fn string(&mut self) -> Result<String, DailyWireErrorV1> {
        let start = self.cursor;
        if self.bytes.get(self.cursor) != Some(&b'"') {
            return Err(DailyWireErrorV1::InvalidJson);
        }
        self.cursor += 1;
        while let Some(&b) = self.bytes.get(self.cursor) {
            match b {
                b'"' => {
                    self.cursor += 1;
                    return serde_json::from_slice(&self.bytes[start..self.cursor])
                        .map_err(|_| DailyWireErrorV1::InvalidJson);
                }
                b'\\' => {
                    self.cursor += 1;
                    match self.bytes.get(self.cursor).copied() {
                        Some(b'u') => {
                            self.cursor += 1;
                            for _ in 0..4 {
                                if !self
                                    .bytes
                                    .get(self.cursor)
                                    .is_some_and(u8::is_ascii_hexdigit)
                                {
                                    return Err(DailyWireErrorV1::InvalidJson);
                                }
                                self.cursor += 1;
                            }
                        }
                        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                            self.cursor += 1;
                        }
                        _ => return Err(DailyWireErrorV1::InvalidJson),
                    }
                }
                0x00..=0x1f => return Err(DailyWireErrorV1::InvalidJson),
                _ => self.cursor += 1,
            }
        }
        Err(DailyWireErrorV1::InvalidJson)
    }

    fn number(&mut self) -> Result<(), DailyWireErrorV1> {
        let start = self.cursor;
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|b| matches!(*b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.cursor += 1;
        }
        if start == self.cursor {
            return Err(DailyWireErrorV1::InvalidJson);
        }
        let token = &self.bytes[start..self.cursor];
        if !valid_json_number(token) {
            return Err(DailyWireErrorV1::InvalidJson);
        }
        let canonical = token == b"0"
            || (!token.is_empty()
                && token[0].is_ascii_digit()
                && token[0] != b'0'
                && token.iter().all(u8::is_ascii_digit));
        if !canonical
            || std::str::from_utf8(token)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .is_none()
        {
            self.invalid_number = true;
        }
        Ok(())
    }

    fn value(&mut self, depth: usize) -> Result<(), DailyWireErrorV1> {
        self.whitespace();
        match self.bytes.get(self.cursor).copied() {
            Some(b'{') => {
                check_container_depth(depth, fixed_limits().max_json_depth)?;
                self.cursor += 1;
                self.whitespace();
                let mut keys = BTreeSet::new();
                if self.bytes.get(self.cursor) == Some(&b'}') {
                    self.cursor += 1;
                    return Ok(());
                }
                loop {
                    self.whitespace();
                    let key = self.string()?;
                    if !keys.insert(key) {
                        self.duplicate_key = true;
                    }
                    self.whitespace();
                    if self.bytes.get(self.cursor) != Some(&b':') {
                        return Err(DailyWireErrorV1::InvalidJson);
                    }
                    self.cursor += 1;
                    self.value(depth + 1)?;
                    self.whitespace();
                    match self.bytes.get(self.cursor).copied() {
                        Some(b',') => self.cursor += 1,
                        Some(b'}') => {
                            self.cursor += 1;
                            return Ok(());
                        }
                        _ => return Err(DailyWireErrorV1::InvalidJson),
                    }
                }
            }
            Some(b'[') => {
                check_container_depth(depth, fixed_limits().max_json_depth)?;
                self.cursor += 1;
                self.whitespace();
                if self.bytes.get(self.cursor) == Some(&b']') {
                    self.cursor += 1;
                    return Ok(());
                }
                let mut count = 0usize;
                loop {
                    self.value(depth + 1)?;
                    count = count
                        .checked_add(1)
                        .ok_or(DailyWireErrorV1::LimitExceeded)?;
                    if check_file_evidence_count(count).is_err() {
                        self.overlarge_array = true;
                    }
                    self.whitespace();
                    match self.bytes.get(self.cursor).copied() {
                        Some(b',') => self.cursor += 1,
                        Some(b']') => {
                            self.cursor += 1;
                            return Ok(());
                        }
                        _ => return Err(DailyWireErrorV1::InvalidJson),
                    }
                }
            }
            Some(b'"') => self.string().map(|_| ()),
            Some(b't')
                if self
                    .bytes
                    .get(self.cursor..self.cursor + 4)
                    .is_some_and(|value| value == b"true") =>
            {
                self.cursor += 4;
                Ok(())
            }
            Some(b'f')
                if self
                    .bytes
                    .get(self.cursor..self.cursor + 5)
                    .is_some_and(|value| value == b"false") =>
            {
                self.cursor += 5;
                Ok(())
            }
            Some(b'n')
                if self
                    .bytes
                    .get(self.cursor..self.cursor + 4)
                    .is_some_and(|value| value == b"null") =>
            {
                self.cursor += 4;
                Ok(())
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(DailyWireErrorV1::InvalidJson),
        }
    }
}

fn parse_json_bytes(bytes: &[u8], limit: usize) -> Result<Value, DailyWireErrorV1> {
    check_collection_len(
        bytes.len(),
        u64::try_from(limit).map_err(|_| DailyWireErrorV1::LimitExceeded)?,
    )?;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) || std::str::from_utf8(bytes).is_err() {
        return Err(DailyWireErrorV1::InvalidJson);
    }
    let mut scanner = JsonScanner {
        bytes,
        cursor: 0,
        duplicate_key: false,
        invalid_number: false,
        overlarge_array: false,
    };
    scanner.value(0)?;
    scanner.whitespace();
    if scanner.cursor != bytes.len() {
        return Err(DailyWireErrorV1::InvalidJson);
    }
    if scanner.duplicate_key {
        return Err(DailyWireErrorV1::DuplicateKey);
    }
    if scanner.invalid_number {
        return Err(DailyWireErrorV1::InvalidType);
    }
    if scanner.overlarge_array {
        return Err(DailyWireErrorV1::LimitExceeded);
    }
    serde_json::from_slice(bytes).map_err(|_| DailyWireErrorV1::InvalidJson)
}

fn object<'a>(
    value: &'a Value,
    fields: &[&str],
) -> Result<&'a Map<String, Value>, DailyWireErrorV1> {
    let map = value.as_object().ok_or(DailyWireErrorV1::InvalidType)?;
    if map.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(DailyWireErrorV1::UnknownField);
    }
    if fields.iter().any(|field| !map.contains_key(*field)) {
        return Err(DailyWireErrorV1::MissingField);
    }
    Ok(map)
}

fn array(value: &Value) -> Result<&[Value], DailyWireErrorV1> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or(DailyWireErrorV1::InvalidType)
}

fn field<'a>(map: &'a Map<String, Value>, name: &str) -> Result<&'a Value, DailyWireErrorV1> {
    map.get(name).ok_or(DailyWireErrorV1::MissingField)
}

fn shape_base(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(value, &["kind", "pins"])?;
    object(
        field(map, "pins")?,
        &[
            "candidate_content_sha256",
            "artifact_manifest_sha256",
            "stage5_manifest_sha256",
            "action_manifest_sha256",
            "approval_registry_sha256",
        ],
    )?;
    Ok(())
}

fn shape_limits(value: &Value) -> Result<(), DailyWireErrorV1> {
    object(
        value,
        &[
            "max_policy_bytes",
            "max_manifest_bytes",
            "max_admission_bytes",
            "max_normalization_bytes",
            "max_bars_bytes",
            "max_sessions",
            "max_civil_dates_per_addition",
            "max_action_chains_per_session",
            "max_file_evidence_per_session",
            "max_source_file_bytes",
            "max_normalized_document_bytes",
            "max_copy_depth",
            "max_pages_per_action_chain",
            "max_cash_rows_per_chain",
            "max_json_depth",
        ],
    )?;
    Ok(())
}

fn shape_file_key(value: &Value) -> Result<(), DailyWireErrorV1> {
    object(
        value,
        &["provider", "market", "batch_id", "file_name", "wire_sha256"],
    )?;
    Ok(())
}

fn shape_request(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(value, &["endpoint", "query", "headers", "mode"])?;
    for pairs_name in ["query", "headers"] {
        for pair in array(field(map, pairs_name)?)? {
            let pair = array(pair)?;
            if pair.len() != 2 || pair.iter().any(|part| !part.is_string()) {
                return Err(DailyWireErrorV1::InvalidType);
            }
        }
    }
    Ok(())
}

fn shape_origin(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "provider",
            "market",
            "batch_id",
            "batch_date",
            "batch_json_sha256",
            "manifest_line_sha256",
            "file_name",
            "kind",
            "wire_sha256",
            "size_bytes",
            "request",
            "response_continuation",
            "retrieved_at",
            "retrieval_precision",
        ],
    )?;
    shape_request(field(map, "request")?)?;
    Ok(())
}

fn shape_file_evidence(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "provider",
            "market",
            "batch_id",
            "batch_date",
            "batch_json_sha256",
            "manifest_line_sha256",
            "file_name",
            "kind",
            "wire_sha256",
            "size_bytes",
            "request",
            "response_continuation",
            "retrieved_at",
            "retrieval_precision",
            "original_retrieved_at",
            "original_retrieval_precision",
            "copied_from",
        ],
    )?;
    shape_request(field(map, "request")?)?;
    for origin in array(field(map, "copied_from")?)? {
        shape_origin(origin)?;
    }
    Ok(())
}

fn shape_cash_row(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "symbol",
            "record_date",
            "dividend_kind",
            "face_value",
            "cash_dividend_amount",
            "cash_dividend_rate",
            "stock_dividend_rate",
            "cash_payment_date",
            "stock_payment_date",
            "odd_lot_payment_date",
            "stock_kind",
            "high_dividend_flag",
            "source_file",
        ],
    )?;
    shape_file_key(field(map, "source_file")?)?;
    Ok(())
}

fn shape_action(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "logical_class",
            "scope",
            "symbol",
            "range_start",
            "range_end",
            "pages",
            "cash_dividends",
            "cash_dividend_rows_sha256",
        ],
    )?;
    for page in array(field(map, "pages")?)? {
        shape_file_evidence(page)?;
    }
    for row in array(field(map, "cash_dividends")?)? {
        shape_cash_row(row)?;
    }
    Ok(())
}

fn shape_calendar(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "civil_date",
            "session_type",
            "exchange",
            "timezone",
            "source_file",
            "normalized_document_sha256",
            "legacy_projection",
        ],
    )?;
    shape_file_evidence(field(map, "source_file")?)?;
    let legacy_value = field(map, "legacy_projection")?;
    if !legacy_value.is_null() {
        object(
            legacy_value,
            &[
                "source",
                "source_version",
                "source_batch_id",
                "source_file_name",
                "document_sha256",
                "retrieved_at",
            ],
        )?;
    }
    Ok(())
}

fn shape_normalized_document(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "kind",
            "session_date",
            "normalizer_version",
            "sha256",
            "size_bytes",
            "row_count",
            "source_files",
            "source_evidence_sha256",
        ],
    )?;
    for file in array(field(map, "source_files")?)? {
        shape_file_key(file)?;
    }
    Ok(())
}

fn shape_session(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "session_date",
            "previous_frontier",
            "instruments",
            "bars",
            "reference",
            "calendar",
            "actions",
            "normalized_documents",
        ],
    )?;
    for name in ["bars", "reference"] {
        for file in array(field(map, name)?)? {
            shape_file_evidence(file)?;
        }
    }
    for binding in array(field(map, "calendar")?)? {
        shape_calendar(binding)?;
    }
    for action in array(field(map, "actions")?)? {
        shape_action(action)?;
    }
    for document in array(field(map, "normalized_documents")?)? {
        shape_normalized_document(document)?;
    }
    Ok(())
}

fn shape_policy(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "schema",
            "capability",
            "audience",
            "vendor_snapshot",
            "strict_pit",
            "base",
            "instruments",
            "price_semantics",
            "cash_action_policy",
            "normalizer_version",
            "materializer_version",
            "source_kinds",
            "limits",
        ],
    )?;
    shape_base(field(map, "base")?)?;
    for source in array(field(map, "source_kinds")?)? {
        object(source, &["provider", "market", "kind"])?;
    }
    shape_limits(field(map, "limits")?)?;
    Ok(())
}

fn shape_manifest(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "schema",
            "capability",
            "audience",
            "vendor_snapshot",
            "strict_pit",
            "base",
            "parent_admission_sha256",
            "range_start",
            "range_end",
            "instruments",
            "sessions",
            "daily_sources",
            "bars",
            "normalization",
            "policy_sha256",
            "materializer_version",
            "code_commit",
            "content_sha256",
        ],
    )?;
    shape_base(field(map, "base")?)?;
    for session in array(field(map, "daily_sources")?)? {
        shape_session(session)?;
    }
    object(
        field(map, "bars")?,
        &["file", "sha256", "size_bytes", "row_count"],
    )?;
    object(
        field(map, "normalization")?,
        &["file", "sha256", "size_bytes", "row_count"],
    )?;
    Ok(())
}

fn shape_admission(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "schema",
            "content_sha256",
            "manifest_sha256",
            "policy_sha256",
            "parent_admission_sha256",
            "base",
            "verifier_code_commit",
            "normalizer_version",
            "evidence_sha256",
            "counts",
        ],
    )?;
    shape_base(field(map, "base")?)?;
    object(
        field(map, "counts")?,
        &[
            "instrument_count",
            "session_count",
            "bar_count",
            "added_session_count",
            "source_file_count",
            "calendar_row_count",
            "action_chain_count",
            "action_page_count",
            "cash_dividend_row_count",
            "normalized_document_count",
        ],
    )?;
    Ok(())
}

fn shape_proof(value: &Value) -> Result<(), DailyWireErrorV1> {
    let map = object(
        value,
        &[
            "schema",
            "normalizer_version",
            "evidence_sha256",
            "documents",
        ],
    )?;
    for document in array(field(map, "documents")?)? {
        shape_normalized_document(document)?;
    }
    Ok(())
}

fn valid_json_number(token: &[u8]) -> bool {
    let mut i = 0;
    if token.get(i) == Some(&b'-') {
        i += 1;
    }
    match token.get(i).copied() {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while token.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if token.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while token.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if start == i {
            return false;
        }
    }
    if matches!(token.get(i).copied(), Some(b'e' | b'E')) {
        i += 1;
        if matches!(token.get(i).copied(), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while token.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if start == i {
            return false;
        }
    }
    i == token.len()
}

fn valid_hash(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        if [8, 13, 18, 23].contains(&index) {
            if *byte != b'-' {
                return false;
            }
        } else if !(byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)) {
            return false;
        }
    }
    value != "00000000-0000-0000-0000-000000000000"
}

fn valid_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_domain_date(value: TradingDate) -> bool {
    let rendered = value.to_iso();
    rendered.len() == 10
        && rendered
            .get(..4)
            .and_then(|year| year.parse::<u32>().ok())
            .is_some_and(|year| (1..=9999).contains(&year))
}

fn valid_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !value.contains("..")
}

fn valid_code(value: &str) -> bool {
    value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn valid_decimal(value: &str) -> bool {
    let Some((whole, fractional)) = value.split_once('.') else {
        return !value.is_empty()
            && (value == "0"
                || (value.as_bytes()[0].is_ascii_digit()
                    && value.as_bytes()[0] != b'0'
                    && value.bytes().all(|b| b.is_ascii_digit())))
            && value.parse::<i128>().is_ok();
    };
    if whole.is_empty()
        || !(whole == "0"
            || (whole.as_bytes()[0].is_ascii_digit()
                && whole.as_bytes()[0] != b'0'
                && whole.bytes().all(|b| b.is_ascii_digit())))
        || fractional.is_empty()
        || fractional.len() > 28
        || !fractional.bytes().all(|b| b.is_ascii_digit())
        || fractional.ends_with('0')
    {
        return false;
    }
    let mantissa = format!("{whole}{fractional}");
    mantissa.parse::<i128>().is_ok() && domain::FixedPoint::parse(value).is_ok()
}

fn date_field(name: &str) -> bool {
    matches!(
        name,
        "batch_date"
            | "range_start"
            | "range_end"
            | "session_date"
            | "previous_frontier"
            | "civil_date"
            | "record_date"
            | "cash_payment_date"
            | "stock_payment_date"
            | "odd_lot_payment_date"
    )
}

fn timestamp_field(name: &str) -> bool {
    matches!(name, "retrieved_at" | "original_retrieved_at")
}

fn hash_field(name: &str) -> bool {
    name == "sha256" || name.ends_with("_sha256")
}

fn validate_literal_fields(value: &Value) -> Result<(), DailyWireErrorV1> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if let Some(text) = child.as_str() {
                    let allowed = match key.as_str() {
                        "schema" => Some(
                            [
                                POLICY_SCHEMA,
                                MANIFEST_SCHEMA,
                                EVIDENCE_SCHEMA,
                                NORMALIZATION_SCHEMA,
                                NORMALIZED_SOURCE_SCHEMA,
                                ADMISSION_SCHEMA,
                            ]
                            .contains(&text),
                        ),
                        "kind" => Some(
                            [
                                "bars",
                                "reference",
                                "calendar",
                                "corporate_actions",
                                BASE_KIND,
                            ]
                            .contains(&text),
                        ),
                        "logical_class" => Some(matches!(
                            text,
                            "paidin-subscription"
                                | "paidin-record"
                                | "bonus-issue"
                                | "dividend"
                                | "merger-split"
                                | "reverse-split"
                                | "capital-decrease"
                        )),
                        "scope" => Some(matches!(text, "whole_market" | "etf11_symbol")),
                        "session_type" => Some(matches!(text, "TRADING" | "CLOSED")),
                        _ => None,
                    };
                    if allowed == Some(false) {
                        return Err(DailyWireErrorV1::InvalidLiteral);
                    }
                } else if matches!(
                    key.as_str(),
                    "schema" | "kind" | "logical_class" | "scope" | "session_type"
                ) {
                    return Err(DailyWireErrorV1::InvalidType);
                }
                validate_literal_fields(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_literal_fields(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_lexical_fields(value: &Value) -> Result<(), DailyWireErrorV1> {
    fn visit(value: &Value, key: Option<&str>) -> Result<(), DailyWireErrorV1> {
        match value {
            Value::String(text) => {
                let key = key.unwrap_or("");
                if matches!(key, "dividend_kind" | "stock_kind" | "high_dividend_flag")
                    && !valid_code(text)
                {
                    return Err(DailyWireErrorV1::InvalidLiteral);
                }
                if text.chars().any(char::is_control) {
                    return Err(DailyWireErrorV1::InvalidName);
                }
                if hash_field(key) && !valid_hash(text) {
                    return Err(DailyWireErrorV1::InvalidHash);
                }
                if date_field(key) {
                    let year = text.get(..4).and_then(|part| part.parse::<u32>().ok());
                    if text.len() != 10
                        || text.as_bytes().get(4) != Some(&b'-')
                        || text.as_bytes().get(7) != Some(&b'-')
                        || !year.is_some_and(|year| (1..=9999).contains(&year))
                        || !text
                            .bytes()
                            .enumerate()
                            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
                        || TradingDate::parse(text).is_err()
                    {
                        return Err(DailyWireErrorV1::InvalidDate);
                    }
                }
                if timestamp_field(key) && DailyWireTimestampV1::parse(text).is_err() {
                    return Err(DailyWireErrorV1::InvalidTimestamp);
                }
                if matches!(key, "batch_id" | "source_batch_id")
                    && (!valid_uuid(text) || BatchId::from_str(text).is_err())
                {
                    return Err(DailyWireErrorV1::InvalidUuid);
                }
                if matches!(key, "code_commit" | "verifier_code_commit")
                    && (!valid_commit(text) || CodeCommit::parse(text).is_err())
                {
                    return Err(DailyWireErrorV1::InvalidCommit);
                }
                if matches!(key, "file_name" | "file") && !valid_name(text) {
                    return Err(DailyWireErrorV1::InvalidName);
                }
                if matches!(
                    key,
                    "face_value"
                        | "cash_dividend_amount"
                        | "cash_dividend_rate"
                        | "stock_dividend_rate"
                ) && !valid_decimal(text)
                {
                    return Err(DailyWireErrorV1::InvalidDecimal);
                }
                if key == "symbol"
                    && (text.len() != 6 || !text.bytes().all(|byte| byte.is_ascii_digit()))
                {
                    return Err(DailyWireErrorV1::InvalidName);
                }
                if key == "response_continuation"
                    && (text.len() > 8 || !text.bytes().all(|byte| byte.is_ascii_alphanumeric()))
                {
                    return Err(DailyWireErrorV1::InvalidName);
                }
            }
            Value::Array(values) => {
                if key == Some("sessions") {
                    for value in values {
                        let text = value.as_str().ok_or(DailyWireErrorV1::InvalidType)?;
                        visit(value, Some("session_date"))?;
                        if text.len() != 10 {
                            return Err(DailyWireErrorV1::InvalidDate);
                        }
                    }
                } else if key == Some("instruments") {
                    for value in values {
                        let text = value.as_str().ok_or(DailyWireErrorV1::InvalidType)?;
                        if !ETF11_INSTRUMENTS.contains(&text) {
                            return Err(DailyWireErrorV1::InvalidLiteral);
                        }
                    }
                } else {
                    for value in values {
                        visit(value, None)?;
                    }
                }
            }
            Value::Object(map) => {
                for (child_key, child) in map {
                    if hash_field(child_key) {
                        if let Some(text) = child.as_str() {
                            if !valid_hash(text) {
                                return Err(DailyWireErrorV1::InvalidHash);
                            }
                        } else if !child.is_null() {
                            return Err(DailyWireErrorV1::InvalidType);
                        }
                    }
                    if matches!(child_key.as_str(), "batch_id" | "source_batch_id")
                        && !child.is_string()
                    {
                        return Err(DailyWireErrorV1::InvalidType);
                    }
                    if matches!(child_key.as_str(), "code_commit" | "verifier_code_commit")
                        && !child.is_string()
                    {
                        return Err(DailyWireErrorV1::InvalidType);
                    }
                    if matches!(child_key.as_str(), "symbol")
                        && !child.is_null()
                        && !child.is_string()
                    {
                        return Err(DailyWireErrorV1::InvalidType);
                    }
                    if matches!(
                        child_key.as_str(),
                        "cash_payment_date" | "stock_payment_date" | "odd_lot_payment_date"
                    ) && !child.is_null()
                        && !child.is_string()
                    {
                        return Err(DailyWireErrorV1::InvalidType);
                    }
                    visit(child, Some(child_key))?;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
        Ok(())
    }
    visit(value, None)
}

fn check_top_shape(value: &Value, kind: &'static str) -> Result<(), DailyWireErrorV1> {
    match kind {
        "policy" => shape_policy(value),
        "manifest" => shape_manifest(value),
        "admission" => shape_admission(value),
        "proof" => shape_proof(value),
        _ => Err(DailyWireErrorV1::InvalidJson),
    }
}

fn parse_typed<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    limit: usize,
    kind: &'static str,
) -> Result<T, DailyWireErrorV1> {
    let value = parse_json_bytes(bytes, limit)?;
    check_top_shape(&value, kind)?;
    validate_literal_fields(&value)?;
    validate_lexical_fields(&value)?;
    serde_json::from_value(value).map_err(|_| DailyWireErrorV1::InvalidType)
}

fn instrument_names(values: &[InstrumentId]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn file_identity_parts(file: &DailyFileEvidenceV1Wire) -> FileIdentityParts {
    (
        file.provider.clone(),
        file.market.clone(),
        file.batch_id.to_string(),
        file.file_name.clone(),
    )
}

fn origin_identity_parts(file: &DailyFileOriginV1Wire) -> FileIdentityParts {
    (
        file.provider.clone(),
        file.market.clone(),
        file.batch_id.to_string(),
        file.file_name.clone(),
    )
}

fn key_identity_parts(key: &DailyFileKeyV1Wire) -> FileIdentityParts {
    (
        key.provider.clone(),
        key.market.clone(),
        key.batch_id.to_string(),
        key.file_name.clone(),
    )
}

fn key_sort_parts(key: &DailyFileKeyV1Wire) -> FileKeySortParts {
    let (provider, market, batch, name) = key_identity_parts(key);
    (provider, market, batch, name, key.wire_sha256.to_string())
}

fn validate_cash_row_scalars(row: &DailyCashDividendRowV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        !ETF11_SYMBOLS.contains(&row.symbol.as_str()),
        DailyWireErrorV1::InvalidName,
    )?;
    invalid_if(
        !valid_domain_date(row.record_date)
            || row
                .cash_payment_date
                .is_some_and(|date| !valid_domain_date(date))
            || row
                .stock_payment_date
                .is_some_and(|date| !valid_domain_date(date))
            || row
                .odd_lot_payment_date
                .is_some_and(|date| !valid_domain_date(date)),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(
        !valid_code(&row.dividend_kind)
            || !valid_code(&row.stock_kind)
            || !valid_code(&row.high_dividend_flag),
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        !valid_decimal(&row.face_value)
            || !valid_decimal(&row.cash_dividend_amount)
            || !valid_decimal(&row.cash_dividend_rate)
            || row.stock_dividend_rate != "0"
            || !valid_decimal(&row.stock_dividend_rate),
        DailyWireErrorV1::InvalidDecimal,
    )?;
    invalid_if(
        row.stock_payment_date.is_some() || row.odd_lot_payment_date.is_some(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    validate_key_set(std::slice::from_ref(&row.source_file))
}

fn evidence_file_key(file: &DailyFileEvidenceV1Wire) -> DailyFileKeyV1Wire {
    DailyFileKeyV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: file.batch_id,
        file_name: file.file_name.clone(),
        wire_sha256: file.wire_sha256.clone(),
    }
}

fn canonicalize_request(request: &mut DailyRequestMetadataV1Wire) {
    request
        .query
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    request
        .headers
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
}

fn canonicalize_file(file: &mut DailyFileEvidenceV1Wire) {
    canonicalize_request(&mut file.request);
    for origin in &mut file.copied_from {
        canonicalize_request(&mut origin.request);
    }
}

/// Internal verifier adapter. This uses the same D4 validation and encoder;
/// it does not turn the returned wire value into an approved artifact.
pub(super) fn canonical_file_evidence(
    file: &DailyFileEvidenceV1Wire,
) -> Result<(DailyFileEvidenceV1Wire, Vec<u8>), DailyWireErrorV1> {
    preflight_file(file)?;
    let mut file = file.clone();
    canonicalize_file(&mut file);
    validate_file_evidence(&file)?;
    let bytes = canonical_bytes_limited(&file, fixed_limits().max_manifest_bytes)?;
    Ok((file, bytes))
}

fn canonicalize_key_set(keys: &mut [DailyFileKeyV1Wire]) {
    keys.sort_by_key(key_sort_parts);
}

fn cash_sort_key(row: &DailyCashDividendRowV1Wire) -> Result<CashRowSortKey, DailyWireErrorV1> {
    validate_cash_row_scalars(row)?;
    let bytes = canonical_bytes_limited(row, 4096)?;
    Ok((
        row.symbol.clone(),
        row.record_date,
        row.dividend_kind.clone(),
        key_sort_parts(&row.source_file),
        bytes,
    ))
}

fn canonicalize_action(action: &mut DailyActionChainV1Wire) -> Result<(), DailyWireErrorV1> {
    for page in &mut action.pages {
        canonicalize_file(page);
    }
    let mut keyed = action
        .cash_dividends
        .drain(..)
        .map(|row| Ok((cash_sort_key(&row)?, row)))
        .collect::<Result<Vec<_>, DailyWireErrorV1>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    action.cash_dividends = keyed.into_iter().map(|(_, row)| row).collect();
    Ok(())
}

fn canonicalize_session(session: &mut DailySessionEvidenceV1Wire) -> Result<(), DailyWireErrorV1> {
    preflight_session(session)?;
    session.instruments.sort_by(|left, right| {
        left.to_string()
            .as_bytes()
            .cmp(right.to_string().as_bytes())
    });
    for file in session.bars.iter_mut().chain(session.reference.iter_mut()) {
        canonicalize_file(file);
    }
    session.bars.sort_by_key(file_identity_parts);
    session.reference.sort_by_key(file_identity_parts);
    for binding in &mut session.calendar {
        canonicalize_file(&mut binding.source_file);
    }
    session.calendar.sort_by_key(|binding| binding.civil_date);
    for action in &mut session.actions {
        canonicalize_action(action)?;
    }
    session.actions.sort_by_key(|action| {
        (
            action.logical_class.literal(),
            action.scope.literal(),
            action.symbol.clone(),
            action.range_start,
            action.range_end,
            action
                .pages
                .first()
                .map(evidence_file_key)
                .map(|key| key_sort_parts(&key)),
        )
    });
    for document in &mut session.normalized_documents {
        canonicalize_key_set(&mut document.source_files);
    }
    session
        .normalized_documents
        .sort_by_key(|document| (document.session_date, document.kind.literal()));
    Ok(())
}

fn canonicalize_policy(value: &mut DailyPricePolicyV1Wire) {
    value.instruments.sort_by(|left, right| {
        left.to_string()
            .as_bytes()
            .cmp(right.to_string().as_bytes())
    });
    value.source_kinds.sort_by_key(|source| {
        (
            source.provider.clone(),
            source.market.clone(),
            source.kind.literal(),
        )
    });
}

fn canonicalize_manifest(value: &mut DailyPriceManifestV1Wire) -> Result<(), DailyWireErrorV1> {
    preflight_manifest(&value.instruments, &value.sessions, &value.daily_sources)?;
    value.instruments.sort_by(|left, right| {
        left.to_string()
            .as_bytes()
            .cmp(right.to_string().as_bytes())
    });
    value.sessions.sort();
    for session in &mut value.daily_sources {
        canonicalize_session(session)?;
    }
    value
        .daily_sources
        .sort_by_key(|session| session.session_date);
    Ok(())
}

fn canonicalize_draft(value: &mut DailyPriceManifestDraftV1Wire) -> Result<(), DailyWireErrorV1> {
    preflight_manifest(&value.instruments, &value.sessions, &value.daily_sources)?;
    value.instruments.sort_by(|left, right| {
        left.to_string()
            .as_bytes()
            .cmp(right.to_string().as_bytes())
    });
    value.sessions.sort();
    for session in &mut value.daily_sources {
        canonicalize_session(session)?;
    }
    value
        .daily_sources
        .sort_by_key(|session| session.session_date);
    Ok(())
}

fn canonicalize_proof(value: &mut DailyNormalizationProofV1Wire) {
    for document in &mut value.documents {
        canonicalize_key_set(&mut document.source_files);
    }
    value
        .documents
        .sort_by_key(|document| (document.session_date, document.kind.literal()));
}

fn canonicalize_evidence(
    sessions: &[DailySessionEvidenceV1Wire],
) -> Result<Vec<DailySessionEvidenceV1Wire>, DailyWireErrorV1> {
    invalid_if(sessions.is_empty(), DailyWireErrorV1::InconsistentFields)?;
    check_session_count(sessions.len())?;
    for session in sessions {
        preflight_session(session)?;
    }
    let mut sessions = sessions.to_vec();
    for session in &mut sessions {
        canonicalize_session(session)?;
    }
    sessions.sort_by_key(|session| session.session_date);
    Ok(sessions)
}

fn invalid_if(condition: bool, error: DailyWireErrorV1) -> Result<(), DailyWireErrorV1> {
    if condition { Err(error) } else { Ok(()) }
}

fn check_limit(actual: u64, maximum: u64) -> Result<(), DailyWireErrorV1> {
    invalid_if(actual > maximum, DailyWireErrorV1::LimitExceeded)
}

fn check_collection_len(actual: usize, maximum: u64) -> Result<(), DailyWireErrorV1> {
    check_limit(
        u64::try_from(actual).map_err(|_| DailyWireErrorV1::LimitExceeded)?,
        maximum,
    )
}

fn check_container_depth(depth: usize, maximum: u64) -> Result<(), DailyWireErrorV1> {
    let depth = u64::try_from(depth).map_err(|_| DailyWireErrorV1::LimitExceeded)?;
    invalid_if(depth >= maximum, DailyWireErrorV1::LimitExceeded)
}

fn check_session_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_sessions)
}

fn check_civil_date_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_civil_dates_per_addition)
}

fn check_action_chain_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_action_chains_per_session)
}

fn check_file_evidence_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_file_evidence_per_session)
}

fn check_copy_hop_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_copy_depth)
}

fn check_action_page_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_pages_per_action_chain)
}

fn check_cash_row_count(count: usize) -> Result<(), DailyWireErrorV1> {
    check_collection_len(count, fixed_limits().max_cash_rows_per_chain)
}

fn check_source_file_size(size: u64) -> Result<(), DailyWireErrorV1> {
    check_limit(size, fixed_limits().max_source_file_bytes)
}

fn check_normalized_document_size(size: u64) -> Result<(), DailyWireErrorV1> {
    check_limit(size, fixed_limits().max_normalized_document_bytes)
}

fn check_bars_descriptor_size(size: u64) -> Result<(), DailyWireErrorV1> {
    check_limit(size, fixed_limits().max_bars_bytes)
}

fn check_normalization_descriptor_size(size: u64) -> Result<(), DailyWireErrorV1> {
    check_limit(size, fixed_limits().max_normalization_bytes)
}

fn hash_bytes(bytes: &[u8]) -> ContentHash {
    ContentHash::from_bytes(bytes)
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl Write for BoundedVecWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(next_len) = self.bytes.len().checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("canonical wire byte limit exceeded"));
        };
        let limit = u64::try_from(self.limit).unwrap_or(u64::MAX);
        if check_collection_len(next_len, limit).is_err() {
            self.exceeded = true;
            return Err(io::Error::other("canonical wire byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn canonical_bytes_limited<T: Serialize>(
    value: &T,
    limit: u64,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    let limit = usize::try_from(limit).map_err(|_| DailyWireErrorV1::LimitExceeded)?;
    let mut writer = BoundedVecWriter {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.bytes),
        Err(_) if writer.exceeded => Err(DailyWireErrorV1::LimitExceeded),
        Err(_) => Err(DailyWireErrorV1::InvalidType),
    }
}

fn fixed_limits() -> DailyResourceLimitsV1Wire {
    DailyResourceLimitsV1Wire {
        max_policy_bytes: 262_144,
        max_manifest_bytes: 16_777_216,
        max_admission_bytes: 1_048_576,
        max_normalization_bytes: 16_777_216,
        max_bars_bytes: 134_217_728,
        max_sessions: 20_000,
        max_civil_dates_per_addition: 370,
        max_action_chains_per_session: 32_768,
        max_file_evidence_per_session: 262_144,
        max_source_file_bytes: 67_108_864,
        max_normalized_document_bytes: 67_108_864,
        max_copy_depth: 16,
        max_pages_per_action_chain: 10,
        max_cash_rows_per_chain: 100_000,
        max_json_depth: 32,
    }
}

fn preflight_request(request: &DailyRequestMetadataV1Wire) -> Result<(), DailyWireErrorV1> {
    check_file_evidence_count(request.query.len())?;
    check_file_evidence_count(request.headers.len())
}

fn preflight_file(file: &DailyFileEvidenceV1Wire) -> Result<(), DailyWireErrorV1> {
    check_copy_hop_count(file.copied_from.len())?;
    preflight_request(&file.request)?;
    for origin in &file.copied_from {
        preflight_request(&origin.request)?;
    }
    Ok(())
}

fn preflight_session(session: &DailySessionEvidenceV1Wire) -> Result<(), DailyWireErrorV1> {
    let limits = fixed_limits();
    check_file_evidence_count(session.instruments.len())?;
    check_file_evidence_count(session.bars.len())?;
    check_file_evidence_count(session.reference.len())?;
    check_civil_date_count(session.calendar.len())?;
    check_action_chain_count(session.actions.len())?;
    check_collection_len(
        session.normalized_documents.len(),
        3 + limits.max_civil_dates_per_addition,
    )?;
    let mut file_count = session
        .bars
        .len()
        .checked_add(session.reference.len())
        .and_then(|count| count.checked_add(session.calendar.len()))
        .ok_or(DailyWireErrorV1::LimitExceeded)?;
    for file in session.bars.iter().chain(session.reference.iter()) {
        preflight_file(file)?;
    }
    for binding in &session.calendar {
        preflight_file(&binding.source_file)?;
    }
    for action in &session.actions {
        check_action_page_count(action.pages.len())?;
        check_cash_row_count(action.cash_dividends.len())?;
        file_count = file_count
            .checked_add(action.pages.len())
            .ok_or(DailyWireErrorV1::LimitExceeded)?;
        for file in &action.pages {
            preflight_file(file)?;
        }
    }
    check_file_evidence_count(file_count)?;
    for document in &session.normalized_documents {
        check_file_evidence_count(document.source_files.len())?;
    }
    Ok(())
}

fn preflight_manifest(
    instruments: &[InstrumentId],
    sessions: &[TradingDate],
    daily_sources: &[DailySessionEvidenceV1Wire],
) -> Result<(), DailyWireErrorV1> {
    check_file_evidence_count(instruments.len())?;
    check_session_count(sessions.len())?;
    check_session_count(daily_sources.len())?;
    for session in daily_sources {
        preflight_session(session)?;
    }
    Ok(())
}

fn preflight_policy(value: &DailyPricePolicyV1Wire) -> Result<(), DailyWireErrorV1> {
    check_file_evidence_count(value.instruments.len())?;
    check_file_evidence_count(value.source_kinds.len())
}

fn preflight_proof(value: &DailyNormalizationProofV1Wire) -> Result<(), DailyWireErrorV1> {
    check_file_evidence_count(value.documents.len())?;
    for document in &value.documents {
        check_file_evidence_count(document.source_files.len())?;
    }
    Ok(())
}

fn validate_base(base: &DailyBaseV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(base.kind != BASE_KIND, DailyWireErrorV1::InvalidLiteral)
}

fn expected_source_kinds() -> Vec<DailySourceKindV1Wire> {
    [
        ("kis", DailyEvidenceKindV1::Bars),
        ("kis", DailyEvidenceKindV1::Calendar),
        ("kis", DailyEvidenceKindV1::CorporateActions),
        ("kis", DailyEvidenceKindV1::Reference),
        ("kis-calendar", DailyEvidenceKindV1::Calendar),
    ]
    .into_iter()
    .map(|(provider, kind)| DailySourceKindV1Wire {
        provider: provider.to_owned(),
        market: "kr".to_owned(),
        kind,
    })
    .collect()
}

fn validate_instruments(values: &[InstrumentId]) -> Result<(), DailyWireErrorV1> {
    let actual = instrument_names(values);
    strictly_increasing(&actual)?;
    let expected: Vec<String> = ETF11_INSTRUMENTS
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    invalid_if(actual != expected, DailyWireErrorV1::InconsistentFields)
}

fn validate_policy(value: &DailyPricePolicyV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        value.schema != POLICY_SCHEMA,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.capability != "PRICE_RETURN_ONLY",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.audience != "OWNER_ONLY",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(!value.vendor_snapshot, DailyWireErrorV1::InvalidLiteral)?;
    invalid_if(value.strict_pit, DailyWireErrorV1::InvalidLiteral)?;
    validate_base(&value.base)?;
    validate_instruments(&value.instruments)?;
    invalid_if(
        value.price_semantics != "KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.cash_action_policy != "CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.normalizer_version != NORMALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.materializer_version != MATERIALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    let mut actual = value.source_kinds.clone();
    actual.sort_by_key(|source| {
        (
            source.provider.clone(),
            source.market.clone(),
            source.kind.literal(),
        )
    });
    for pair in actual.windows(2) {
        if pair[0].provider == pair[1].provider
            && pair[0].market == pair[1].market
            && pair[0].kind == pair[1].kind
        {
            return Err(DailyWireErrorV1::DuplicateEntry);
        }
    }
    let expected = expected_source_kinds();
    invalid_if(actual != expected, DailyWireErrorV1::InconsistentFields)?;
    invalid_if(
        value.limits != fixed_limits(),
        DailyWireErrorV1::InvalidLiteral,
    )
}

fn pair_value<'a>(pairs: &'a [(String, String)], key: &str) -> Result<&'a str, DailyWireErrorV1> {
    let mut found = None;
    for (name, value) in pairs {
        if name == key && found.replace(value.as_str()).is_some() {
            return Err(DailyWireErrorV1::DuplicateEntry);
        }
    }
    found.ok_or(DailyWireErrorV1::InconsistentFields)
}

fn validate_pair_order(pairs: &[(String, String)]) -> Result<(), DailyWireErrorV1> {
    for pair in pairs.windows(2) {
        match pair[0].0.as_bytes().cmp(pair[1].0.as_bytes()) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(DailyWireErrorV1::DuplicateEntry),
            std::cmp::Ordering::Greater => return Err(DailyWireErrorV1::InvalidOrder),
        }
    }
    Ok(())
}

fn validate_request_base(request: &DailyRequestMetadataV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        request.mode != "credentialed",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    validate_pair_order(&request.query)?;
    validate_pair_order(&request.headers)?;
    let headers: BTreeMap<&str, &str> = request
        .headers
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    invalid_if(
        headers.len() != 5
            || !["appkey", "appsecret", "authorization", "tr_cont", "tr_id"]
                .iter()
                .all(|key| headers.contains_key(key)),
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        ["appkey", "appsecret", "authorization"]
            .iter()
            .any(|key| headers.get(key) != Some(&"[REDACTED]")),
        DailyWireErrorV1::InvalidLiteral,
    )?;
    let tr_cont = headers
        .get("tr_cont")
        .ok_or(DailyWireErrorV1::InconsistentFields)?;
    invalid_if(
        !matches!(*tr_cont, "" | "N"),
        DailyWireErrorV1::InconsistentFields,
    )?;
    let endpoint = request.endpoint.as_str();
    let (expected_tr, expected_kind) = match endpoint {
        "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice" => {
            ("FHKST03010100", DailyEvidenceKindV1::Bars)
        }
        "/uapi/domestic-stock/v1/quotations/inquire-price" => {
            ("FHKST01010100", DailyEvidenceKindV1::Reference)
        }
        "/uapi/domestic-stock/v1/quotations/chk-holiday" => {
            ("CTCA0903R", DailyEvidenceKindV1::Calendar)
        }
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin" => {
            ("HHKDB669100C0", DailyEvidenceKindV1::CorporateActions)
        }
        "/uapi/domestic-stock/v1/ksdinfo/bonus-issue" => {
            ("HHKDB669101C0", DailyEvidenceKindV1::CorporateActions)
        }
        "/uapi/domestic-stock/v1/ksdinfo/dividend" => {
            ("HHKDB669102C0", DailyEvidenceKindV1::CorporateActions)
        }
        "/uapi/domestic-stock/v1/ksdinfo/merger-split" => {
            ("HHKDB669104C0", DailyEvidenceKindV1::CorporateActions)
        }
        "/uapi/domestic-stock/v1/ksdinfo/rev-split" => {
            ("HHKDB669105C0", DailyEvidenceKindV1::CorporateActions)
        }
        "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs" => {
            ("HHKDB669106C0", DailyEvidenceKindV1::CorporateActions)
        }
        _ => return Err(DailyWireErrorV1::InvalidLiteral),
    };
    invalid_if(
        headers.get("tr_id") != Some(&expected_tr),
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        !request.endpoint.starts_with(DAILY_ENDPOINT_PREFIX),
        DailyWireErrorV1::InvalidLiteral,
    )?;
    // The response kind is consumed by endpoint-specific checks below.
    let _ = expected_kind;
    Ok(())
}

fn action_request_spec(
    class: DailyActionClassV1,
) -> (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
) {
    match class {
        DailyActionClassV1::PaidinSubscription => {
            ("ksdinfo/paidin-capin", "HHKDB669100C0", Some(("GB1", "1")))
        }
        DailyActionClassV1::PaidinRecord => {
            ("ksdinfo/paidin-capin", "HHKDB669100C0", Some(("GB1", "2")))
        }
        DailyActionClassV1::BonusIssue => ("ksdinfo/bonus-issue", "HHKDB669101C0", None),
        DailyActionClassV1::Dividend => ("ksdinfo/dividend", "HHKDB669102C0", None),
        DailyActionClassV1::MergerSplit => ("ksdinfo/merger-split", "HHKDB669104C0", None),
        DailyActionClassV1::ReverseSplit => ("ksdinfo/rev-split", "HHKDB669105C0", None),
        DailyActionClassV1::CapitalDecrease => ("ksdinfo/cap-dcrs", "HHKDB669106C0", None),
    }
}

fn expected_action_query(
    class: DailyActionClassV1,
    start: TradingDate,
    end: TradingDate,
    symbol: Option<&str>,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::from([
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), start.to_iso().replace('-', "")),
        ("T_DT".to_owned(), end.to_iso().replace('-', "")),
        ("SHT_CD".to_owned(), symbol.unwrap_or("").to_owned()),
    ]);
    match class {
        DailyActionClassV1::PaidinSubscription => {
            values.insert("GB1".to_owned(), "1".to_owned());
        }
        DailyActionClassV1::PaidinRecord => {
            values.insert("GB1".to_owned(), "2".to_owned());
        }
        DailyActionClassV1::Dividend => {
            values.insert("GB1".to_owned(), "0".to_owned());
            values.insert("HIGH_GB".to_owned(), String::new());
        }
        DailyActionClassV1::ReverseSplit => {
            values.insert("MARKET_GB".to_owned(), "0".to_owned());
        }
        DailyActionClassV1::BonusIssue
        | DailyActionClassV1::MergerSplit
        | DailyActionClassV1::CapitalDecrease => {}
    }
    values
}

fn expected_headers(tr_id: &str, tr_cont: &str) -> Vec<(String, String)> {
    let mut result = vec![
        ("appkey".to_owned(), "[REDACTED]".to_owned()),
        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
        ("authorization".to_owned(), "[REDACTED]".to_owned()),
        ("tr_cont".to_owned(), tr_cont.to_owned()),
        ("tr_id".to_owned(), tr_id.to_owned()),
    ];
    result.sort_by(|left, right| left.0.cmp(&right.0));
    result
}

fn expected_pairs(values: BTreeMap<String, String>) -> Vec<(String, String)> {
    values.into_iter().collect()
}

fn validate_request_exact(
    request: &DailyRequestMetadataV1Wire,
    endpoint: &str,
    tr_id: &str,
    query: BTreeMap<String, String>,
    tr_cont: &str,
) -> Result<(), DailyWireErrorV1> {
    validate_request_base(request)?;
    invalid_if(
        request.endpoint != endpoint,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        request.query != expected_pairs(query),
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        request.headers != expected_headers(tr_id, tr_cont),
        DailyWireErrorV1::InconsistentFields,
    )
}

fn validate_policy_and_bytes(
    value: &mut DailyPricePolicyV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    canonicalize_policy(value);
    validate_policy(value)?;
    let bytes = canonical_bytes_limited(value, fixed_limits().max_policy_bytes)?;
    check_collection_len(bytes.len(), fixed_limits().max_policy_bytes)?;
    Ok(bytes)
}

fn kind_for_endpoint(endpoint: &str) -> Option<DailyEvidenceKindV1> {
    match endpoint {
        "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice" => {
            Some(DailyEvidenceKindV1::Bars)
        }
        "/uapi/domestic-stock/v1/quotations/inquire-price" => Some(DailyEvidenceKindV1::Reference),
        "/uapi/domestic-stock/v1/quotations/chk-holiday" => Some(DailyEvidenceKindV1::Calendar),
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin"
        | "/uapi/domestic-stock/v1/ksdinfo/bonus-issue"
        | "/uapi/domestic-stock/v1/ksdinfo/dividend"
        | "/uapi/domestic-stock/v1/ksdinfo/merger-split"
        | "/uapi/domestic-stock/v1/ksdinfo/rev-split"
        | "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs" => Some(DailyEvidenceKindV1::CorporateActions),
        _ => None,
    }
}

fn validate_request_kind(
    request: &DailyRequestMetadataV1Wire,
    kind: DailyEvidenceKindV1,
) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        kind_for_endpoint(&request.endpoint) != Some(kind),
        DailyWireErrorV1::InconsistentFields,
    )
}

struct OriginFields<'a> {
    provider: &'a str,
    market: &'a str,
    file_name: &'a str,
    kind: DailyEvidenceKindV1,
    size_bytes: u64,
    request: &'a DailyRequestMetadataV1Wire,
    response_continuation: &'a Option<String>,
    retrieval_precision: &'a str,
}

fn validate_origin_fields(fields: OriginFields<'_>) -> Result<(), DailyWireErrorV1> {
    let OriginFields {
        provider,
        market,
        file_name,
        kind,
        size_bytes,
        request,
        response_continuation,
        retrieval_precision,
    } = fields;
    invalid_if(
        !matches!(provider, "kis" | "kis-calendar") || market != "kr",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        provider == "kis-calendar" && kind != DailyEvidenceKindV1::Calendar,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(!valid_name(file_name), DailyWireErrorV1::InvalidName)?;
    invalid_if(size_bytes == 0, DailyWireErrorV1::LimitExceeded)?;
    check_source_file_size(size_bytes)?;
    invalid_if(
        retrieval_precision != "batch",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    if let Some(marker) = response_continuation {
        invalid_if(
            marker.len() > 8 || !marker.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            DailyWireErrorV1::InvalidName,
        )?;
    }
    validate_request_base(request)?;
    validate_request_kind(request, kind)
}

fn origin_from_evidence(file: &DailyFileEvidenceV1Wire) -> DailyFileOriginV1Wire {
    DailyFileOriginV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: file.batch_id,
        batch_date: file.batch_date,
        batch_json_sha256: file.batch_json_sha256.clone(),
        manifest_line_sha256: file.manifest_line_sha256.clone(),
        file_name: file.file_name.clone(),
        kind: file.kind,
        wire_sha256: file.wire_sha256.clone(),
        size_bytes: file.size_bytes,
        request: file.request.clone(),
        response_continuation: file.response_continuation.clone(),
        retrieved_at: file.retrieved_at,
        retrieval_precision: file.retrieval_precision.clone(),
    }
}

fn validate_file_evidence(file: &DailyFileEvidenceV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        file.batch_id.as_uuid().is_nil(),
        DailyWireErrorV1::InvalidUuid,
    )?;
    invalid_if(
        !valid_domain_date(file.batch_date),
        DailyWireErrorV1::InvalidDate,
    )?;
    validate_origin_fields(OriginFields {
        provider: &file.provider,
        market: &file.market,
        file_name: &file.file_name,
        kind: file.kind,
        size_bytes: file.size_bytes,
        request: &file.request,
        response_continuation: &file.response_continuation,
        retrieval_precision: &file.retrieval_precision,
    })?;
    invalid_if(
        file.original_retrieval_precision != "batch",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    check_copy_hop_count(file.copied_from.len())?;
    let mut seen = BTreeSet::new();
    seen.insert(file_identity_parts(file));
    let mut prior_time = file.retrieved_at;
    for origin in &file.copied_from {
        invalid_if(
            origin.batch_id.as_uuid().is_nil(),
            DailyWireErrorV1::InvalidUuid,
        )?;
        invalid_if(
            !valid_domain_date(origin.batch_date),
            DailyWireErrorV1::InvalidDate,
        )?;
        validate_origin_fields(OriginFields {
            provider: &origin.provider,
            market: &origin.market,
            file_name: &origin.file_name,
            kind: origin.kind,
            size_bytes: origin.size_bytes,
            request: &origin.request,
            response_continuation: &origin.response_continuation,
            retrieval_precision: &origin.retrieval_precision,
        })?;
        invalid_if(
            origin.kind != file.kind
                || origin.wire_sha256 != file.wire_sha256
                || origin.size_bytes != file.size_bytes
                || origin.request != file.request
                || origin.response_continuation != file.response_continuation,
            DailyWireErrorV1::InconsistentFields,
        )?;
        invalid_if(
            prior_time.as_timestamp() < origin.retrieved_at.as_timestamp(),
            DailyWireErrorV1::InvalidOrder,
        )?;
        prior_time = origin.retrieved_at;
        invalid_if(
            !seen.insert(origin_identity_parts(origin)),
            DailyWireErrorV1::DuplicateEntry,
        )?;
    }
    let expected_original = file
        .copied_from
        .last()
        .map(|origin| origin.retrieved_at)
        .unwrap_or(file.retrieved_at);
    invalid_if(
        file.original_retrieved_at != expected_original,
        DailyWireErrorV1::InconsistentFields,
    )
}

fn terminal_origin_provider(file: &DailyFileEvidenceV1Wire) -> &str {
    file.copied_from
        .last()
        .map(|origin| origin.provider.as_str())
        .unwrap_or(file.provider.as_str())
}

fn terminal_origin_batch(file: &DailyFileEvidenceV1Wire) -> BatchId {
    file.copied_from
        .last()
        .map(|origin| origin.batch_id)
        .unwrap_or(file.batch_id)
}

fn terminal_origin_batch_json_hash(file: &DailyFileEvidenceV1Wire) -> &ContentHash {
    file.copied_from
        .last()
        .map(|origin| &origin.batch_json_sha256)
        .unwrap_or(&file.batch_json_sha256)
}

fn terminal_origin_batch_date(file: &DailyFileEvidenceV1Wire) -> TradingDate {
    file.copied_from
        .last()
        .map(|origin| origin.batch_date)
        .unwrap_or(file.batch_date)
}

fn terminal_origin_timestamp(file: &DailyFileEvidenceV1Wire) -> DailyWireTimestampV1 {
    file.copied_from
        .last()
        .map(|origin| origin.retrieved_at)
        .unwrap_or(file.retrieved_at)
}

fn validate_file_request_for_symbol(
    file: &DailyFileEvidenceV1Wire,
    kind: DailyEvidenceKindV1,
    symbol: &str,
    date: TradingDate,
) -> Result<(), DailyWireErrorV1> {
    validate_file_evidence(file)?;
    invalid_if(
        file.kind != kind || file.provider != "kis",
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        terminal_origin_batch_date(file) != date,
        DailyWireErrorV1::InconsistentFields,
    )?;
    let (path, tr_id, query) = match kind {
        DailyEvidenceKindV1::Bars => (
            "quotations/inquire-daily-itemchartprice",
            "FHKST03010100",
            BTreeMap::from([
                ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
                (
                    "FID_INPUT_DATE_1".to_owned(),
                    date.to_iso().replace('-', ""),
                ),
                (
                    "FID_INPUT_DATE_2".to_owned(),
                    date.to_iso().replace('-', ""),
                ),
                ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
                ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
            ]),
        ),
        DailyEvidenceKindV1::Reference => (
            "quotations/inquire-price",
            "FHKST01010100",
            BTreeMap::from([
                ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
            ]),
        ),
        _ => return Err(DailyWireErrorV1::InconsistentFields),
    };
    validate_request_exact(
        &file.request,
        &format!("{DAILY_ENDPOINT_PREFIX}{path}"),
        tr_id,
        query,
        "",
    )
}

pub(super) fn validate_calendar_request(
    file: &DailyFileEvidenceV1Wire,
) -> Result<(), DailyWireErrorV1> {
    validate_file_evidence(file)?;
    invalid_if(
        file.kind != DailyEvidenceKindV1::Calendar,
        DailyWireErrorV1::InconsistentFields,
    )?;
    let date = terminal_origin_batch_date(file);
    validate_request_exact(
        &file.request,
        "/uapi/domestic-stock/v1/quotations/chk-holiday",
        "CTCA0903R",
        BTreeMap::from([
            ("BASS_DT".to_owned(), date.to_iso().replace('-', "")),
            ("CTX_AREA_FK".to_owned(), String::new()),
            ("CTX_AREA_NK".to_owned(), String::new()),
        ]),
        "",
    )
}

fn validate_action_file_request(
    file: &DailyFileEvidenceV1Wire,
    action: &DailyActionChainV1Wire,
    page_index: usize,
) -> Result<(), DailyWireErrorV1> {
    validate_file_evidence(file)?;
    invalid_if(
        file.kind != DailyEvidenceKindV1::CorporateActions || file.provider != "kis",
        DailyWireErrorV1::InconsistentFields,
    )?;
    let (path, tr_id, _) = action_request_spec(action.logical_class);
    let symbol = match action.scope {
        DailyActionScopeV1::WholeMarket => None,
        DailyActionScopeV1::Etf11Symbol => action.symbol.as_deref(),
    };
    let cont = if page_index == 0 { "" } else { "N" };
    validate_request_exact(
        &file.request,
        &format!("{DAILY_ENDPOINT_PREFIX}{path}"),
        tr_id,
        expected_action_query(
            action.logical_class,
            action.range_start,
            action.range_end,
            symbol,
        ),
        cont,
    )
}

fn file_key_matches(file: &DailyFileEvidenceV1Wire, key: &DailyFileKeyV1Wire) -> bool {
    evidence_file_key(file) == *key
}

fn date_range(start: TradingDate, end: TradingDate) -> Result<Vec<TradingDate>, DailyWireErrorV1> {
    if end < start {
        return Err(DailyWireErrorV1::InconsistentFields);
    }
    let mut dates = Vec::new();
    let mut date = start;
    loop {
        let next_len = dates
            .len()
            .checked_add(1)
            .ok_or(DailyWireErrorV1::LimitExceeded)?;
        check_civil_date_count(next_len)?;
        dates.push(date);
        if date == end {
            break;
        }
        date = date
            .checked_add_days(1)
            .map_err(|_| DailyWireErrorV1::InvalidDate)?;
    }
    Ok(dates)
}

fn unique_file_keys(
    files: impl IntoIterator<Item = DailyFileKeyV1Wire>,
) -> Result<Vec<DailyFileKeyV1Wire>, DailyWireErrorV1> {
    let mut by_identity = BTreeMap::new();
    for key in files {
        let identity = key_identity_parts(&key);
        if let Some(previous) = by_identity.get(&identity) {
            if previous != &key {
                return Err(DailyWireErrorV1::InconsistentFields);
            }
        } else {
            by_identity.insert(identity, key);
        }
    }
    let mut result: Vec<_> = by_identity.into_values().collect();
    canonicalize_key_set(&mut result);
    Ok(result)
}

fn validate_key_set(keys: &[DailyFileKeyV1Wire]) -> Result<(), DailyWireErrorV1> {
    for key in keys {
        invalid_if(
            !matches!(key.provider.as_str(), "kis" | "kis-calendar") || key.market != "kr",
            DailyWireErrorV1::InvalidLiteral,
        )?;
        invalid_if(!valid_name(&key.file_name), DailyWireErrorV1::InvalidName)?;
        invalid_if(
            key.batch_id.as_uuid().is_nil(),
            DailyWireErrorV1::InvalidUuid,
        )?;
    }
    for pair in keys.windows(2) {
        let left_identity = key_identity_parts(&pair[0]);
        let right_identity = key_identity_parts(&pair[1]);
        if left_identity == right_identity {
            return Err(if pair[0].wire_sha256 == pair[1].wire_sha256 {
                DailyWireErrorV1::DuplicateEntry
            } else {
                DailyWireErrorV1::InconsistentFields
            });
        }
        if key_sort_parts(&pair[0]) >= key_sort_parts(&pair[1]) {
            return Err(DailyWireErrorV1::InvalidOrder);
        }
    }
    Ok(())
}

fn validate_cash_rows(
    rows: &[DailyCashDividendRowV1Wire],
    chain: &DailyActionChainV1Wire,
    previous_frontier: TradingDate,
    session_date: TradingDate,
) -> Result<(), DailyWireErrorV1> {
    check_cash_row_count(rows.len())?;
    let mut prior_key: Option<CashRowSortKey> = None;
    for row in rows {
        validate_cash_row_scalars(row)?;
        invalid_if(
            row.record_date < chain.range_start
                || row.record_date > chain.range_end
                || row.record_date <= previous_frontier
                || row.record_date > session_date,
            DailyWireErrorV1::InconsistentFields,
        )?;
        invalid_if(
            chain.scope == DailyActionScopeV1::Etf11Symbol
                && chain.symbol.as_deref() != Some(row.symbol.as_str()),
            DailyWireErrorV1::InconsistentFields,
        )?;
        invalid_if(
            !chain
                .pages
                .iter()
                .any(|page| file_key_matches(page, &row.source_file)),
            DailyWireErrorV1::InconsistentFields,
        )?;
        let sort_key = cash_sort_key(row)?;
        if let Some(previous) = &prior_key {
            match previous.cmp(&sort_key) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => return Err(DailyWireErrorV1::DuplicateEntry),
                std::cmp::Ordering::Greater => return Err(DailyWireErrorV1::InvalidOrder),
            }
        }
        prior_key = Some(sort_key);
    }
    let bytes = canonical_bytes_limited(&rows, fixed_limits().max_manifest_bytes)?;
    invalid_if(
        hash_bytes(&bytes) != chain.cash_dividend_rows_sha256,
        DailyWireErrorV1::HashMismatch,
    )
}

fn validate_action_chain(
    action: &DailyActionChainV1Wire,
    previous_frontier: TradingDate,
    session_date: TradingDate,
) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        !valid_domain_date(action.range_start) || !valid_domain_date(action.range_end),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(
        action.range_end < action.range_start,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        action.pages.is_empty(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    check_action_page_count(action.pages.len())?;
    match action.scope {
        DailyActionScopeV1::WholeMarket => invalid_if(
            action.symbol.is_some(),
            DailyWireErrorV1::InconsistentFields,
        )?,
        DailyActionScopeV1::Etf11Symbol => {
            let symbol = action
                .symbol
                .as_deref()
                .ok_or(DailyWireErrorV1::InconsistentFields)?;
            invalid_if(
                !ETF11_SYMBOLS.contains(&symbol),
                DailyWireErrorV1::InvalidName,
            )?;
        }
    }
    invalid_if(
        action.logical_class != DailyActionClassV1::Dividend && !action.cash_dividends.is_empty(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    let first = action
        .pages
        .first()
        .ok_or(DailyWireErrorV1::InconsistentFields)?;
    let terminal_first = (
        terminal_origin_provider(first).to_owned(),
        terminal_origin_batch(first),
        terminal_origin_batch_date(first),
        terminal_origin_timestamp(first),
        first.original_retrieval_precision.clone(),
    );
    let mut file_identities = BTreeSet::new();
    let mut wire_hashes = BTreeSet::new();
    for (index, page) in action.pages.iter().enumerate() {
        validate_action_file_request(page, action, index)?;
        invalid_if(
            page.batch_id != first.batch_id
                || page.batch_date != first.batch_date
                || page.provider != first.provider
                || page.batch_json_sha256 != first.batch_json_sha256
                || page.original_retrieval_precision != first.original_retrieval_precision
                || terminal_origin_batch_json_hash(page) != terminal_origin_batch_json_hash(first)
                || (
                    terminal_origin_provider(page).to_owned(),
                    terminal_origin_batch(page),
                    terminal_origin_batch_date(page),
                    terminal_origin_timestamp(page),
                    page.original_retrieval_precision.clone(),
                ) != terminal_first,
            DailyWireErrorV1::InconsistentFields,
        )?;
        invalid_if(
            !file_identities.insert(file_identity_parts(page))
                || !wire_hashes.insert(page.wire_sha256.to_string()),
            DailyWireErrorV1::DuplicateEntry,
        )?;
        let marker = page.response_continuation.as_deref().unwrap_or("");
        if index + 1 < action.pages.len() {
            invalid_if(marker != "M", DailyWireErrorV1::InvalidOrder)?;
        } else {
            invalid_if(marker == "M", DailyWireErrorV1::InvalidOrder)?;
        }
    }
    validate_cash_rows(
        &action.cash_dividends,
        action,
        previous_frontier,
        session_date,
    )
}

fn validate_calendar_binding(
    binding: &DailyCalendarBindingV1Wire,
    session_date: TradingDate,
) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        !valid_domain_date(binding.civil_date),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(binding.exchange != "KRX", DailyWireErrorV1::InvalidLiteral)?;
    invalid_if(
        binding.timezone != "Asia/Seoul",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    validate_calendar_request(&binding.source_file)?;
    if let Some(legacy) = &binding.legacy_projection {
        invalid_if(
            legacy.source != "kis"
                || legacy.source_version != "kis-chk-holiday-v1:schema-1"
                || legacy.source_file_name != "calendar.json",
            DailyWireErrorV1::InvalidLiteral,
        )?;
        invalid_if(
            terminal_origin_provider(&binding.source_file) != "kis"
                || legacy.source_batch_id != terminal_origin_batch(&binding.source_file)
                || legacy.retrieved_at != terminal_origin_timestamp(&binding.source_file),
            DailyWireErrorV1::InconsistentFields,
        )?;
    }
    let expected_type = if binding.civil_date == session_date {
        DailySessionTypeV1::Trading
    } else {
        DailySessionTypeV1::Closed
    };
    invalid_if(
        binding.session_type.literal() != expected_type.literal(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    Ok(())
}

fn compare_file_identity_metadata(
    file: &DailyFileEvidenceV1Wire,
    full: &mut BTreeMap<FileIdentityParts, DailyFileEvidenceV1Wire>,
    origins: &mut BTreeMap<FileIdentityParts, DailyFileOriginV1Wire>,
) -> Result<(), DailyWireErrorV1> {
    let identity = file_identity_parts(file);
    if let Some(previous) = full.get(&identity) {
        invalid_if(previous != file, DailyWireErrorV1::InconsistentFields)?;
    } else {
        full.insert(identity.clone(), file.clone());
    }
    let current_origin = origin_from_evidence(file);
    if let Some(previous) = origins.get(&identity) {
        invalid_if(
            previous != &current_origin,
            DailyWireErrorV1::InconsistentFields,
        )?;
    } else {
        origins.insert(identity, current_origin);
    }
    for origin in &file.copied_from {
        let identity = origin_identity_parts(origin);
        if let Some(previous) = origins.get(&identity) {
            invalid_if(previous != origin, DailyWireErrorV1::InconsistentFields)?;
        } else {
            origins.insert(identity, origin.clone());
        }
    }
    Ok(())
}

fn session_file_occurrences(session: &DailySessionEvidenceV1Wire) -> Vec<&DailyFileEvidenceV1Wire> {
    let mut files = Vec::new();
    files.extend(session.bars.iter());
    files.extend(session.reference.iter());
    files.extend(session.calendar.iter().map(|binding| &binding.source_file));
    files.extend(
        session
            .actions
            .iter()
            .flat_map(|action| action.pages.iter()),
    );
    files
}

fn strictly_increasing<T: Ord>(values: &[T]) -> Result<(), DailyWireErrorV1> {
    for pair in values.windows(2) {
        match pair[0].cmp(&pair[1]) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(DailyWireErrorV1::DuplicateEntry),
            std::cmp::Ordering::Greater => return Err(DailyWireErrorV1::InvalidOrder),
        }
    }
    Ok(())
}

fn validate_symbol_file_set(
    files: &[DailyFileEvidenceV1Wire],
    kind: DailyEvidenceKindV1,
    session_date: TradingDate,
) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        files.len() != ETF11_SYMBOLS.len(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    let mut symbols = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for file in files {
        let symbol = pair_value(&file.request.query, "FID_INPUT_ISCD")?;
        invalid_if(
            !ETF11_SYMBOLS.contains(&symbol),
            DailyWireErrorV1::InvalidName,
        )?;
        invalid_if(
            !symbols.insert(symbol.to_owned()),
            DailyWireErrorV1::DuplicateEntry,
        )?;
        invalid_if(
            !identities.insert(file_identity_parts(file)),
            DailyWireErrorV1::DuplicateEntry,
        )?;
        validate_file_request_for_symbol(file, kind, symbol, session_date)?;
    }
    let expected: BTreeSet<&str> = ETF11_SYMBOLS.into_iter().collect();
    invalid_if(
        symbols.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected,
        DailyWireErrorV1::InconsistentFields,
    )
}

fn validate_action_coverage(
    actions: &[DailyActionChainV1Wire],
    required_start: TradingDate,
    session_date: TradingDate,
) -> Result<(), DailyWireErrorV1> {
    for class in [
        DailyActionClassV1::PaidinSubscription,
        DailyActionClassV1::PaidinRecord,
        DailyActionClassV1::BonusIssue,
        DailyActionClassV1::Dividend,
        DailyActionClassV1::MergerSplit,
        DailyActionClassV1::ReverseSplit,
        DailyActionClassV1::CapitalDecrease,
    ] {
        for symbol in ETF11_SYMBOLS {
            let mut intervals: Vec<_> = actions
                .iter()
                .filter(|action| {
                    action.logical_class == class
                        && (action.scope == DailyActionScopeV1::WholeMarket
                            || action.symbol.as_deref() == Some(symbol))
                })
                .map(|action| (action.range_start, action.range_end))
                .collect();
            intervals.sort();
            let mut covered_through = required_start.previous_day();
            for (start, end) in intervals {
                let next_required = covered_through
                    .checked_add_days(1)
                    .map_err(|_| DailyWireErrorV1::InvalidDate)?;
                if start > next_required {
                    break;
                }
                if end > covered_through {
                    covered_through = end;
                }
                if covered_through >= session_date {
                    break;
                }
            }
            invalid_if(
                covered_through < session_date,
                DailyWireErrorV1::InconsistentFields,
            )?;
        }
    }
    Ok(())
}

fn expected_document_files(
    session: &DailySessionEvidenceV1Wire,
    kind: DailyEvidenceKindV1,
    date: TradingDate,
) -> Result<Vec<DailyFileKeyV1Wire>, DailyWireErrorV1> {
    let files = match kind {
        DailyEvidenceKindV1::Bars if date == session.session_date => session
            .bars
            .iter()
            .map(evidence_file_key)
            .collect::<Vec<_>>(),
        DailyEvidenceKindV1::Reference if date == session.session_date => session
            .reference
            .iter()
            .map(evidence_file_key)
            .collect::<Vec<_>>(),
        DailyEvidenceKindV1::CorporateActions if date == session.session_date => session
            .actions
            .iter()
            .flat_map(|action| action.pages.iter().map(evidence_file_key))
            .collect::<Vec<_>>(),
        DailyEvidenceKindV1::Calendar => session
            .calendar
            .iter()
            .find(|binding| binding.civil_date == date)
            .map(|binding| vec![evidence_file_key(&binding.source_file)])
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    unique_file_keys(files)
}

fn validate_normalized_document(
    document: &DailyNormalizedDocumentV1Wire,
    expected_files: &[DailyFileKeyV1Wire],
    expected_rows: Option<(u64, u64)>,
) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        !valid_domain_date(document.session_date),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(
        document.normalizer_version != NORMALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(document.size_bytes == 0, DailyWireErrorV1::LimitExceeded)?;
    check_normalized_document_size(document.size_bytes)?;
    invalid_if(
        document.source_files.is_empty(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    validate_key_set(&document.source_files)?;
    invalid_if(
        document.source_files != expected_files,
        DailyWireErrorV1::InconsistentFields,
    )?;
    if let Some((min, max)) = expected_rows {
        invalid_if(
            document.row_count < min || document.row_count > max,
            DailyWireErrorV1::InconsistentFields,
        )?;
    }
    let commitment = DailyNormalizedSourceCommitmentV1Wire {
        schema: NORMALIZED_SOURCE_SCHEMA.to_owned(),
        kind: document.kind,
        session_date: document.session_date,
        source_files: document.source_files.clone(),
    };
    let bytes = canonical_bytes_limited(&commitment, fixed_limits().max_normalized_document_bytes)?;
    invalid_if(
        hash_bytes(&bytes) != document.source_evidence_sha256,
        DailyWireErrorV1::HashMismatch,
    )
}

fn validate_session(session: &DailySessionEvidenceV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        !valid_domain_date(session.previous_frontier) || !valid_domain_date(session.session_date),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(
        session.previous_frontier
            < TradingDate::parse(BASE_FRONTIER).map_err(|_| DailyWireErrorV1::InvalidDate)?
            || session.session_date <= session.previous_frontier,
        DailyWireErrorV1::InconsistentFields,
    )?;
    validate_instruments(&session.instruments)?;
    validate_symbol_file_set(
        &session.bars,
        DailyEvidenceKindV1::Bars,
        session.session_date,
    )?;
    validate_symbol_file_set(
        &session.reference,
        DailyEvidenceKindV1::Reference,
        session.session_date,
    )?;

    let first_civil_date = session
        .previous_frontier
        .checked_add_days(1)
        .map_err(|_| DailyWireErrorV1::InvalidDate)?;
    let expected_civil_dates = date_range(first_civil_date, session.session_date)?;
    check_civil_date_count(expected_civil_dates.len())?;
    invalid_if(
        session.calendar.len() != expected_civil_dates.len(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    let calendar_dates: Vec<_> = session
        .calendar
        .iter()
        .map(|binding| binding.civil_date)
        .collect();
    strictly_increasing(&calendar_dates)?;
    for (binding, expected_date) in session.calendar.iter().zip(expected_civil_dates.iter()) {
        invalid_if(
            binding.civil_date != *expected_date,
            DailyWireErrorV1::InconsistentFields,
        )?;
        validate_calendar_binding(binding, session.session_date)?;
    }

    invalid_if(
        session.actions.len() < 7,
        DailyWireErrorV1::InconsistentFields,
    )?;
    check_action_chain_count(session.actions.len())?;
    let mut previous_action_sort_key: Option<ActionSortKey> = None;
    for action in &session.actions {
        validate_action_chain(action, session.previous_frontier, session.session_date)?;
        let sort_key: ActionSortKey = (
            action.logical_class.literal(),
            action.scope.literal(),
            action.symbol.clone(),
            action.range_start,
            action.range_end,
            action
                .pages
                .first()
                .map(|page| key_sort_parts(&evidence_file_key(page))),
        );
        if let Some(previous) = previous_action_sort_key {
            match previous.cmp(&sort_key) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => return Err(DailyWireErrorV1::DuplicateEntry),
                std::cmp::Ordering::Greater => return Err(DailyWireErrorV1::InvalidOrder),
            }
        }
        previous_action_sort_key = Some(sort_key);
    }
    let required_start = first_civil_date;
    validate_action_coverage(&session.actions, required_start, session.session_date)?;

    let files = session_file_occurrences(session);
    check_file_evidence_count(files.len())?;
    let mut full_files = BTreeMap::new();
    let mut origin_files = BTreeMap::new();
    for file in &files {
        compare_file_identity_metadata(file, &mut full_files, &mut origin_files)?;
    }
    let mut action_witness_rows = 0u64;
    for action in &session.actions {
        action_witness_rows = action_witness_rows
            .checked_add(
                u64::try_from(action.cash_dividends.len())
                    .map_err(|_| DailyWireErrorV1::LimitExceeded)?,
            )
            .ok_or(DailyWireErrorV1::LimitExceeded)?;
    }

    let expected_doc_count = 3usize
        .checked_add(session.calendar.len())
        .ok_or(DailyWireErrorV1::LimitExceeded)?;
    invalid_if(
        session.normalized_documents.len() != expected_doc_count,
        DailyWireErrorV1::InconsistentFields,
    )?;
    let doc_keys: Vec<_> = session
        .normalized_documents
        .iter()
        .map(|document| (document.session_date, document.kind.literal()))
        .collect();
    strictly_increasing(&doc_keys)?;
    let mut found_required = BTreeSet::new();
    for document in &session.normalized_documents {
        let expected_files =
            expected_document_files(session, document.kind, document.session_date)?;
        let expected_rows = match document.kind {
            DailyEvidenceKindV1::Bars | DailyEvidenceKindV1::Reference => Some((11, 11)),
            DailyEvidenceKindV1::Calendar => Some((1, 1)),
            DailyEvidenceKindV1::CorporateActions => Some((0, action_witness_rows)),
        };
        validate_normalized_document(document, &expected_files, expected_rows)?;
        invalid_if(
            expected_files.is_empty(),
            DailyWireErrorV1::InconsistentFields,
        )?;
        found_required.insert((document.session_date, document.kind));
    }
    for kind in [
        DailyEvidenceKindV1::Bars,
        DailyEvidenceKindV1::Reference,
        DailyEvidenceKindV1::CorporateActions,
    ] {
        invalid_if(
            !found_required.contains(&(session.session_date, kind)),
            DailyWireErrorV1::InconsistentFields,
        )?;
    }
    for binding in &session.calendar {
        invalid_if(
            !found_required.contains(&(binding.civil_date, DailyEvidenceKindV1::Calendar)),
            DailyWireErrorV1::InconsistentFields,
        )?;
        let document = session
            .normalized_documents
            .iter()
            .find(|document| {
                document.session_date == binding.civil_date
                    && document.kind == DailyEvidenceKindV1::Calendar
            })
            .ok_or(DailyWireErrorV1::InconsistentFields)?;
        invalid_if(
            binding.normalized_document_sha256 != document.sha256,
            DailyWireErrorV1::HashMismatch,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DerivedCounts {
    instrument_count: u64,
    session_count: u64,
    bar_count: u64,
    added_session_count: u64,
    source_file_count: u64,
    calendar_row_count: u64,
    action_chain_count: u64,
    action_page_count: u64,
    cash_dividend_row_count: u64,
    normalized_document_count: u64,
}

fn add_count(target: &mut u64, value: usize) -> Result<(), DailyWireErrorV1> {
    *target = target
        .checked_add(u64::try_from(value).map_err(|_| DailyWireErrorV1::LimitExceeded)?)
        .ok_or(DailyWireErrorV1::LimitExceeded)?;
    Ok(())
}

fn derive_evidence_counts(
    daily_sources: &[DailySessionEvidenceV1Wire],
    full_files: &mut BTreeMap<FileIdentityParts, DailyFileEvidenceV1Wire>,
    origin_files: &mut BTreeMap<FileIdentityParts, DailyFileOriginV1Wire>,
) -> Result<DerivedCounts, DailyWireErrorV1> {
    let mut counts = DerivedCounts {
        instrument_count: 11,
        added_session_count: u64::try_from(daily_sources.len())
            .map_err(|_| DailyWireErrorV1::LimitExceeded)?,
        ..DerivedCounts::default()
    };
    let mut unique_files = BTreeMap::new();
    let mut unique_documents = BTreeSet::new();
    let mut claimed_calendar_dates = BTreeSet::new();
    for session in daily_sources {
        validate_session(session)?;
        claimed_calendar_dates.extend(session.calendar.iter().map(|binding| binding.civil_date));
        add_count(&mut counts.calendar_row_count, session.calendar.len())?;
        add_count(&mut counts.action_chain_count, session.actions.len())?;
        add_count(
            &mut counts.normalized_document_count,
            session.normalized_documents.len(),
        )?;
        for action in &session.actions {
            add_count(&mut counts.action_page_count, action.pages.len())?;
            add_count(
                &mut counts.cash_dividend_row_count,
                action.cash_dividends.len(),
            )?;
        }
        for file in session_file_occurrences(session) {
            compare_file_identity_metadata(file, full_files, origin_files)?;
            let key = evidence_file_key(file);
            let identity = key_identity_parts(&key);
            if let Some(existing) = unique_files.get(&identity) {
                invalid_if(existing != &key, DailyWireErrorV1::InconsistentFields)?;
            } else {
                unique_files.insert(identity, key);
            }
        }
        for document in &session.normalized_documents {
            invalid_if(
                !unique_documents.insert((document.session_date, document.kind.literal())),
                DailyWireErrorV1::DuplicateEntry,
            )?;
        }
    }
    if let Some(last_added_date) = daily_sources.last().map(|session| session.session_date) {
        let base_frontier =
            TradingDate::parse(BASE_FRONTIER).map_err(|_| DailyWireErrorV1::InvalidDate)?;
        for file in daily_sources
            .iter()
            .flat_map(session_file_occurrences)
            .filter(|file| file.kind == DailyEvidenceKindV1::Calendar)
        {
            let requested_date = terminal_origin_batch_date(file);
            if requested_date > base_frontier && requested_date <= last_added_date {
                invalid_if(
                    !claimed_calendar_dates.contains(&requested_date),
                    DailyWireErrorV1::InconsistentFields,
                )?;
            }
        }
    }
    counts.source_file_count =
        u64::try_from(unique_files.len()).map_err(|_| DailyWireErrorV1::LimitExceeded)?;
    counts.normalized_document_count =
        u64::try_from(unique_documents.len()).map_err(|_| DailyWireErrorV1::LimitExceeded)?;
    Ok(counts)
}

fn derive_counts(
    total_session_count: usize,
    daily_sources: &[DailySessionEvidenceV1Wire],
    full_files: &mut BTreeMap<FileIdentityParts, DailyFileEvidenceV1Wire>,
    origin_files: &mut BTreeMap<FileIdentityParts, DailyFileOriginV1Wire>,
) -> Result<DerivedCounts, DailyWireErrorV1> {
    let mut counts = derive_evidence_counts(daily_sources, full_files, origin_files)?;
    counts.session_count =
        u64::try_from(total_session_count).map_err(|_| DailyWireErrorV1::LimitExceeded)?;
    counts.bar_count = counts
        .session_count
        .checked_mul(11)
        .ok_or(DailyWireErrorV1::LimitExceeded)?;
    Ok(counts)
}

fn validate_descriptor_name(name: &str, expected: &str) -> Result<(), DailyWireErrorV1> {
    invalid_if(name != expected, DailyWireErrorV1::InvalidLiteral)
}

fn validate_manifest_payload(
    value: &DailyPriceManifestDraftV1Wire,
) -> Result<DerivedCounts, DailyWireErrorV1> {
    invalid_if(
        value.schema != MANIFEST_SCHEMA,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.capability != "PRICE_RETURN_ONLY",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.audience != "OWNER_ONLY",
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(!value.vendor_snapshot, DailyWireErrorV1::InvalidLiteral)?;
    invalid_if(value.strict_pit, DailyWireErrorV1::InvalidLiteral)?;
    validate_base(&value.base)?;
    validate_instruments(&value.instruments)?;
    invalid_if(
        !valid_domain_date(value.range_start) || !valid_domain_date(value.range_end),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(
        !valid_commit(value.code_commit.as_str()),
        DailyWireErrorV1::InvalidCommit,
    )?;
    let range_start_expected =
        TradingDate::parse(BASE_START).map_err(|_| DailyWireErrorV1::InvalidDate)?;
    let frontier = TradingDate::parse(BASE_FRONTIER).map_err(|_| DailyWireErrorV1::InvalidDate)?;
    invalid_if(
        value.range_start != range_start_expected || value.range_end <= frontier,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        value.sessions.len() < 3,
        DailyWireErrorV1::InconsistentFields,
    )?;
    check_session_count(value.sessions.len())?;
    strictly_increasing(&value.sessions)?;
    invalid_if(
        value.sessions.iter().any(|date| !valid_domain_date(*date)),
        DailyWireErrorV1::InvalidDate,
    )?;
    invalid_if(
        value.sessions.first().copied() != Some(value.range_start)
            || value.sessions.last().copied() != Some(value.range_end)
            || !value.sessions.contains(&frontier),
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        value.daily_sources.is_empty()
            || value.daily_sources.len()
                != value
                    .sessions
                    .iter()
                    .filter(|date| **date > frontier)
                    .count(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    let source_dates: Vec<_> = value
        .daily_sources
        .iter()
        .map(|source| source.session_date)
        .collect();
    strictly_increasing(&source_dates)?;
    let added_dates: Vec<_> = value
        .sessions
        .iter()
        .copied()
        .filter(|date| *date > frontier)
        .collect();
    invalid_if(
        source_dates != added_dates,
        DailyWireErrorV1::InconsistentFields,
    )?;
    let mut previous = frontier;
    for session in &value.daily_sources {
        invalid_if(
            session.previous_frontier != previous,
            DailyWireErrorV1::InconsistentFields,
        )?;
        previous = session.session_date;
    }
    invalid_if(
        previous != value.range_end,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        value.parent_admission_sha256.is_none() != (value.daily_sources.len() == 1),
        DailyWireErrorV1::InconsistentFields,
    )?;
    validate_descriptor_name(&value.bars.file, "bars.ndjson")?;
    validate_descriptor_name(&value.normalization.file, "normalization.json")?;
    invalid_if(value.bars.size_bytes == 0, DailyWireErrorV1::LimitExceeded)?;
    check_bars_descriptor_size(value.bars.size_bytes)?;
    invalid_if(
        value.normalization.size_bytes == 0,
        DailyWireErrorV1::LimitExceeded,
    )?;
    check_normalization_descriptor_size(value.normalization.size_bytes)?;
    let expected_bars = u64::try_from(value.sessions.len())
        .map_err(|_| DailyWireErrorV1::LimitExceeded)?
        .checked_mul(11)
        .ok_or(DailyWireErrorV1::LimitExceeded)?;
    invalid_if(
        value.bars.row_count != expected_bars,
        DailyWireErrorV1::InconsistentFields,
    )?;
    let expected_documents = value.daily_sources.iter().try_fold(0u64, |sum, source| {
        let calendar_count =
            u64::try_from(source.calendar.len()).map_err(|_| DailyWireErrorV1::LimitExceeded)?;
        sum.checked_add(3)
            .and_then(|value| value.checked_add(calendar_count))
            .ok_or(DailyWireErrorV1::LimitExceeded)
    })?;
    invalid_if(
        value.normalization.row_count != expected_documents,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        value.materializer_version != MATERIALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    let mut full_files = BTreeMap::new();
    let mut origin_files = BTreeMap::new();
    derive_counts(
        value.sessions.len(),
        &value.daily_sources,
        &mut full_files,
        &mut origin_files,
    )
}

fn draft_from_manifest(value: &DailyPriceManifestV1Wire) -> DailyPriceManifestDraftV1Wire {
    DailyPriceManifestDraftV1Wire {
        schema: value.schema.clone(),
        capability: value.capability.clone(),
        audience: value.audience.clone(),
        vendor_snapshot: value.vendor_snapshot,
        strict_pit: value.strict_pit,
        base: value.base.clone(),
        parent_admission_sha256: value.parent_admission_sha256.clone(),
        range_start: value.range_start,
        range_end: value.range_end,
        instruments: value.instruments.clone(),
        sessions: value.sessions.clone(),
        daily_sources: value.daily_sources.clone(),
        bars: value.bars.clone(),
        normalization: value.normalization.clone(),
        policy_sha256: value.policy_sha256.clone(),
        materializer_version: value.materializer_version.clone(),
        code_commit: value.code_commit.clone(),
    }
}

fn manifest_from_draft(
    draft: &DailyPriceManifestDraftV1Wire,
    content_sha256: ContentHash,
) -> DailyPriceManifestV1Wire {
    DailyPriceManifestV1Wire {
        schema: draft.schema.clone(),
        capability: draft.capability.clone(),
        audience: draft.audience.clone(),
        vendor_snapshot: draft.vendor_snapshot,
        strict_pit: draft.strict_pit,
        base: draft.base.clone(),
        parent_admission_sha256: draft.parent_admission_sha256.clone(),
        range_start: draft.range_start,
        range_end: draft.range_end,
        instruments: draft.instruments.clone(),
        sessions: draft.sessions.clone(),
        daily_sources: draft.daily_sources.clone(),
        bars: draft.bars.clone(),
        normalization: draft.normalization.clone(),
        policy_sha256: draft.policy_sha256.clone(),
        materializer_version: draft.materializer_version.clone(),
        code_commit: draft.code_commit.clone(),
        content_sha256,
    }
}

fn validate_draft(draft: &mut DailyPriceManifestDraftV1Wire) -> Result<(), DailyWireErrorV1> {
    canonicalize_draft(draft)?;
    validate_manifest_payload(draft)?;
    Ok(())
}

fn validate_manifest(value: &mut DailyPriceManifestV1Wire) -> Result<(), DailyWireErrorV1> {
    canonicalize_manifest(value)?;
    let draft = draft_from_manifest(value);
    validate_manifest_payload(&draft)?;
    let bytes = canonical_bytes_limited(&draft, fixed_limits().max_manifest_bytes)?;
    invalid_if(
        hash_bytes(&bytes) != value.content_sha256,
        DailyWireErrorV1::HashMismatch,
    )
}

fn checked_mul(left: u64, right: u64) -> Result<u64, DailyWireErrorV1> {
    left.checked_mul(right)
        .ok_or(DailyWireErrorV1::LimitExceeded)
}

fn validate_admission(value: &DailyPriceAdmissionV1Wire) -> Result<(), DailyWireErrorV1> {
    invalid_if(
        value.schema != ADMISSION_SCHEMA,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    validate_base(&value.base)?;
    invalid_if(
        value.normalizer_version != NORMALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        !valid_commit(value.verifier_code_commit.as_str()),
        DailyWireErrorV1::InvalidCommit,
    )?;
    let counts = &value.counts;
    invalid_if(
        counts.instrument_count != 11,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        counts.session_count < 3,
        DailyWireErrorV1::InconsistentFields,
    )?;
    check_limit(counts.session_count, fixed_limits().max_sessions)?;
    invalid_if(
        counts.bar_count != checked_mul(11, counts.session_count)?,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        counts.added_session_count == 0
            || counts.added_session_count
                > counts
                    .session_count
                    .checked_sub(2)
                    .ok_or(DailyWireErrorV1::InconsistentFields)?,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        counts.calendar_row_count < counts.added_session_count
            || counts.calendar_row_count
                > checked_mul(
                    fixed_limits().max_civil_dates_per_addition,
                    counts.added_session_count,
                )?,
        if counts.calendar_row_count
            > checked_mul(
                fixed_limits().max_civil_dates_per_addition,
                counts.added_session_count,
            )?
        {
            DailyWireErrorV1::LimitExceeded
        } else {
            DailyWireErrorV1::InconsistentFields
        },
    )?;
    invalid_if(
        counts.action_chain_count < checked_mul(7, counts.added_session_count)?
            || counts.action_chain_count
                > checked_mul(
                    fixed_limits().max_action_chains_per_session,
                    counts.added_session_count,
                )?,
        if counts.action_chain_count
            > checked_mul(
                fixed_limits().max_action_chains_per_session,
                counts.added_session_count,
            )?
        {
            DailyWireErrorV1::LimitExceeded
        } else {
            DailyWireErrorV1::InconsistentFields
        },
    )?;
    invalid_if(
        counts.action_page_count < counts.action_chain_count
            || counts.action_page_count
                > checked_mul(
                    fixed_limits().max_pages_per_action_chain,
                    counts.action_chain_count,
                )?,
        if counts.action_page_count
            > checked_mul(
                fixed_limits().max_pages_per_action_chain,
                counts.action_chain_count,
            )?
        {
            DailyWireErrorV1::LimitExceeded
        } else {
            DailyWireErrorV1::InconsistentFields
        },
    )?;
    invalid_if(
        counts.source_file_count == 0
            || counts.source_file_count
                > checked_mul(
                    fixed_limits().max_file_evidence_per_session,
                    counts.added_session_count,
                )?,
        if counts.source_file_count
            > checked_mul(
                fixed_limits().max_file_evidence_per_session,
                counts.added_session_count,
            )?
        {
            DailyWireErrorV1::LimitExceeded
        } else {
            DailyWireErrorV1::InconsistentFields
        },
    )?;
    invalid_if(
        counts.cash_dividend_row_count
            > checked_mul(
                fixed_limits().max_cash_rows_per_chain,
                counts.action_chain_count,
            )?,
        DailyWireErrorV1::LimitExceeded,
    )?;
    invalid_if(
        counts.normalized_document_count
            != counts
                .added_session_count
                .checked_mul(3)
                .and_then(|value| value.checked_add(counts.calendar_row_count))
                .ok_or(DailyWireErrorV1::LimitExceeded)?,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        value.parent_admission_sha256.is_none() != (counts.added_session_count == 1),
        DailyWireErrorV1::InconsistentFields,
    )
}

fn validate_proof(value: &mut DailyNormalizationProofV1Wire) -> Result<(), DailyWireErrorV1> {
    canonicalize_proof(value);
    invalid_if(
        value.schema != NORMALIZATION_SCHEMA,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        value.normalizer_version != NORMALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    let mut prior: Option<(TradingDate, &'static str)> = None;
    for document in &value.documents {
        let sort_key = (document.session_date, document.kind.literal());
        if let Some(previous) = prior {
            match previous.cmp(&sort_key) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => return Err(DailyWireErrorV1::DuplicateEntry),
                std::cmp::Ordering::Greater => return Err(DailyWireErrorV1::InvalidOrder),
            }
        }
        prior = Some(sort_key);
        invalid_if(
            !valid_domain_date(document.session_date),
            DailyWireErrorV1::InvalidDate,
        )?;
        invalid_if(
            document.normalizer_version != NORMALIZER_VERSION,
            DailyWireErrorV1::InvalidLiteral,
        )?;
        invalid_if(document.size_bytes == 0, DailyWireErrorV1::LimitExceeded)?;
        check_normalized_document_size(document.size_bytes)?;
        invalid_if(
            document.source_files.is_empty(),
            DailyWireErrorV1::InconsistentFields,
        )?;
        validate_key_set(&document.source_files)?;
        let commitment = DailyNormalizedSourceCommitmentV1Wire {
            schema: NORMALIZED_SOURCE_SCHEMA.to_owned(),
            kind: document.kind,
            session_date: document.session_date,
            source_files: document.source_files.clone(),
        };
        let bytes =
            canonical_bytes_limited(&commitment, fixed_limits().max_normalized_document_bytes)?;
        invalid_if(
            hash_bytes(&bytes) != document.source_evidence_sha256,
            DailyWireErrorV1::HashMismatch,
        )?;
    }
    Ok(())
}

fn admission_counts(derived: DerivedCounts) -> DailyAdmissionCountsV1Wire {
    DailyAdmissionCountsV1Wire {
        instrument_count: derived.instrument_count,
        session_count: derived.session_count,
        bar_count: derived.bar_count,
        added_session_count: derived.added_session_count,
        source_file_count: derived.source_file_count,
        calendar_row_count: derived.calendar_row_count,
        action_chain_count: derived.action_chain_count,
        action_page_count: derived.action_page_count,
        cash_dividend_row_count: derived.cash_dividend_row_count,
        normalized_document_count: derived.normalized_document_count,
    }
}

/// Returns the exact canonical preimage for a policy's content hash.
pub fn encode_daily_price_policy_v1(
    value: &DailyPricePolicyV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    preflight_policy(value)?;
    let mut value = value.clone();
    validate_policy_and_bytes(&mut value)
}

/// Parses an unapproved policy value with duplicate-aware JSON validation.
pub fn parse_daily_price_policy_v1(
    bytes: &[u8],
) -> Result<DailyPricePolicyV1Wire, DailyWireErrorV1> {
    let mut value: DailyPricePolicyV1Wire =
        parse_typed(bytes, fixed_limits().max_policy_bytes as usize, "policy")?;
    canonicalize_policy(&mut value);
    validate_policy(&value)?;
    let encoded = canonical_bytes_limited(&value, fixed_limits().max_policy_bytes)?;
    invalid_if(
        encoded.len() > fixed_limits().max_policy_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(value)
}

/// Hashes canonical policy bytes; the result is not a trusted policy pin.
pub fn hash_daily_price_policy_v1(
    value: &DailyPricePolicyV1Wire,
) -> Result<ContentHash, DailyWireErrorV1> {
    Ok(hash_bytes(&encode_daily_price_policy_v1(value)?))
}

/// Returns canonical manifest bytes after checking its self-hash.
pub fn encode_daily_price_manifest_v1(
    value: &DailyPriceManifestV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    preflight_manifest(&value.instruments, &value.sessions, &value.daily_sources)?;
    let mut value = value.clone();
    validate_manifest(&mut value)?;
    let bytes = canonical_bytes_limited(&value, fixed_limits().max_manifest_bytes)?;
    invalid_if(
        bytes.len() > fixed_limits().max_manifest_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(bytes)
}

/// Parses an unapproved manifest, canonicalizing set arrays before hash checks.
pub fn parse_daily_price_manifest_v1(
    bytes: &[u8],
) -> Result<DailyPriceManifestV1Wire, DailyWireErrorV1> {
    let mut value: DailyPriceManifestV1Wire = parse_typed(
        bytes,
        fixed_limits().max_manifest_bytes as usize,
        "manifest",
    )?;
    validate_manifest(&mut value)?;
    let encoded = canonical_bytes_limited(&value, fixed_limits().max_manifest_bytes)?;
    invalid_if(
        encoded.len() > fixed_limits().max_manifest_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(value)
}

/// Encodes the manifest draft preimage, omitting only the final content hash.
pub fn daily_price_content_preimage_v1(
    value: &DailyPriceManifestDraftV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    preflight_manifest(&value.instruments, &value.sessions, &value.daily_sources)?;
    let mut value = value.clone();
    validate_draft(&mut value)?;
    let bytes = canonical_bytes_limited(&value, fixed_limits().max_manifest_bytes)?;
    invalid_if(
        bytes.len() > fixed_limits().max_manifest_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(bytes)
}

/// Adds a self-consistency hash to an unapproved draft; this does not admit it.
pub fn seal_daily_price_manifest_v1(
    value: &DailyPriceManifestDraftV1Wire,
) -> Result<DailyPriceManifestV1Wire, DailyWireErrorV1> {
    let preimage = daily_price_content_preimage_v1(value)?;
    let mut result = manifest_from_draft(value, hash_bytes(&preimage));
    validate_manifest(&mut result)?;
    canonical_bytes_limited(&result, fixed_limits().max_manifest_bytes)?;
    Ok(result)
}

/// Hashes the complete canonical manifest, after checking its content hash.
pub fn hash_daily_price_manifest_v1(
    value: &DailyPriceManifestV1Wire,
) -> Result<ContentHash, DailyWireErrorV1> {
    Ok(hash_bytes(&encode_daily_price_manifest_v1(value)?))
}

/// Parses a locally bounded, unapproved admission claim.
pub fn parse_daily_price_admission_v1(
    bytes: &[u8],
) -> Result<DailyPriceAdmissionV1Wire, DailyWireErrorV1> {
    let value: DailyPriceAdmissionV1Wire = parse_typed(
        bytes,
        fixed_limits().max_admission_bytes as usize,
        "admission",
    )?;
    validate_admission(&value)?;
    Ok(value)
}

/// Returns canonical admission bytes after local count validation.
pub fn encode_daily_price_admission_v1(
    value: &DailyPriceAdmissionV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    validate_admission(value)?;
    let bytes = canonical_bytes_limited(value, fixed_limits().max_admission_bytes)?;
    invalid_if(
        bytes.len() > fixed_limits().max_admission_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(bytes)
}

/// Hashes canonical admission bytes; parsing or hashing grants no authority.
pub fn hash_daily_price_admission_v1(
    value: &DailyPriceAdmissionV1Wire,
) -> Result<ContentHash, DailyWireErrorV1> {
    Ok(hash_bytes(&encode_daily_price_admission_v1(value)?))
}

/// Builds the evidence-commitment preimage from structurally valid sessions.
pub fn daily_price_evidence_preimage_v1(
    value: &[DailySessionEvidenceV1Wire],
) -> Result<Vec<u8>, DailyWireErrorV1> {
    let sessions = canonicalize_evidence(value)?;
    let mut full_files = BTreeMap::new();
    let mut origins = BTreeMap::new();
    derive_evidence_counts(&sessions, &mut full_files, &mut origins)?;
    let commitment = DailyEvidenceCommitmentV1Wire {
        schema: EVIDENCE_SCHEMA.to_owned(),
        daily_sources: sessions,
    };
    let bytes = canonical_bytes_limited(&commitment, fixed_limits().max_manifest_bytes)?;
    invalid_if(
        bytes.len() > fixed_limits().max_manifest_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(bytes)
}

/// Builds the source-file commitment preimage for one normalized document.
pub fn daily_normalized_source_preimage_v1(
    value: &DailyNormalizedSourceCommitmentV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    invalid_if(
        value.schema != NORMALIZED_SOURCE_SCHEMA,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        !valid_domain_date(value.session_date),
        DailyWireErrorV1::InvalidDate,
    )?;
    check_file_evidence_count(value.source_files.len())?;
    let mut value = value.clone();
    canonicalize_key_set(&mut value.source_files);
    invalid_if(
        value.source_files.is_empty(),
        DailyWireErrorV1::InconsistentFields,
    )?;
    validate_key_set(&value.source_files)?;
    let bytes = canonical_bytes_limited(&value, fixed_limits().max_normalized_document_bytes)?;
    invalid_if(
        bytes.len() > fixed_limits().max_normalized_document_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(bytes)
}

/// Returns the canonical sorted cash-witness preimage (`[]` when empty).
pub fn daily_cash_dividend_rows_preimage_v1(
    value: &[DailyCashDividendRowV1Wire],
) -> Result<Vec<u8>, DailyWireErrorV1> {
    check_cash_row_count(value.len())?;
    for row in value {
        validate_cash_row_scalars(row)?;
    }
    let mut keyed = value
        .iter()
        .cloned()
        .map(|row| Ok((cash_sort_key(&row)?, row)))
        .collect::<Result<Vec<_>, DailyWireErrorV1>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    for pair in keyed.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(DailyWireErrorV1::DuplicateEntry);
        }
    }
    let rows: Vec<_> = keyed.into_iter().map(|(_, row)| row).collect();
    canonical_bytes_limited(&rows, fixed_limits().max_manifest_bytes)
}

/// Parses and canonicalizes the proof carried by `normalization.json`.
pub fn parse_daily_normalization_proof_v1(
    bytes: &[u8],
) -> Result<DailyNormalizationProofV1Wire, DailyWireErrorV1> {
    let mut value: DailyNormalizationProofV1Wire = parse_typed(
        bytes,
        fixed_limits().max_normalization_bytes as usize,
        "proof",
    )?;
    validate_proof(&mut value)?;
    let encoded = canonical_bytes_limited(&value, fixed_limits().max_normalization_bytes)?;
    invalid_if(
        encoded.len() > fixed_limits().max_normalization_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(value)
}

/// Encodes a canonical normalization proof without claiming document truth.
pub fn encode_daily_normalization_proof_v1(
    value: &DailyNormalizationProofV1Wire,
) -> Result<Vec<u8>, DailyWireErrorV1> {
    preflight_proof(value)?;
    let mut value = value.clone();
    validate_proof(&mut value)?;
    let bytes = canonical_bytes_limited(&value, fixed_limits().max_normalization_bytes)?;
    invalid_if(
        bytes.len() > fixed_limits().max_normalization_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    Ok(bytes)
}

fn flattened_documents(
    sessions: &[DailySessionEvidenceV1Wire],
) -> Result<Vec<DailyNormalizedDocumentV1Wire>, DailyWireErrorV1> {
    let mut by_key = BTreeMap::new();
    for document in sessions
        .iter()
        .flat_map(|session| session.normalized_documents.iter())
    {
        let key = (document.session_date, document.kind.literal());
        if let Some(previous) = by_key.get(&key) {
            invalid_if(previous != document, DailyWireErrorV1::InconsistentFields)?;
        } else {
            by_key.insert(key, document.clone());
        }
    }
    Ok(by_key.into_values().collect())
}

/// Verifies internal links among unapproved policy, manifest, proof, admission,
/// and pins. It does not inspect external bytes or grant trust/approval.
pub fn validate_daily_price_wire_bundle_v1(
    policy: &DailyPricePolicyV1Wire,
    manifest: &DailyPriceManifestV1Wire,
    proof: &DailyNormalizationProofV1Wire,
    admission: &DailyPriceAdmissionV1Wire,
    pins: &DailyPricePinsV1Wire,
) -> Result<(), DailyWireErrorV1> {
    let policy_bytes = encode_daily_price_policy_v1(policy)?;
    let policy_sha256 = hash_bytes(&policy_bytes);
    let manifest_bytes = encode_daily_price_manifest_v1(manifest)?;
    let manifest_sha256 = hash_bytes(&manifest_bytes);
    preflight_proof(proof)?;
    let mut canonical_proof = proof.clone();
    validate_proof(&mut canonical_proof)?;
    let proof_bytes =
        canonical_bytes_limited(&canonical_proof, fixed_limits().max_normalization_bytes)?;
    invalid_if(
        proof_bytes.len() > fixed_limits().max_normalization_bytes as usize,
        DailyWireErrorV1::LimitExceeded,
    )?;
    let admission_bytes = encode_daily_price_admission_v1(admission)?;
    let admission_sha256 = hash_bytes(&admission_bytes);

    let mut canonical_manifest = manifest.clone();
    validate_manifest(&mut canonical_manifest)?;
    let mut canonical_policy = policy.clone();
    canonicalize_policy(&mut canonical_policy);
    let canonical_admission = admission.clone();
    validate_admission(&canonical_admission)?;

    invalid_if(
        canonical_policy.base != canonical_manifest.base
            || canonical_admission.base != canonical_manifest.base,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        canonical_manifest.policy_sha256 != policy_sha256
            || canonical_admission.policy_sha256 != policy_sha256,
        DailyWireErrorV1::HashMismatch,
    )?;
    invalid_if(
        canonical_manifest.instruments != canonical_policy.instruments,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        canonical_admission.content_sha256 != canonical_manifest.content_sha256
            || canonical_admission.manifest_sha256 != manifest_sha256
            || canonical_admission.parent_admission_sha256
                != canonical_manifest.parent_admission_sha256,
        DailyWireErrorV1::HashMismatch,
    )?;
    invalid_if(
        canonical_admission.normalizer_version != NORMALIZER_VERSION,
        DailyWireErrorV1::InvalidLiteral,
    )?;
    invalid_if(
        canonical_manifest.normalization.sha256 != hash_bytes(&proof_bytes)
            || canonical_manifest.normalization.size_bytes
                != u64::try_from(proof_bytes.len()).map_err(|_| DailyWireErrorV1::LimitExceeded)?
            || canonical_manifest.normalization.row_count
                != u64::try_from(canonical_proof.documents.len())
                    .map_err(|_| DailyWireErrorV1::LimitExceeded)?,
        DailyWireErrorV1::HashMismatch,
    )?;

    let expected_documents = flattened_documents(&canonical_manifest.daily_sources)?;
    invalid_if(
        canonical_proof.documents != expected_documents,
        DailyWireErrorV1::InconsistentFields,
    )?;
    let evidence = daily_price_evidence_preimage_v1(&canonical_manifest.daily_sources)?;
    let evidence_sha256 = hash_bytes(&evidence);
    invalid_if(
        canonical_proof.evidence_sha256 != evidence_sha256
            || canonical_admission.evidence_sha256 != evidence_sha256,
        DailyWireErrorV1::HashMismatch,
    )?;

    let mut full_files = BTreeMap::new();
    let mut origin_files = BTreeMap::new();
    let derived = derive_counts(
        canonical_manifest.sessions.len(),
        &canonical_manifest.daily_sources,
        &mut full_files,
        &mut origin_files,
    )?;
    invalid_if(
        admission_counts(derived) != canonical_admission.counts,
        DailyWireErrorV1::InconsistentFields,
    )?;
    invalid_if(
        pins.content_sha256 != canonical_manifest.content_sha256
            || pins.manifest_sha256 != manifest_sha256
            || pins.admission_sha256 != admission_sha256
            || pins.policy_sha256 != policy_sha256,
        DailyWireErrorV1::HashMismatch,
    )
}

#[cfg(test)]
mod resource_guard_tests {
    use super::*;

    type CollectionGuard = (&'static str, u64, fn(usize) -> Result<(), DailyWireErrorV1>);
    type DeclaredSizeGuard = (&'static str, u64, fn(u64) -> Result<(), DailyWireErrorV1>);

    #[test]
    fn every_collection_guard_accepts_its_bound_and_rejects_one_more() {
        let limits = fixed_limits();
        let guards: [CollectionGuard; 7] = [
            ("max_sessions", limits.max_sessions, check_session_count),
            (
                "max_civil_dates_per_addition",
                limits.max_civil_dates_per_addition,
                check_civil_date_count,
            ),
            (
                "max_action_chains_per_session",
                limits.max_action_chains_per_session,
                check_action_chain_count,
            ),
            (
                "max_file_evidence_per_session",
                limits.max_file_evidence_per_session,
                check_file_evidence_count,
            ),
            (
                "max_copy_depth",
                limits.max_copy_depth,
                check_copy_hop_count,
            ),
            (
                "max_pages_per_action_chain",
                limits.max_pages_per_action_chain,
                check_action_page_count,
            ),
            (
                "max_cash_rows_per_chain",
                limits.max_cash_rows_per_chain,
                check_cash_row_count,
            ),
        ];

        for (name, maximum, guard) in guards {
            let maximum_len = usize::try_from(maximum).unwrap();
            assert_eq!(guard(maximum_len), Ok(()), "{name} at bound",);
            assert_eq!(
                guard(maximum_len + 1),
                Err(DailyWireErrorV1::LimitExceeded),
                "{name} above bound",
            );
        }
    }

    #[test]
    fn every_declared_size_guard_accepts_its_bound_and_rejects_one_more() {
        let limits = fixed_limits();
        let guards: [DeclaredSizeGuard; 4] = [
            (
                "max_source_file_bytes",
                limits.max_source_file_bytes,
                check_source_file_size,
            ),
            (
                "max_normalized_document_bytes",
                limits.max_normalized_document_bytes,
                check_normalized_document_size,
            ),
            (
                "max_bars_bytes",
                limits.max_bars_bytes,
                check_bars_descriptor_size,
            ),
            (
                "max_normalization_bytes",
                limits.max_normalization_bytes,
                check_normalization_descriptor_size,
            ),
        ];
        for (name, maximum, guard) in guards {
            assert_eq!(guard(maximum), Ok(()), "{name} at bound");
            assert_eq!(
                guard(maximum + 1),
                Err(DailyWireErrorV1::LimitExceeded),
                "{name} above bound",
            );
        }
    }

    #[test]
    fn bounded_json_depth_and_civil_dates_accept_the_bound_and_reject_plus_one() {
        let depth_limit = usize::try_from(fixed_limits().max_json_depth).unwrap();
        for (depth, expected) in [
            (depth_limit, Ok(())),
            (depth_limit + 1, Err(DailyWireErrorV1::LimitExceeded)),
        ] {
            let json = format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(
                parse_json_bytes(&json.into_bytes(), 1024).map(|_| ()),
                expected,
                "JSON container depth {depth}",
            );
        }

        let first = TradingDate::new(2026, 1, 1).unwrap();
        let at_bound = first.checked_add_days(369).unwrap();
        assert_eq!(date_range(first, at_bound).unwrap().len(), 370);
        let above_bound = first.checked_add_days(370).unwrap();
        assert_eq!(
            date_range(first, above_bound),
            Err(DailyWireErrorV1::LimitExceeded),
        );
    }

    #[test]
    fn json_array_evidence_count_guard_accepts_its_bound_and_rejects_one_more() {
        fn scan_numeric_array(count: usize) -> bool {
            let mut bytes = Vec::with_capacity(count.saturating_mul(2).saturating_add(2));
            bytes.push(b'[');
            for index in 0..count {
                if index > 0 {
                    bytes.push(b',');
                }
                bytes.push(b'0');
            }
            bytes.push(b']');
            let mut scanner = JsonScanner {
                bytes: &bytes,
                cursor: 0,
                duplicate_key: false,
                invalid_number: false,
                overlarge_array: false,
            };
            scanner.value(0).unwrap();
            scanner.overlarge_array
        }

        let maximum = usize::try_from(fixed_limits().max_file_evidence_per_session).unwrap();
        assert!(!scan_numeric_array(maximum));
        assert!(scan_numeric_array(maximum + 1));
    }

    #[test]
    fn bounded_json_input_and_canonical_output_allow_exact_bytes_only() {
        assert_eq!(parse_json_bytes(b"null", 4), Ok(Value::Null));
        assert_eq!(
            parse_json_bytes(b"null ", 4),
            Err(DailyWireErrorV1::LimitExceeded),
        );
        assert_eq!(
            canonical_bytes_limited(&"xxxxxx", 8),
            Ok(b"\"xxxxxx\"".to_vec())
        );
        assert_eq!(
            canonical_bytes_limited(&"xxxxxx", 7),
            Err(DailyWireErrorV1::LimitExceeded),
        );

        // Legal policy/admission values cannot approach their fixed byte bounds.
        // Isolate the writer for those bounds; the public encoders retain their
        // legal success regressions in the integration tests.
        let limits = fixed_limits();
        for (name, actual_limit, fixed_value) in [
            ("max_policy_bytes", limits.max_policy_bytes, 262_144),
            ("max_manifest_bytes", limits.max_manifest_bytes, 16_777_216),
            ("max_admission_bytes", limits.max_admission_bytes, 1_048_576),
            (
                "max_normalization_bytes",
                limits.max_normalization_bytes,
                16_777_216,
            ),
        ] {
            assert_eq!(actual_limit, fixed_value, "{name} fixed value");
            let limit = usize::try_from(actual_limit).unwrap();
            let exact = canonical_bytes_limited(&"x".repeat(limit - 2), actual_limit).unwrap();
            assert_eq!(exact.len(), limit, "{name} at bound");
            drop(exact);
            assert_eq!(
                canonical_bytes_limited(&"x".repeat(limit - 1), actual_limit),
                Err(DailyWireErrorV1::LimitExceeded),
                "{name} above bound",
            );
        }
    }
}
