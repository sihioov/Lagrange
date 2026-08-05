//! `selector` - Lagrange Station fixed-universe selector and constrained target portfolios.
//!
//! Todo 12 delivers the **fixed Korean ETF v1 universe** as a versioned,
//! immutable snapshot ([`universe`], [`publish`]):
//!
//! - [`universe::parse_manifest`] parses `configs/universes/kr-etf-core-v1.yaml`
//!   into a typed [`universe::UniverseManifest`] (exact canonical
//!   `InstrumentId = {symbol}.KRX` entries, benchmark, KRW base currency,
//!   unleveraged/non-inverse eligibility, effective window, KRX source
//!   snapshot) — malformed YAML, unknown fields, and inverted windows are
//!   typed [`universe::UniverseError`] errors, never panics;
//! - [`universe::UniverseManifest::canonical_hash`] computes the immutable
//!   `universe_snapshot_id` (SHA-256 over the canonical manifest): repeated
//!   builds hash identically, a changed manifest yields a new id;
//! - [`publish::UniversePublisher`] resolves every member against the
//!   instrument master (Todo 9), the entitlement gate (Todo 5), and the
//!   required-dataset quality report (Todo 11). Publication BLOCKS — typed
//!   error naming the exact instrument + reason — when an id is inactive,
//!   unsupported (asset class / leverage / inverse), duplicated, or
//!   unlicensed; it NEVER substitutes a different product automatically.
//!
//! Todo 16 (target portfolios) builds directly on the published snapshot.

pub mod builder;
pub mod eligibility;
pub mod error;
pub mod publish;
pub mod reason;
pub mod spec;
pub mod universe;

pub use builder::{PreparedUniverse, UniverseBuilder};
pub use eligibility::{EligibilityFilter, EligibilityOutcome, EligibleInstrument, Exclusion};
pub use error::SelectorError;
pub use publish::{ProductKind, PublishedSnapshot, UniversePublisher};
pub use universe::{
    Eligibility, SourceSnapshot, UniverseError, UniverseInstrumentEntry, UniverseManifest,
    UniverseSpec, parse_manifest,
};
