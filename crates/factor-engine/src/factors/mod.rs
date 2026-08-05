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

/// The documented factor ids in canonical order (used by tests and QA).
pub fn mvp_factor_ids() -> Vec<&'static str> {
    all_mvp_factors().iter().map(|f| f.id()).collect()
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
