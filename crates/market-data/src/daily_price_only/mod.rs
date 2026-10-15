//! Daily ETF11 wire contracts and read-only, unapproved Raw evidence.
//!
//! Source verification is separate from admission, database, or runtime authority.

pub mod calendar;
pub mod candidate;
mod contract;
pub mod discovery;
pub mod history;
pub mod observation;
pub mod reconstruct;
pub mod source;
pub mod verify;

pub use contract::*;
