//! Build script for the tenancy gate.
//!
//! `sqlx::migrate!("../../migrations")` (used by the integration harness in
//! `tests/tenancy_rls.rs`) embeds the workspace migration SQL at compile time.
//! This build script makes Cargo rebuild the crate whenever the `migrations/`
//! directory changes (files added/removed/edited), so the embedded Migrator
//! always matches the working tree. Path is relative to this package root
//! (`crates/api-server` -> workspace root).

fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
