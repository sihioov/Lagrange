//! The versioned MVP factor registry (design §6.5).
//!
//! Every factor is a versioned, documented transformation; the registry is
//! the canonical set consumed by [`crate::snapshot::FactorSnapshotBuilder`].

pub mod drawdown;
pub mod liquidity;
pub mod momentum;
pub mod returns;
pub mod trend;
pub mod volatility;

pub use drawdown::DrawdownFactor;
pub use liquidity::AvgValueFactor;
pub use momentum::MomentumFactor;
pub use returns::ReturnFactor;
pub use trend::TrendFactor;
pub use volatility::RealizedVolFactor;

use crate::contract::{Factor, FactorError};

/// All MVP factors at their current versions (13 factors), in canonical order.
pub fn all_mvp_factors() -> Vec<Box<dyn Factor>> {
    vec![
        Box::new(ReturnFactor::one_month()),
        Box::new(ReturnFactor::three_months()),
        Box::new(ReturnFactor::six_months()),
        Box::new(ReturnFactor::twelve_months()),
        Box::new(MomentumFactor),
        Box::new(TrendFactor::new(50).expect("documented window")),
        Box::new(TrendFactor::new(100).expect("documented window")),
        Box::new(TrendFactor::new(200).expect("documented window")),
        Box::new(RealizedVolFactor::new(20).expect("documented window")),
        Box::new(RealizedVolFactor::new(60).expect("documented window")),
        Box::new(RealizedVolFactor::new(120).expect("documented window")),
        Box::new(AvgValueFactor),
        Box::new(DrawdownFactor),
    ]
}

/// The owner-beta price-only MVP registry.
///
/// Liquidity is intentionally absent: the approved input carries adjusted
/// close only, even though its source rows retain raw trading value for the
/// source contract. The order is stable and matches the generic MVP registry
/// with `avg_value_20` removed.
pub fn all_price_only_factors() -> Vec<Box<dyn Factor>> {
    vec![
        Box::new(ReturnFactor::one_month()),
        Box::new(ReturnFactor::three_months()),
        Box::new(ReturnFactor::six_months()),
        Box::new(ReturnFactor::twelve_months()),
        Box::new(MomentumFactor),
        Box::new(TrendFactor::new(50).expect("documented window")),
        Box::new(TrendFactor::new(100).expect("documented window")),
        Box::new(TrendFactor::new(200).expect("documented window")),
        Box::new(RealizedVolFactor::new(20).expect("documented window")),
        Box::new(RealizedVolFactor::new(60).expect("documented window")),
        Box::new(RealizedVolFactor::new(120).expect("documented window")),
        Box::new(DrawdownFactor),
    ]
}

/// Stable ids of the owner-beta price-only MVP registry.
pub fn price_only_factor_ids() -> Vec<String> {
    all_price_only_factors()
        .iter()
        .map(|factor| factor.id().to_owned())
        .collect()
}

/// The documented factor ids in canonical order (used by tests and QA).
pub fn mvp_factor_ids() -> Vec<String> {
    all_mvp_factors()
        .iter()
        .map(|f| f.id().to_owned())
        .collect()
}

/// Builds a registry with the given factors, validating ids are unique.
pub fn registry_with(factors: Vec<Box<dyn Factor>>) -> Result<Vec<Box<dyn Factor>>, FactorError> {
    let mut seen = std::collections::BTreeSet::new();
    for f in &factors {
        if !seen.insert(f.id()) {
            return Err(FactorError::InvalidDefinition {
                detail: format!("duplicate factor id {:?}", f.id()),
            });
        }
    }
    Ok(factors)
}
