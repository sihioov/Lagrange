//! Versioned liquidity factor (design §6.5 "20일 평균 거래대금").
//!
//! `avg_value_20 = SMA_20(reported trading value)` with the documented
//! STRICT window policy: a NULL input inside the window -> NULL output (no
//! partial-window reuse, no zero-fill). Implemented with a `-inf` sentinel
//! fill so the rolling kernel never skips the missing observation.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, instruments_of, map_per_instrument, rolling};

/// The 20-day average trading value factor (version 1.0.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvgValueFactor;

impl Factor for AvgValueFactor {
    fn id(&self) -> &str {
        "avg_value_20"
    }

    fn version(&self) -> domain::FactorVersion {
        domain::FactorVersion::parse("1.0.0").expect("static version")
    }

    fn required_fields(&self) -> &[Field] {
        &[Field::CLOSE, Field::TRADING_VALUE]
    }

    fn lookback(&self) -> Lookback {
        Lookback::FixedWindow {
            window: 20,
            min_periods: 20,
        }
    }

    fn null_policy(&self) -> NullPolicy {
        NullPolicy::StrictWindow
    }

    fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
        let out = map_per_instrument(&ctx.bars.lazy_frame(), &instruments_of(ctx), |part| {
            let filled = col("trading_value").fill_null(f64::NEG_INFINITY);
            let mean = filled.clone().rolling_mean(rolling(20));
            let worst = filled.clone().rolling_min(rolling(20));
            let value = polars::prelude::when(worst.gt(lit(-1e300)))
                .then(mean)
                .otherwise(lit(polars::prelude::Null {}));
            part.select([
                col("instrument_id"),
                col("trading_date"),
                value.alias("value"),
            ])
        })?;
        collect_factor_frame(out, self.id())
    }
}
