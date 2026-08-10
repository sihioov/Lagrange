//! Build script for the migration-contract gate.
//!
//! `sqlx::migrate!("../../../migrations")` embeds the workspace migration SQL at
//! compile time. This build script makes Cargo rebuild the crate whenever the
//! `migrations/` directory changes (files added/removed/edited), so the embedded
//! Migrator always matches the working tree. Path is relative to this package
//! root (`tests/integration/migration-contract` -> workspace root).

fn main() {
    println!("cargo:rerun-if-changed=../../../migrations");
    println!("cargo:rerun-if-changed=../../../deploy/compose/research-schema-check.sql");
}
