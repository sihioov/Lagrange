//! Versioned trend factors (design §6.5 "50·100·200일 이동평균 대비 가격").
//!
//! `trend_N(date) = close / SMA_N(close) - 1` over the trailing N trading
//! days. The full window is required (min_periods == N): shorter history is a
//! typed NULL, and the window never crosses instrument boundaries.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, FactorId, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, instruments_of, map_per_instrument, rolling};

/// The 50/100/200-day price-vs-moving-average factor (version 1.0.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrendFactor {
    window: usize,
}

impl TrendFactor {
    pub fn new(window: usize) -> Result<Self, FactorError> {
        if matches!(window, 50 | 100 | 200) {
            Ok(Self { window })
        } else {
            Err(FactorError::InvalidDefinition {
                detail: format!("unsupported trend window {window} (documented: 50/100/200)"),
            })
        }
    }

    /// The documented trading-day window.
    pub fn window(&self) -> usize {
        self.window
    }
}

impl Factor for TrendFactor {
    fn id(&self) -> FactorId {
        match self.window {
            50 => "trend_50",
            100 => "trend_100",
            200 => "trend_200",
            _ => unreachable!("window validated by construction"),
        }
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
