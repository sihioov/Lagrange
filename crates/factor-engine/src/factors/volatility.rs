//! Versioned realized-volatility factors (design §6.5 "20·60·120일 실현
//! 변동성").
//!
//! `vol_N = sqrt(252) * sample_std(log returns, trailing N)` (ddof = 1,
//! matching the polars rolling kernel). The first N bars of a series yield
//! NULL (N returns need N+1 bars); windows never cross instrument boundaries.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, FactorId, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, instruments_of, map_per_instrument, rolling};

/// The 20/60/120-day annualized realized-volatility factor (version 1.0.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealizedVolFactor {
    window: usize,
}

impl RealizedVolFactor {
    pub fn new(window: usize) -> Result<Self, FactorError> {
        if matches!(window, 20 | 60 | 120) {
            Ok(Self { window })
        } else {
            Err(FactorError::InvalidDefinition {
                detail: format!("unsupported volatility window {window} (documented: 20/60/120)"),
            })
        }
    }

    /// The documented trading-day window.
    pub fn window(&self) -> usize {
        self.window
    }
}

impl Factor for RealizedVolFactor {
    fn id(&self) -> FactorId {
        match self.window {
            20 => "vol_20",
            60 => "vol_60",
            120 => "vol_120",
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
