//! Versioned recent-high drawdown (design §6.5 "최근 고점 대비 낙폭").
//!
//! `drawdown = close / running_max(close) - 1 <= 0` over the instrument's
//! full available history (running maximum, no window). The factor is defined
//! from the first bar (its own high -> 0.0), so there is no lookback NULL.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, instruments_of, map_per_instrument};

/// The recent-high drawdown factor (version 1.0.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawdownFactor;

impl Factor for DrawdownFactor {
    fn id(&self) -> &str {
        "drawdown"
    }

    fn version(&self) -> domain::FactorVersion {
        domain::FactorVersion::parse("1.0.0").expect("static version")
    }

    fn required_fields(&self) -> &[Field] {
        &[Field::CLOSE]
    }

    fn lookback(&self) -> Lookback {
        Lookback::FullHistory
    }

    fn null_policy(&self) -> NullPolicy {
        NullPolicy::InsufficientLookback
    }

    fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
        let out = map_per_instrument(&ctx.bars.lazy_frame(), &instruments_of(ctx), |part| {
            part.select([
                col("instrument_id"),
                col("trading_date"),
                (col("close") / col("close").cum_max(false) - lit(1.0)).alias("value"),
            ])
        })?;
        collect_factor_frame(out, self.id())
    }
}
