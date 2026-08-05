//! Todo 10 red-phase integration tests: curated bar normalization and provenance.
//!
//! These tests pin the documented Curated contract (requirements §8.2 bar
//! fields; design §7.1 partition layout) BEFORE the curation pipeline exists.
//! Red phase: the `market_data::curate` module does not exist yet, so this
//! file fails to compile. The failing transcript is captured in the Todo 10
//! evidence bundle, then the implementation turns it green.

use std::fs::File;
use std::path::Path;

use arrow::datatypes::DataType;
use domain::{BatchId, ContentHash, Currency, DatasetId, Price, TradingDate, UtcTimestamp};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::{Value, json};

use market_data::contract::{FetchMode, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::curate::schema::CuratedSchema;
use market_data::curate::{CurateError, CurateRequest, CurateStore, curate_batch, read_bars};
use market_data::{BatchSpec, ManifestEntry, RawStore, krx_2020, seed_universe};

/// The golden Todo 6 fixture: 3 seed ETFs, 27 bars, no corporate actions.
const GOLDEN_BARS: &[u8] = include_bytes!("../../../tests/fixtures/kr-etf/2020-01-31/bars.json");
const EMPTY_ACTIONS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/contract/corporate-actions-response.json");

/// A curation clock after every fixture `announced_at` (fixtures are 2020).
fn now() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2020-02-10T00:00:00Z").expect("valid clock")
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

/// Curates the golden fixture into a fresh temp store, returning the store,
/// the outcome, and the batch id.
fn curate_golden(
    temp: &Path,
) -> (
    RawStore,
    CurateStore,
    ManifestEntry,
    market_data::curate::CurateOutcome,
) {
    let (raw, entry) = fixture_batch(temp, GOLDEN_BARS, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.join("data"));
    let outcome = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid dataset id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect("golden fixture curates");
    (raw, curated, entry, outcome)
}

fn bars_path(curated: &CurateStore, version: u32) -> std::path::PathBuf {
    curated.bars_path("kr", "069500.KRX", 2020, version)
}

/// Mutates a field of bar `i` in the golden fixture JSON.
fn mutated_golden_bars(mutate: impl Fn(&mut Value)) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(GOLDEN_BARS).expect("fixture parses");
    mutate(&mut value);
    serde_json::to_vec(&value).expect("mutated fixture serializes")
}

// ---------------------------------------------------------------------------
// OHLC normalization
// ---------------------------------------------------------------------------

#[test]
fn ohlc_high_below_close_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["bars"][0]["high"] = json!(1)); // close 10250
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("impossible OHLC must be rejected");
    assert!(matches!(err, CurateError::ImpossibleOhlc { .. }), "{err}");
}

#[test]
fn ohlc_low_above_open_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["bars"][0]["low"] = json!(99000));
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("impossible OHLC must be rejected");
    assert!(matches!(err, CurateError::ImpossibleOhlc { .. }), "{err}");
}

#[test]
fn negative_price_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["bars"][0]["open"] = json!(-10150));
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("negative price must be rejected");
    assert!(matches!(err, CurateError::NonPositivePrice { .. }), "{err}");
}

#[test]
fn zero_price_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["bars"][0]["close"] = json!(0));
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("zero price must be rejected");
    assert!(matches!(err, CurateError::NonPositivePrice { .. }), "{err}");
}

#[test]
fn non_finite_price_rejected() {
    // 1e999 overflows f64 at the JSON boundary: serde_json refuses to parse
    // it into a Value, so the pipeline rejects the raw bytes with a typed
    // error instead of ever curating a non-finite price.
    let temp = tempfile::tempdir().expect("temp dir");
    let raw = String::from_utf8_lossy(GOLDEN_BARS).replace("\"high\": 10280", "\"high\": 1e999");
    let (raw_store, entry) = fixture_batch(temp.path(), raw.as_bytes(), EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw_store,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("non-finite price must be rejected at the JSON boundary");
    assert!(
        matches!(err, CurateError::MalformedBars { .. }),
        "expected MalformedBars, got {err}"
    );
}

#[test]
fn negative_volume_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["bars"][0]["volume"] = json!(-100));
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("negative volume must be rejected");
    assert!(matches!(err, CurateError::NegativeVolume { .. }), "{err}");
}

#[test]
fn conflicting_currency_rejected() {
    // Dataset-level currency USD while the instrument master says KRW.
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["currency"] = json!("USD"));
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("conflicting currency must be rejected");
    assert!(matches!(err, CurateError::CurrencyConflict { .. }), "{err}");
}

#[test]
fn bar_on_non_session_date_rejected() {
    // 2020-02-01 is a Saturday - never a KRX session.
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| {
        v["bars"].as_array_mut().expect("array").push(json!({
            "instrument": "069500.KRX",
            "date": "2020-02-01",
            "open": 10300, "high": 10400, "low": 10200, "close": 10350,
            "volume": 1000, "value": 10000000
        }));
    });
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("non-session bar must be rejected");
    assert!(matches!(err, CurateError::NotASession { .. }), "{err}");
}

#[test]
fn unknown_instrument_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| v["bars"][0]["instrument"] = json!("999999.KRX"));
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("unknown instrument must be rejected");
    assert!(
        matches!(err, CurateError::UnknownInstrument { .. }),
        "{err}"
    );
}

#[test]
fn duplicate_bar_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let bars = mutated_golden_bars(|v| {
        let duplicate = v["bars"][0].clone();
        v["bars"].as_array_mut().expect("array").push(duplicate);
    });
    let (raw, entry) = fixture_batch(temp.path(), &bars, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &DatasetId::parse("kr-etf-daily").expect("valid id"),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("duplicate (instrument, date) must be rejected");
    assert!(matches!(err, CurateError::DuplicateBar { .. }), "{err}");
}

// ---------------------------------------------------------------------------
// Curated output contract
// ---------------------------------------------------------------------------

#[test]
fn curated_schema_matches_documented_fields() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, curated, _, outcome) = curate_golden(temp.path());
    let path = bars_path(&curated, outcome.dataset_version);
    assert!(path.exists(), "bars parquet written at {path:?}");
    let file = File::open(&path).expect("open bars parquet");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("read parquet header");
    assert_eq!(
        builder.schema().as_ref(),
        &CuratedSchema::bars(),
        "parquet schema must match the documented Curated bar fields exactly"
    );
}

#[test]
fn no_nan_or_negative_prices_in_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, curated, _, outcome) = curate_golden(temp.path());
    // Per-symbol partitions (design §7.1): one parquet file per instrument.
    let mut total = 0usize;
    for symbol in ["069500.KRX", "229200.KRX", "114260.KRX"] {
        let path = curated.bars_path("kr", symbol, 2020, outcome.dataset_version);
        assert!(path.exists(), "partition written for {symbol}");
        let file = File::open(&path).expect("open bars parquet");
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("read header");
        // No float columns at all: NaN is structurally impossible in the output.
        for field in builder.schema().fields() {
            assert!(
                !matches!(field.data_type(), DataType::Float32 | DataType::Float64),
                "column {} must not be a float type (NaN structurally impossible)",
                field.name()
            );
        }
        let bars = read_bars(&path).expect("bars read back");
        total += bars.len();
        for bar in &bars {
            assert!(bar.open.amount().is_positive());
            assert!(bar.high.amount().is_positive());
            assert!(bar.low.amount().is_positive());
            assert!(bar.close.amount().is_positive());
            assert!(bar.volume >= 0, "volume non-negative");
        }
    }
    assert_eq!(total, 27, "all 27 golden bars curated across partitions");
}

#[test]
fn session_open_close_timestamps_come_from_calendar() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, curated, _, outcome) = curate_golden(temp.path());
    let bars = read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read back");
    let feb_03 = bars
        .iter()
        .find(|b| {
            b.instrument_id.to_string() == "069500.KRX" && b.trading_date.to_iso() == "2020-02-03"
        })
        .expect("2020-02-03 bar present");
    // KRX session 09:00-15:30 KST = 00:00-06:30 UTC (design §6.4, Todo 9).
    assert_eq!(
        feb_03.market_open_ts,
        UtcTimestamp::parse_rfc3339("2020-02-03T00:00:00Z").expect("ts")
    );
    assert_eq!(
        feb_03.market_close_ts,
        UtcTimestamp::parse_rfc3339("2020-02-03T06:30:00Z").expect("ts")
    );
}

#[test]
fn partition_layout_follows_design() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, curated, _, outcome) = curate_golden(temp.path());
    let path = bars_path(&curated, outcome.dataset_version);
    let rel = path
        .strip_prefix(temp.path())
        .expect("path under temp")
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        rel.contains("curated/bars/market=kr/symbol=069500.KRX/year=2020/version=1/bars.parquet"),
        "unexpected partition layout: {rel}"
    );
}

#[test]
fn raw_open_preserved_for_execution() {
    // Execution always uses the RAW open (requirements §9.2): the raw table
    // must carry the exact provider open, untouched by any adjustment.
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, curated, _, outcome) = curate_golden(temp.path());
    let bars = read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read back");
    let bar = bars
        .iter()
        .find(|b| {
            b.instrument_id.to_string() == "069500.KRX" && b.trading_date.to_iso() == "2020-02-03"
        })
        .expect("bar present");
    assert_eq!(bar.open, Price::parse("10300").expect("price"));
}

#[test]
fn bars_carry_batch_and_raw_hash_provenance() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, curated, entry, outcome) = curate_golden(temp.path());
    let bars = read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read back");
    let bar = &bars[0];
    assert_eq!(bar.source, "krx");
    assert_eq!(bar.batch_id, entry.batch_id);
    assert_eq!(bar.raw_hash, ContentHash::from_bytes(GOLDEN_BARS));
    assert_eq!(bar.ingested_at, now());
    assert_eq!(bar.currency, Currency::KRW);
    assert_eq!(
        bar.trading_date,
        TradingDate::new(2020, 1, 20).expect("date")
    );
}

#[test]
fn manifest_records_dataset_version_and_counts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, _, _, outcome) = curate_golden(temp.path());
    assert_eq!(outcome.dataset_version, 1);
    assert_eq!(outcome.bars_written, 27);
    assert_eq!(outcome.actions_written, 0);
    let manifest = outcome.manifest;
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.bar_count, 27);
    assert_eq!(manifest.source_batches.len(), 1);
    assert_eq!(
        manifest.source_batches[0].bars_hash,
        ContentHash::from_bytes(GOLDEN_BARS)
    );
}
