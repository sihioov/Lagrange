//! Versioned return factors (design §6.5 "1·3·6·12개월 수익률").
//!
//! `return_Nm(date) = close(date) / close(ref_Nm(date)) - 1` where the
//! reference is the LAST bar on or before `date - N calendar months`
//! (day-of-month clamped, see [`crate::months`]). Insufficient history is a
//! typed NULL; a bar after the target is never used (no forward fill).

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, FactorId, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, ref_close};

/// The 1/3/6/12-month trailing return factor (version 1.0.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnFactor {
    months: u32,
}

impl ReturnFactor {
    pub fn one_month() -> Self {
        Self { months: 1 }
    }

    pub fn three_months() -> Self {
        Self { months: 3 }
    }

    pub fn six_months() -> Self {
        Self { months: 6 }
    }

    pub fn twelve_months() -> Self {
        Self { months: 12 }
    }

    /// The calendar month window.
    pub fn months(&self) -> u32 {
        self.months
    }

    fn target_column(&self) -> Result<&'static str, FactorError> {
        match self.months {
            1 => Ok("target_1m"),
            3 => Ok("target_3m"),
            6 => Ok("target_6m"),
            12 => Ok("target_12m"),
            _ => Err(FactorError::InvalidDefinition {
                detail: format!("unsupported month window {}", self.months),
            }),
        }
    }
}

impl Factor for ReturnFactor {
    fn id(&self) -> FactorId {
        match self.months {
            1 => "return_1m",
            3 => "return_3m",
            6 => "return_6m",
            12 => "return_12m",
            _ => unreachable!("months validated by construction"),
        }
    }

    fn version(&self) -> domain::FactorVersion {
        domain::FactorVersion::parse("1.0.0").expect("static version")
    }

    fn required_fields(&self) -> &[Field] {
        &[Field::CLOSE]
    }

    fn lookback(&self) -> Lookback {
        Lookback::CalendarMonths(self.months)
    }

    fn null_policy(&self) -> NullPolicy {
        NullPolicy::InsufficientLookback
    }

    fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
        let lf = ctx.bars.lazy_frame();
        let joined = ref_close(&lf, self.target_column()?, "ref_close")?;
        let out = joined.select([
            col("instrument_id"),
            col("trading_date"),
            (col("close") / col("ref_close") - lit(1.0)).alias("value"),
        ]);
        collect_factor_frame(out, self.id())
    }
}
