//! `domain` - Lagrange Station typed domain contracts.
//!
//! This crate owns every shared primitive of the platform so services never
//! duplicate them: branded IDs, UTC/venue-local timestamps, `TradingDate`,
//! fixed-point `Money`/`Price`/`Quantity`/`Weight`, currency/venue/instrument
//! types, version/hash/provenance IDs, lifecycle enums, and the typed
//! `DomainError`.
//!
//! JSON rule: decimal values cross JSON boundaries as STRINGS at a per-type
//! canonical scale (byte-equivalent after canonicalization). Floats exist only
//! for reported statistics ([`ReportedStat`]) behind finite-value checks.
//! Email/symbol strings are never durable identities — use the branded types.

pub mod currency;
pub mod error;
pub mod fixed;

pub use currency::{Currency, Venue};
pub use error::DomainError;
pub use fixed::{
    FixedPoint, Money, Price, Quantity, Weight, MAX_SCALE, MONEY_SCALE, PRICE_SCALE,
    QUANTITY_SCALE, WEIGHT_SCALE,
};
