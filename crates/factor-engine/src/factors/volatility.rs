//! Versioned bounded-window realized-volatility factors.
//!
//! `vol_N = sqrt(252) * sample_std(log returns, trailing N)` (ddof = 1,
//! matching the polars rolling kernel). The first N bars of a series yield
//! NULL (N returns need N+1 bars); windows never cross instrument boundaries.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, instruments_of, map_per_instrument, rolling};

/// A 2..=252-day annualized realized-volatility factor (version 1.0.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealizedVolFactor {
    window: usize,
    id: String,
}

impl RealizedVolFactor {
    pub fn new(window: usize) -> Result<Self, FactorError> {
        if (2..=252).contains(&window) {
            Ok(Self {
                window,
                id: format!("vol_{window}"),
            })
        } else {
            Err(FactorError::InvalidDefinition {
                detail: format!("unsupported volatility window {window} (bounded: 2..=252)"),
            })
        }
    }

    /// The documented trading-day window.
    pub fn window(&self) -> usize {
        self.window
    }
}

impl Factor for RealizedVolFactor {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> domain::FactorVersion {
        domain::FactorVersion::parse("1.0.0").expect("static version")
    }

    fn required_fields(&self) -> &[Field] {
        &[Field::CLOSE]
    }

    fn lookback(&self) -> Lookback {
        Lookback::TradingDays {
            window: self.window,
            min_periods: self.window,
        }
    }

    fn null_policy(&self) -> NullPolicy {
        NullPolicy::InsufficientLookback
    }

    fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
        let window = self.window;
        let out = map_per_instrument(&ctx.bars.lazy_frame(), &instruments_of(ctx), move |part| {
            let log_return = col("close").log(lit(std::f64::consts::E))
                - col("close").shift(lit(1i64)).log(lit(std::f64::consts::E));
            part.select([
                col("instrument_id"),
                col("trading_date"),
                (log_return.rolling_std(rolling(window)) * lit(252.0f64.sqrt())).alias("value"),
            ])
        })?;
        collect_factor_frame(out, self.id())
    }
}
