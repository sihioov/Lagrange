//! Todo 11 red-phase integration tests: the dataset quality gate, issue
//! severities, READY|WARNING|BLOCKED state transitions, the AT-05 missing-bar
//! policy, downstream use denial, and immutable-versioning invariants.
//!
//! Red phase: the `market_data::quality` module does not exist yet, so this
//! file fails to compile. The failing transcript is captured in the Todo 11
//! evidence bundle, then the implementation turns it green.
//!
//! The gate consumes a **Curated dataset version** (Todo 10 layout:
//! `data/curated/bars/.../version={v}/bars.parquet` etc. plus the per-version
//! `manifest.json`) and classifies every finding into a severity with a
//! deterministic `READY | WARNING | BLOCKED` dataset state (FR-DATA-004,
//! design §6.3, AT-05).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    Date32Array, Decimal128Builder, Int64Array, StringArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use domain::{
    BatchId, ContentHash, Currency, DataState, DatasetId, FixedPoint, InstrumentId, Price,
    TradingDate, UtcTimestamp,
};
use parquet::arrow::ArrowWriter;
use serde_json::json;

use market_data::contract::{FetchMode, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::curate::adjust::{AdjustmentBar, AdjustmentKind};
use market_data::curate::schema::{
    CuratedSchema, PRICE_PRECISION, PRICE_SCALE, write_adjusted_bars, write_bars,
};
use market_data::curate::{CurateRequest, CurateStore, curate_batch, dataset_manifest_hash};
use market_data::quality::{
    DataUse, ExclusionRecord, FreshnessPolicy, IssueCode, OptionalExclusion, QualityGate,
    QualityIssue, QualityPolicy, Severity,
};
use market_data::{
    BatchSpec, Capability, CurateOutcome, DatasetManifest, ManifestEntry, RawStore, krx_2020,
    seed_universe,
};

/// The golden Todo 6 fixture: 3 seed ETFs, 27 bars, no corporate actions.
const GOLDEN_BARS: &[u8] = include_bytes!("../../../tests/fixtures/kr-etf/2020-01-31/bars.json");
const EMPTY_ACTIONS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/contract/corporate-actions-response.json");
/// The Todo 6 AT-05 fixture: 069500 missing the 2020-01-31 daily bar.
const MISSING_BARS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/variants/missing/missing_bars.json");

/// A curation clock after every fixture `announced_at` (fixtures are 2020).
fn now() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2020-02-10T00:00:00Z").expect("valid clock")
}

fn ds() -> DatasetId {
    DatasetId::parse("kr-etf-daily").expect("valid dataset id")
}

/// Stores a synthetic raw batch (bars + corporate actions) in `root` and
/// returns the store plus its manifest entry.
fn fixture_batch(root: &Path, bars: &[u8], actions: &[u8]) -> (RawStore, ManifestEntry) {
    let raw = RawStore::new(root.join("data"));
    let batch_id = BatchId::generate();
    let request = RequestMetadata {
        endpoint: "krx.eod.bars.v1".to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        mode: FetchMode::Synthetic,
    };
    let envelopes = vec![
        RawEnvelope::new(
            batch_id,
            ResponseKind::Bars,
            "bars.json",
            bars.to_vec(),
            now(),
            request.clone(),
        ),
        RawEnvelope::new(
            batch_id,
            ResponseKind::CorporateActions,
            "corporate-actions.json",
            actions.to_vec(),
            now(),
            request,
        ),
    ];
    let spec = BatchSpec {
        provider: "krx",
        market: "kr",
        date: &TradingDate::new(2020, 1, 31).expect("valid date"),
        batch_id,
        entitlement_reference: None,
        mode: FetchMode::Synthetic,
    };
    let entry = raw
        .store_batch(&spec, &envelopes)
        .expect("fixture batch stores");
    (raw, entry)
}

/// Curates `bars` bytes into a fresh temp store, returning the curated store
/// and the outcome.
fn curate(root: &Path, bars: &[u8], actions: &[u8]) -> (CurateStore, CurateOutcome) {
    let (raw, entry) = fixture_batch(root, bars, actions);
    let curated = CurateStore::new(root.join("data"));
    let outcome = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &ds(),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect("fixture curates");
    (curated, outcome)
}

/// Curates the golden fixture (27 bars, no actions) into a fresh temp store.
fn curate_golden(root: &Path) -> (CurateStore, CurateOutcome) {
    curate(root, GOLDEN_BARS, EMPTY_ACTIONS)
}

/// The seed universe (Todo 12 symbol set) as a set of canonical IDs.
fn universe(symbols: &[&str]) -> BTreeSet<InstrumentId> {
    symbols
        .iter()
        .map(|s| InstrumentId::parse(&format!("{s}.KRX")).expect("valid instrument id"))
        .collect()
}

/// The default quality policy: required universe, freshness reference date,
/// and staleness grace in sessions.
fn policy(required: &[&str], reference: &str, stale_grace: u32) -> QualityPolicy {
    QualityPolicy {
        required_universe: universe(required),
        optional_exclusion: None,
        freshness: FreshnessPolicy {
            reference_date: TradingDate::parse(reference).expect("valid reference date"),
            max_stale_sessions: stale_grace,
        },
        outlier_threshold_pct: 0.20,
    }
}

fn gate(curated: &CurateStore, policy: QualityPolicy) -> QualityGate {
    QualityGate::new(curated.clone(), krx_2020(), seed_universe(), "kr", policy)
}

fn has_issue(
    report: &market_data::quality::QualityReport,
    code: IssueCode,
    severity: Severity,
) -> bool {
    report
        .issues
        .iter()
        .any(|i| i.code == code && i.severity == severity)
}

/// A synthetic, session-correct curated bar for one KRX session.
fn make_bar(
    symbol: &str,
    date: &str,
    open: i64,
    high: i64,
    low: i64,
    close: i64,
    volume: i64,
) -> market_data::CuratedBar {
    let trading_date = TradingDate::parse(date).expect("valid trading date");
    let calendar = krx_2020();
    let (market_open_ts, market_close_ts) = match calendar.session_open_utc(trading_date) {
        Ok(open_ts) => (
            open_ts,
            calendar
                .session_close_utc(trading_date)
                .expect("session close"),
        ),
        // Non-session dates (session-conformance fixtures) carry a placeholder
        // instant; the session check fires before timestamps are compared.
        Err(_) => (
            UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z").expect("ts"),
            UtcTimestamp::parse_rfc3339("2020-01-01T06:30:00Z").expect("ts"),
        ),
    };
    market_data::CuratedBar {
        instrument_id: InstrumentId::parse(&format!("{symbol}.KRX")).expect("valid id"),
        trading_date,
        market_open_ts,
        market_close_ts,
        open: Price::parse(&open.to_string()).expect("price"),
        high: Price::parse(&high.to_string()).expect("price"),
        low: Price::parse(&low.to_string()).expect("price"),
        close: Price::parse(&close.to_string()).expect("price"),
        volume,
        trading_value: Some(open * volume),
        currency: Currency::KRW,
        source: "krx".to_owned(),
        ingested_at: now(),
        batch_id: BatchId::generate(),
        raw_hash: ContentHash::from_bytes(b"fixture"),
    }
}

/// The split-adjusted row for a raw bar (no action, factor 1).
fn make_adjustment(bar: &market_data::CuratedBar, factor: &str) -> AdjustmentBar {
    AdjustmentBar {
        instrument_id: bar.instrument_id.clone(),
        trading_date: bar.trading_date,
        market_open_ts: bar.market_open_ts,
        market_close_ts: bar.market_close_ts,
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.volume,
        trading_value: bar.trading_value,
        adjustment_kind: AdjustmentKind::Split,
        adjustment_factor: FixedPoint::parse(factor).expect("factor"),
        adjustment_events: String::new(),
        currency: bar.currency,
        source: bar.source.clone(),
        ingested_at: bar.ingested_at,
        batch_id: bar.batch_id,
        raw_hash: bar.raw_hash.clone(),
    }
}

/// Writes one version's bars + adjusted + total-return partitions and a
/// content-hashed manifest under `curated`, returning the written manifest.
#[allow(clippy::too_many_arguments)]
fn fabricate_version(
    curated: &CurateStore,
    version: u32,
    symbol: &str,
    year: i32,
    bars: Vec<market_data::CuratedBar>,
    adjusted: Vec<AdjustmentBar>,
) -> DatasetManifest {
    let _ = fs::create_dir_all(curated.dataset_dir(&ds(), version));
    write_bars(&curated.bars_path("kr", symbol, year, version), &bars).expect("bars write");
    write_adjusted_bars(
        &curated.adjusted_bars_path("kr", symbol, year, version),
        &adjusted,
    )
    .expect("adjusted write");
    write_adjusted_bars(
        &curated.total_return_bars_path("kr", symbol, year, version),
        &[],
    )
    .expect("total return write");
    write_manifest(curated, version, bars.len() as u64)
}

fn write_manifest(curated: &CurateStore, version: u32, bar_count: u64) -> DatasetManifest {
    let manifest = DatasetManifest {
        dataset_id: ds(),
        version,
        capability: Capability::PriceReturnOnly,
        created_at: now(),
        source_batches: Vec::new(),
        bar_count,
        action_count: 0,
        content_hash: ContentHash::from_bytes(b"placeholder"),
    };
    let content_hash = dataset_manifest_hash(&manifest).expect("manifest hash");
    let manifest = DatasetManifest {
        content_hash,
        ..manifest
    };
    curated
        .write_dataset_manifest(&manifest)
        .expect("manifest write");
    manifest
}

/// Writes a parquet file with a deliberately WRONG bars schema (instrument_id
/// and trading_date are both strings).
fn write_wrong_schema_parquet(path: &Path) {
    let schema = Schema::new(vec![
        Field::new("instrument_id", DataType::Utf8, false),
        Field::new("trading_date", DataType::Utf8, false),
    ]);
    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(StringArray::from(vec!["069500.KRX"])),
            Arc::new(StringArray::from(vec!["2020-02-03"])),
        ],
    )
    .expect("wrong-schema batch builds");
    write_parquet(path, Arc::new(schema), batch);
}

/// Writes a full-schema bars parquet whose OPEN column carries a non-positive
/// decimal (0) — value-level corruption the gate must classify, never panic on.
fn write_bars_with_zero_open(path: &Path) {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    let days = (NaiveDate::from_ymd_opt(2020, 2, 3).expect("date") - epoch).num_days() as i32;
    let mut open = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("decimal params");
    open.append_value(0i128); // 0 <= 0: a non-positive price value
    let mut rest = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("decimal params");
    rest.append_value(10000i128);
    let rest_array = rest.finish();
    let columns: Vec<Arc<dyn arrow::array::Array>> = vec![
        Arc::new(StringArray::from(vec!["069500.KRX"])),
        Arc::new(Date32Array::from(vec![days])),
        Arc::new(TimestampMicrosecondArray::from(vec![0])),
        Arc::new(TimestampMicrosecondArray::from(vec![0])),
        Arc::new(open.finish()),
        Arc::new(rest_array.clone()),
        Arc::new(rest_array.clone()),
        Arc::new(rest_array),
        Arc::new(Int64Array::from(vec![1000i64])),
        Arc::new(Int64Array::from(vec![10_000_000i64])),
        Arc::new(StringArray::from(vec!["KRW"])),
        Arc::new(StringArray::from(vec!["krx"])),
        Arc::new(TimestampMicrosecondArray::from(vec![0])),
        Arc::new(StringArray::from(vec![BatchId::generate().to_string()])),
        Arc::new(StringArray::from(vec![
            ContentHash::from_bytes(b"x").as_str(),
        ])),
    ];
    let batch = RecordBatch::try_new(Arc::new(CuratedSchema::bars()), columns)
        .expect("zero-open batch builds");
    write_parquet(path, Arc::new(CuratedSchema::bars()), batch);
}

fn write_parquet(path: &Path, schema: Arc<Schema>, batch: RecordBatch) {
    let file = fs::File::create(path).expect("create parquet");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("arrow writer");
    writer.write(&batch).expect("write batch");
    writer.close().expect("close writer");
}

// ---------------------------------------------------------------------------
// READY | WARNING | BLOCKED: the happy path
// ---------------------------------------------------------------------------

#[test]
fn complete_dataset_is_ready_and_permits_all_uses() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    let report = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-03", 0),
    )
    .validate_dataset(&ds(), 1)
    .expect("validation runs");
    assert_eq!(report.state, DataState::Ready, "{:?}", report.issues);
    assert!(report.issues.is_empty(), "{:?}", report.issues);
    assert!(report.exclusions.is_empty());
    for use_case in [
        DataUse::Recommendation,
        DataUse::Backtest,
        DataUse::Paper,
        DataUse::Live,
    ] {
        assert!(
            report.permits(use_case).is_ok(),
            "{use_case:?} must be permitted on READY"
        );
    }
}

// ---------------------------------------------------------------------------
// Schema / duplicates / OHLC / timezone / volume / currency checks
// ---------------------------------------------------------------------------

#[test]
fn duplicate_trading_date_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar(
        "069500",
        "2020-02-03",
        10300,
        10420,
        10280,
        10380,
        1_240_000,
    );
    fabricate_version(
        &curated,
        1,
        "069500.KRX",
        2020,
        vec![bar.clone(), bar],
        vec![],
    );
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::DuplicateDate,
        Severity::Blocking
    ));
}

#[test]
fn impossible_ohlc_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    // low 10500 > min(open 10300, close 10380): violates documented §6.3 rule.
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10500, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::ImpossibleOhlc,
        Severity::Blocking
    ));
}

#[test]
fn non_positive_price_blocks_without_panic() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    write_bars_with_zero_open(&curated.bars_path("kr", "069500.KRX", 2020, 1));
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::NonPositivePrice,
        Severity::Blocking
    ));
}

#[test]
fn bar_on_non_session_date_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    // 2020-02-01 is a Saturday: never a KRX session.
    let bar = make_bar("069500", "2020-02-01", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::NotASession,
        Severity::Blocking
    ));
}

#[test]
fn timestamp_mismatch_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let mut bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    bar.market_open_ts = UtcTimestamp::parse_rfc3339("2020-02-03T01:00:00Z").expect("ts");
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::TimestampMismatch,
        Severity::Blocking
    ));
}

#[test]
fn negative_volume_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, -5);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::NegativeVolume,
        Severity::Blocking
    ));
}

#[test]
fn zero_volume_warns() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 0);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Warning, "{:?}", report.issues);
    assert!(has_issue(&report, IssueCode::ZeroVolume, Severity::Warning));
}

#[test]
fn currency_mismatch_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let mut bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    bar.currency = Currency::from_code("USD").expect("usd");
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::CurrencyMismatch,
        Severity::Blocking
    ));
}

#[test]
fn bar_for_unknown_instrument_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("999999", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "999999.KRX", 2020, vec![bar], vec![]);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::UnknownInstrument,
        Severity::Blocking
    ));
}

// ---------------------------------------------------------------------------
// Outliers and suspicious splits (documented rules)
// ---------------------------------------------------------------------------

#[test]
fn suspicious_close_move_warns() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    // 01-30 close 10000 -> 01-31 close 20000 is a +100% move: >= the
    // documented 20% outlier threshold.
    let bars = vec![
        make_bar("069500", "2020-01-30", 10000, 10100, 9900, 10000, 100),
        make_bar("069500", "2020-01-31", 19900, 20050, 19800, 20000, 100),
        make_bar("069500", "2020-02-03", 20000, 20100, 19900, 20100, 100),
    ];
    fabricate_version(&curated, 1, "069500.KRX", 2020, bars, vec![]);
    let report = gate(&curated, policy(&[], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Warning, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::SuspiciousMove,
        Severity::Warning
    ));
}

#[test]
fn suspicious_split_warns_without_action_record() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    // Split-adjusted factor 2.0 with NO corporate action on record.
    let adjusted = vec![make_adjustment(&bar, "2.0")];
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], adjusted);
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Warning, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::SuspiciousSplit,
        Severity::Warning
    ));
}

// ---------------------------------------------------------------------------
// AT-05: missing bars (required vs optional policy)
// ---------------------------------------------------------------------------

#[test]
fn at05_missing_required_bar_blocks_downstream() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate(temp.path(), MISSING_BARS, EMPTY_ACTIONS);
    let report = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-03", 0),
    )
    .validate_dataset(&ds(), 1)
    .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    let missing: Vec<&QualityIssue> = report
        .issues
        .iter()
        .filter(|i| i.code == IssueCode::MissingRequiredBar)
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", report.issues);
    assert_eq!(
        missing[0]
            .instrument
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("069500.KRX")
    );
    assert_eq!(
        missing[0].date.map(|d| d.to_iso()).as_deref(),
        Some("2020-01-31")
    );
    assert_eq!(missing[0].severity, Severity::Blocking);
    // Downstream uses are ALL denied with the blocking code.
    for use_case in [
        DataUse::Recommendation,
        DataUse::Backtest,
        DataUse::Paper,
        DataUse::Live,
    ] {
        let denial = report
            .permits(use_case)
            .expect_err("blocked dataset must deny every use");
        assert_eq!(denial.use_case, use_case);
        assert!(
            denial
                .blocking_issues
                .contains(&IssueCode::MissingRequiredBar),
            "{:?}",
            denial.blocking_issues
        );
    }
}

#[test]
fn at05_optional_missing_excludes_with_declared_policy() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate(temp.path(), MISSING_BARS, EMPTY_ACTIONS);
    let p = QualityPolicy {
        required_universe: universe(&["229200", "114260"]),
        optional_exclusion: Some(OptionalExclusion {
            reason: "trend_following tolerates missing 069500 bars".to_owned(),
        }),
        freshness: FreshnessPolicy {
            reference_date: TradingDate::parse("2020-02-03").expect("date"),
            max_stale_sessions: 0,
        },
        outlier_threshold_pct: 0.20,
    };
    let report = gate(&curated, p)
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Warning, "{:?}", report.issues);
    let missing: Vec<&QualityIssue> = report
        .issues
        .iter()
        .filter(|i| i.code == IssueCode::MissingOptionalBar)
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", report.issues);
    assert_eq!(missing[0].severity, Severity::Warning);
    // The exclusion is RECORDED with the strategy-declared reason.
    assert_eq!(report.exclusions.len(), 1);
    let exclusion: &ExclusionRecord = &report.exclusions[0];
    assert_eq!(exclusion.instrument.to_string(), "069500.KRX");
    assert_eq!(
        exclusion.reason,
        "trend_following tolerates missing 069500 bars"
    );
    assert_eq!(
        exclusion.missing_dates,
        vec![TradingDate::new(2020, 1, 31).expect("date")]
    );
    // WARNING still permits research uses (downstream decides exclusions).
    for use_case in [
        DataUse::Recommendation,
        DataUse::Backtest,
        DataUse::Paper,
        DataUse::Live,
    ] {
        assert!(report.permits(use_case).is_ok(), "{use_case:?}");
    }
}

#[test]
fn at05_optional_missing_without_policy_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate(temp.path(), MISSING_BARS, EMPTY_ACTIONS);
    // 069500 is NOT required and NO exclusion policy is declared: fail closed.
    let report = gate(&curated, policy(&["229200", "114260"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::MissingOptionalBar,
        Severity::Blocking
    ));
    assert!(
        report.exclusions.is_empty(),
        "no exclusion without a policy"
    );
}

// ---------------------------------------------------------------------------
// Malformed input: typed BLOCKED, never a panic
// ---------------------------------------------------------------------------

#[test]
fn corrupt_parquet_blocks_without_panic() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    fs::write(
        curated.bars_path("kr", "069500.KRX", 2020, 1),
        b"this is not a parquet file",
    )
    .expect("corrupt write");
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::CorruptParquet,
        Severity::Blocking
    ));
}

#[test]
fn schema_mismatch_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    write_wrong_schema_parquet(&curated.bars_path("kr", "069500.KRX", 2020, 1));
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::SchemaMismatch,
        Severity::Blocking
    ));
}

// ---------------------------------------------------------------------------
// Immutable manifests and deterministic re-validation
// ---------------------------------------------------------------------------

#[test]
fn tampered_manifest_hash_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    // Rewrite the manifest with a WRONG content hash (post-hoc tamper).
    let tampered = DatasetManifest {
        content_hash: ContentHash::from_bytes(b"tampered"),
        ..write_manifest(&curated, 1, 1)
    };
    curated
        .write_dataset_manifest(&tampered)
        .expect("tamper write");
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::ManifestHashMismatch,
        Severity::Blocking
    ));
}

#[test]
fn manifest_bar_count_mismatch_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    fabricate_version(&curated, 1, "069500.KRX", 2020, vec![bar], vec![]);
    // Declares 99 bars but only 1 exists on disk (hash kept valid so the
    // count inconsistency itself is what blocks).
    let mut mismatch = write_manifest(&curated, 1, 1);
    mismatch.bar_count = 99;
    let hash = dataset_manifest_hash(&mismatch).expect("hash");
    curated
        .write_dataset_manifest(&DatasetManifest {
            bar_count: 99,
            content_hash: hash,
            ..mismatch
        })
        .expect("mismatch write");
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::ManifestCorrupt,
        Severity::Blocking
    ));
}

#[test]
fn missing_manifest_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let curated = CurateStore::new(temp.path().join("data"));
    let bar = make_bar("069500", "2020-02-03", 10300, 10420, 10280, 10380, 100);
    let _ = fs::create_dir_all(curated.dataset_dir(&ds(), 1));
    write_bars(&curated.bars_path("kr", "069500.KRX", 2020, 1), &[bar]).expect("bars write");
    // No manifest.json was written for this version.
    let report = gate(&curated, policy(&["069500"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(
        &report,
        IssueCode::ManifestCorrupt,
        Severity::Blocking
    ));
}

#[test]
fn revalidation_is_identical() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    let gate = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-03", 0),
    );
    let first = gate.validate_dataset(&ds(), 1).expect("first run");
    let second = gate.validate_dataset(&ds(), 1).expect("second run");
    assert_eq!(first, second, "re-validation must be byte-identical");
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first.state, DataState::Ready);
}

// ---------------------------------------------------------------------------
// Freshness: DATA_STALE classification (reference date after the last close)
// ---------------------------------------------------------------------------

#[test]
fn stale_close_blocks_required_universe() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    // The golden data's latest close is 2020-02-03; the reference date is
    // 2020-02-05, so 02-04 and 02-05 are both stale sessions.
    let report = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-05", 0),
    )
    .validate_dataset(&ds(), 1)
    .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    let stale: Vec<&QualityIssue> = report
        .issues
        .iter()
        .filter(|i| i.code == IssueCode::DataStale)
        .collect();
    assert_eq!(stale.len(), 3, "{:?}", report.issues);
    assert!(
        stale.iter().all(|i| i.severity == Severity::Blocking),
        "{:?}",
        report.issues
    );
    for issue in &stale {
        assert!(
            issue.detail.contains("2 stale session(s)"),
            "{}",
            issue.detail
        );
    }
    let denial = report
        .permits(DataUse::Backtest)
        .expect_err("stale required data must block backtest");
    assert!(denial.blocking_issues.contains(&IssueCode::DataStale));
}

#[test]
fn stale_close_warns_for_optional_with_declared_policy() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    let p = QualityPolicy {
        required_universe: BTreeSet::new(),
        optional_exclusion: Some(OptionalExclusion {
            reason: "daily recommendation only needs 2020-02-03 data".to_owned(),
        }),
        freshness: FreshnessPolicy {
            reference_date: TradingDate::parse("2020-02-05").expect("date"),
            max_stale_sessions: 0,
        },
        outlier_threshold_pct: 0.20,
    };
    let report = gate(&curated, p)
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Warning, "{:?}", report.issues);
    let stale: Vec<&QualityIssue> = report
        .issues
        .iter()
        .filter(|i| i.code == IssueCode::DataStale)
        .collect();
    assert!(!stale.is_empty());
    assert!(
        stale.iter().all(|i| i.severity == Severity::Warning),
        "{:?}",
        report.issues
    );
    assert_eq!(report.exclusions.len(), stale.len());
    for exclusion in &report.exclusions {
        assert_eq!(
            exclusion.reason,
            "daily recommendation only needs 2020-02-03 data"
        );
    }
    assert!(report.permits(DataUse::Recommendation).is_ok());
}

#[test]
fn stale_optional_without_policy_blocks() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    let report = gate(&curated, policy(&[], "2020-02-05", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked, "{:?}", report.issues);
    assert!(has_issue(&report, IssueCode::DataStale, Severity::Blocking));
}

#[test]
fn stale_within_grace_is_ready() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    // Reference 2020-02-04 with a 1-session grace: 02-04 is the only stale
    // session, so the dataset is still fresh enough to be READY.
    let report = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-04", 1),
    )
    .validate_dataset(&ds(), 1)
    .expect("validation runs");
    assert_eq!(report.state, DataState::Ready, "{:?}", report.issues);
    assert!(!has_issue(
        &report,
        IssueCode::DataStale,
        Severity::Blocking
    ));
}

#[test]
fn stale_required_data_returns_typed_data_stale() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    let report = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-06", 0),
    )
    .validate_dataset(&ds(), 1)
    .expect("validation runs");
    let stale = report
        .issues
        .iter()
        .find(|i| i.code == IssueCode::DataStale)
        .expect("DATA_STALE issue present");
    assert_eq!(stale.code.as_str(), "DATA_STALE");
    assert_eq!(stale.severity, Severity::Blocking);
    assert_eq!(report.state, DataState::Blocked);
}

// ---------------------------------------------------------------------------
// Admin approval: WARNING-class states only; structural BLOCKED requires a
// new dataset version
// ---------------------------------------------------------------------------

#[test]
fn approval_cannot_clear_structural_blocked() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate(temp.path(), MISSING_BARS, EMPTY_ACTIONS);
    let report = gate(&curated, policy(&["069500", "229200", "114260"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Blocked);
    let audit = market_data::quality::apply_approval(
        &report,
        &market_data::quality::AdminApproval {
            granted_by: "admin@lagrange".to_owned(),
            granted_at: now(),
            note: "approve anyway".to_owned(),
        },
    );
    assert!(!audit.approved, "approval must NOT clear a structural BLOCKED");
    assert_eq!(audit.state, DataState::Blocked);
    assert!(
        audit.reason.contains("new dataset version"),
        "reason must demand a new dataset version: {}",
        audit.reason
    );
    assert!(
        audit.reason.contains("MISSING_REQUIRED_BAR"),
        "reason must name the blocking codes: {}",
        audit.reason
    );
    assert_eq!(audit.report_hash, report.content_hash);
}

#[test]
fn approval_transitions_warning_to_ready() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate(temp.path(), MISSING_BARS, EMPTY_ACTIONS);
    let p = QualityPolicy {
        required_universe: universe(&["229200", "114260"]),
        optional_exclusion: Some(OptionalExclusion {
            reason: "trend_following tolerates missing 069500 bars".to_owned(),
        }),
        freshness: FreshnessPolicy {
            reference_date: TradingDate::parse("2020-02-03").expect("date"),
            max_stale_sessions: 0,
        },
        outlier_threshold_pct: 0.20,
    };
    let report = gate(&curated, p)
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Warning);
    let audit = market_data::quality::apply_approval(
        &report,
        &market_data::quality::AdminApproval {
            granted_by: "admin@lagrange".to_owned(),
            granted_at: now(),
            note: "warning-class issues are acceptable".to_owned(),
        },
    );
    assert!(audit.approved, "WARNING-class states are approvable");
    assert_eq!(audit.state, DataState::Ready);
    assert_eq!(audit.dataset_id, ds());
    assert_eq!(audit.version, 1);
    assert_eq!(audit.granted_by, "admin@lagrange");
    assert_eq!(audit.note, "warning-class issues are acceptable");
}

#[test]
fn approval_on_ready_is_a_noop() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate_golden(temp.path());
    let report = gate(&curated, policy(&["069500", "229200", "114260"], "2020-02-03", 0))
        .validate_dataset(&ds(), 1)
        .expect("validation runs");
    assert_eq!(report.state, DataState::Ready);
    let audit = market_data::quality::apply_approval(
        &report,
        &market_data::quality::AdminApproval {
            granted_by: "admin@lagrange".to_owned(),
            granted_at: now(),
            note: "nothing to do".to_owned(),
        },
    );
    assert!(!audit.approved, "approval only transitions WARNING-class states");
    assert_eq!(audit.state, DataState::Ready);
}

#[test]
fn only_a_new_dataset_version_resolves_structural_blocked() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, _) = curate(temp.path(), MISSING_BARS, EMPTY_ACTIONS);
    let gate = gate(&curated, policy(&["069500", "229200", "114260"], "2020-02-03", 0));
    let v1 = gate.validate_dataset(&ds(), 1).expect("v1 validation");
    assert_eq!(v1.state, DataState::Blocked);

    let approval = market_data::quality::AdminApproval {
        granted_by: "admin@lagrange".to_owned(),
        granted_at: now(),
        note: "approve anyway".to_owned(),
    };
    let denied = market_data::quality::apply_approval(&v1, &approval);
    assert!(!denied.approved);

    // The corrected dataset (the complete golden bars) curates as version 2.
    let (_, v2_outcome) = curate(temp.path(), GOLDEN_BARS, EMPTY_ACTIONS);
    assert_eq!(v2_outcome.dataset_version, 2);
    let v2 = gate.validate_dataset(&ds(), 2).expect("v2 validation");
    assert_eq!(v2.state, DataState::Ready, "{:?}", v2.issues);

    // The old version stays BLOCKED: approval cannot rewrite history.
    let v1_again = gate.validate_dataset(&ds(), 1).expect("v1 re-validation");
    assert_eq!(v1_again, v1);
}

#[test]
fn correction_creates_new_version_and_preserves_old() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, v1_outcome) = curate_golden(temp.path());
    assert_eq!(v1_outcome.dataset_version, 1);
    let gate = gate(
        &curated,
        policy(&["069500", "229200", "114260"], "2020-02-03", 0),
    );
    let v1_report = gate.validate_dataset(&ds(), 1).expect("v1 validation");
    assert_eq!(v1_report.state, DataState::Ready);
    let v1_manifest_hash = v1_outcome.manifest.content_hash.clone();

    // A corrected batch (one close changed) curates as version 2.
    let corrected: Vec<u8> = {
        let mut value: serde_json::Value =
            serde_json::from_slice(GOLDEN_BARS).expect("fixture parses");
        value["bars"][0]["close"] = json!(10100);
        serde_json::to_vec(&value).expect("corrected fixture serializes")
    };
    let (_, v2_outcome) = curate(temp.path(), &corrected, EMPTY_ACTIONS);
    assert_eq!(
        v2_outcome.dataset_version, 2,
        "correction must bump the version"
    );

    // The new version carries a NEW content hash; the old version's manifest
    // hash and quality report are untouched.
    assert_ne!(v2_outcome.manifest.content_hash, v1_manifest_hash);
    let v1_again = gate.validate_dataset(&ds(), 1).expect("v1 re-validation");
    assert_eq!(v1_again, v1_report, "old version must stay immutable");
    let v2_report = gate.validate_dataset(&ds(), 2).expect("v2 validation");
    assert_eq!(v2_report.state, DataState::Ready, "{:?}", v2_report.issues);
    assert_ne!(v2_report.content_hash, v1_report.content_hash);
}
