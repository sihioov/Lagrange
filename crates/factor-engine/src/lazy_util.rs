//! Polars-lazy plumbing shared by the factor implementations.
//!
//! `map_per_instrument` applies a transform to each instrument's partition
//! and concatenates the results in deterministic (instrument, date) order —
//! rolling/cumulative windows must never cross instrument boundaries, so the
//! partition is explicit (the frozen universe is small; this is cheap).
//!
//! `ref_close` resolves the reference close of a month target column with an
//! exact join on (instrument, target == trading_date): the target columns
//! already point at a real bar (see [`crate::bars`]), so no as-of join or
//! tolerance is needed and no post-target observation can leak in.

use domain::InstrumentId;
use polars::prelude::*;

use crate::contract::{FactorError, FactorFrame, FactorId, FactorValue};

/// Applies `transform` to one partition per instrument (canonical order) and
/// concatenates the lazy results in the same order.
pub fn map_per_instrument(
    lf: &LazyFrame,
    instruments: &[InstrumentId],
    transform: impl Fn(LazyFrame) -> LazyFrame,
) -> Result<LazyFrame, FactorError> {
    let parts: Vec<LazyFrame> = instruments
        .iter()
        .map(|id| {
            transform(
                lf.clone()
                    .filter(col("instrument_id").eq(lit(id.to_string()))),
            )
        })
        .collect();
    concat(
        parts,
        UnionArgs {
            parallel: false,
            rechunk: false,
            to_supertypes: false,
            diagonal: false,
            strict: true,
            from_partitioned_ds: false,
            maintain_order: true,
        },
    )
    .map_err(|e| FactorError::Polars {
        detail: format!("concat per instrument: {e}"),
    })
}

/// Joins the reference close for `target` (a Date32 column of the base frame)
/// onto the frame as `out`. Rows without a reference (insufficient history)
/// yield NULL.
pub fn ref_close(lf: &LazyFrame, target: &str, out: &str) -> Result<LazyFrame, FactorError> {
    let left = lf.clone().select([
        col("instrument_id"),
        col("trading_date"),
        col("close"),
        col(target).alias("_target"),
    ]);
    let right = lf.clone().select([
        col("instrument_id"),
        col("trading_date"),
        col("close").alias(out),
    ]);
    let joined = left.join(
        right,
        [col("instrument_id"), col("_target")],
        [col("instrument_id"), col("trading_date")],
        JoinArgs {
            how: JoinType::Left,
            suffix: Some("_right".into()),
            ..Default::default()
        },
    );
    Ok(joined)
}

/// Collects a `value`-shaped lazy frame into a typed [`FactorFrame`], sorted
/// by (instrument, date). Non-finite values are typed rejections.
pub fn collect_factor_frame(lf: LazyFrame, factor: FactorId) -> Result<FactorFrame, FactorError> {
    let df = lf
        .select([col("instrument_id"), col("trading_date"), col("value")])
        .collect()
        .map_err(|e| FactorError::Polars {
            detail: format!("{} collect: {e}", factor),
        })?;
    let instruments = df
        .column("instrument_id")
        .and_then(|c| c.str())
        .map_err(|e| FactorError::Polars {
            detail: format!("instrument_id column: {e}"),
        })?;
    let date_phys = df
        .column("trading_date")
        .and_then(|c| c.cast(&DataType::Int32))
        .map_err(|e| FactorError::Polars {
            detail: format!("trading_date physical cast: {e}"),
        })?;
    let dates = date_phys.i32().map_err(|e| FactorError::Polars {
        detail: format!("trading_date column: {e}"),
    })?;
    let values = df
        .column("value")
        .and_then(|c| c.f64())
        .map_err(|e| FactorError::Polars {
            detail: format!("value column: {e}"),
        })?;
    let mut rows = Vec::with_capacity(df.height());
    for i in 0..df.height() {
        let symbol = instruments.get(i).ok_or_else(|| FactorError::Polars {
            detail: format!("missing instrument at row {i}"),
        })?;
        let days = dates.get(i).ok_or_else(|| FactorError::Polars {
            detail: format!("missing date at row {i}"),
        })?;
        let date = crate::bars::days_to_date(days);
        let value = values.get(i);
        if let Some(v) = value
            && !v.is_finite()
        {
            return Err(FactorError::NonFinite {
                factor: factor.to_owned(),
                instrument: symbol.to_owned(),
                date: date.to_iso(),
                value: v,
            });
        }
        rows.push(FactorValue {
            instrument: InstrumentId::parse(symbol).map_err(|e| FactorError::Polars {
                detail: format!("instrument {symbol:?}: {e}"),
            })?,
            date,
            value,
        });
    }
    Ok(FactorFrame { factor, rows })
}

/// The sorted instrument list of a context (canonical order).
pub fn instruments_of(ctx: &crate::contract::FactorContext<'_>) -> Vec<InstrumentId> {
    ctx.bars.instruments().cloned().collect()
}

/// A fixed-window rolling options value with the strict full-window policy
/// (min_periods == window; NULL inputs handled per factor).
pub fn rolling(window: usize) -> RollingOptionsFixedWindow {
    RollingOptionsFixedWindow {
        window_size: window,
        min_periods: window,
        weights: None,
        center: false,
        fn_params: None,
    }
}
