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
    Array, ArrayRef, Date32Array, Date32Builder, Decimal128Array, Decimal128Builder, Float64Array,
    Float64Builder, Int64Array, Int64Builder, StringBuilder, TimestampMicrosecondArray,
    TimestampMicrosecondBuilder,
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

fn days_to_date_checked(days: i32, col: &str) -> Result<TradingDate, CurateError> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    let naive = epoch + Duration::days(i64::from(days));
    TradingDate::new(naive.year(), naive.month(), naive.day()).map_err(|e| CurateError::StoreIo {
        context: format!("{col} parse"),
        detail: e.to_string(),
    })
}

fn ts_to_micros(ts: UtcTimestamp) -> i64 {
    ts.as_datetime().timestamp_micros()
}

fn ts_at_checked(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
) -> Result<UtcTimestamp, CurateError> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .expect("timestamp(us) column");
    let micros = array.value(i);
    let dt =
        chrono::DateTime::from_timestamp_micros(micros).ok_or_else(|| CurateError::StoreIo {
            context: format!("{col} parse"),
            detail: format!("timestamp {micros} is out of range"),
        })?;
    Ok(UtcTimestamp::from_datetime(dt))
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

fn date_at(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
) -> Result<TradingDate, CurateError> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("date32 column");
    days_to_date_checked(array.value(i), col)
}

fn date_opt_at(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
) -> Result<Option<TradingDate>, CurateError> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("date32 column");
    if array.is_null(i) {
        Ok(None)
    } else {
        Ok(Some(days_to_date_checked(array.value(i), col)?))
    }
}

fn price_at_checked(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
    instrument: &str,
    date: &str,
) -> Result<Price, CurateError> {
    let array = batch
        .column_by_name(col)
        .expect("schema column present")
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .expect("decimal column");
    let value = decimal_to_fixed(array.value(i), PRICE_SCALE);
    Price::from_fixed(value).map_err(|_| CurateError::NonPositivePrice {
        instrument: instrument.to_owned(),
        date: date.to_owned(),
        field: col.to_owned(),
        value: value.to_string(),
    })
}

fn currency_at_checked(
    batch: &arrow::record_batch::RecordBatch,
    col: &str,
    i: usize,
) -> Result<Currency, CurateError> {
    let code = str_at(batch, col, i);
    Currency::from_code(code).map_err(|e| CurateError::StoreIo {
        context: format!("{col} parse"),
        detail: format!("unknown currency {code:?}: {e}"),
    })
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
///
/// Value-safe: non-positive prices, unknown currencies, and malformed dates
/// or timestamps are typed [`CurateError`]s — this reader never panics on
/// on-disk values (Todo 11: the quality gate classifies, it does not crash).
pub fn read_bars(path: &Path) -> Result<Vec<CuratedBar>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            let instrument_id =
                InstrumentId::parse(str_at(&batch, "instrument_id", i)).map_err(|e| {
                    CurateError::StoreIo {
                        context: "instrument_id parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?;
            let trading_date = date_at(&batch, "trading_date", i)?;
            let instrument = instrument_id.to_string();
            let date = trading_date.to_iso();
            rows.push(CuratedBar {
                instrument_id,
                trading_date,
                market_open_ts: ts_at_checked(&batch, "market_open_ts", i)?,
                market_close_ts: ts_at_checked(&batch, "market_close_ts", i)?,
                open: price_at_checked(&batch, "open", i, &instrument, &date)?,
                high: price_at_checked(&batch, "high", i, &instrument, &date)?,
                low: price_at_checked(&batch, "low", i, &instrument, &date)?,
                close: price_at_checked(&batch, "close", i, &instrument, &date)?,
                volume: i64_at(&batch, "volume", i),
                trading_value: i64_opt_at(&batch, "trading_value", i),
                currency: currency_at_checked(&batch, "currency", i)?,
                source: str_at(&batch, "source", i).to_owned(),
                ingested_at: ts_at_checked(&batch, "ingested_at", i)?,
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

/// Reads the adjusted bars table back into typed rows (value-safe, see
/// [`read_bars`]).
pub fn read_adjusted_bars(path: &Path) -> Result<Vec<AdjustmentBar>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            let instrument_id =
                InstrumentId::parse(str_at(&batch, "instrument_id", i)).map_err(|e| {
                    CurateError::StoreIo {
                        context: "instrument_id parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?;
            let trading_date = date_at(&batch, "trading_date", i)?;
            let instrument = instrument_id.to_string();
            let date = trading_date.to_iso();
            let adjustment_kind = AdjustmentKind::parse(str_at(&batch, "adjustment_kind", i))
                .ok_or_else(|| CurateError::StoreIo {
                    context: "adjustment_kind parse".to_owned(),
                    detail: format!(
                        "unknown adjustment kind {:?}",
                        str_at(&batch, "adjustment_kind", i)
                    ),
                })?;
            rows.push(AdjustmentBar {
                instrument_id,
                trading_date,
                market_open_ts: ts_at_checked(&batch, "market_open_ts", i)?,
                market_close_ts: ts_at_checked(&batch, "market_close_ts", i)?,
                open: price_at_checked(&batch, "open", i, &instrument, &date)?,
                high: price_at_checked(&batch, "high", i, &instrument, &date)?,
                low: price_at_checked(&batch, "low", i, &instrument, &date)?,
                close: price_at_checked(&batch, "close", i, &instrument, &date)?,
                volume: i64_at(&batch, "volume", i),
                trading_value: i64_opt_at(&batch, "trading_value", i),
                adjustment_kind,
                adjustment_factor: fixed_at(&batch, "adjustment_factor", i, FACTOR_SCALE),
                adjustment_events: str_at(&batch, "adjustment_events", i).to_owned(),
                currency: currency_at_checked(&batch, "currency", i)?,
                source: str_at(&batch, "source", i).to_owned(),
                ingested_at: ts_at_checked(&batch, "ingested_at", i)?,
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

/// Reads the corporate-actions table back into typed rows (value-safe, see
/// [`read_bars`]).
pub fn read_corporate_actions(path: &Path) -> Result<Vec<CorporateAction>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            let instrument_id =
                InstrumentId::parse(str_at(&batch, "instrument_id", i)).map_err(|e| {
                    CurateError::StoreIo {
                        context: "instrument_id parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?;
            let ex_date = date_at(&batch, "ex_date", i)?;
            let currency = currency_at_checked(&batch, "currency", i)?;
            let event_type = CorporateActionType::parse(str_at(&batch, "event_type", i))
                .ok_or_else(|| CurateError::StoreIo {
                    context: "event_type parse".to_owned(),
                    detail: format!("unknown event type {:?}", str_at(&batch, "event_type", i)),
                })?;
            let amount_per_share = match fixed_opt_at(&batch, "amount_per_share", i, PRICE_SCALE) {
                Some(f) => Some(domain::Money::from_fixed(f, currency).map_err(|e| {
                    CurateError::StoreIo {
                        context: "amount_per_share parse".to_owned(),
                        detail: e.to_string(),
                    }
                })?),
                None => None,
            };
            rows.push(CorporateAction {
                instrument_id,
                event_type,
                ex_date,
                record_date: date_opt_at(&batch, "record_date", i)?,
                pay_date: date_opt_at(&batch, "pay_date", i)?,
                ratio: str_opt_at(&batch, "ratio", i).map(str::to_owned),
                split_factor: fixed_opt_at(&batch, "split_factor", i, FACTOR_SCALE),
                amount_per_share,
                tax_withholding_pct: fixed_opt_at(&batch, "tax_withholding_pct", i, TAX_SCALE),
                currency,
                announced_at: ts_at_checked(&batch, "announced_at", i)?,
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
                ingested_at: ts_at_checked(&batch, "ingested_at", i)?,
            });
        }
    }
    Ok(rows)
}

/// One point-in-time fundamentals observation.
///
/// Two dates, and the distinction between them is the entire point.
/// `period_end` says WHICH period the number describes; `known_from` says when
/// it first became usable. A backtest stepping through 2020 must see the
/// figure that was public in 2020, not the restatement published in 2021 --
/// the requirements' §14 risk table names 미래정보 참조 as a cause of 허위 성과,
/// and a single-date fundamentals table is how that happens silently.
///
/// `known_from` is defined as THE FIRST TRADING DATE THIS VALUE MAY BE USED.
/// Announcements land after the close, so an ingester that maps an
/// announcement timestamp to the same calendar day grants the strategy a day
/// of foresight; the convention is stated here so that mapping is the
/// ingester's explicit job rather than an assumption each reader re-invents.
///
/// `revision` orders restatements that share a `known_from`; 0 is the original.
#[derive(Debug, Clone, PartialEq)]
pub struct CuratedFundamental {
    pub instrument_id: InstrumentId,
    /// The last day of the fiscal period this value describes.
    ///
    /// A date rather than a `2020Q1` label: dates order without anyone having
    /// to agree on a fiscal-year convention, and Korean issuers do not all
    /// share one.
    pub period_end: TradingDate,
    /// The metric name (e.g. `net_income`), free-form by design at this layer.
    pub metric: String,
    pub value: f64,
    /// The first trading date this value may be used.
    pub known_from: TradingDate,
    /// Restatement ordinal within `(instrument, period_end)`; 0 is the original.
    pub revision: i64,
}

impl CuratedSchema {
    /// The point-in-time fundamentals table.
    pub fn fundamentals() -> Schema {
        Schema::new(vec![
            Field::new("instrument_id", DataType::Utf8, false),
            Field::new("period_end", DataType::Date32, false),
            Field::new("metric", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
            Field::new("known_from", DataType::Date32, false),
            Field::new("revision", DataType::Int64, false),
        ])
    }
}

/// Writes the point-in-time fundamentals table to `path`.
pub fn write_fundamentals(path: &Path, rows: &[CuratedFundamental]) -> Result<(), CurateError> {
    let mut instrument = StringBuilder::new();
    let mut period_end = Date32Builder::new();
    let mut metric = StringBuilder::new();
    let mut value = Float64Builder::new();
    let mut known_from = Date32Builder::new();
    let mut revision = Int64Builder::new();

    for row in rows {
        instrument.append_value(row.instrument_id.to_string());
        period_end.append_value(date_to_days(row.period_end));
        metric.append_value(&row.metric);
        value.append_value(row.value);
        known_from.append_value(date_to_days(row.known_from));
        revision.append_value(row.revision);
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(instrument.finish()),
        Arc::new(period_end.finish()),
        Arc::new(metric.finish()),
        Arc::new(value.finish()),
        Arc::new(known_from.finish()),
        Arc::new(revision.finish()),
    ];
    write_record_batch(path, CuratedSchema::fundamentals(), columns)
}

/// Reads the point-in-time fundamentals table from `path`.
pub fn read_fundamentals(path: &Path) -> Result<Vec<CuratedFundamental>, CurateError> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        for i in 0..batch.num_rows() {
            let instrument_id = InstrumentId::parse(str_at(&batch, "instrument_id", i))
                .map_err(|e| CurateError::StoreIo {
                    context: "instrument_id parse".to_owned(),
                    detail: e.to_string(),
                })?;
            rows.push(CuratedFundamental {
                instrument_id,
                period_end: date_at(&batch, "period_end", i)?,
                metric: str_at(&batch, "metric", i).to_owned(),
                value: f64_at(&batch, "value", i),
                known_from: date_at(&batch, "known_from", i)?,
                revision: i64_at(&batch, "revision", i),
            });
        }
    }
    Ok(rows)
}

fn f64_at(batch: &arrow::record_batch::RecordBatch, name: &str, i: usize) -> f64 {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
        .map(|a| a.value(i))
        .unwrap_or(f64::NAN)
}
