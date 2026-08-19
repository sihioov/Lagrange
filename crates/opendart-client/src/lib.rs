//! Lagrange Station OpenDART disclosure adapter (read-only, Stage6).
//!
//! Owns HTTP for the three approved OpenDART read surfaces:
//! `/api/list.json`, `/api/corpCode.xml`, `/api/company.json`. This crate
//! does not interpret OpenDART responses -- `market-data` does that in a
//! follow-up task -- it owns getting bytes back *safely*:
//!
//! - a fixed path allowlist, checked before any request is built
//!   ([`allowlist`]);
//! - a file-only credential, read via `OPENDART_CRTFC_KEY_FILE`, with a
//!   redacted `Debug` and no env-var/argv path for the value itself
//!   ([`credential`]);
//! - an error type with no room for a URL, a query string, a response
//!   body, or a `reqwest::Error` to leak through ([`error`]);
//! - single-flight plus a 1-request-per-second ceiling, sharing one lock
//!   ([`rate`]);
//! - a small, bounded retry policy that never probes OpenDART's
//!   undocumented daily quota ([`status`]).
//!
//! Deliberately does **not** depend on `kis-client`: that crate is
//! order-capable, and this one must stay narrowly read-only. Where this
//! crate's shape mirrors `kis-client`'s live transport (the `Transport`
//! seam, the `Secret`/credential-source split), the pieces are duplicated
//! rather than imported, by design.

mod allowlist;
mod client;
mod credential;
mod error;
mod rate;
mod status;
mod transport;

pub use allowlist::ALLOWED_PATHS;
pub use client::{ClientConfig, OpenDartClient};
pub use credential::{
    CRTFC_KEY_FILE_ENV_VAR, CredentialError, CredentialRef, CredentialSource, Secret,
    SystemCredentialSource,
};
pub use error::{ApplicationStatus, OpenDartTransportError};
