//! Negative CLI checks for the dedicated source-only runner binary.
//!
//! These tests use an empty temporary root and a missing synthetic receipt.
//! They never open the approved input root, connect to PostgreSQL, or perform
//! artifact-backed computation.

use std::{
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_owner-beta-simulation-runner"))
}

fn output(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .env_clear()
        .env("APP_ENV", "development")
        .env("DATABASE_URL", "postgres://127.0.0.1:1/unused")
        .output()
        .expect("runner process starts")
}

#[test]
fn owner_beta_simulation_runner_requires_exactly_one_mode_and_closed_arguments() {
    let root = TempDir::new().expect("synthetic root");
    let result = output(&["--once"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&result.stderr).trim(),
        "simulation runner arguments are invalid"
    );

    let both_modes = output(&[
        "--once",
        "--daemon",
        "--artifact-root",
        root.path().to_str().expect("UTF-8 temp path"),
    ]);
    assert_eq!(both_modes.status.code(), Some(2));
    assert!(both_modes.stdout.is_empty());
}

#[test]
fn owner_beta_simulation_runner_rejects_provenance_before_database_connect() {
    let root = TempDir::new().expect("synthetic root");
    let receipt = root.path().join("missing-receipt.json");
    let digest = "a".repeat(64);
    let result = output(&[
        "--once",
        "--artifact-root",
        root.path().to_str().expect("UTF-8 temp path"),
        "--build-receipt",
        receipt.to_str().expect("UTF-8 receipt path"),
        "--expected-receipt-sha256",
        &digest,
        "--worker-id",
        "fixture-worker",
    ]);
    let (expected_code, expected_stderr) = if cfg!(target_os = "linux") {
        (3, "simulation runner provenance rejected")
    } else {
        (7, "simulation runner supports Linux only")
    };
    assert_eq!(result.status.code(), Some(expected_code));
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&result.stderr).trim(),
        expected_stderr
    );
}

#[test]
fn owner_beta_simulation_child_rejects_untrusted_startup_before_request_processing() {
    let root = TempDir::new().expect("synthetic root");
    let receipt = root.path().join("missing-receipt.json");
    let digest = "b".repeat(64);
    let mut child = Command::new(binary())
        .args([
            "--child",
            "--artifact-root",
            root.path().to_str().expect("UTF-8 temp path"),
            "--build-receipt",
            receipt.to_str().expect("UTF-8 receipt path"),
            "--expected-receipt-sha256",
            &digest,
        ])
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("child process starts");
    use std::io::Write as _;
    let _ = child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"not-json");
    let result = child.wait_with_output().expect("child exits");
    let expected_code = if cfg!(target_os = "linux") { 21 } else { 25 };
    assert_eq!(result.status.code(), Some(expected_code));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
}
