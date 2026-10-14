//! Daily ETF11 wire contracts and read-only, unapproved Raw evidence.
//!
//! Parsing, hashing, and capturing evidence convey no provider verification,
//! admission, database, or runtime authority.

mod contract;
pub mod source;

pub use contract::*;
