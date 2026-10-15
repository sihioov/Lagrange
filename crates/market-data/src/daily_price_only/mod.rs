//! Daily ETF11 wire contracts and read-only, unapproved Raw evidence.
//!
//! Source verification is separate from admission, database, or runtime authority.

pub mod calendar;
mod contract;
pub mod source;
pub mod verify;

pub use contract::*;
