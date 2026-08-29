//! Fixed owner-only original-price daily-bar artifact, bound to immutable Raw evidence.
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const FIXED_STOCK_PRICE_BETA_SCHEMA_ID: &str = "kr-stock-price-beta-daily-bars";
pub const FIXED_STOCK_PRICE_BETA_SCHEMA_VERSION: u32 = 1;
pub const FIXED_STOCK_PRICE_BETA_UNIVERSE_ID: &str = "kr-stock-price-beta-v1";
pub const FIXED_30_ID_LIST_SHA256: &str =
    "0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79";
pub const FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256: &str =
    "2a0d55143df0274fcfa357f2824ed752e2969469f93254ed7dfa64766a00dde1";
pub const FIXED_STOCK_PRICE_BETA_RANGE_START: &str = "2025-08-04";
pub const FIXED_STOCK_PRICE_BETA_RANGE_END: &str = "2026-08-28";
pub const FIXED_STOCK_PRICE_BETA_MIN_COMMON_SESSIONS: usize = 120;
pub const FIXED_STOCK_PRICE_BETA_RAW_CONTRACT_VERSION: u32 = 1;
pub const FIXED_STOCK_PRICE_BETA_RAW_PROVIDER_SCOPE: &str =
    "kis-fixed-stock-price-beta-daily-bars-raw-v1";
pub const KIS_DAILY_BARS_PATH: &str =
    "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
pub const KIS_DAILY_BARS_TR_ID: &str = "FHKST03010100";
pub const KIS_FID_ORG_ADJ_PRC: &str = "1";
pub const ORIGINAL_PRICE_WARNING: &str = "Original (unadjusted) prices are used. Corporate actions can distort returns and drawdown; adjusted-return continuity is not claimed.";
pub const FIXED_30_INSTRUMENT_IDS: [&str; 30] = [
    "005930.KRX",
    "000660.KRX",
    "373220.KRX",
    "207940.KRX",
    "005380.KRX",
    "000270.KRX",
    "105560.KRX",
    "055550.KRX",
    "068270.KRX",
    "035420.KRX",
    "035720.KRX",
    "005490.KRX",
    "051910.KRX",
    "006400.KRX",
    "012330.KRX",
    "028260.KRX",
    "012450.KRX",
    "329180.KRX",
    "034020.KRX",
    "015760.KRX",
    "017670.KRX",
    "030200.KRX",
    "066570.KRX",
    "009150.KRX",
    "096770.KRX",
    "036570.KRX",
    "090430.KRX",
    "011200.KRX",
    "003490.KRX",
    "000810.KRX",
];
pub const FIXED_30_INSTRUMENT_NAMES: [&str; 30] = [
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedStockPriceBetaUniverse {
    pub universe_id: String,
    pub file_sha256: String,
    pub instruments: Vec<FixedStockPriceBetaUniverseInstrument>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedStockPriceBetaUniverseInstrument {
    pub id: String,
    pub name: String,
}
pub fn parse_fixed_stock_price_beta_universe(
    bytes: &[u8],
) -> Result<FixedStockPriceBetaUniverse, FixedStockPriceBetaError> {
    if hash(bytes) != FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256 {
        return Err(FixedStockPriceBetaError::Invalid(
            "universe file hash mismatch",
        ));
    }
    #[derive(Deserialize)]
    struct F {
        universe_id: String,
        audience: String,
        capability: String,
        vendor_snapshot: bool,
        strict_pit: bool,
        instrument_count: usize,
        selection_basis: String,
        instruments: Vec<I>,
    }
    #[derive(Deserialize)]
    struct I {
        id: String,
        name: String,
    }
    let f: F = serde_json::from_slice(bytes)
        .map_err(|_| FixedStockPriceBetaError::Invalid("malformed universe file"))?;
    if f.universe_id != FIXED_STOCK_PRICE_BETA_UNIVERSE_ID
        || f.audience != "OWNER_ONLY"
        || f.capability != "PRICE_VOLUME_RESEARCH_ONLY"
        || !f.vendor_snapshot
        || f.strict_pit
        || !f.selection_basis.contains("KOSPI 200")
        || f.instrument_count != 30
        || f.instruments.len() != 30
    {
        return Err(FixedStockPriceBetaError::Invalid(
            "universe claims do not match fixed contract",
        ));
    }
    if f.instruments
        .iter()
        .enumerate()
        .any(|(i, v)| v.id != FIXED_30_INSTRUMENT_IDS[i] || v.name != FIXED_30_INSTRUMENT_NAMES[i])
    {
        return Err(FixedStockPriceBetaError::Invalid(
            "universe instrument order, ID, or name mismatch",
        ));
    }
    Ok(FixedStockPriceBetaUniverse {
        universe_id: f.universe_id,
        file_sha256: FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256.into(),
        instruments: f
            .instruments
            .into_iter()
            .map(|v| FixedStockPriceBetaUniverseInstrument {
                id: v.id,
                name: v.name,
            })
            .collect(),
    })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyBar {
    pub instrument_id: String,
    pub date: String,
    pub open: i64,
    pub high: i64,
    pub low: i64,
    pub close: i64,
    pub volume: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedStockPriceBetaRawWindow {
    pub window_id: String,
    pub range_start: String,
    pub range_end: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedStockPriceBetaRawFileEvidence {
    pub relative_path: String,
    pub instrument_id: String,
    pub window_id: String,
    pub page_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub method: String,
    pub path: String,
    pub tr_id: String,
    pub query_symbol: String,
    pub query_range_start: String,
    pub query_range_end: String,
    pub fid_org_adj_prc: String,
    pub response_continuation: String,
}
/// Immutable Raw metadata; deliberately has no HTTP-status field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedStockPriceBetaRawBatchEvidence {
    pub contract_version: u32,
    pub provider_scope: String,
    pub requested_range_start: String,
    pub requested_range_end: String,
    pub entitlement_reference: String,
    pub entitlement_sha256: String,
    pub capture_commit: String,
    pub batch_json_sha256: String,
    pub manifest_sha256: String,
    pub windows: Vec<FixedStockPriceBetaRawWindow>,
    pub files: Vec<FixedStockPriceBetaRawFileEvidence>,
}
/// Body paired one-to-one with an evidence file. The path is an identity, never opened here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedStockPriceBetaRawSourceFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedStockPriceBetaArtifact {
    pub schema_id: String,
    pub schema_version: u32,
    pub universe_id: String,
    pub universe_file_sha256: String,
    pub audience: String,
    pub capability: String,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub selection_basis: String,
    pub index_membership: String,
    pub range_start: String,
    pub range_end: String,
    pub original_price: bool,
    pub warning: String,
    pub evidence: FixedStockPriceBetaRawBatchEvidence,
    pub instruments: Vec<String>,
    pub sessions: Vec<String>,
    pub bars: Vec<DailyBar>,
    pub content_sha256: String,
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FixedStockPriceBetaError {
    #[error("invalid fixed stock price beta input: {0}")]
    Invalid(&'static str),
    #[error("source bytes do not match committed Raw evidence")]
    SourceTampered,
    #[error("immutable artifact conflict")]
    Conflict,
    #[error("artifact is missing or tampered")]
    Tampered,
    #[error("artifact path is unsafe")]
    UnsafePath,
    #[error("platform cannot provide descriptor-safe artifact I/O")]
    UnsupportedPlatform,
    #[error("I/O failure")]
    Io,
    #[error("serialization failure")]
    Serialize,
}

impl FixedStockPriceBetaArtifact {
    pub fn build(
        universe_bytes: &[u8],
        evidence: FixedStockPriceBetaRawBatchEvidence,
        sources: Vec<FixedStockPriceBetaRawSourceFile>,
        bars: Vec<DailyBar>,
    ) -> Result<Self, FixedStockPriceBetaError> {
        let u = parse_fixed_stock_price_beta_universe(universe_bytes)?;
        evidence_ok(&evidence, Some(&sources))?;
        // Generic construction retains the original API. Trust boundaries use
        // `verify_against_raw_sources`, which re-parses provider bytes and is
        // mandatory in the provider-free materializer and approval check.
        let (s, b) = bars_ok(&bars)?;
        Self::new(&u, evidence, s, b)
    }
    /// Reopens the immutable source bodies and derives the bars again.  This is
    /// required at every trust boundary (materialization and approval reads).
    pub fn verify_against_raw_sources(
        &self,
        universe_bytes: &[u8],
        sources: &[FixedStockPriceBetaRawSourceFile],
    ) -> Result<(), FixedStockPriceBetaError> {
        parse_fixed_stock_price_beta_universe(universe_bytes)?;
        self.verify()?;
        let parsed = parse_fixed_stock_price_beta_raw_sources(&self.evidence, sources)?;
        if self.bars != parsed {
            return Err(FixedStockPriceBetaError::SourceTampered);
        }
        Ok(())
    }
    fn new(
        u: &FixedStockPriceBetaUniverse,
        evidence: FixedStockPriceBetaRawBatchEvidence,
        sessions: Vec<String>,
        bars: Vec<DailyBar>,
    ) -> Result<Self, FixedStockPriceBetaError> {
        let mut a = Self {
            schema_id: FIXED_STOCK_PRICE_BETA_SCHEMA_ID.into(),
            schema_version: FIXED_STOCK_PRICE_BETA_SCHEMA_VERSION,
            universe_id: u.universe_id.clone(),
            universe_file_sha256: u.file_sha256.clone(),
            audience: "OWNER_ONLY".into(),
            capability: "PRICE_VOLUME_RESEARCH_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            selection_basis: "CONFIGURED_FIXED_LIST".into(),
            index_membership: "NOT_EVALUATED".into(),
            range_start: FIXED_STOCK_PRICE_BETA_RANGE_START.into(),
            range_end: FIXED_STOCK_PRICE_BETA_RANGE_END.into(),
            original_price: true,
            warning: ORIGINAL_PRICE_WARNING.into(),
            evidence,
            instruments: FIXED_30_INSTRUMENT_IDS
                .iter()
                .map(|x| (*x).into())
                .collect(),
            sessions,
            bars,
            content_sha256: String::new(),
        };
        a.content_sha256 = a.compute_hash()?;
        Ok(a)
    }
    /// Reconstructs sessions, sorted bars, exact metadata and evidence. Source bodies
    /// are checked during build and only their immutable commitments are retained.
    pub fn verify(&self) -> Result<(), FixedStockPriceBetaError> {
        evidence_ok(&self.evidence, None).map_err(|_| FixedStockPriceBetaError::Tampered)?;
        let (s, b) = bars_ok(&self.bars).map_err(|_| FixedStockPriceBetaError::Tampered)?;
        let u = FixedStockPriceBetaUniverse {
            universe_id: FIXED_STOCK_PRICE_BETA_UNIVERSE_ID.into(),
            file_sha256: FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256.into(),
            instruments: FIXED_30_INSTRUMENT_IDS
                .iter()
                .zip(FIXED_30_INSTRUMENT_NAMES)
                .map(|(id, n)| FixedStockPriceBetaUniverseInstrument {
                    id: (*id).into(),
                    name: (*n).into(),
                })
                .collect(),
        };
        let x = Self::new(&u, self.evidence.clone(), s, b)
            .map_err(|_| FixedStockPriceBetaError::Tampered)?;
        if &x != self
            || self
                .compute_hash()
                .map_err(|_| FixedStockPriceBetaError::Tampered)?
                != self.content_sha256
        {
            Err(FixedStockPriceBetaError::Tampered)
        } else {
            Ok(())
        }
    }
    pub fn compute_hash(&self) -> Result<String, FixedStockPriceBetaError> {
        let mut x = self.clone();
        x.content_sha256.clear();
        serde_json::to_vec(&x)
            .map(|b| hash(&b))
            .map_err(|_| FixedStockPriceBetaError::Serialize)
    }
}

/// Provider-free KIS daily-bars decoding.  It intentionally exposes no
/// response prose: callers receive a typed invariant failure only.
pub fn parse_fixed_stock_price_beta_raw_sources(
    evidence: &FixedStockPriceBetaRawBatchEvidence,
    sources: &[FixedStockPriceBetaRawSourceFile],
) -> Result<Vec<DailyBar>, FixedStockPriceBetaError> {
    evidence_ok(evidence, Some(sources))?;
    let by_path: BTreeMap<_, _> = sources
        .iter()
        .map(|s| (s.relative_path.as_str(), s))
        .collect();
    let windows: BTreeMap<_, _> = evidence
        .windows
        .iter()
        .map(|w| (w.window_id.as_str(), w))
        .collect();
    let mut bars = Vec::new();
    let mut seen = BTreeSet::new();
    for file in &evidence.files {
        let source = by_path
            .get(file.relative_path.as_str())
            .ok_or(FixedStockPriceBetaError::SourceTampered)?;
        let window = windows
            .get(file.window_id.as_str())
            .ok_or(FixedStockPriceBetaError::Invalid("missing Raw window"))?;
        let value: serde_json::Value = serde_json::from_slice(&source.bytes)
            .map_err(|_| FixedStockPriceBetaError::Invalid("malformed Raw JSON"))?;
        let object = value.as_object().ok_or(FixedStockPriceBetaError::Invalid(
            "Raw response is not object",
        ))?;
        if object.get("rt_cd").and_then(|v| v.as_str()) != Some("0") || raw_has_cursor(object) {
            return Err(FixedStockPriceBetaError::Invalid(
                "invalid Raw response status or continuation",
            ));
        }
        if object
            .get("output1")
            .and_then(|v| v.get("stck_shrn_iscd"))
            .and_then(|v| v.as_str())
            != Some(file.query_symbol.as_str())
        {
            return Err(FixedStockPriceBetaError::Invalid(
                "Raw response symbol mismatch",
            ));
        }
        let rows = object
            .get("output2")
            .and_then(|v| v.as_array())
            .ok_or(FixedStockPriceBetaError::Invalid("invalid Raw output2"))?;
        if rows.is_empty() || rows.len() >= 100 {
            return Err(FixedStockPriceBetaError::Invalid(
                "invalid Raw output2 count",
            ));
        }
        let mut prior: Option<String> = None;
        for row in rows {
            let row = row
                .as_object()
                .ok_or(FixedStockPriceBetaError::Invalid("invalid Raw row"))?;
            let raw_date = row
                .get("stck_bsop_date")
                .and_then(|v| v.as_str())
                .ok_or(FixedStockPriceBetaError::Invalid("missing Raw date"))?;
            if raw_date.len() != 8 || !raw_date.bytes().all(|b| b.is_ascii_digit()) {
                return Err(FixedStockPriceBetaError::Invalid("invalid Raw date"));
            }
            let observed_date =
                format!("{}-{}-{}", &raw_date[..4], &raw_date[4..6], &raw_date[6..]);
            if !date(&observed_date)
                || observed_date < window.range_start
                || observed_date > window.range_end
            {
                return Err(FixedStockPriceBetaError::Invalid("Raw date outside window"));
            }
            if prior.as_ref().is_some_and(|p| observed_date >= *p) {
                return Err(FixedStockPriceBetaError::Invalid(
                    "Raw dates not newest first",
                ));
            }
            prior = Some(observed_date.clone());
            let number =
                |key: &'static str, positive: bool| -> Result<i64, FixedStockPriceBetaError> {
                    let n = row
                        .get(key)
                        .and_then(|v| v.as_str())
                        .ok_or(FixedStockPriceBetaError::Invalid("invalid Raw OHLCV"))?
                        .parse::<i64>()
                        .map_err(|_| FixedStockPriceBetaError::Invalid("invalid Raw OHLCV"))?;
                    if (positive && n <= 0) || (!positive && n < 0) {
                        Err(FixedStockPriceBetaError::Invalid("invalid Raw OHLCV"))
                    } else {
                        Ok(n)
                    }
                };
            let bar = DailyBar {
                instrument_id: file.instrument_id.clone(),
                date: observed_date,
                open: number("stck_oprc", true)?,
                high: number("stck_hgpr", true)?,
                low: number("stck_lwpr", true)?,
                close: number("stck_clpr", true)?,
                volume: number("acml_vol", false)?,
            };
            if bar.low > bar.open
                || bar.low > bar.close
                || bar.high < bar.open
                || bar.high < bar.close
                || !seen.insert((bar.instrument_id.clone(), bar.date.clone()))
            {
                return Err(FixedStockPriceBetaError::Invalid(
                    "invalid or duplicate Raw bar",
                ));
            }
            bars.push(bar);
        }
    }
    bars.sort_by(|a, b| (&a.date, &a.instrument_id).cmp(&(&b.date, &b.instrument_id)));
    bars_ok(&bars)?;
    Ok(bars)
}

fn raw_has_cursor(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.iter().any(|(key, value)| {
        let key = key.to_ascii_lowercase();
        (key.contains("ctx")
            || key.contains("cts")
            || key.contains("continu")
            || key == "next"
            || key == "has_more"
            || key == "more")
            && match value {
                serde_json::Value::Null => false,
                serde_json::Value::String(v) => !v.is_empty(),
                serde_json::Value::Bool(v) => *v,
                serde_json::Value::Number(v) => v.as_i64() != Some(0),
                serde_json::Value::Array(v) => !v.is_empty(),
                serde_json::Value::Object(v) => !v.is_empty(),
            }
    })
}
fn bars_ok(bars: &[DailyBar]) -> Result<(Vec<String>, Vec<DailyBar>), FixedStockPriceBetaError> {
    if bars.is_empty() {
        return Err(FixedStockPriceBetaError::Invalid("bars must not be empty"));
    }
    let allowed: BTreeSet<_> = FIXED_30_INSTRUMENT_IDS.into_iter().collect();
    let mut m: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for b in bars {
        if !allowed.contains(b.instrument_id.as_str()) {
            return Err(FixedStockPriceBetaError::Invalid(
                "instrument is not in fixed list",
            ));
        }
        if !date(&b.date)
            || b.date.as_str() < FIXED_STOCK_PRICE_BETA_RANGE_START
            || b.date.as_str() > FIXED_STOCK_PRICE_BETA_RANGE_END
        {
            return Err(FixedStockPriceBetaError::Invalid(
                "bar date outside configured range",
            ));
        }
        if !seen.insert((&b.instrument_id, &b.date)) {
            return Err(FixedStockPriceBetaError::Invalid(
                "duplicate instrument/date",
            ));
        }
        if b.open <= 0 || b.high <= 0 || b.low <= 0 || b.close <= 0 {
            return Err(FixedStockPriceBetaError::Invalid("OHLC must be positive"));
        }
        if b.low > b.open
            || b.low > b.close
            || b.high < b.open
            || b.high < b.close
            || b.low > b.high
        {
            return Err(FixedStockPriceBetaError::Invalid(
                "invalid OHLC relationship",
            ));
        }
        if b.volume < 0 {
            return Err(FixedStockPriceBetaError::Invalid(
                "volume must not be negative",
            ));
        }
        m.entry(b.instrument_id.clone())
            .or_default()
            .insert(b.date.clone());
    }
    if m.len() != 30 {
        return Err(FixedStockPriceBetaError::Invalid(
            "missing fixed instrument",
        ));
    }
    let s = m
        .values()
        .next()
        .ok_or(FixedStockPriceBetaError::Invalid("missing sessions"))?
        .clone();
    if s.len() < 120 {
        return Err(FixedStockPriceBetaError::Invalid(
            "fewer than 120 common dates",
        ));
    }
    if m.values().any(|x| x != &s) {
        return Err(FixedStockPriceBetaError::Invalid(
            "instruments have different session sets",
        ));
    }
    let mut v = bars.to_vec();
    v.sort_by(|a, b| (&a.date, &a.instrument_id).cmp(&(&b.date, &b.instrument_id)));
    Ok((s.into_iter().collect(), v))
}
fn evidence_ok(
    e: &FixedStockPriceBetaRawBatchEvidence,
    sources: Option<&[FixedStockPriceBetaRawSourceFile]>,
) -> Result<(), FixedStockPriceBetaError> {
    if e.contract_version != 1
        || e.provider_scope != FIXED_STOCK_PRICE_BETA_RAW_PROVIDER_SCOPE
        || e.requested_range_start != FIXED_STOCK_PRICE_BETA_RANGE_START
        || e.requested_range_end != FIXED_STOCK_PRICE_BETA_RANGE_END
        || e.entitlement_reference.is_empty()
        || !hex(&e.entitlement_sha256)
        || !commit(&e.capture_commit)
        || !hex(&e.batch_json_sha256)
        || !hex(&e.manifest_sha256)
    {
        return Err(FixedStockPriceBetaError::Invalid(
            "invalid Raw batch provenance",
        ));
    }
    if e.windows.is_empty() || !strict(&e.windows, |x| x.window_id.clone()) {
        return Err(FixedStockPriceBetaError::Invalid(
            "invalid Raw window order",
        ));
    }
    let mut wm = BTreeMap::new();
    for w in &e.windows {
        if !window(&w.window_id)
            || !date(&w.range_start)
            || !date(&w.range_end)
            || w.range_start > w.range_end
            || w.range_start < e.requested_range_start
            || w.range_end > e.requested_range_end
            || wm.insert(w.window_id.as_str(), w).is_some()
        {
            return Err(FixedStockPriceBetaError::Invalid("invalid Raw window"));
        }
    }
    let n = 30usize
        .checked_mul(e.windows.len())
        .ok_or(FixedStockPriceBetaError::Invalid(
            "Raw evidence count overflow",
        ))?;
    if e.files.len() != n
        || !strict(&e.files, |x| {
            (
                x.instrument_id.clone(),
                x.window_id.clone(),
                x.page_id.clone(),
            )
        })
    {
        return Err(FixedStockPriceBetaError::Invalid(
            "Raw file matrix is not exact",
        ));
    }
    let mut ids = BTreeSet::new();
    for f in &e.files {
        let w = wm
            .get(f.window_id.as_str())
            .ok_or(FixedStockPriceBetaError::Invalid("unknown Raw file window"))?;
        let symbol = symbol(&f.instrument_id).ok_or(FixedStockPriceBetaError::Invalid(
            "invalid Raw file instrument",
        ))?;
        if f.relative_path != rel(&f.instrument_id, &f.window_id)
            || f.page_id != "single"
            || !ids.insert((&f.instrument_id, &f.window_id, &f.page_id))
            || !hex(&f.sha256)
            || f.size_bytes == 0
            || f.method != "GET"
            || f.path != KIS_DAILY_BARS_PATH
            || f.tr_id != KIS_DAILY_BARS_TR_ID
            || f.query_symbol != symbol
            || f.query_range_start != w.range_start
            || f.query_range_end != w.range_end
            || f.fid_org_adj_prc != KIS_FID_ORG_ADJ_PRC
            || !f.response_continuation.is_empty()
        {
            return Err(FixedStockPriceBetaError::Invalid(
                "unexpected Raw daily-bars file evidence",
            ));
        }
    }
    if let Some(src) = sources {
        if src.len() != e.files.len() {
            return Err(FixedStockPriceBetaError::SourceTampered);
        }
        let mut sm = BTreeMap::new();
        for x in src {
            if sm.insert(x.relative_path.as_str(), x).is_some() {
                return Err(FixedStockPriceBetaError::SourceTampered);
            }
        }
        for f in &e.files {
            let x = sm
                .remove(f.relative_path.as_str())
                .ok_or(FixedStockPriceBetaError::SourceTampered)?;
            if x.bytes.len() as u64 != f.size_bytes || hash(&x.bytes) != f.sha256 {
                return Err(FixedStockPriceBetaError::SourceTampered);
            }
        }
        if !sm.is_empty() {
            return Err(FixedStockPriceBetaError::SourceTampered);
        }
    }
    Ok(())
}
fn rel(id: &str, w: &str) -> String {
    format!("daily-bars/{id}/{w}.json")
}
fn symbol(id: &str) -> Option<&str> {
    id.strip_suffix(".KRX")
        .filter(|x| x.len() == 6 && x.bytes().all(|b| b.is_ascii_digit()))
}
fn hex(x: &str) -> bool {
    x.len() == 64
        && x.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn commit(x: &str) -> bool {
    (7..=64).contains(&x.len())
        && x.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn window(x: &str) -> bool {
    !x.is_empty()
        && x.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn date(x: &str) -> bool {
    NaiveDate::parse_from_str(x, "%Y-%m-%d").is_ok()
}
fn hash(x: &[u8]) -> String {
    format!("{:x}", Sha256::digest(x))
}
fn strict<T, K: Ord>(v: &[T], f: impl Fn(&T) -> K) -> bool {
    v.windows(2).all(|x| f(&x[0]) < f(&x[1]))
}

pub fn write_fixed_stock_price_beta_artifact(
    root: &Path,
    artifact: &FixedStockPriceBetaArtifact,
) -> Result<PathBuf, FixedStockPriceBetaError> {
    artifact.verify()?;
    #[cfg(unix)]
    {
        unix::write(root, artifact)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, artifact);
        Err(FixedStockPriceBetaError::UnsupportedPlatform)
    }
}
pub fn read_fixed_stock_price_beta_artifact(
    root: &Path,
    content_sha256: &str,
) -> Result<FixedStockPriceBetaArtifact, FixedStockPriceBetaError> {
    if !hex(content_sha256) {
        return Err(FixedStockPriceBetaError::UnsafePath);
    }
    #[cfg(unix)]
    {
        unix::read(root, content_sha256)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, content_sha256);
        Err(FixedStockPriceBetaError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RenameFlags, fstat, fsync, mkdirat, open, openat,
        renameat_with, statat, unlinkat,
    };
    use rustix::process::geteuid;
    use std::io::{Read, Write};
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static STAGE: AtomicU64 = AtomicU64::new(1);
    const MAX: usize = 64 * 1024 * 1024;
    fn err(e: rustix::io::Errno) -> FixedStockPriceBetaError {
        if e == rustix::io::Errno::LOOP || e == rustix::io::Errno::NOTDIR {
            FixedStockPriceBetaError::UnsafePath
        } else {
            FixedStockPriceBetaError::Io
        }
    }
    fn md(s: &rustix::fs::Stat) -> u32 {
        Mode::from_raw_mode(s.st_mode).bits() & 0o7777
    }
    fn dir(s: &rustix::fs::Stat) -> Result<(), FixedStockPriceBetaError> {
        if FileType::from_raw_mode(s.st_mode) != FileType::Directory
            || s.st_uid != geteuid().as_raw()
            || md(s) != 0o700
        {
            Err(FixedStockPriceBetaError::UnsafePath)
        } else {
            Ok(())
        }
    }
    fn file(s: &rustix::fs::Stat) -> Result<(), FixedStockPriceBetaError> {
        if FileType::from_raw_mode(s.st_mode) != FileType::RegularFile
            || s.st_uid != geteuid().as_raw()
            || md(s) != 0o600
            || s.st_nlink != 1
            || s.st_size < 0
            || s.st_size as usize > MAX
        {
            Err(FixedStockPriceBetaError::UnsafePath)
        } else {
            Ok(())
        }
    }
    fn comps(p: &Path) -> Result<Vec<Vec<u8>>, FixedStockPriceBetaError> {
        let b = p.as_os_str().as_bytes();
        if b.len() < 2 || b[0] != b'/' || b[1] == b'/' || b.ends_with(b"/") {
            return Err(FixedStockPriceBetaError::UnsafePath);
        }
        let v: Vec<_> = b[1..].split(|x| *x == b'/').map(|x| x.to_vec()).collect();
        if v.iter()
            .any(|x| x.is_empty() || x == b"." || x == b".." || x.contains(&0))
        {
            Err(FixedStockPriceBetaError::UnsafePath)
        } else {
            Ok(v)
        }
    }
    fn root(p: &Path) -> Result<OwnedFd, FixedStockPriceBetaError> {
        let mut x = open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?;
        for c in comps(p)? {
            x = openat(
                &x,
                &c,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(err)?
        }
        dir(&fstat(&x).map_err(err)?)?;
        Ok(x)
    }
    fn leaf(p: &impl AsFd, n: &[u8]) -> Result<OwnedFd, FixedStockPriceBetaError> {
        let a = statat(p, n, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
        dir(&a)?;
        let x = openat(
            p,
            n,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?;
        let b = fstat(&x).map_err(err)?;
        dir(&b)?;
        if a.st_dev != b.st_dev || a.st_ino != b.st_ino {
            Err(FixedStockPriceBetaError::UnsafePath)
        } else {
            Ok(x)
        }
    }
    fn only(d: &OwnedFd) -> Result<(), FixedStockPriceBetaError> {
        use rustix::fs::{RawDir, SeekFrom, seek};
        use rustix::io::dup;
        use std::mem::MaybeUninit;
        seek(d, SeekFrom::Start(0)).map_err(err)?;
        let x = dup(d).map_err(err)?;
        let mut a = [MaybeUninit::<u8>::uninit(); 4096];
        let mut r = RawDir::new(&x, &mut a);
        let mut v = Vec::new();
        while let Some(e) = r.next() {
            let entry = e.map_err(err)?;
            let n = entry.file_name().to_bytes();
            if n != b"." && n != b".." {
                v.push(n.to_vec())
            }
        }
        v.sort_unstable();
        if v == [b"artifact.json".to_vec()] {
            Ok(())
        } else {
            Err(FixedStockPriceBetaError::Tampered)
        }
    }
    fn stage(p: &impl AsFd) -> Result<(OwnedFd, Vec<u8>), FixedStockPriceBetaError> {
        for _ in 0..128 {
            let n = format!(
                ".stage-{}-{}",
                std::process::id(),
                STAGE.fetch_add(1, Ordering::Relaxed)
            )
            .into_bytes();
            match mkdirat(p, &n, Mode::from_raw_mode(0o700)) {
                Ok(()) => {
                    let d = leaf(p, &n)?;
                    fsync(p).map_err(err)?;
                    return Ok((d, n));
                }
                Err(e) if e == rustix::io::Errno::EXIST => {}
                Err(e) => return Err(err(e)),
            }
        }
        Err(FixedStockPriceBetaError::Conflict)
    }
    fn put(p: &impl AsFd, b: &[u8]) -> Result<(), FixedStockPriceBetaError> {
        let x = openat(
            p,
            &b"artifact.json"[..],
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(err)?;
        let mut f = std::fs::File::from(x);
        file(&fstat(&f).map_err(err)?)?;
        f.write_all(b).map_err(|_| FixedStockPriceBetaError::Io)?;
        f.sync_all().map_err(|_| FixedStockPriceBetaError::Io)?;
        file(&fstat(&f).map_err(err)?)?;
        Ok(())
    }
    fn cleanup(p: &impl AsFd, d: &OwnedFd, n: &[u8]) {
        if only(d).is_ok() {
            let _ = unlinkat(d, &b"artifact.json"[..], AtFlags::empty());
        }
        let _ = unlinkat(p, n, AtFlags::REMOVEDIR);
        let _ = fsync(p);
    }
    fn same(a: &rustix::fs::Stat, b: &rustix::fs::Stat) -> bool {
        a.st_dev == b.st_dev && a.st_ino == b.st_ino && a.st_uid == b.st_uid && md(a) == md(b)
    }
    pub(super) fn write(
        path: &Path,
        a: &FixedStockPriceBetaArtifact,
    ) -> Result<PathBuf, FixedStockPriceBetaError> {
        let r = root(path)?;
        let n = a.content_sha256.as_bytes();
        match statat(&r, n, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => {
                return match read(path, &a.content_sha256) {
                    Ok(x) if x == *a => Ok(path.join(&a.content_sha256)),
                    _ => Err(FixedStockPriceBetaError::Conflict),
                };
            }
            Err(e) if e == rustix::io::Errno::NOENT => {}
            Err(e) => return Err(err(e)),
        };
        let (d, s) = stage(&r)?;
        let stage_stat = fstat(&d).map_err(err)?;
        let b = serde_json::to_vec(a).map_err(|_| FixedStockPriceBetaError::Serialize)?;
        if let Err(e) = put(&d, &b)
            .and_then(|_| only(&d))
            .and_then(|_| fsync(&d).map_err(err))
        {
            cleanup(&r, &d, &s);
            return Err(e);
        }
        let named_stage = match statat(&r, &s, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if same(&stat, &stage_stat) => stat,
            Ok(_) => {
                cleanup(&r, &d, &s);
                return Err(FixedStockPriceBetaError::UnsafePath);
            }
            Err(error) => {
                cleanup(&r, &d, &s);
                return Err(err(error));
            }
        };
        if !same(&fstat(&d).map_err(err)?, &named_stage) {
            cleanup(&r, &d, &s);
            return Err(FixedStockPriceBetaError::UnsafePath);
        }
        match renameat_with(&r, &s, &r, n, RenameFlags::NOREPLACE) {
            Ok(()) => {
                fsync(&r).map_err(err)?;
                read(path, &a.content_sha256).map(|_| path.join(&a.content_sha256))
            }
            Err(e) if e == rustix::io::Errno::EXIST => {
                cleanup(&r, &d, &s);
                match read(path, &a.content_sha256) {
                    Ok(x) if x == *a => Ok(path.join(&a.content_sha256)),
                    _ => Err(FixedStockPriceBetaError::Conflict),
                }
            }
            Err(e) => {
                cleanup(&r, &d, &s);
                Err(err(e))
            }
        }
    }
    pub(super) fn read(
        path: &Path,
        h: &str,
    ) -> Result<FixedStockPriceBetaArtifact, FixedStockPriceBetaError> {
        let r = root(path)?;
        let d = leaf(&r, h.as_bytes())?;
        only(&d)?;
        let a = statat(&d, &b"artifact.json"[..], AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
        file(&a)?;
        let x = openat(
            &d,
            &b"artifact.json"[..],
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(err)?;
        let mut f = std::fs::File::from(x);
        let b = fstat(&f).map_err(err)?;
        file(&b)?;
        if a.st_dev != b.st_dev || a.st_ino != b.st_ino {
            return Err(FixedStockPriceBetaError::UnsafePath);
        }
        let mut v = Vec::with_capacity(b.st_size as usize);
        f.read_to_end(&mut v)
            .map_err(|_| FixedStockPriceBetaError::Io)?;
        if v.len() != b.st_size as usize {
            return Err(FixedStockPriceBetaError::Tampered);
        }
        let a: FixedStockPriceBetaArtifact =
            serde_json::from_slice(&v).map_err(|_| FixedStockPriceBetaError::Tampered)?;
        if a.content_sha256 != h {
            return Err(FixedStockPriceBetaError::Tampered);
        }
        a.verify()?;
        Ok(a)
    }
}
