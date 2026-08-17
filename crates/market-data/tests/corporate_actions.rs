//! Todo 10 red-phase integration tests: point-in-time corporate actions.
//!
//! Pins the documented corporate-action behavior (requirements §8.2 action
//! fields, §8.3 point-in-time rules, §9.2 price policy) BEFORE the curation
//! pipeline exists: split value preservation on ex-date, pay-date dividend
//! cash credit, revision -> NEW dataset version with the old version byte
//! unchanged, nothing visible before `announced_at`, future-announced actions
//! rejected, and explicit `PRICE_RETURN_ONLY|TOTAL_RETURN_CAPABLE` capability.
//! Red phase: `market_data::curate` does not exist yet, so this file fails to
//! compile; the failing transcript is captured in the Todo 10 evidence bundle.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{Date32Builder, Decimal128Builder, StringBuilder, TimestampMicrosecondBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use domain::{
    BatchId, ContentHash, Currency, DataState, DatasetId, FixedPoint, Money, Price, Quantity,
    TradingDate, UtcTimestamp,
};
use parquet::arrow::ArrowWriter;
use serde_json::{Value, json};

use market_data::contract::{FetchMode, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::curate::actions::{
    CorporateActionType, DividendCredit, SplitAdjustment, visible_actions,
};
use market_data::curate::adjust::{AdjustmentKind, adjusted_series};
use market_data::curate::schema::{
    CORPORATE_ACTIONS_SCHEMA_VERSION, CORPORATE_ACTIONS_SCHEMA_VERSION_KEY, CuratedSchema,
};
use market_data::curate::{
    Capability, CurateError, CurateRequest, CurateStore, curate_batch, read_bars,
    read_corporate_actions,
};
use market_data::quality::{FreshnessPolicy, IssueCode, QualityGate, QualityPolicy, Severity};
use market_data::{BatchSpec, ManifestEntry, RawStore, krx_2020, seed_universe};

/// Pre-split bars for 069500.KRX (2020-01-20..2020-01-31) plus the post-split
/// 2020-02-03 bar (2:1 split ex-date; open 5150 = 10300 / 2).
const SPLIT_BARS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/variants/split-dividend/bars.json");
/// Split (2:1, ex 2020-02-03, announced 2020-01-22T06:00Z) + cash dividend
/// (150.00 KRW, ex 2020-02-03, pay 2020-02-14, announced 2020-01-23T06:00Z).
const SPLIT_ACTIONS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/variants/split-dividend/actions.json");
const GOLDEN_BARS: &[u8] = include_bytes!("../../../tests/fixtures/kr-etf/2020-01-31/bars.json");
const EMPTY_ACTIONS: &[u8] =
    include_bytes!("../../../tests/fixtures/kr-etf/contract/corporate-actions-response.json");

fn now() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2020-02-10T00:00:00Z").expect("valid clock")
}

fn dataset() -> DatasetId {
    DatasetId::parse("kr-etf-daily").expect("valid dataset id")
}

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

fn bars_path(curated: &CurateStore, version: u32) -> std::path::PathBuf {
    curated.bars_path("kr", "069500.KRX", 2020, version)
}

fn actions_path(curated: &CurateStore, version: u32) -> std::path::PathBuf {
    curated.corporate_actions_path("kr", "069500.KRX", 2020, version)
}

/// Writes the pre-v2 action schema used by the direct-read compatibility test.
/// This helper intentionally lives only in the test: production curation must
/// never emit or mutate this legacy layout.
fn write_legacy_actions(path: &Path, action: &market_data::CorporateAction) {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    let days = |date: TradingDate| (date.as_naive_date() - epoch).num_days() as i32;
    let micros = |timestamp: UtcTimestamp| timestamp.as_datetime().timestamp_micros();

    let mut instrument = StringBuilder::new();
    instrument.append_value(action.instrument_id.to_string());
    let mut event_type = StringBuilder::new();
    event_type.append_value(action.event_type.as_str());
    let mut ex_date = Date32Builder::new();
    ex_date.append_value(days(action.ex_date));
    let mut record_date = Date32Builder::new();
    record_date.append_null();
    let mut pay_date = Date32Builder::new();
    pay_date.append_null();
    let mut ratio = StringBuilder::new();
    ratio.append_value(action.ratio.as_deref().unwrap_or("2:1"));
    let mut split_factor = Decimal128Builder::new()
        .with_precision_and_scale(18, 8)
        .expect("decimal params");
    split_factor.append_value(
        action
            .split_factor
            .expect("legacy fixture has split factor")
            .with_scale(8)
            .expect("split scale")
            .bits(),
    );
    let mut amount_per_share = Decimal128Builder::new()
        .with_precision_and_scale(18, 4)
        .expect("decimal params");
    amount_per_share.append_null();
    let mut tax_withholding_pct = Decimal128Builder::new()
        .with_precision_and_scale(18, 6)
        .expect("decimal params");
    tax_withholding_pct.append_null();
    let mut currency = StringBuilder::new();
    currency.append_value(action.currency.code());
    let mut announced_at = TimestampMicrosecondBuilder::new();
    let announced = action
        .announced_at
        .expect("legacy fixture has announcement");
    announced_at.append_value(micros(announced));
    let mut source = StringBuilder::new();
    source.append_value(&action.source);
    let mut batch_id = StringBuilder::new();
    batch_id.append_value(action.batch_id.to_string());
    let mut raw_hash = StringBuilder::new();
    raw_hash.append_value(action.raw_hash.as_str());
    let mut ingested_at = TimestampMicrosecondBuilder::new();
    ingested_at.append_value(micros(action.ingested_at));

    // This is exactly the v1 field set: announced_at was mandatory and there
    // was no available_at column or schema metadata.
    let schema = Schema::new(vec![
        Field::new("instrument_id", DataType::Utf8, false),
        Field::new("event_type", DataType::Utf8, false),
        Field::new("ex_date", DataType::Date32, false),
        Field::new("record_date", DataType::Date32, true),
        Field::new("pay_date", DataType::Date32, true),
        Field::new("ratio", DataType::Utf8, true),
        Field::new("split_factor", DataType::Decimal128(18, 8), true),
        Field::new("amount_per_share", DataType::Decimal128(18, 4), true),
        Field::new("tax_withholding_pct", DataType::Decimal128(18, 6), true),
        Field::new("currency", DataType::Utf8, false),
        Field::new(
            "announced_at",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            false,
        ),
        Field::new("source", DataType::Utf8, false),
        Field::new("batch_id", DataType::Utf8, false),
        Field::new("raw_hash", DataType::Utf8, false),
        Field::new(
            "ingested_at",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            false,
        ),
    ]);
    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(instrument.finish()),
            Arc::new(event_type.finish()),
            Arc::new(ex_date.finish()),
            Arc::new(record_date.finish()),
            Arc::new(pay_date.finish()),
            Arc::new(ratio.finish()),
            Arc::new(split_factor.finish()),
            Arc::new(amount_per_share.finish()),
            Arc::new(tax_withholding_pct.finish()),
            Arc::new(currency.finish()),
            Arc::new(announced_at.finish()),
            Arc::new(source.finish()),
            Arc::new(batch_id.finish()),
            Arc::new(raw_hash.finish()),
            Arc::new(ingested_at.finish()),
        ],
    )
    .expect("legacy record batch");
    let file = File::create(path).expect("legacy parquet path");
    let mut writer = ArrowWriter::try_new(file, Arc::new(schema), None).expect("legacy writer");
    writer.write(&batch).expect("legacy write");
    writer.close().expect("legacy close");
}

/// Curates the split+dividend fixture at version 1.
fn curate_split_fixture(
    temp: &Path,
    actions: &[u8],
    now: UtcTimestamp,
) -> (
    CurateStore,
    market_data::curate::CurateOutcome,
    ManifestEntry,
) {
    let (raw, entry) = fixture_batch(temp, SPLIT_BARS, actions);
    let curated = CurateStore::new(temp.join("data"));
    let outcome = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &dataset(),
            market: "kr",
            source: "krx",
            now,
        },
    )
    .expect("split fixture curates");
    (curated, outcome, entry)
}

// ---------------------------------------------------------------------------
// Split: value preservation + ex-date holding adjustment
// ---------------------------------------------------------------------------

#[test]
fn split_value_preserved_on_ex_date() {
    // 100 shares at the pre-split close 10300 == 200 shares at the post-split
    // price 5150 (fixture invariant: pre-split close 10300 = 2 x 5150).
    let pre_qty = Quantity::parse("100").expect("qty");
    let pre_price = Price::parse("10300").expect("price");
    let post_qty = Quantity::parse("200").expect("qty");
    let post_price = Price::parse("5150").expect("price");
    assert!(
        SplitAdjustment::value_preserved(
            &pre_qty,
            &pre_price,
            &post_qty,
            &post_price,
            Currency::KRW
        )
        .expect("value comparison")
    );
    assert_eq!(
        SplitAdjustment::apply_to_holdings(&pre_qty, &FixedPoint::parse("2").expect("factor"))
            .expect("holdings adjust"),
        post_qty
    );
}

#[test]
fn split_back_adjusts_prices_before_ex_date() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let actions = read_corporate_actions(&actions_path(&curated, outcome.dataset_version))
        .expect("actions read back");
    let split = actions
        .iter()
        .find(|a| a.event_type == CorporateActionType::Split)
        .expect("split present");
    assert_eq!(split.ex_date, TradingDate::new(2020, 2, 3).expect("date"));
    assert_eq!(
        split.split_factor.expect("factor"),
        FixedPoint::parse("2").expect("factor")
    );

    let series = adjusted_series(
        &read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read back"),
        &visible_actions(&actions, now()),
    )
    .expect("adjusted series");

    // 2020-01-31 (pre-ex): raw close 10300 -> split-adjusted 5150.0000.
    let jan_31 = series
        .split
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31 bar");
    assert_eq!(jan_31.close, Price::parse("5150").expect("price"));
    assert_eq!(jan_31.open, Price::parse("5135").expect("price"));
    assert_eq!(jan_31.adjustment_kind, AdjustmentKind::Split);
    assert!(!jan_31.adjustment_events.is_empty());

    // 2020-02-03 (on/after ex-date): raw prices are already post-split.
    let feb_03 = series
        .split
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-02-03")
        .expect("feb 03 bar");
    assert_eq!(feb_03.close, Price::parse("5190").expect("price"));
    assert_eq!(
        feb_03.adjustment_factor,
        FixedPoint::parse("1").expect("factor")
    );
}

// ---------------------------------------------------------------------------
// Dividend: pay-date cash credit
// ---------------------------------------------------------------------------

#[test]
fn dividend_credited_on_pay_date_not_ex_date() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let actions = read_corporate_actions(&actions_path(&curated, outcome.dataset_version))
        .expect("actions read back");
    let div = actions
        .iter()
        .find(|a| a.event_type == CorporateActionType::CashDividend)
        .expect("dividend present");
    assert_eq!(div.ex_date, TradingDate::new(2020, 2, 3).expect("date"));
    assert_eq!(
        div.pay_date,
        Some(TradingDate::new(2020, 2, 14).expect("date")),
        "pay_date must be configured (not derived from ex_date)"
    );
    // Cash is credited on the configured pay-date, never on the ex-date.
    assert_eq!(
        DividendCredit::credit_date(div).expect("credit date"),
        TradingDate::new(2020, 2, 14).expect("date")
    );
    // 200 post-split shares x 150.00 KRW = 30,000.00 KRW gross.
    let qty = Quantity::parse("200").expect("qty");
    let amount = Money::parse("150.00", Currency::KRW).expect("money");
    assert_eq!(
        DividendCredit::gross_credit(&qty, &amount).expect("gross credit"),
        Money::parse("30000.00", Currency::KRW).expect("money")
    );
}

#[test]
fn dividend_without_pay_date_caps_dataset_at_price_return_only() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut value: Value = serde_json::from_slice(SPLIT_ACTIONS).expect("parses");
    let div = value
        .get_mut("actions")
        .expect("actions array")
        .as_array_mut()
        .expect("array")
        .iter_mut()
        .find(|a| a["type"] == "cash_dividend")
        .expect("dividend record");
    div.as_object_mut().expect("object").remove("pay_date");
    let actions = serde_json::to_vec(&value).expect("serializes");

    let (_, outcome, _) = curate_split_fixture(temp.path(), &actions, now());
    assert_eq!(
        outcome.capability,
        Capability::PriceReturnOnly,
        "missing dividend pay-date data must cap the dataset at price returns"
    );
}

// ---------------------------------------------------------------------------
// Point-in-time visibility (no look-ahead)
// ---------------------------------------------------------------------------

#[test]
fn action_invisible_before_announced_at_visible_after() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let actions = read_corporate_actions(&actions_path(&curated, outcome.dataset_version))
        .expect("actions read back");
    assert_eq!(actions.len(), 2);

    let before_split = UtcTimestamp::parse_rfc3339("2020-01-22T05:59:59Z").expect("ts");
    let at_split = UtcTimestamp::parse_rfc3339("2020-01-22T06:00:00Z").expect("ts");
    let after_div = UtcTimestamp::parse_rfc3339("2020-01-23T06:00:00Z").expect("ts");

    assert_eq!(
        visible_actions(&actions, before_split).len(),
        0,
        "nothing may be visible before its announced_at"
    );
    assert_eq!(visible_actions(&actions, at_split).len(), 1);
    assert_eq!(visible_actions(&actions, after_div).len(), 2);
}

#[test]
fn adjusted_series_as_of_before_announcement_equals_raw() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let actions = read_corporate_actions(&actions_path(&curated, outcome.dataset_version))
        .expect("actions read back");
    let bars = read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read back");

    // As-of before any announcement: no action is visible, so the adjusted
    // series must equal the raw prices (factor 1.0 everywhere).
    let as_of = UtcTimestamp::parse_rfc3339("2020-01-22T05:59:59Z").expect("ts");
    let series = adjusted_series(&bars, &visible_actions(&actions, as_of)).expect("series");
    assert_eq!(series.split.len(), bars.len());
    for (adj, raw) in series.split.iter().zip(bars.iter()) {
        assert_eq!(adj.close, raw.close, "no look-ahead: raw prices untouched");
        assert_eq!(
            adj.adjustment_factor,
            FixedPoint::parse("1").expect("factor"),
            "factor must be 1.0 when the announcement is not yet visible"
        );
    }

    // As-of after the split announcement: the 2:1 split back-adjusts
    // pre-ex-date prices by 0.5.
    let as_of = UtcTimestamp::parse_rfc3339("2020-01-22T06:00:00Z").expect("ts");
    let series = adjusted_series(&bars, &visible_actions(&actions, as_of)).expect("series");
    let jan_31 = series
        .split
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31");
    assert_eq!(jan_31.close, Price::parse("5150").expect("price"));
}

#[test]
fn future_announced_action_rejected_at_curation() {
    // Curate with a clock BEFORE the split's announced_at: the announcement
    // is in the future and must be rejected, not silently included.
    let before_announcement = UtcTimestamp::parse_rfc3339("2020-01-22T05:59:59Z").expect("ts");
    let temp = tempfile::tempdir().expect("temp dir");
    let (raw, entry) = fixture_batch(temp.path(), SPLIT_BARS, SPLIT_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &dataset(),
            market: "kr",
            source: "krx",
            now: before_announcement,
        },
    )
    .expect_err("future-announced action must be rejected");
    assert!(
        matches!(err, CurateError::FutureAnnouncedAction { .. }),
        "{err}"
    );
}

#[test]
fn legacy_action_readback_is_compatible_but_quality_gate_requires_schema_v2() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let current_path = actions_path(&curated, outcome.dataset_version);
    let current = read_corporate_actions(&current_path).expect("current actions");
    let split = current
        .iter()
        .find(|action| action.event_type == CorporateActionType::Split)
        .expect("split action");
    let legacy_path = temp.path().join("legacy-corporate-actions.parquet");
    write_legacy_actions(&legacy_path, split);

    // Direct consumers retain a deliberately narrow compatibility path: the
    // old mandatory announcement is used as availability when no explicit
    // available_at column exists.
    let legacy = read_corporate_actions(&legacy_path).expect("legacy readback");
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].announced_at, split.announced_at);
    assert_eq!(legacy[0].available_at, split.announced_at.unwrap());

    // Put the legacy bytes in a disposable test version only. The production
    // writer never overwrites an existing generation; this simulates an old
    // installed partition being inspected by the new quality gate.
    write_legacy_actions(&current_path, split);
    let report = QualityGate::new(
        curated.clone(),
        krx_2020(),
        seed_universe(),
        "kr",
        QualityPolicy {
            required_universe: ["069500.KRX".parse().expect("instrument")]
                .into_iter()
                .collect(),
            optional_exclusion: None,
            freshness: FreshnessPolicy {
                reference_date: TradingDate::parse("2020-02-03").expect("date"),
                max_stale_sessions: 0,
            },
            outlier_threshold_pct: 0.20,
        },
    )
    .validate_dataset(&dataset(), outcome.dataset_version)
    .expect("quality gate runs");
    assert_eq!(report.state, DataState::Blocked);
    assert!(report.issues.iter().any(
        |issue| issue.code == IssueCode::SchemaMismatch && issue.severity == Severity::Blocking
    ));
    assert_eq!(
        CuratedSchema::corporate_actions()
            .metadata()
            .get(CORPORATE_ACTIONS_SCHEMA_VERSION_KEY)
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(CORPORATE_ACTIONS_SCHEMA_VERSION, 2);
    assert_ne!(CuratedSchema::corporate_actions(), {
        // A legacy schema has no availability column and no v2 metadata.
        let mut fields = CuratedSchema::corporate_actions().fields().to_vec();
        fields.retain(|field| field.name() != "available_at");
        Schema::new(fields)
    });
}

// ---------------------------------------------------------------------------
// Revision -> NEW dataset version; the old version is immutable
// ---------------------------------------------------------------------------

#[test]
fn correction_creates_new_dataset_version_old_unchanged() {
    let temp = tempfile::tempdir().expect("temp dir");
    // Version 1: golden close 10300 for 069500.KRX on 2020-02-03.
    let (raw1, entry1) = fixture_batch(temp.path(), GOLDEN_BARS, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    let v1 = curate_batch(
        &raw1,
        &entry1,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &dataset(),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect("v1 curates");
    assert_eq!(v1.dataset_version, 1);

    let v1_path = bars_path(&curated, 1);
    let v1_bytes_before = std::fs::read(&v1_path).expect("v1 bars readable");
    let v1_bars_before = read_bars(&v1_path).expect("v1 bars read back");
    let feb_03_v1 = v1_bars_before
        .iter()
        .find(|b| {
            b.instrument_id.to_string() == "069500.KRX" && b.trading_date.to_iso() == "2020-02-03"
        })
        .expect("bar present");
    assert_eq!(feb_03_v1.close, Price::parse("10380").expect("price"));

    // Correction: a second, different batch corrects the close to 10400.
    let mut corrected: Value = serde_json::from_slice(GOLDEN_BARS).expect("parses");
    for bar in corrected
        .get_mut("bars")
        .expect("bars array")
        .as_array_mut()
        .expect("array")
    {
        if bar["instrument"].as_str() == Some("069500.KRX") && bar["date"] == "2020-02-03" {
            bar["close"] = json!(10400);
        }
    }
    let corrected_bytes = serde_json::to_vec(&corrected).expect("serializes");
    let (raw2, entry2) = fixture_batch(temp.path(), &corrected_bytes, EMPTY_ACTIONS);
    let v2 = curate_batch(
        &raw2,
        &entry2,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &dataset(),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect("corrected batch curates as a new version");
    assert_eq!(
        v2.dataset_version, 2,
        "correction must produce a NEW dataset version, never a backfill"
    );

    // The old version's bytes and hash are unchanged.
    let v1_bytes_after = std::fs::read(&v1_path).expect("v1 bars still readable");
    assert_eq!(
        v1_bytes_before, v1_bytes_after,
        "version 1 parquet immutable"
    );
    assert_eq!(
        ContentHash::from_bytes(&v1_bytes_after),
        ContentHash::from_bytes(&v1_bytes_before),
        "version 1 content hash unchanged"
    );

    // The new version carries the corrected close.
    let v2_bars = read_bars(&bars_path(&curated, 2)).expect("v2 bars read back");
    let feb_03_v2 = v2_bars
        .iter()
        .find(|b| {
            b.instrument_id.to_string() == "069500.KRX" && b.trading_date.to_iso() == "2020-02-03"
        })
        .expect("bar present");
    assert_eq!(feb_03_v2.close, Price::parse("10400").expect("price"));
}

#[test]
fn duplicate_curation_of_same_batch_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (raw, entry) = fixture_batch(temp.path(), GOLDEN_BARS, EMPTY_ACTIONS);
    let curated = CurateStore::new(temp.path().join("data"));
    curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &dataset(),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect("first curation ok");
    let err = curate_batch(
        &raw,
        &entry,
        &krx_2020(),
        &seed_universe(),
        &curated,
        &CurateRequest {
            dataset_id: &dataset(),
            market: "kr",
            source: "krx",
            now: now(),
        },
    )
    .expect_err("re-curating the same batch must be rejected");
    assert!(
        matches!(err, CurateError::BatchAlreadyCurated { .. }),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// Capability + execution split
// ---------------------------------------------------------------------------

#[test]
fn capability_total_return_with_complete_pay_dates() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (_, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    assert_eq!(outcome.capability, Capability::TotalReturnCapable);
    assert_eq!(outcome.actions_written, 2);
}

#[test]
fn raw_open_for_execution_vs_adjusted_open_for_signals() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, _) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let raw_bars = read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read");
    let actions = read_corporate_actions(&actions_path(&curated, outcome.dataset_version))
        .expect("actions read");
    let series = adjusted_series(&raw_bars, &actions).expect("series");

    let jan_31_raw = raw_bars
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31 raw");
    let jan_31_adj = series
        .split
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31 adjusted");

    // Execution uses the raw open; signals may use the adjusted series.
    assert_eq!(jan_31_raw.open, Price::parse("10270").expect("price"));
    assert_eq!(jan_31_adj.open, Price::parse("5135").expect("price"));
    assert_ne!(jan_31_raw.open, jan_31_adj.open);
}

// ---------------------------------------------------------------------------
// QA channel: curate the split+dividend fixture end-to-end
// ---------------------------------------------------------------------------

#[test]
fn qa_curate_split_dividend_fixture_end_to_end() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (curated, outcome, entry) = curate_split_fixture(temp.path(), SPLIT_ACTIONS, now());
    let raw_bars = read_bars(&bars_path(&curated, outcome.dataset_version)).expect("bars read");
    let actions = read_corporate_actions(&actions_path(&curated, outcome.dataset_version))
        .expect("actions read");
    let series = adjusted_series(&raw_bars, &actions).expect("series");

    println!("=== QA: split+dividend fixture curated end-to-end ===");
    println!(
        "dataset version: {} capability: {}",
        outcome.dataset_version, outcome.capability
    );
    println!(
        "batch: {} bars: {} actions: {}",
        entry.batch_id, outcome.bars_written, outcome.actions_written
    );

    let feb_03 = raw_bars
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-02-03")
        .expect("feb 03 raw");
    println!(
        "session 2020-02-03 open: {} close: {} (raw open {})",
        feb_03.market_open_ts, feb_03.market_close_ts, feb_03.open
    );
    assert_eq!(
        feb_03.market_open_ts,
        UtcTimestamp::parse_rfc3339("2020-02-03T00:00:00Z").expect("ts")
    );
    assert_eq!(
        feb_03.market_close_ts,
        UtcTimestamp::parse_rfc3339("2020-02-03T06:30:00Z").expect("ts")
    );

    let jan_31 = raw_bars
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31 raw");
    let jan_31_split = series
        .split
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31 split");
    let jan_31_tr = series
        .total_return
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-01-31")
        .expect("jan 31 total return");

    println!(
        "2020-01-31 raw OHL: {} {} {} C {} V {}",
        jan_31.open, jan_31.high, jan_31.low, jan_31.close, jan_31.volume
    );
    println!(
        "2020-01-31 split-adjusted: O {} H {} L {} C {} factor {} events [{}]",
        jan_31_split.open,
        jan_31_split.high,
        jan_31_split.low,
        jan_31_split.close,
        jan_31_split.adjustment_factor,
        jan_31_split.adjustment_events
    );
    println!(
        "2020-01-31 total-return: O {} H {} L {} C {} factor {} events [{}]",
        jan_31_tr.open,
        jan_31_tr.high,
        jan_31_tr.low,
        jan_31_tr.close,
        jan_31_tr.adjustment_factor,
        jan_31_tr.adjustment_events
    );

    // Hand-computed reconciliation (fixed-point, deterministic):
    //   split close = 10300 / 2 = 5150.0000 (exact)
    //   total-return factor = 0.5 x (5190 + 150) / 5190, composed at scale 8:
    //     0.50000000 x 1.02890173 = 0.51445086
    //   total-return close = 10300 x 0.51445086 = 5298.8439 (scale 4)
    //   total-return open  = 10270 x 0.51445086 = 5283.4103 (scale 4)
    assert_eq!(jan_31_split.close, Price::parse("5150").expect("price"));
    let split_back = SplitAdjustment::back_adjust_factor(&FixedPoint::parse("2").expect("factor"))
        .expect("back factor");
    let dividend_factor = FixedPoint::parse("5340")
        .expect("factor num")
        .checked_div(&FixedPoint::parse("5190").expect("factor den"), 8)
        .expect("factor");
    let expected_factor = split_back
        .checked_mul(&dividend_factor)
        .expect("factor mul")
        .with_scale(8)
        .expect("factor scale");
    assert_eq!(jan_31_tr.adjustment_factor, expected_factor);
    assert_eq!(
        jan_31_tr.close,
        Price::parse("5298.8439").expect("price"),
        "total-return close must equal 10300 x 0.51445086 at scale 4"
    );
    assert_eq!(
        jan_31_tr.open,
        Price::parse("5283.4103").expect("price"),
        "total-return open must equal 10270 x 0.51445086 at scale 4"
    );

    // The 2020-02-03 bar is on/after both ex-dates: no adjustment applies.
    let feb_03_tr = series
        .total_return
        .iter()
        .find(|b| b.trading_date.to_iso() == "2020-02-03")
        .expect("feb 03 tr");
    assert_eq!(
        feb_03_tr.adjustment_factor,
        FixedPoint::parse("1").expect("factor")
    );
    assert_eq!(feb_03_tr.close, Price::parse("5190").expect("price"));

    // The versioned adjusted files on disk must round-trip exactly.
    let split_path = curated.adjusted_bars_path("kr", "069500.KRX", 2020, outcome.dataset_version);
    let tr_path = curated.total_return_bars_path("kr", "069500.KRX", 2020, outcome.dataset_version);
    let split_on_disk =
        market_data::curate::read_adjusted_bars(&split_path).expect("read split file");
    let tr_on_disk = market_data::curate::read_adjusted_bars(&tr_path).expect("read tr file");
    assert_eq!(
        split_on_disk, series.split,
        "split series round-trips through parquet"
    );
    assert_eq!(
        tr_on_disk, series.total_return,
        "total-return series round-trips through parquet"
    );

    // Dividend record carries the configured pay-date (not the ex-date).
    let div = actions
        .iter()
        .find(|a| a.event_type == CorporateActionType::CashDividend)
        .expect("dividend");
    assert_eq!(
        div.pay_date,
        Some(TradingDate::new(2020, 2, 14).expect("date"))
    );
    println!("=== QA end ===");
}
