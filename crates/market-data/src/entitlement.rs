//! Todo 5 entitlement gate wiring for the raw zone.
//!
//! - [`governing_entitlement_reference`] records the governing licensed-data
//!   contract reference on each manifest row.
//! - The full access gate (visibility tagging + gated batch reads) lives in
//!   `entitlement::gate` and is exercised by the `raw_entitlement` tests.

pub use auth::entitlement::EntitlementService;

use domain::TradingDate;

/// The contract document reference of the entitlement governing
/// `krx_eod_bars` on `date`, if any. Recorded on manifest rows so every raw
/// batch traces to its licensed contract.
pub fn governing_entitlement_reference(
    service: &EntitlementService,
    date: TradingDate,
) -> Option<String> {
    use auth::entitlement::{CalendarDate, DatasetId};
    let as_of = CalendarDate::parse(&date.to_iso()).ok()?;
    let (id, _) = service.governing_state(&DatasetId::krx_eod_bars(), as_of)?;
    service
        .entitlements()
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.contract.document_reference.clone())
}
