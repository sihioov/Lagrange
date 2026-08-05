//! `auth` - Lagrange Station authentication and authorization (Auth0 OIDC, sessions, RBAC).
//!
//! Todo 5 implements the **KRX market-data entitlement gate**: the `data_entitlements`
//! lifecycle (`PENDING | ACTIVE | EXPIRED | REVOKED`) with typed transitions, and the
//! shared [`entitlement::EntitlementService`] that every API / scheduler / report /
//! artifact layer uses to answer *"is this KR-derived use allowed for this user on
//! this date"*.
//!
//! Todo 22 adds the **confidential OIDC/session authority**: [`oidc`] (Authorization
//! Code + PKCE S256 with exact redirect, state+nonce, RS256 JWT/JWKS validation),
//! [`invites`] (single-use normalized-email invitations requiring `email_verified`,
//! immutable `(iss, sub)` identity binding), [`sessions`] (random opaque
//! `__Host-lagrange_session` cookie hashed in the [`sessions::SessionStore`]),
//! [`csrf`] (synchronizer tokens) and [`stepup`] (Owner MFA/fresh-auth via
//! `auth_time`/`amr`). Identity is keyed by `(issuer, subject)`, never by email, and
//! provider tokens never leave the server. The Axum router lives in
//! `apps/api-server/auth` and delegates here.
//!
//! ## Session persistence seam (Todo 3 is BLOCKED)
//!
//! `web_sessions` is a Todo-3 migration table that does not exist yet.
//! [`sessions::SessionStore`] is the typed async trait contract (opaque cookie
//! value hashed before storage; lookup/revoke/expiry; ownership binding to the
//! internal user); the tested in-memory implementation ships now, and the
//! PostgreSQL implementation lands with Todo 3. The same seam pattern applies to
//! [`invites::InviteStore`], [`invites::UserStore`], and
//! [`oidc::PendingAuthStore`].
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
//! - No uninvited identity logs in; invites are single-use; `(iss, sub)` is the
//!   immutable identity key; sessions are short, opaque, revocable, and
//!   CSRF-protected; Owner-sensitive actions require fresh MFA (`auth_time`/`amr`).

pub mod entitlement;
pub mod oidc;
pub mod simulator;

mod testkey;
