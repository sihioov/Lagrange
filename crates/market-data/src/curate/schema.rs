//! Curated Parquet schema and row codecs (Todo 10).
//!
//! The schema below is the **documented Curated bar contract** (requirements
//! §8.2 일봉 가격: `instrument_id, trading_date, market_open_ts,
//! market_close_ts, open, high, low, close, volume, currency, source,
//! ingested_at, raw_hash`) plus the adjustment/corporate-action extension
//! columns (design §7.1 layout, §9.3 corporate-action model).
//!
//! Encoding choices (documented, deterministic):
//! - prices: `Decimal128(18, 4)` — the domain's canonical [`Price`] scale;
//!   **no float columns exist anywhere**, so NaN is structurally impossible;
//! - timestamps: `Timestamp(Microsecond, None)` — UTC epoch microseconds,
//!   normalized from the venue calendar (design §6.4);
//! - dates: `Date32` (days since 1970-01-01);
//! - volume/trading value: `Int64` (reported provider units, never adjusted);
//! - every row carries `source`, `ingested_at`, `batch_id`, `raw_hash` so the
//!   curated zone is fully provenance-linked to its immutable Raw batch.
//!
//! Partition layout (design §7.1 + versioning mandate requirements §8.3):
//! ```text
//! data/curated/
//!   bars/market={m}/symbol={instrument}/year={yyyy}/version={v}/bars.parquet
//!   bars/.../version={v}/adjusted_bars.parquet
//!   bars/.../version={v}/total_return_bars.parquet
//!   corporate_actions/market={m}/symbol={instrument}/year={yyyy}/version={v}/corporate_actions.parquet
//!   datasets/{dataset_id}/version={v}/manifest.json
//! ```
//! A corrected value NEVER touches an existing `version={v}` directory — it
//! is written under `version={v+1}` (immutability by construction).

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, Date32Array, Date32Builder, Decimal128Array, Decimal128Builder, Int64Array,
    Int64Builder, StringBuilder, TimestampMicrosecondArray, TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema};
use chrono::{Datelike, Duration, NaiveDate};
use domain::{
    BatchId, ContentHash, Currency, FixedPoint, InstrumentId, Price, TradingDate, UtcTimestamp,
};

use super::CurateError;
use crate::curate::actions::{CorporateAction, CorporateActionType};
use crate::curate::adjust::{AdjustmentBar, AdjustmentKind};

/// Decimal precision of curated prices and monetary amounts.
pub const PRICE_PRECISION: u8 = 18;
/// Decimal scale of curated prices — matches [`domain::PRICE_SCALE`].
pub const PRICE_SCALE: u8 = 4;
/// Decimal scale of cumulative adjustment factors.
pub const FACTOR_SCALE: u8 = 8;
/// Decimal scale of tax withholding percentages.
pub const TAX_SCALE: u8 = 6;

fn price_type() -> DataType {
    DataType::Decimal128(PRICE_PRECISION, PRICE_SCALE as i8)
}

fn factor_type() -> DataType {
    DataType::Decimal128(PRICE_PRECISION, FACTOR_SCALE as i8)
}

fn timestamp_type() -> DataType {
    DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None)
}

/// A normalized, provenance-linked raw bar row (the Curated bars table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratedBar {
    pub instrument_id: InstrumentId,
    pub trading_date: TradingDate,
    pub market_open_ts: UtcTimestamp,
    pub market_close_ts: UtcTimestamp,
    pub open: Price,
    pub high: Price,
    pub low: Price,
    pub close: Price,
    pub volume: i64,
    /// Reported trading value (거래대금, provider units) — optional extension.
    pub trading_value: Option<i64>,
    pub currency: Currency,
    pub source: String,
    pub ingested_at: UtcTimestamp,
    pub batch_id: BatchId,
    pub raw_hash: ContentHash,
}

/// The documented Curated schema constants.
pub struct CuratedSchema;

impl CuratedSchema {
    /// The raw bars table: documented §8.2 fields + provenance columns.
    pub fn bars() -> Schema {
        Schema::new(vec![
            Field::new("instrument_id", DataType::Utf8, false),
            Field::new("trading_date", DataType::Date32, false),
            Field::new("market_open_ts", timestamp_type(), false),
            Field::new("market_close_ts", timestamp_type(), false),
            Field::new("open", price_type(), false),
            Field::new("high", price_type(), false),
            Field::new("low", price_type(), false),
            Field::new("close", price_type(), false),
            Field::new("volume", DataType::Int64, false),
            Field::new("trading_value", DataType::Int64, true),
            Field::new("currency", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new("ingested_at", timestamp_type(), false),
            Field::new("batch_id", DataType::Utf8, false),
            Field::new("raw_hash", DataType::Utf8, false),
        ])
    }

    /// The adjusted bars table (signals): split- or total-return-adjusted
    /// prices with the cumulative adjustment factor and its event provenance.
    pub fn adjusted_bars() -> Schema {
        Schema::new(vec![
            Field::new("instrument_id", DataType::Utf8, false),
            Field::new("trading_date", DataType::Date32, false),
            Field::new("market_open_ts", timestamp_type(), false),
            Field::new("market_close_ts", timestamp_type(), false),
            Field::new("open", price_type(), false),
            Field::new("high", price_type(), false),
            Field::new("low", price_type(), false),
            Field::new("close", price_type(), false),
            Field::new("volume", DataType::Int64, false),
            Field::new("trading_value", DataType::Int64, true),
            Field::new("adjustment_kind", DataType::Utf8, false),
            Field::new("adjustment_factor", factor_type(), false),
            Field::new("adjustment_events", DataType::Utf8, false),
            Field::new("currency", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new("ingested_at", timestamp_type(), false),
            Field::new("batch_id", DataType::Utf8, false),
            Field::new("raw_hash", DataType::Utf8, false),
        ])
    }

    /// The corporate-actions table (requirements §8.2 기업행사: `instrument_id,
    /// event_type, ex_date, record_date, pay_date, ratio_or_amount, currency,
    /// announced_at, source`) + split factor + provenance columns.
    pub fn corporate_actions() -> Schema {
        Schema::new(vec![
            Field::new("instrument_id", DataType::Utf8, false),
            Field::new("event_type", DataType::Utf8, false),
            Field::new("ex_date", DataType::Date32, false),
            Field::new("record_date", DataType::Date32, true),
            Field::new("pay_date", DataType::Date32, true),
            Field::new("ratio", DataType::Utf8, true),
            Field::new("split_factor", factor_type(), true),
            Field::new("amount_per_share", price_type(), true),
            Field::new(
                "tax_withholding_pct",
                DataType::Decimal128(PRICE_PRECISION, TAX_SCALE as i8),
                true,
            ),
            Field::new("currency", DataType::Utf8, false),
            Field::new("announced_at", timestamp_type(), false),
            Field::new("source", DataType::Utf8, false),
            Field::new("batch_id", DataType::Utf8, false),
            Field::new("raw_hash", DataType::Utf8, false),
            Field::new("ingested_at", timestamp_type(), false),
        ])
    }
}

fn date_to_days(date: TradingDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    (date.as_naive_date() - epoch).num_days() as i32
}

fn days_to_date(days: i32) -> TradingDate {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    let naive = epoch + Duration::days(i64::from(days));
    TradingDate::new(naive.year(), naive.month(), naive.day()).expect("valid date from parquet")
}

fn ts_to_micros(ts: UtcTimestamp) -> i64 {
    ts.as_datetime().timestamp_micros()
}

fn micros_to_ts(micros: i64) -> UtcTimestamp {
    UtcTimestamp::from_datetime(
        chrono::DateTime::from_timestamp_micros(micros).expect("valid timestamp from parquet"),
    )
}

fn fixed_to_decimal(value: &FixedPoint, scale: u8) -> i128 {
    value
        .with_scale(scale)
        .expect("fixed point fits curated decimal scale")
        .bits()
}

fn decimal_to_fixed(bits: i128, scale: u8) -> FixedPoint {
    FixedPoint::from_i128(bits, scale).expect("parquet decimal fits fixed point")
}

/// Writes the raw bars table to `path`.
pub fn write_bars(path: &Path, rows: &[CuratedBar]) -> Result<(), CurateError> {
    let mut instrument = StringBuilder::new();
    let mut trading_date = Date32Builder::new();
    let mut open_ts = TimestampMicrosecondBuilder::new();
    let mut close_ts = TimestampMicrosecondBuilder::new();
    let mut open = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut high = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut low = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut close = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut volume = Int64Builder::new();
    let mut trading_value = Int64Builder::new();
    let mut currency = StringBuilder::new();
    let mut source = StringBuilder::new();
    let mut ingested_at = TimestampMicrosecondBuilder::new();
    let mut batch_id = StringBuilder::new();
    let mut raw_hash = StringBuilder::new();

    for row in rows {
        instrument.append_value(row.instrument_id.to_string());
        trading_date.append_value(date_to_days(row.trading_date));
        open_ts.append_value(ts_to_micros(row.market_open_ts));
        close_ts.append_value(ts_to_micros(row.market_close_ts));
        open.append_value(fixed_to_decimal(&row.open.amount(), PRICE_SCALE));
        high.append_value(fixed_to_decimal(&row.high.amount(), PRICE_SCALE));
        low.append_value(fixed_to_decimal(&row.low.amount(), PRICE_SCALE));
        close.append_value(fixed_to_decimal(&row.close.amount(), PRICE_SCALE));
        volume.append_value(row.volume);
        match row.trading_value {
            Some(v) => trading_value.append_value(v),
            None => trading_value.append_null(),
        }
        currency.append_value(row.currency.code());
        source.append_value(&row.source);
        ingested_at.append_value(ts_to_micros(row.ingested_at));
        batch_id.append_value(row.batch_id.to_string());
        raw_hash.append_value(row.raw_hash.as_str());
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(instrument.finish()),
        Arc::new(trading_date.finish()),
        Arc::new(open_ts.finish()),
        Arc::new(close_ts.finish()),
        Arc::new(open.finish()),
        Arc::new(high.finish()),
        Arc::new(low.finish()),
        Arc::new(close.finish()),
        Arc::new(volume.finish()),
        Arc::new(trading_value.finish()),
        Arc::new(currency.finish()),
        Arc::new(source.finish()),
        Arc::new(ingested_at.finish()),
        Arc::new(batch_id.finish()),
        Arc::new(raw_hash.finish()),
    ];
    write_record_batch(path, CuratedSchema::bars(), columns)
}

/// Writes the adjusted bars table (split or total-return) to `path`.
pub fn write_adjusted_bars(path: &Path, rows: &[AdjustmentBar]) -> Result<(), CurateError> {
    let mut instrument = StringBuilder::new();
    let mut trading_date = Date32Builder::new();
    let mut open_ts = TimestampMicrosecondBuilder::new();
    let mut close_ts = TimestampMicrosecondBuilder::new();
    let mut open = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut high = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut low = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut close = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut volume = Int64Builder::new();
    let mut trading_value = Int64Builder::new();
    let mut adjustment_kind = StringBuilder::new();
    let mut adjustment_factor = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, FACTOR_SCALE as i8)
        .expect("valid decimal params");
    let mut adjustment_events = StringBuilder::new();
    let mut currency = StringBuilder::new();
    let mut source = StringBuilder::new();
    let mut ingested_at = TimestampMicrosecondBuilder::new();
    let mut batch_id = StringBuilder::new();
    let mut raw_hash = StringBuilder::new();

    for row in rows {
        instrument.append_value(row.instrument_id.to_string());
        trading_date.append_value(date_to_days(row.trading_date));
        open_ts.append_value(ts_to_micros(row.market_open_ts));
        close_ts.append_value(ts_to_micros(row.market_close_ts));
        open.append_value(fixed_to_decimal(&row.open.amount(), PRICE_SCALE));
        high.append_value(fixed_to_decimal(&row.high.amount(), PRICE_SCALE));
        low.append_value(fixed_to_decimal(&row.low.amount(), PRICE_SCALE));
        close.append_value(fixed_to_decimal(&row.close.amount(), PRICE_SCALE));
        volume.append_value(row.volume);
        match row.trading_value {
            Some(v) => trading_value.append_value(v),
            None => trading_value.append_null(),
        }
        adjustment_kind.append_value(row.adjustment_kind.as_str());
        adjustment_factor.append_value(fixed_to_decimal(&row.adjustment_factor, FACTOR_SCALE));
        adjustment_events.append_value(&row.adjustment_events);
        currency.append_value(row.currency.code());
        source.append_value(&row.source);
        ingested_at.append_value(ts_to_micros(row.ingested_at));
        batch_id.append_value(row.batch_id.to_string());
        raw_hash.append_value(row.raw_hash.as_str());
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(instrument.finish()),
        Arc::new(trading_date.finish()),
        Arc::new(open_ts.finish()),
        Arc::new(close_ts.finish()),
        Arc::new(open.finish()),
        Arc::new(high.finish()),
        Arc::new(low.finish()),
        Arc::new(close.finish()),
        Arc::new(volume.finish()),
        Arc::new(trading_value.finish()),
        Arc::new(adjustment_kind.finish()),
        Arc::new(adjustment_factor.finish()),
        Arc::new(adjustment_events.finish()),
        Arc::new(currency.finish()),
        Arc::new(source.finish()),
        Arc::new(ingested_at.finish()),
        Arc::new(batch_id.finish()),
        Arc::new(raw_hash.finish()),
    ];
    write_record_batch(path, CuratedSchema::adjusted_bars(), columns)
}

/// Writes the corporate-actions table to `path`.
pub fn write_corporate_actions(path: &Path, rows: &[CorporateAction]) -> Result<(), CurateError> {
    let mut instrument = StringBuilder::new();
    let mut event_type = StringBuilder::new();
    let mut ex_date = Date32Builder::new();
    let mut record_date = Date32Builder::new();
    let mut pay_date = Date32Builder::new();
    let mut ratio = StringBuilder::new();
    let mut split_factor = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, FACTOR_SCALE as i8)
        .expect("valid decimal params");
    let mut amount_per_share = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, PRICE_SCALE as i8)
        .expect("valid decimal params");
    let mut tax_withholding_pct = Decimal128Builder::new()
        .with_precision_and_scale(PRICE_PRECISION, TAX_SCALE as i8)
        .expect("valid decimal params");
    let mut currency = StringBuilder::new();
    let mut announced_at = TimestampMicrosecondBuilder::new();
    let mut source = StringBuilder::new();
    let mut batch_id = StringBuilder::new();
    let mut raw_hash = StringBuilder::new();
    let mut ingested_at = TimestampMicrosecondBuilder::new();

    for row in rows {
        instrument.append_value(row.instrument_id.to_string());
        event_type.append_value(row.event_type.as_str());
        ex_date.append_value(date_to_days(row.ex_date));
        match row.record_date {
            Some(d) => record_date.append_value(date_to_days(d)),
            None => record_date.append_null(),
        }
        match row.pay_date {
            Some(d) => pay_date.append_value(date_to_days(d)),
            None => pay_date.append_null(),
        }
        match &row.ratio {
            Some(r) => ratio.append_value(r),
            None => ratio.append_null(),
        }
        match &row.split_factor {
            Some(f) => split_factor.append_value(fixed_to_decimal(f, FACTOR_SCALE)),
            None => split_factor.append_null(),
        }
        match &row.amount_per_share {
            Some(m) => amount_per_share.append_value(fixed_to_decimal(&m.amount(), PRICE_SCALE)),
            None => amount_per_share.append_null(),
        }
        match &row.tax_withholding_pct {
            Some(t) => tax_withholding_pct.append_value(fixed_to_decimal(t, TAX_SCALE)),
            None => tax_withholding_pct.append_null(),
        }
        currency.append_value(row.currency.code());
        announced_at.append_value(ts_to_micros(row.announced_at));
        source.append_value(&row.source);
        batch_id.append_value(row.batch_id.to_string());
        raw_hash.append_value(row.raw_hash.as_str());
        ingested_at.append_value(ts_to_micros(row.ingested_at));
    }

    let columns: Vec<ArrayRef> = vec![
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
    ];
    write_record_batch(path, CuratedSchema::corporate_actions(), columns)
}

/// Writes one record batch as a snappy-compressed Parquet file.
fn write_record_batch(
    path: &Path,
    schema: Schema,
    columns: Vec<ArrayRef>,
) -> Result<(), CurateError> {
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;

    let parent = path.parent().ok_or_else(|| CurateError::StoreIo {
        context: "parquet parent".to_owned(),
        detail: path.display().to_string(),
    })?;
    std::fs::create_dir_all(parent).map_err(|e| CurateError::StoreIo {
        context: format!("create {}", parent.display()),
        detail: e.to_string(),
    })?;
    let batch =
        RecordBatch::try_new(Arc::new(schema), columns).map_err(|e| CurateError::StoreIo {
            context: "record batch".to_owned(),
            detail: e.to_string(),
        })?;
    let file = File::create(path).map_err(|e| CurateError::StoreIo {
        context: format!("create {}", path.display()),
        detail: e.to_string(),
    })?;
    let mut writer =
        ArrowWriter::try_new(file, batch.schema(), None).map_err(|e| CurateError::StoreIo {
            context: "parquet writer".to_owned(),
            detail: e.to_string(),
        })?;
    writer.write(&batch).map_err(|e| CurateError::StoreIo {
        context: "parquet write".to_owned(),
        detail: e.to_string(),
    })?;
    writer.close().map_err(|e| CurateError::StoreIo {
        context: "parquet close".to_owned(),
        detail: e.to_string(),
    })?;
    Ok(())
}

/// Reads all rows of a curated Parquet table as record batches.
fn read_batches(path: &Path) -> Result<Vec<arrow::record_batch::RecordBatch>, CurateError> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let file = File::open(path).map_err(|e| CurateError::StoreIo {
        context: format!("open {}", path.display()),
        detail: e.to_string(),
    })?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| CurateError::StoreIo {
            context: format!("read header {}", path.display()),
            detail: e.to_string(),
        })?;
    let reader = builder.build().map_err(|e| CurateError::StoreIo {
        context: format!("build reader {}", path.display()),
        detail: e.to_string(),
    })?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| CurateError::StoreIo {
            context: format!("read rows {}", path.display()),
            detail: e.to_string(),
        })
}

fn str_at<'a>(batch: &'a arrow::record_batch::RecordBatch, col: &str, i: usize) -> &'a str {
    batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .expect("utf8 column")
        .value(i)
}

fn i64_at(batch: &arrow::record_batch::RecordBatch, col: &str, i: usize) -> i64 {
    batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("int64 column")
        .value(i)
}

fn i64_opt_at(batch: &arrow::record_batch::RecordBatch, col: &str, i: usize) -> Option<i64> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("int64 column");
    if array.is_null(i) {
        None
    } else {
        Some(array.value(i))
    }
}

fn date_at(batch: &arrow::record_batch::RecordBatch, col: &str, i: usize) -> TradingDate {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("date32 column");
    days_to_date(array.value(i))
}

fn date_opt_at(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
) -> Option<TradingDate> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("date32 column");
    if array.is_null(i) {
        None
    } else {
        Some(days_to_date(array.value(i)))
    }
}

fn ts_at(batch: &arrow::record_batch::RecordBatch, col: &str, i: usize) -> UtcTimestamp {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("timestamp(us) column");
    micros_to_ts(array.value(i))
}

fn price_at(batch: &arrow::record_batch::RecordBatch, col: &str, i: usize) -> Price {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("decimal column");
    let value = decimal_to_fixed(array.value(i), PRICE_SCALE);
    Price::from_fixed(value).expect("curated price is positive by construction")
}

fn fixed_at(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
    scale: u8,
) -> FixedPoint {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("decimal column");
    decimal_to_fixed(array.value(i), scale)
}

fn fixed_opt_at(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
    scale: u8,
) -> Option<FixedPoint> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("decimal column");
    if array.is_null(i) {
        None
    } else {
        Some(decimal_to_fixed(array.value(i), scale))
    }
}

fn str_opt_at<'a>(
    batch: &'a arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
) -> Option<&'a str> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .expect("utf8 column");
    if array.is_null(i) {
        None
    } else {
        Some(array.value(i))
    }
}

/// Reads the raw bars table back into typed rows.
pub fn read_bars(path: &Path) -> Result<Vec<CuratedBar>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            rows.push(CuratedBar {
                instrument_id: InstrumentId::parse(str_at(&batch, "instrument_id", i)).map_err(
                    |e| CurateError::StoreIo {
                        context: "instrument_id parse".to_owned(),
                        detail: e.to_string(),
                    },
                )?,
                trading_date: date_at(&batch, "trading_date", i),
                market_open_ts: ts_at(&batch, "market_open_ts", i),
                market_close_ts: ts_at(&batch, "market_close_ts", i),
                open: price_at(&batch, "open", i),
                high: price_at(&batch, "high", i),
                low: price_at(&batch, "low", i),
                close: price_at(&batch, "close", i),
                volume: i64_at(&batch, "volume", i),
                trading_value: i64_opt_at(&batch, "trading_value", i),
                currency: Currency::from_code(str_at(&batch, "currency", i))
                    .expect("curated currency is valid by construction"),
                source: str_at(&batch, "source", i).to_owned(),
                ingested_at: ts_at(&batch, "ingested_at", i),
                batch_id: str_at(&batch, "batch_id", i)
                    .parse::<BatchId>()
                    .map_err(|e| CurateError::StoreIo {
                        context: "batch_id parse".to_owned(),
                        detail: e.to_string(),
                    })?,
                raw_hash: ContentHash::parse(str_at(&batch, "raw_hash", i)).map_err(|e| {
                    CurateError::StoreIo {
                        context: "raw_hash parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?,
            });
        }
    }
    Ok(rows)
}

/// Reads the adjusted bars table back into typed rows.
pub fn read_adjusted_bars(path: &Path) -> Result<Vec<AdjustmentBar>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            rows.push(AdjustmentBar {
                instrument_id: InstrumentId::parse(str_at(&batch, "instrument_id", i)).map_err(
                    |e| CurateError::StoreIo {
                        context: "instrument_id parse".to_owned(),
                        detail: e.to_string(),
                    },
                )?,
                trading_date: date_at(&batch, "trading_date", i),
                market_open_ts: ts_at(&batch, "market_open_ts", i),
                market_close_ts: ts_at(&batch, "market_close_ts", i),
                open: price_at(&batch, "open", i),
                high: price_at(&batch, "high", i),
                low: price_at(&batch, "low", i),
                close: price_at(&batch, "close", i),
                volume: i64_at(&batch, "volume", i),
                trading_value: i64_opt_at(&batch, "trading_value", i),
                adjustment_kind: AdjustmentKind::parse(str_at(&batch, "adjustment_kind", i))
                    .expect("curated adjustment kind is valid by construction"),
                adjustment_factor: fixed_at(&batch, "adjustment_factor", i, FACTOR_SCALE),
                adjustment_events: str_at(&batch, "adjustment_events", i).to_owned(),
                currency: Currency::from_code(str_at(&batch, "currency", i))
                    .expect("curated currency is valid by construction"),
                source: str_at(&batch, "source", i).to_owned(),
                ingested_at: ts_at(&batch, "ingested_at", i),
                batch_id: str_at(&batch, "batch_id", i)
                    .parse::<BatchId>()
                    .map_err(|e| CurateError::StoreIo {
                        context: "batch_id parse".to_owned(),
                        detail: e.to_string(),
                    })?,
                raw_hash: ContentHash::parse(str_at(&batch, "raw_hash", i)).map_err(|e| {
                    CurateError::StoreIo {
                        context: "raw_hash parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?,
            });
        }
    }
    Ok(rows)
}

/// Reads the corporate-actions table back into typed rows.
pub fn read_corporate_actions(path: &Path) -> Result<Vec<CorporateAction>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            let currency = Currency::from_code(str_at(&batch, "currency", i))
                .expect("curated currency is valid by construction");
            rows.push(CorporateAction {
                instrument_id: InstrumentId::parse(str_at(&batch, "instrument_id", i)).map_err(
                    |e| CurateError::StoreIo {
                        context: "instrument_id parse".to_owned(),
                        detail: e.to_string(),
                    },
                )?,
                event_type: CorporateActionType::parse(str_at(&batch, "event_type", i))
                    .expect("curated event type is valid by construction"),
                ex_date: date_at(&batch, "ex_date", i),
                record_date: date_opt_at(&batch, "record_date", i),
                pay_date: date_opt_at(&batch, "pay_date", i),
                ratio: str_opt_at(&batch, "ratio", i).map(str::to_owned),
                split_factor: fixed_opt_at(&batch, "split_factor", i, FACTOR_SCALE),
                amount_per_share: fixed_opt_at(&batch, "amount_per_share", i, PRICE_SCALE).map(
                    |f| {
                        domain::Money::from_fixed(f, currency)
                            .expect("curated amount is non-negative by construction")
                    },
                ),
                tax_withholding_pct: fixed_opt_at(&batch, "tax_withholding_pct", i, TAX_SCALE),
                currency,
                announced_at: ts_at(&batch, "announced_at", i),
                source: str_at(&batch, "source", i).to_owned(),
                batch_id: str_at(&batch, "batch_id", i)
                    .parse::<BatchId>()
                    .map_err(|e| CurateError::StoreIo {
                        context: "batch_id parse".to_owned(),
                        detail: e.to_string(),
                    })?,
                raw_hash: ContentHash::parse(str_at(&batch, "raw_hash", i)).map_err(|e| {
                    CurateError::StoreIo {
                        context: "raw_hash parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?,
                ingested_at: ts_at(&batch, "ingested_at", i),
            });
        }
    }
    Ok(rows)
}
