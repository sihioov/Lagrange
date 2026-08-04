//! `lagrange-auth` - Lagrange Station authentication and authorization (Auth0 OIDC, sessions, RBAC).
//!
//! Todo 5 implements the **KRX market-data entitlement gate**: the `data_entitlements`
//! lifecycle (`PENDING | ACTIVE | EXPIRED | REVOKED`) with typed transitions, and the
//! shared [`entitlement::EntitlementService`] that every API / scheduler / report /
//! artifact layer uses to answer *"is this KR-derived use allowed for this user on
//! this date"*.
//!
//! ## Fail-closed contract
//!
//! - A Member-visible KR-derived use is allowed **only** when an `ACTIVE` entitlement
//!   covers the dataset, the use, and the user on the as-of date.
//! - `PENDING`, `EXPIRED`, and `REVOKED` deny **every** Member-visible use.
//! - Owner-only development paths remain allowed for the Owner independent of any
//!   entitlement; Members never get development paths.
//! - Rights are **never inferred from an API key or any other credential**: an
//!   explicit entitlement record is the only source of permission.
//! - No contract contents are ever stored - only a document hash and a storage
//!   reference (see [`entitlement::ContractRef`]).

pub mod entitlement;
