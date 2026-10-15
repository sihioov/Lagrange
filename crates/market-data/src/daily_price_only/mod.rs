//! Daily ETF11 wire contracts and read-only, unapproved Raw evidence.
//!
//! Source verification is separate from admission, database, or runtime authority.

pub mod admission;
pub mod artifact;
pub mod calendar;
pub mod candidate;
pub mod continuation;
mod contract;
pub mod discovery;
pub mod frontier;
pub mod history;
pub mod observation;
mod price_extension;
pub mod reconstruct;
pub mod source;
pub mod supply;
pub mod verify;

pub use contract::*;
