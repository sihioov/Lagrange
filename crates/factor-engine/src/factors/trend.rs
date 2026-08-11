//! Versioned bounded-window trend factors.
//!
//! `trend_N(date) = close / SMA_N(close) - 1` over the trailing N trading
//! days. The full window is required (min_periods == N): shorter history is a
//! typed NULL, and the window never crosses instrument boundaries.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, instruments_of, map_per_instrument, rolling};

/// A 5..=500-day price-vs-moving-average factor (version 1.0.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrendFactor {
    window: usize,
    id: String,
}

impl TrendFactor {
    pub fn new(window: usize) -> Result<Self, FactorError> {
        if (5..=500).contains(&window) {
            Ok(Self {
                window,
                id: format!("trend_{window}"),
            })
        } else {
            Err(FactorError::InvalidDefinition {
                detail: format!("unsupported trend window {window} (bounded: 5..=500)"),
            })
        }
    }

    /// The documented trading-day window.
    pub fn window(&self) -> usize {
        self.window
    }
}

impl Factor for TrendFactor {
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
            part.select([
                col("instrument_id"),
                col("trading_date"),
                (col("close") / col("close").rolling_mean(rolling(window)) - lit(1.0))
                    .alias("value"),
            ])
        })?;
        collect_factor_frame(out, self.id())
    }
}
