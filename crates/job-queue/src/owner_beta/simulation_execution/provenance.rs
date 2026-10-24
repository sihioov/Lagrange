//! Closed build receipt and executable binding for the dedicated simulation
//! parent/child binary.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::owner_beta::simulation::{EngineIdentity, parse_strict_json};

use super::runner::SimulationWorkerError;

const BUILD_RECEIPT_SCHEMA: &str = "owner-beta-simulation-build-v1";
const MAX_BUILD_RECEIPT_BYTES: usize = 1024 * 1024;

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BuildReceipt {
    schema: String,
    git_commit: String,
    toolchain: String,
    target: String,
    profile: String,
    workflow_run_id: String,
    workflow_run_attempt: String,
    binary_sha256: String,
    source_hashes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompileIdentity {
    git_commit: String,
    toolchain: String,
    target: String,
    profile: String,
    workflow_run_id: String,
    workflow_run_attempt: String,
}

/// A validated binding between the operator-pinned build receipt and the
/// executable currently running this process. Its fields are intentionally
/// private so callers cannot manufacture an admitted runtime.
pub struct TrustedSimulationRuntime {
    artifact_root: PathBuf,
    receipt_path: PathBuf,
    expected_receipt_sha256: String,
    executable_path: PathBuf,
    engine: EngineIdentity,
}

impl TrustedSimulationRuntime {
    /// Loads the operator-pinned receipt and validates it against this exact
    /// executable and the build identity embedded by the hosted build.
    pub fn load(
        artifact_root: &Path,
        receipt_path: &Path,
        expected_receipt_sha256: &str,
    ) -> Result<Self, SimulationWorkerError> {
        let artifact_root = validate_artifact_root(artifact_root)?;
        if !receipt_path.is_absolute() || !is_lower_sha256(expected_receipt_sha256) {
            return Err(SimulationWorkerError::ProvenanceInvalid);
        }
        let receipt_path = receipt_path.to_path_buf();
        let executable_path =
            std::env::current_exe().map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
        if !executable_path.is_absolute() {
            return Err(SimulationWorkerError::ProvenanceInvalid);
        }
        let expected_receipt_sha256 = expected_receipt_sha256.to_owned();
        let (engine, _) = validate_current_material(
            &receipt_path,
            &expected_receipt_sha256,
            &executable_path,
            compiled_identity(),
        )?;
        Ok(Self {
            artifact_root,
            receipt_path,
            expected_receipt_sha256,
            executable_path,
            engine,
        })
    }

    /// Rechecks receipt bytes, compile identity, executable digest, and the
    /// approved-root directory immediately before claim and child spawn.
    pub fn revalidate(&self) -> Result<(), SimulationWorkerError> {
        validate_artifact_root(&self.artifact_root)?;
        let current_executable =
            std::env::current_exe().map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
        if current_executable != self.executable_path {
            return Err(SimulationWorkerError::ProvenanceInvalid);
        }
        let (engine, _) = validate_current_material(
            &self.receipt_path,
            &self.expected_receipt_sha256,
            &self.executable_path,
            compiled_identity(),
        )?;
        if engine != self.engine {
            return Err(SimulationWorkerError::ProvenanceInvalid);
        }
        Ok(())
    }

    pub(super) fn artifact_root(&self) -> &Path {
        &self.artifact_root
    }

    pub(super) fn receipt_path(&self) -> &Path {
        &self.receipt_path
    }

    pub(super) fn expected_receipt_sha256(&self) -> &str {
        &self.expected_receipt_sha256
    }

    pub(super) fn executable_path(&self) -> &Path {
        &self.executable_path
    }

    pub(super) fn engine(&self) -> &EngineIdentity {
        &self.engine
    }
}

fn validate_current_material(
    receipt_path: &Path,
    expected_receipt_sha256: &str,
    executable_path: &Path,
    compile_identity: Result<CompileIdentity, ()>,
) -> Result<(EngineIdentity, BuildReceipt), SimulationWorkerError> {
    if !is_lower_sha256(expected_receipt_sha256) {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let receipt_bytes = read_bounded_regular_file(receipt_path, MAX_BUILD_RECEIPT_BYTES)?;
    if sha256_hex(&receipt_bytes) != expected_receipt_sha256 {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let receipt = parse_build_receipt(&receipt_bytes)?;
    let compile_identity =
        compile_identity.map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if !receipt_matches_compile(&receipt, &compile_identity) {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    validate_receipt_fields(&receipt)?;
    let executable_sha256 = sha256_file(executable_path)?;
    if executable_sha256 != receipt.binary_sha256 {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    // The build receipt wire keeps the digest as bare lowercase hex. The
    // frozen EngineIdentity API accepts the existing canonical sha256: form.
    let engine_sha256 = format!("sha256:{}", receipt.binary_sha256);
    let engine = EngineIdentity::new(
        &receipt.git_commit,
        &engine_sha256,
        &receipt.toolchain,
        &receipt.target,
        &receipt.profile,
    )
    .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    Ok((engine, receipt))
}

fn parse_build_receipt(bytes: &[u8]) -> Result<BuildReceipt, SimulationWorkerError> {
    if bytes.is_empty() || bytes.len() > MAX_BUILD_RECEIPT_BYTES {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let value = parse_strict_json(bytes).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    serde_json::from_value(value).map_err(|_| SimulationWorkerError::ProvenanceInvalid)
}

fn validate_receipt_fields(receipt: &BuildReceipt) -> Result<(), SimulationWorkerError> {
    if receipt.schema != BUILD_RECEIPT_SCHEMA
        || !is_hex(&receipt.git_commit, 40, false)
        || !is_safe_identity(&receipt.toolchain)
        || !is_safe_identity(&receipt.target)
        || !is_safe_identity(&receipt.profile)
        || !is_positive_ascii_decimal(&receipt.workflow_run_id)
        || !is_positive_ascii_decimal(&receipt.workflow_run_attempt)
        || !is_lower_sha256(&receipt.binary_sha256)
        || receipt.source_hashes.is_empty()
        || receipt
            .source_hashes
            .iter()
            .any(|(path, digest)| !is_repository_relative_path(path) || !is_lower_sha256(digest))
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    Ok(())
}

fn receipt_matches_compile(receipt: &BuildReceipt, compile: &CompileIdentity) -> bool {
    receipt.git_commit == compile.git_commit
        && receipt.toolchain == compile.toolchain
        && receipt.target == compile.target
        && receipt.profile == compile.profile
        && receipt.workflow_run_id == compile.workflow_run_id
        && receipt.workflow_run_attempt == compile.workflow_run_attempt
}

fn compiled_identity() -> Result<CompileIdentity, ()> {
    compile_identity_from([
        option_env!("R4_SIMULATION_BUILD_GIT_COMMIT"),
        option_env!("R4_SIMULATION_BUILD_TOOLCHAIN"),
        option_env!("R4_SIMULATION_BUILD_TARGET"),
        option_env!("R4_SIMULATION_BUILD_PROFILE"),
        option_env!("R4_SIMULATION_BUILD_WORKFLOW_RUN_ID"),
        option_env!("R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT"),
    ])
}

fn compile_identity_from(fields: [Option<&str>; 6]) -> Result<CompileIdentity, ()> {
    let [
        Some(git_commit),
        Some(toolchain),
        Some(target),
        Some(profile),
        Some(workflow_run_id),
        Some(workflow_run_attempt),
    ] = fields
    else {
        return Err(());
    };
    let identity = CompileIdentity {
        git_commit: git_commit.to_owned(),
        toolchain: toolchain.to_owned(),
        target: target.to_owned(),
        profile: profile.to_owned(),
        workflow_run_id: workflow_run_id.to_owned(),
        workflow_run_attempt: workflow_run_attempt.to_owned(),
    };
    if !is_hex(&identity.git_commit, 40, false)
        || !is_safe_identity(&identity.toolchain)
        || !is_safe_identity(&identity.target)
        || !is_safe_identity(&identity.profile)
        || !is_positive_ascii_decimal(&identity.workflow_run_id)
        || !is_positive_ascii_decimal(&identity.workflow_run_attempt)
    {
        return Err(());
    }
    Ok(identity)
}

fn validate_artifact_root(path: &Path) -> Result<PathBuf, SimulationWorkerError> {
    if !path.is_absolute() {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    Ok(path.to_path_buf())
}

fn read_bounded_regular_file(
    path: &Path,
    max_bytes: usize,
) -> Result<Vec<u8>, SimulationWorkerError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes as u64
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let file = File::open(path).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if !file
        .metadata()
        .map(|opened| opened.is_file())
        .unwrap_or(false)
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    BufReader::new(file)
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    Ok(bytes)
}

fn sha256_file(path: &Path) -> Result<String, SimulationWorkerError> {
    let file = File::open(path).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    if !file
        .metadata()
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(hex_digest(&digest))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_digest(&digest)
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

fn is_lower_sha256(value: &str) -> bool {
    is_hex(value, 64, true)
}

fn is_hex(value: &str, len: usize, lowercase: bool) -> bool {
    value.len() == len
        && value.bytes().all(|byte| {
            byte.is_ascii_digit()
                || (b'a'..=b'f').contains(&byte)
                || (!lowercase && (b'A'..=b'F').contains(&byte))
        })
}

fn is_positive_ascii_decimal(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0)
}

fn is_safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

fn is_repository_relative_path(value: &str) -> bool {
    if value.is_empty() || value.contains(['\\', ':']) || value.starts_with('/') {
        return false;
    }
    let path = Path::new(value);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const TOOLCHAIN: &str = "1.97.1";
    const TARGET: &str = "x86_64-unknown-linux-gnu";
    const PROFILE: &str = "release";
    const RUN: &str = "40000000001";
    const ATTEMPT: &str = "1";

    fn compile() -> CompileIdentity {
        CompileIdentity {
            git_commit: COMMIT.to_owned(),
            toolchain: TOOLCHAIN.to_owned(),
            target: TARGET.to_owned(),
            profile: PROFILE.to_owned(),
            workflow_run_id: RUN.to_owned(),
            workflow_run_attempt: ATTEMPT.to_owned(),
        }
    }

    fn receipt(binary_sha256: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": BUILD_RECEIPT_SCHEMA,
            "git_commit": COMMIT,
            "toolchain": TOOLCHAIN,
            "target": TARGET,
            "profile": PROFILE,
            "workflow_run_id": RUN,
            "workflow_run_attempt": ATTEMPT,
            "binary_sha256": binary_sha256,
            "source_hashes": { "crates/job-queue/src/lib.rs": "a".repeat(64) }
        }))
        .expect("synthetic receipt serializes")
    }

    #[test]
    fn provenance_receipt_is_closed_duplicate_free_and_trailing_free() {
        let bytes = receipt(&"b".repeat(64));
        assert!(parse_build_receipt(&bytes).is_ok());
        assert!(
            parse_build_receipt(br#"{"schema":"owner-beta-simulation-build-v1","schema":"x"}"#)
                .is_err()
        );
        let mut unknown = bytes.clone();
        unknown.pop();
        unknown.extend_from_slice(br#", "unknown": true}"#);
        assert!(parse_build_receipt(&unknown).is_err());
        let mut trailing = bytes;
        trailing.extend_from_slice(b" {} ");
        assert!(parse_build_receipt(&trailing).is_err());
    }

    #[test]
    fn provenance_rejects_expected_receipt_compile_and_executable_mismatches() {
        let executable = b"synthetic executable bytes";
        let digest = sha256_hex(executable);
        let receipt_bytes = receipt(&digest);
        let expected_receipt_sha256 = sha256_hex(&receipt_bytes);
        let directory = tempfile::tempdir().expect("synthetic provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        let receipt_path = directory.path().join("receipt-fixture.json");
        std::fs::write(&executable_path, executable).expect("synthetic executable writes");
        std::fs::write(&receipt_path, &receipt_bytes).expect("synthetic receipt writes");

        assert!(
            validate_current_material(
                &receipt_path,
                &expected_receipt_sha256,
                &executable_path,
                Ok(compile()),
            )
            .is_ok()
        );
        assert!(
            validate_current_material(
                &receipt_path,
                &"0".repeat(64),
                &executable_path,
                Ok(compile()),
            )
            .is_err()
        );
        assert!(
            validate_current_material(
                &receipt_path,
                &expected_receipt_sha256,
                &executable_path,
                Ok(CompileIdentity {
                    workflow_run_attempt: "2".to_owned(),
                    ..compile()
                }),
            )
            .is_err()
        );
        std::fs::write(&executable_path, b"replaced executable")
            .expect("replacement executable writes");
        assert!(
            validate_current_material(
                &receipt_path,
                &expected_receipt_sha256,
                &executable_path,
                Ok(compile()),
            )
            .is_err()
        );
    }

    #[test]
    fn provenance_accepts_matching_receipt_compile_and_executable() {
        let executable = b"synthetic accepted executable";
        let binary_sha256 = sha256_hex(executable);
        assert!(!binary_sha256.starts_with("sha256:"));
        let receipt_bytes = receipt(&binary_sha256);
        let expected_receipt_sha256 = sha256_hex(&receipt_bytes);
        let directory = tempfile::tempdir().expect("synthetic provenance directory");
        let executable_path = directory.path().join("runner-fixture");
        let receipt_path = directory.path().join("receipt-fixture.json");
        std::fs::write(&executable_path, executable).expect("synthetic executable writes");
        std::fs::write(&receipt_path, &receipt_bytes).expect("synthetic receipt writes");

        let (engine, parsed_receipt) = validate_current_material(
            &receipt_path,
            &expected_receipt_sha256,
            &executable_path,
            Ok(compile()),
        )
        .expect("matching receipt and executable are accepted");
        assert_eq!(parsed_receipt.binary_sha256, binary_sha256);
        assert_eq!(
            engine,
            EngineIdentity::new(
                COMMIT,
                &format!("sha256:{binary_sha256}"),
                TOOLCHAIN,
                TARGET,
                PROFILE,
            )
            .expect("canonical engine digest")
        );
    }

    #[test]
    fn provenance_requires_complete_compile_variables_and_safe_source_paths() {
        let complete = [
            Some(COMMIT),
            Some(TOOLCHAIN),
            Some(TARGET),
            Some(PROFILE),
            Some(RUN),
            Some(ATTEMPT),
        ];
        assert!(compile_identity_from(complete).is_ok());
        assert!(
            compile_identity_from([
                Some(COMMIT),
                Some(TOOLCHAIN),
                Some(TARGET),
                Some(PROFILE),
                Some(RUN),
                None,
            ])
            .is_err()
        );
        assert!(!is_repository_relative_path("../secret.rs"));
        assert!(!is_repository_relative_path("/absolute.rs"));
        assert!(!is_repository_relative_path("crates\\job-queue\\src.rs"));
        assert!(is_repository_relative_path("crates/job-queue/src/lib.rs"));
    }
}
