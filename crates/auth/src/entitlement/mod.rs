//! KRX market-data entitlement gate (fail closed).
//!
//! Module layout:
//! - [`date`] - dependency-free civil `CalendarDate` arithmetic.
//! - [`state`] - the `PENDING | ACTIVE | EXPIRED | REVOKED` lifecycle.
//! - [`identity`] - actors, roles, and branded IDs.
//! - [`entitlement`] - the redacted `Entitlement` record and `ContractRef`.
//! - [`use_registry`] - the registry of KR-derived uses and Member-visible surfaces.
//! - [`service`] - the shared authorization service (fail closed).
//! - [`audit`] - authorization audit events.
//! - [`error`] - typed denial and transition errors.

pub mod audit;
pub mod date;
pub mod error;
pub mod identity;
pub mod record;
pub mod service;
pub mod state;
pub mod use_registry;

pub use audit::{AuditDecision, AuditEvent, AuditLog, audit_event_for};
pub use date::{CalendarDate, DateError};
pub use error::{DenialCode, DenialReason, EntitlementDenied, TransitionError};
pub use identity::{Actor, DataProvider, DatasetId, EntitlementId, Role, UserId};
pub use record::{ContractRef, DocumentHash, Entitlement, EntitlementBuilder};
pub use service::{AccessRequest, EntitlementService, Grant, OwnerDevGrant};
pub use state::EntitlementState;
pub use use_registry::{KrMemberSurface, KrUse, KrUseRegistry, Layer};
