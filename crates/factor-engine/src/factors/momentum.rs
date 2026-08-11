//! Versioned 12-minus-1 momentum (design §6.5 "최근 12개월에서 최근 1개월
//! 제외 모멘텀").
//!
//! `momentum_12_1(date) = close(ref_1m) / close(ref_12m) - 1`: the return
//! over months 2..12. NULL when either reference is missing.

use polars::prelude::*;

use crate::contract::{
    Factor, FactorContext, FactorError, FactorFrame, Field, Lookback, NullPolicy,
};
use crate::lazy_util::{collect_factor_frame, ref_close};

/// The 12-minus-1 momentum factor (version 1.0.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MomentumFactor;

impl Factor for MomentumFactor {
    fn id(&self) -> &str {
        "momentum_12_1"
    }

    fn version(&self) -> domain::FactorVersion {
        domain::FactorVersion::parse("1.0.0").expect("static version")
    }

    fn required_fields(&self) -> &[Field] {
        &[Field::CLOSE]
    }

    fn lookback(&self) -> Lookback {
        Lookback::CalendarMonths(12)
    }

    fn null_policy(&self) -> NullPolicy {
        NullPolicy::InsufficientLookback
    }

    fn compute(&self, ctx: &FactorContext) -> Result<FactorFrame, FactorError> {
        let lf = ctx.bars.lazy_frame();
        let with_1m = ref_close(&lf, "target_1m", "ref_1m")?;
        let with_12m = ref_close(&lf, "target_12m", "ref_12m")?;
        let joined = with_1m.join(
            with_12m,
            [col("instrument_id"), col("trading_date")],
            [col("instrument_id"), col("trading_date")],
            JoinArgs {
                how: JoinType::Left,
                suffix: Some("_12m".into()),
                ..Default::default()
            },
        );
        let out = joined.select([
            col("instrument_id"),
            col("trading_date"),
            (col("ref_1m") / col("ref_12m") - lit(1.0)).alias("value"),
        ]);
        collect_factor_frame(out, self.id())
    }
}
