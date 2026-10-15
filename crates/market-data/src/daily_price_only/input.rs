//! Borrowed computation input from reopened sealed admission files.
//!
//! This capability proves immutable local input, not current DB authorization.
//! Live use additionally requires the current admission/ancestor and Owner gates.

use domain::TradingDate;

use super::admission::DailyAdmissionVerificationError;
use super::continuation::SealedDailyParent;
use super::{DailyPriceManifestV1Wire, DailyPricePinsV1Wire};
use crate::HistoricalPriceOnlyBar;

/// An opaque daily input. Neither wire pins, bars nor a filesystem path can
/// construct this value: both parent variants already require protected sealed
/// files, an actual approved historical base and a hash-pinned policy.
pub struct AdmittedDailyPriceInputV1<'input, 'base> {
    sealed: SealedDailyParent<'input, 'base>,
}

impl<'input, 'base> AdmittedDailyPriceInputV1<'input, 'base> {
    pub fn from_sealed(
        sealed: SealedDailyParent<'input, 'base>,
    ) -> Result<Self, DailyAdmissionVerificationError> {
        sealed.revalidate()?;
        Ok(Self { sealed })
    }

    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        self.sealed.pins()
    }

    pub fn manifest(&self) -> &DailyPriceManifestV1Wire {
        self.sealed.manifest()
    }

    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        self.sealed.bars()
    }

    pub fn as_of(&self) -> TradingDate {
        self.manifest().range_end
    }

    /// Retained descriptors remain subject to identity, size and byte checks.
    pub fn revalidate(&self) -> Result<(), DailyAdmissionVerificationError> {
        self.sealed.revalidate()
    }
}

impl std::fmt::Debug for AdmittedDailyPriceInputV1<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedDailyPriceInputV1")
            .field("as_of", &self.as_of())
            .finish_non_exhaustive()
    }
}
