//! Closed build receipt and executable binding for the dedicated simulation
//! parent/child binary.

mod official;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::owner_beta::simulation::{EngineIdentity, parse_strict_json};

use super::process_config::SimulationProcessConfig;
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum BuildCompileIdentity {
    Hosted(CompileIdentity),
    Official(official::CompileIdentity),
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
    official_process_binding: Option<OfficialProcessBinding>,
}

struct OfficialProcessBinding {
    config: SimulationProcessConfig,
    expected_commit: String,
    expected_receipt_sha256: String,
}

struct ValidatedOfficialConfig {
    artifact_root: PathBuf,
    receipt_path: PathBuf,
    expected_receipt_sha256: String,
    executable_path: PathBuf,
    engine: EngineIdentity,
}

trait OfficialProvenanceIo {
    fn compile_identity(&self) -> Result<BuildCompileIdentity, ()>;
    fn validate_artifact_root(&self, path: &Path) -> Result<PathBuf, SimulationWorkerError>;
    fn current_executable(&self) -> Result<PathBuf, SimulationWorkerError>;
    fn read_bounded_regular_file(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, SimulationWorkerError>;
    fn sha256_file(&self, path: &Path) -> Result<String, SimulationWorkerError>;
}

struct SystemOfficialProvenanceIo;

impl OfficialProvenanceIo for SystemOfficialProvenanceIo {
    fn compile_identity(&self) -> Result<BuildCompileIdentity, ()> {
        compiled_identity()
    }

    fn validate_artifact_root(&self, path: &Path) -> Result<PathBuf, SimulationWorkerError> {
        validate_artifact_root(path)
    }

    fn current_executable(&self) -> Result<PathBuf, SimulationWorkerError> {
        std::env::current_exe().map_err(|_| SimulationWorkerError::ProvenanceInvalid)
    }

    fn read_bounded_regular_file(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, SimulationWorkerError> {
        read_bounded_regular_file(path, max_bytes)
    }

    fn sha256_file(&self, path: &Path) -> Result<String, SimulationWorkerError> {
        sha256_file(path)
    }
}

impl TrustedSimulationRuntime {
    /// Loads the operator-pinned receipt and validates it against this exact
    /// executable and the identity embedded by either supported closed
    /// build origin.
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
            official_process_binding: None,
        })
    }

    /// Loads an official runtime from an already checked, fixed process config
    /// and independent commit/receipt pins.
    pub fn load_official_config(
        config: &SimulationProcessConfig,
        expected_commit: &str,
        expected_receipt_sha256: &str,
    ) -> Result<Self, SimulationWorkerError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (config, expected_commit, expected_receipt_sha256);
            return Err(SimulationWorkerError::UnsupportedPlatform);
        }

        #[cfg(target_os = "linux")]
        {
            let validated = validate_official_config_with(
                config,
                expected_commit,
                expected_receipt_sha256,
                &SystemOfficialProvenanceIo,
            )?;
            Ok(Self {
                artifact_root: validated.artifact_root,
                receipt_path: validated.receipt_path,
                expected_receipt_sha256: validated.expected_receipt_sha256.clone(),
                executable_path: validated.executable_path,
                engine: validated.engine,
                official_process_binding: Some(OfficialProcessBinding {
                    config: config.clone(),
                    expected_commit: expected_commit.to_owned(),
                    expected_receipt_sha256: validated.expected_receipt_sha256,
                }),
            })
        }
    }

    /// Rechecks receipt bytes, compile identity, executable digest, and the
    /// approved-root directory immediately before claim and child spawn.
    pub fn revalidate(&self) -> Result<(), SimulationWorkerError> {
        if self.official_process_binding.is_some() {
            #[cfg(not(target_os = "linux"))]
            return Err(SimulationWorkerError::UnsupportedPlatform);

            #[cfg(target_os = "linux")]
            return revalidate_official_runtime_with(self, &SystemOfficialProvenanceIo);
        }

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

fn validate_official_config_with<I: OfficialProvenanceIo>(
    config: &SimulationProcessConfig,
    expected_commit: &str,
    expected_receipt_sha256: &str,
    io: &I,
) -> Result<ValidatedOfficialConfig, SimulationWorkerError> {
    if !is_lower_nonzero_hex(expected_commit, 40)
        || !is_lower_sha256(expected_receipt_sha256)
        || config.code_commit() != expected_commit
        || config.receipt_sha256() != format!("sha256:{expected_receipt_sha256}")
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }

    let BuildCompileIdentity::Official(compile) = io
        .compile_identity()
        .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?
    else {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    };
    if !official::compile_matches_process_config(
        &compile,
        config.code_commit(),
        config.source_input_sha256(),
        config.recipe_sha256(),
    ) {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }

    let artifact_root = io.validate_artifact_root(Path::new(config.artifact_root()))?;
    let executable_path = PathBuf::from(config.executable_path());
    let current_executable = io.current_executable()?;
    if !executable_path.is_absolute()
        || !current_executable.is_absolute()
        || current_executable != executable_path
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }

    let receipt_path = PathBuf::from(config.receipt_path());
    let receipt_bytes = io.read_bounded_regular_file(&receipt_path, MAX_BUILD_RECEIPT_BYTES)?;
    let (engine, binary_sha256) = validate_current_material_bytes(
        &receipt_bytes,
        expected_receipt_sha256,
        &executable_path,
        Ok(BuildCompileIdentity::Official(compile)),
        |path| io.sha256_file(path),
    )?;
    if config.binary_sha256() != format!("sha256:{binary_sha256}") {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }

    let complete_path = PathBuf::from(config.complete_path());
    let complete_bytes = io.read_bounded_regular_file(&complete_path, 65)?;
    let mut expected_complete = expected_receipt_sha256.as_bytes().to_vec();
    expected_complete.push(b'\n');
    if complete_bytes.len() != 65 || complete_bytes != expected_complete {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }

    Ok(ValidatedOfficialConfig {
        artifact_root,
        receipt_path,
        expected_receipt_sha256: expected_receipt_sha256.to_owned(),
        executable_path,
        engine,
    })
}

fn revalidate_official_runtime_with<I: OfficialProvenanceIo>(
    runtime: &TrustedSimulationRuntime,
    io: &I,
) -> Result<(), SimulationWorkerError> {
    let binding = runtime
        .official_process_binding
        .as_ref()
        .ok_or(SimulationWorkerError::ProvenanceInvalid)?;
    let validated = validate_official_config_with(
        &binding.config,
        &binding.expected_commit,
        &binding.expected_receipt_sha256,
        io,
    )?;
    if validated.artifact_root != runtime.artifact_root
        || validated.receipt_path != runtime.receipt_path
        || validated.expected_receipt_sha256 != runtime.expected_receipt_sha256
        || validated.executable_path != runtime.executable_path
        || validated.engine != runtime.engine
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    Ok(())
}

fn validate_current_material(
    receipt_path: &Path,
    expected_receipt_sha256: &str,
    executable_path: &Path,
    compile_identity: Result<BuildCompileIdentity, ()>,
) -> Result<(EngineIdentity, String), SimulationWorkerError> {
    if !is_lower_sha256(expected_receipt_sha256) {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let receipt_bytes = read_bounded_regular_file(receipt_path, MAX_BUILD_RECEIPT_BYTES)?;
    validate_current_material_bytes(
        &receipt_bytes,
        expected_receipt_sha256,
        executable_path,
        compile_identity,
        sha256_file,
    )
}

fn validate_current_material_bytes(
    receipt_bytes: &[u8],
    expected_receipt_sha256: &str,
    executable_path: &Path,
    compile_identity: Result<BuildCompileIdentity, ()>,
    sha256_executable: impl FnOnce(&Path) -> Result<String, SimulationWorkerError>,
) -> Result<(EngineIdentity, String), SimulationWorkerError> {
    if receipt_bytes.is_empty()
        || receipt_bytes.len() > MAX_BUILD_RECEIPT_BYTES
        || !is_lower_sha256(expected_receipt_sha256)
        || sha256_hex(receipt_bytes) != expected_receipt_sha256
    {
        return Err(SimulationWorkerError::ProvenanceInvalid);
    }
    let value =
        parse_strict_json(receipt_bytes).map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
    match compile_identity.map_err(|_| SimulationWorkerError::ProvenanceInvalid)? {
        BuildCompileIdentity::Hosted(compile) => {
            let receipt: BuildReceipt = serde_json::from_value(value)
                .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
            if !receipt_matches_compile(&receipt, &compile) {
                return Err(SimulationWorkerError::ProvenanceInvalid);
            }
            validate_receipt_fields(&receipt)?;
            let executable_sha256 = sha256_executable(executable_path)?;
            if executable_sha256 != receipt.binary_sha256 {
                return Err(SimulationWorkerError::ProvenanceInvalid);
            }
            // The hosted receipt wire keeps the digest as bare lowercase hex.
            // EngineIdentity retains its existing canonical sha256: form.
            let engine_sha256 = format!("sha256:{}", receipt.binary_sha256);
            let engine = EngineIdentity::new(
                &receipt.git_commit,
                &engine_sha256,
                &receipt.toolchain,
                &receipt.target,
                &receipt.profile,
            )
            .map_err(|_| SimulationWorkerError::ProvenanceInvalid)?;
            Ok((engine, receipt.binary_sha256))
        }
        BuildCompileIdentity::Official(compile) => official::validate_receipt_value_with(
            value,
            &compile,
            executable_path,
            sha256_executable,
        ),
    }
}

#[cfg(test)]
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

fn compiled_identity() -> Result<BuildCompileIdentity, ()> {
    compile_identity_from(
        option_env!("R4_SIMULATION_BUILD_ORIGIN"),
        [
            option_env!("R4_SIMULATION_BUILD_GIT_COMMIT"),
            option_env!("R4_SIMULATION_BUILD_TOOLCHAIN"),
            option_env!("R4_SIMULATION_BUILD_TARGET"),
            option_env!("R4_SIMULATION_BUILD_PROFILE"),
        ],
        [
            option_env!("R4_SIMULATION_BUILD_WORKFLOW_RUN_ID"),
            option_env!("R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT"),
        ],
        [
            option_env!("LAGRANGE_CODE_COMMIT"),
            option_env!("R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256"),
            option_env!("R4_SIMULATION_BUILD_RECIPE_SHA256"),
        ],
    )
}

fn compile_identity_from(
    origin: Option<&str>,
    common: [Option<&str>; 4],
    hosted_workflow: [Option<&str>; 2],
    official_release: [Option<&str>; 3],
) -> Result<BuildCompileIdentity, ()> {
    let [
        Some(git_commit),
        Some(toolchain),
        Some(target),
        Some(profile),
    ] = common
    else {
        return Err(());
    };
    if !is_hex(git_commit, 40, false)
        || !is_safe_identity(toolchain)
        || !is_safe_identity(target)
        || !is_safe_identity(profile)
    {
        return Err(());
    }
    match origin {
        None => {
            // With ORIGIN absent, both release-only hashes must also be
            // absent; never infer a release origin from partial identity.
            // LAGRANGE_CODE_COMMIT may be unrelated hosted metadata, so ignore
            // it on this path.
            if official_release[1].is_some() || official_release[2].is_some() {
                return Err(());
            }
            let [Some(workflow_run_id), Some(workflow_run_attempt)] = hosted_workflow else {
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
            if !is_positive_ascii_decimal(&identity.workflow_run_id)
                || !is_positive_ascii_decimal(&identity.workflow_run_attempt)
            {
                return Err(());
            }
            Ok(BuildCompileIdentity::Hosted(identity))
        }
        Some("official-release-v1") => {
            if hosted_workflow.iter().any(Option::is_some) {
                return Err(());
            }
            let [Some(code_commit), Some(input_sha256), Some(recipe_sha256)] = official_release
            else {
                return Err(());
            };
            if code_commit != git_commit
                || toolchain != official::TOOLCHAIN
                || target != official::TARGET
                || profile != official::PROFILE
                || !is_lower_sha256(input_sha256)
                || !is_lower_sha256(recipe_sha256)
            {
                return Err(());
            }
            let identity = official::CompileIdentity::new(
                git_commit,
                toolchain,
                target,
                profile,
                input_sha256,
                recipe_sha256,
            )?;
            Ok(BuildCompileIdentity::Official(identity))
        }
        Some(_) => Err(()),
    }
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

fn is_lower_nonzero_hex(value: &str, len: usize) -> bool {
    is_hex(value, len, true) && value.bytes().any(|byte| byte != b'0')
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
    use std::{cell::RefCell, collections::BTreeMap};

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
                Ok(BuildCompileIdentity::Hosted(compile())),
            )
            .is_ok()
        );
        assert!(
            validate_current_material(
                &receipt_path,
                &"0".repeat(64),
                &executable_path,
                Ok(BuildCompileIdentity::Hosted(compile())),
            )
            .is_err()
        );
        assert!(
            validate_current_material(
                &receipt_path,
                &expected_receipt_sha256,
                &executable_path,
                Ok(BuildCompileIdentity::Hosted(CompileIdentity {
                    workflow_run_attempt: "2".to_owned(),
                    ..compile()
                })),
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
                Ok(BuildCompileIdentity::Hosted(compile())),
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
            Ok(BuildCompileIdentity::Hosted(compile())),
        )
        .expect("matching receipt and executable are accepted");
        assert_eq!(parsed_receipt, binary_sha256);
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
        let common = [Some(COMMIT), Some(TOOLCHAIN), Some(TARGET), Some(PROFILE)];
        assert!(
            compile_identity_from(None, common, [Some(RUN), Some(ATTEMPT)], [None, None, None],)
                .is_ok()
        );
        assert!(
            compile_identity_from(None, common, [Some(RUN), None], [None, None, None],).is_err()
        );
        assert!(!is_repository_relative_path("../secret.rs"));
        assert!(!is_repository_relative_path("/absolute.rs"));
        assert!(!is_repository_relative_path("crates\\job-queue\\src.rs"));
        assert!(is_repository_relative_path("crates/job-queue/src/lib.rs"));
    }

    fn official_compile() -> BuildCompileIdentity {
        BuildCompileIdentity::Official(
            official::CompileIdentity::new(
                COMMIT,
                official::TOOLCHAIN,
                official::TARGET,
                official::PROFILE,
                &"c".repeat(64),
                &"d".repeat(64),
            )
            .expect("synthetic official identity is valid"),
        )
    }

    fn official_receipt(binary_sha256: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "binary_mode": "0755",
            "binary_sha256": binary_sha256,
            "bin": "owner-beta-simulation-runner",
            "cache_key": "a".repeat(64),
            "cargo_success": true,
            "compile_env": {
                "CARGO_BUILD_JOBS": "2",
                "CARGO_TARGET_DIR": "/cargo-target",
                "RUSTFLAGS": "<unset>",
                "LAGRANGE_CODE_COMMIT": COMMIT,
                "R4_SIMULATION_BUILD_GIT_COMMIT": COMMIT,
                "R4_SIMULATION_BUILD_TOOLCHAIN": "1.97.1",
                "R4_SIMULATION_BUILD_TARGET": "x86_64-unknown-linux-musl",
                "R4_SIMULATION_BUILD_PROFILE": "release",
                "R4_SIMULATION_BUILD_ORIGIN": "official-release-v1",
                "R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256": "c".repeat(64),
                "R4_SIMULATION_BUILD_RECIPE_SHA256": "d".repeat(64),
            },
            "features": [],
            "format": "lagrange-rust-artifact-v1",
            "h_sha256": "b".repeat(64),
            "host_triple": "x86_64-unknown-linux-musl",
            "input_sha256": "c".repeat(64),
            "package": "job-queue",
            "platform": "linux/amd64",
            "profile": "release",
            "recipe_sha256": "d".repeat(64),
            "source_commit": COMMIT,
        }))
        .expect("synthetic official receipt serializes")
    }

    fn official_process_config(
        code_commit: &str,
        receipt_sha256: &str,
        binary_sha256: &str,
        source_input_sha256: &str,
        recipe_sha256: &str,
    ) -> SimulationProcessConfig {
        let (bytes, config_sha256) = official_process_config_bytes(
            code_commit,
            receipt_sha256,
            binary_sha256,
            source_input_sha256,
            recipe_sha256,
        );
        SimulationProcessConfig::from_json(&bytes, code_commit, &config_sha256)
            .expect("synthetic process config is checked from its raw bytes")
    }

    fn official_process_config_bytes(
        code_commit: &str,
        receipt_sha256: &str,
        binary_sha256: &str,
        source_input_sha256: &str,
        recipe_sha256: &str,
    ) -> (Vec<u8>, String) {
        let value = serde_json::json!({
            "schema": "owner-beta-simulation-process-v1",
            "code_commit": code_commit,
            "manifest_format": "LAGRANGE_RELEASE_MANIFEST_V3",
            "manifest_sha256": format!("sha256:{}", "1".repeat(64)),
            "image_id": format!("sha256:{}", "2".repeat(64)),
            "receipt_sha256": format!("sha256:{receipt_sha256}"),
            "binary_sha256": format!("sha256:{binary_sha256}"),
            "source_input_sha256": format!("sha256:{source_input_sha256}"),
            "recipe_sha256": format!("sha256:{recipe_sha256}"),
            "build_origin": "official-release-v1",
            "package": "job-queue",
            "bin": "owner-beta-simulation-runner",
            "target": "x86_64-unknown-linux-musl",
            "profile": "release",
            "toolchain": "1.97.1",
            "owner": "11111111-2222-3333-4444-555555555555",
            "entitlement_reference": "fixture:synthetic-owner-rights-v1",
            "entitlement_sha256": format!("sha256:{}", "7".repeat(64)),
            "historical_root": "/synthetic/history/δ",
            "reader_uid": 10001,
            "reader_gid": 10001,
            "paths": {
                "config": "/run/owner-beta-simulation/config.json",
                "artifact_root": "/data/owner-beta-artifacts",
                "receipt": "/usr/share/lagrange/owner-beta-simulation/artifact.json",
                "complete": "/usr/share/lagrange/owner-beta-simulation/COMPLETE",
                "executable": "/usr/local/bin/owner-beta-simulation-runner",
                "worker_id": "owner-beta-simulation-runner"
            },
            "limits": {
                "memory_bytes": 1234567,
                "nano_cpus": 2345678,
                "health_tmpfs_bytes": 3456789,
                "pids_limit": 42,
                "startup_timeout_seconds": 123,
                "health_max_age_seconds": 234,
                "stop_timeout_seconds": 345
            }
        });
        let bytes = serde_json::to_vec(&value).expect("synthetic process config serializes");
        let config_sha256 = format!("sha256:{}", sha256_hex(&bytes));
        (bytes, config_sha256)
    }

    struct FakeOfficialProvenanceIo {
        compile_identity: Result<BuildCompileIdentity, ()>,
        artifact_root: PathBuf,
        current_executable: PathBuf,
        files: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
        executable_sha256: RefCell<String>,
        events: RefCell<Vec<&'static str>>,
    }

    impl FakeOfficialProvenanceIo {
        fn new(
            config: &SimulationProcessConfig,
            compile_identity: Result<BuildCompileIdentity, ()>,
            receipt_bytes: Vec<u8>,
            complete_bytes: Vec<u8>,
            executable_sha256: &str,
        ) -> Self {
            let mut files = BTreeMap::new();
            files.insert(PathBuf::from(config.receipt_path()), receipt_bytes);
            files.insert(PathBuf::from(config.complete_path()), complete_bytes);
            Self {
                compile_identity,
                artifact_root: PathBuf::from(config.artifact_root()),
                current_executable: PathBuf::from(config.executable_path()),
                files: RefCell::new(files),
                executable_sha256: RefCell::new(executable_sha256.to_owned()),
                events: RefCell::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<&'static str> {
            self.events.borrow().clone()
        }

        fn clear_events(&self) {
            self.events.borrow_mut().clear();
        }

        fn set_file(&self, path: &str, bytes: Vec<u8>) {
            self.files.borrow_mut().insert(PathBuf::from(path), bytes);
        }

        fn remove_file(&self, path: &str) {
            self.files.borrow_mut().remove(Path::new(path));
        }

        fn set_executable_sha256(&self, digest: &str) {
            *self.executable_sha256.borrow_mut() = digest.to_owned();
        }

        fn set_current_executable(&mut self, path: &str) {
            self.current_executable = PathBuf::from(path);
        }

        fn set_compile_identity(&mut self, identity: Result<BuildCompileIdentity, ()>) {
            self.compile_identity = identity;
        }
    }

    impl OfficialProvenanceIo for FakeOfficialProvenanceIo {
        fn compile_identity(&self) -> Result<BuildCompileIdentity, ()> {
            self.events.borrow_mut().push("compile-identity");
            self.compile_identity.clone()
        }

        fn validate_artifact_root(&self, path: &Path) -> Result<PathBuf, SimulationWorkerError> {
            self.events.borrow_mut().push("artifact-root");
            if path != self.artifact_root {
                return Err(SimulationWorkerError::ProvenanceInvalid);
            }
            Ok(path.to_path_buf())
        }

        fn current_executable(&self) -> Result<PathBuf, SimulationWorkerError> {
            self.events.borrow_mut().push("current-executable");
            Ok(self.current_executable.clone())
        }

        fn read_bounded_regular_file(
            &self,
            path: &Path,
            max_bytes: usize,
        ) -> Result<Vec<u8>, SimulationWorkerError> {
            let event =
                if path == Path::new("/usr/share/lagrange/owner-beta-simulation/artifact.json") {
                    "receipt-read"
                } else if path == Path::new("/usr/share/lagrange/owner-beta-simulation/COMPLETE") {
                    "complete-read"
                } else {
                    return Err(SimulationWorkerError::ProvenanceInvalid);
                };
            self.events.borrow_mut().push(event);
            let bytes = self
                .files
                .borrow()
                .get(path)
                .cloned()
                .ok_or(SimulationWorkerError::ProvenanceInvalid)?;
            if bytes.is_empty() || bytes.len() > max_bytes {
                return Err(SimulationWorkerError::ProvenanceInvalid);
            }
            Ok(bytes)
        }

        fn sha256_file(&self, path: &Path) -> Result<String, SimulationWorkerError> {
            self.events.borrow_mut().push("executable-hash");
            if path != self.current_executable {
                return Err(SimulationWorkerError::ProvenanceInvalid);
            }
            Ok(self.executable_sha256.borrow().clone())
        }
    }

    struct OfficialFixture {
        config: SimulationProcessConfig,
        receipt_sha256: String,
        binary_sha256: String,
        receipt_bytes: Vec<u8>,
        complete_bytes: Vec<u8>,
        io: FakeOfficialProvenanceIo,
    }

    fn official_fixture() -> OfficialFixture {
        let binary_sha256 = sha256_hex(b"synthetic official runner");
        let receipt_bytes = official_receipt(&binary_sha256);
        let receipt_sha256 = sha256_hex(&receipt_bytes);
        let config = official_process_config(
            COMMIT,
            &receipt_sha256,
            &binary_sha256,
            &"c".repeat(64),
            &"d".repeat(64),
        );
        let complete_bytes = format!("{receipt_sha256}\n").into_bytes();
        let io = FakeOfficialProvenanceIo::new(
            &config,
            Ok(official_compile()),
            receipt_bytes.clone(),
            complete_bytes.clone(),
            &binary_sha256,
        );
        OfficialFixture {
            config,
            receipt_sha256,
            binary_sha256,
            receipt_bytes,
            complete_bytes,
            io,
        }
    }

    fn runtime_from_official_validation(
        config: &SimulationProcessConfig,
        expected_commit: &str,
        expected_receipt_sha256: &str,
        validated: ValidatedOfficialConfig,
    ) -> TrustedSimulationRuntime {
        TrustedSimulationRuntime {
            artifact_root: validated.artifact_root,
            receipt_path: validated.receipt_path,
            expected_receipt_sha256: validated.expected_receipt_sha256.clone(),
            executable_path: validated.executable_path,
            engine: validated.engine,
            official_process_binding: Some(OfficialProcessBinding {
                config: config.clone(),
                expected_commit: expected_commit.to_owned(),
                expected_receipt_sha256: expected_receipt_sha256.to_owned(),
            }),
        }
    }

    #[test]
    fn checked_process_config_fixture_rejects_a_raw_byte_mutation_under_its_original_pin() {
        let (bytes, expected_config_sha256) = official_process_config_bytes(
            COMMIT,
            &"a".repeat(64),
            &"b".repeat(64),
            &"c".repeat(64),
            &"d".repeat(64),
        );
        assert!(
            SimulationProcessConfig::from_json(&bytes, COMMIT, &expected_config_sha256).is_ok()
        );

        let mut changed_bytes = bytes;
        changed_bytes.insert(0, b' ');
        assert!(matches!(
            SimulationProcessConfig::from_json(&changed_bytes, COMMIT, &expected_config_sha256),
            Err(crate::owner_beta::simulation_execution::SimulationProcessConfigError::Invalid)
        ));
    }

    #[test]
    fn provenance_origin_selection_is_compile_time_closed_and_disjoint() {
        let common = [Some(COMMIT), Some(TOOLCHAIN), Some(TARGET), Some(PROFILE)];
        let official_common = [
            Some(COMMIT),
            Some(official::TOOLCHAIN),
            Some(official::TARGET),
            Some(official::PROFILE),
        ];
        let input_sha256 = "c".repeat(64);
        let recipe_sha256 = "d".repeat(64);
        let hosted = compile_identity_from(
            None,
            common,
            [Some(RUN), Some(ATTEMPT)],
            [Some("1123456789abcdef0123456789abcdef01234567"), None, None],
        );
        assert!(matches!(hosted, Ok(BuildCompileIdentity::Hosted(_))));

        let official = compile_identity_from(
            Some("official-release-v1"),
            official_common,
            [None, None],
            [
                Some(COMMIT),
                Some(input_sha256.as_str()),
                Some(recipe_sha256.as_str()),
            ],
        );
        assert!(matches!(official, Ok(BuildCompileIdentity::Official(_))));

        assert!(
            compile_identity_from(
                Some(""),
                common,
                [Some(RUN), Some(ATTEMPT)],
                [None, None, None]
            )
            .is_err()
        );
        assert!(
            compile_identity_from(
                Some("unknown"),
                common,
                [Some(RUN), Some(ATTEMPT)],
                [None, None, None]
            )
            .is_err()
        );
        assert!(
            compile_identity_from(
                None,
                common,
                [Some(RUN), Some(ATTEMPT)],
                [None, Some("c"), None]
            )
            .is_err()
        );
        assert!(
            compile_identity_from(
                Some("official-release-v1"),
                official_common,
                [None, Some(ATTEMPT)],
                [
                    Some(COMMIT),
                    Some(input_sha256.as_str()),
                    Some(recipe_sha256.as_str())
                ]
            )
            .is_err()
        );
        assert!(
            compile_identity_from(
                Some("official-release-v1"),
                official_common,
                [None, None],
                [Some(COMMIT), Some(input_sha256.as_str()), None]
            )
            .is_err()
        );
        assert!(
            compile_identity_from(
                Some("official-release-v1"),
                official_common,
                [None, None],
                [
                    Some("1123456789abcdef0123456789abcdef01234567"),
                    Some(input_sha256.as_str()),
                    Some(recipe_sha256.as_str())
                ]
            )
            .is_err()
        );
        let zero_commit = "0".repeat(40);
        assert!(
            compile_identity_from(
                Some("official-release-v1"),
                [
                    Some(zero_commit.as_str()),
                    Some(official::TOOLCHAIN),
                    Some(official::TARGET),
                    Some(official::PROFILE),
                ],
                [None, None],
                [
                    Some(zero_commit.as_str()),
                    Some(input_sha256.as_str()),
                    Some(recipe_sha256.as_str()),
                ]
            )
            .is_err()
        );

        for (toolchain, target, profile) in [
            ("1.97.2", official::TARGET, official::PROFILE),
            (
                official::TOOLCHAIN,
                "x86_64-unknown-linux-gnu",
                official::PROFILE,
            ),
            (official::TOOLCHAIN, official::TARGET, "debug"),
        ] {
            assert!(
                compile_identity_from(
                    Some("official-release-v1"),
                    [Some(COMMIT), Some(toolchain), Some(target), Some(profile)],
                    [None, None],
                    [
                        Some(COMMIT),
                        Some(input_sha256.as_str()),
                        Some(recipe_sha256.as_str()),
                    ],
                )
                .is_err()
            );
        }
    }

    #[test]
    fn provenance_official_origin_rejects_hosted_receipts_and_hosted_origin_rejects_official() {
        let executable = b"synthetic cross-origin executable";
        let executable_digest = sha256_hex(executable);
        let directory = tempfile::tempdir().expect("synthetic cross-origin directory");
        let executable_path = directory.path().join("runner-fixture");
        let receipt_path = directory.path().join("receipt-fixture.json");
        std::fs::write(&executable_path, executable).expect("synthetic executable writes");

        let hosted_receipt = receipt(&executable_digest);
        let hosted_receipt_digest = sha256_hex(&hosted_receipt);
        std::fs::write(&receipt_path, &hosted_receipt).expect("hosted fixture writes");
        assert!(
            validate_current_material(
                &receipt_path,
                &hosted_receipt_digest,
                &executable_path,
                Ok(official_compile()),
            )
            .is_err()
        );

        let official_receipt = official_receipt(&executable_digest);
        let official_receipt_digest = sha256_hex(&official_receipt);
        std::fs::write(&receipt_path, &official_receipt).expect("official fixture writes");
        assert!(
            validate_current_material(
                &receipt_path,
                &official_receipt_digest,
                &executable_path,
                Ok(BuildCompileIdentity::Hosted(compile())),
            )
            .is_err()
        );
    }

    #[test]
    fn provenance_official_origin_requires_pinned_receipt_and_original_executable() {
        let executable = b"synthetic official executable";
        let executable_digest = sha256_hex(executable);
        let receipt_bytes = official_receipt(&executable_digest);
        let expected_receipt_digest = sha256_hex(&receipt_bytes);
        let directory = tempfile::tempdir().expect("synthetic official directory");
        let executable_path = directory.path().join("runner-fixture");
        let receipt_path = directory.path().join("artifact.json");
        std::fs::write(&executable_path, executable).expect("synthetic executable writes");
        std::fs::write(&receipt_path, &receipt_bytes).expect("synthetic official receipt writes");

        assert!(
            validate_current_material(
                &receipt_path,
                &expected_receipt_digest,
                &executable_path,
                Ok(official_compile()),
            )
            .is_ok()
        );
        assert!(
            validate_current_material(
                &receipt_path,
                &"0".repeat(64),
                &executable_path,
                Ok(official_compile()),
            )
            .is_err()
        );

        std::fs::write(&executable_path, b"replacement executable")
            .expect("replacement executable writes");
        assert!(
            validate_current_material(
                &receipt_path,
                &expected_receipt_digest,
                &executable_path,
                Ok(official_compile()),
            )
            .is_err()
        );
    }

    #[test]
    fn official_checked_config_uses_shared_receipt_and_complete_binding_flow() {
        let fixture = official_fixture();
        let expected_input = format!("sha256:{}", "c".repeat(64));
        let expected_recipe = format!("sha256:{}", "d".repeat(64));
        assert_eq!(
            fixture.config.source_input_sha256(),
            expected_input.as_str()
        );
        assert_eq!(fixture.config.recipe_sha256(), expected_recipe.as_str());
        let BuildCompileIdentity::Official(compile) = official_compile() else {
            panic!("fixture compile identity is official");
        };
        assert!(official::compile_matches_process_config(
            &compile,
            COMMIT,
            fixture.config.source_input_sha256(),
            fixture.config.recipe_sha256(),
        ));
        assert!(!official::compile_matches_process_config(
            &compile,
            COMMIT,
            &"c".repeat(64),
            fixture.config.recipe_sha256(),
        ));
        assert!(!official::compile_matches_process_config(
            &compile,
            COMMIT,
            &format!("sha256:{}", fixture.config.source_input_sha256()),
            fixture.config.recipe_sha256(),
        ));
        assert!(!official::compile_matches_process_config(
            &compile,
            COMMIT,
            fixture.config.source_input_sha256(),
            &"d".repeat(64),
        ));
        assert!(!official::compile_matches_process_config(
            &compile,
            COMMIT,
            fixture.config.source_input_sha256(),
            &format!("sha256:{}", fixture.config.recipe_sha256()),
        ));
        let validated = validate_official_config_with(
            &fixture.config,
            COMMIT,
            &fixture.receipt_sha256,
            &fixture.io,
        )
        .expect("matching checked config, receipt, executable, and COMPLETE are accepted");

        assert_eq!(validated.expected_receipt_sha256, fixture.receipt_sha256);
        assert_eq!(
            validated.executable_path,
            PathBuf::from(fixture.config.executable_path())
        );
        assert_eq!(
            validated.engine,
            EngineIdentity::new(
                COMMIT,
                &format!("sha256:{}", fixture.binary_sha256),
                official::TOOLCHAIN,
                official::TARGET,
                official::PROFILE,
            )
            .expect("official fixture engine identity is valid")
        );
        assert_eq!(
            fixture.io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read",
                "executable-hash",
                "complete-read",
            ]
        );
    }

    #[test]
    fn official_checked_config_rejects_independent_pin_and_compile_mismatches_before_reads() {
        let mut fixture = official_fixture();
        let malformed_commits = [
            String::new(),
            " ".to_owned(),
            "0123456789ABCDEF0123456789abcdef01234567".to_owned(),
            COMMIT[..39].to_owned(),
            "0".repeat(40),
            format!("sha256:{COMMIT}"),
        ];
        for malformed in malformed_commits {
            assert!(
                validate_official_config_with(
                    &fixture.config,
                    &malformed,
                    &fixture.receipt_sha256,
                    &fixture.io,
                )
                .is_err(),
                "malformed commit pin {malformed:?} is rejected"
            );
            assert!(fixture.io.events().is_empty());
        }

        let malformed_receipts = [
            String::new(),
            " ".to_owned(),
            "A".repeat(64),
            "f".repeat(63),
            format!("sha256:{}", fixture.receipt_sha256),
            format!("SHA256:{}", fixture.receipt_sha256),
            format!("sha256:sha256:{}", fixture.receipt_sha256),
        ];
        for malformed in malformed_receipts {
            assert!(
                validate_official_config_with(&fixture.config, COMMIT, &malformed, &fixture.io,)
                    .is_err(),
                "malformed receipt pin {malformed:?} is rejected"
            );
            assert!(fixture.io.events().is_empty());
        }

        let zero_receipt = "0".repeat(64);
        let zero_receipt_config = official_process_config(
            COMMIT,
            &zero_receipt,
            &fixture.binary_sha256,
            &"c".repeat(64),
            &"d".repeat(64),
        );
        let zero_receipt_io = FakeOfficialProvenanceIo::new(
            &zero_receipt_config,
            Ok(official_compile()),
            fixture.receipt_bytes.clone(),
            format!("{zero_receipt}\n").into_bytes(),
            &fixture.binary_sha256,
        );
        assert!(
            validate_official_config_with(
                &zero_receipt_config,
                COMMIT,
                &zero_receipt,
                &zero_receipt_io,
            )
            .is_err()
        );
        assert_eq!(
            zero_receipt_io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read",
            ],
            "zero is valid SHA syntax and fails only when compared with receipt bytes"
        );

        let other_commit = "1123456789abcdef0123456789abcdef01234567";
        assert!(
            validate_official_config_with(
                &fixture.config,
                other_commit,
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert!(fixture.io.events().is_empty());

        assert!(
            validate_official_config_with(
                &fixture.config,
                &"0".repeat(40),
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert!(fixture.io.events().is_empty());
        assert!(
            validate_official_config_with(
                &fixture.config,
                COMMIT,
                &format!("sha256:{}", fixture.receipt_sha256),
                &fixture.io,
            )
            .is_err()
        );
        assert!(fixture.io.events().is_empty());

        let other_receipt = "f".repeat(64);
        assert!(
            validate_official_config_with(&fixture.config, COMMIT, &other_receipt, &fixture.io,)
                .is_err()
        );
        assert!(fixture.io.events().is_empty());

        let config_commit_mismatch = official_process_config(
            other_commit,
            &fixture.receipt_sha256,
            &fixture.binary_sha256,
            &"c".repeat(64),
            &"d".repeat(64),
        );
        assert!(
            validate_official_config_with(
                &config_commit_mismatch,
                other_commit,
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert_eq!(fixture.io.events(), vec!["compile-identity"]);

        fixture.io.clear_events();
        let input_mismatch = official_process_config(
            COMMIT,
            &fixture.receipt_sha256,
            &fixture.binary_sha256,
            &"e".repeat(64),
            &"d".repeat(64),
        );
        assert!(
            validate_official_config_with(
                &input_mismatch,
                COMMIT,
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert_eq!(fixture.io.events(), vec!["compile-identity"]);

        fixture.io.clear_events();
        let recipe_mismatch = official_process_config(
            COMMIT,
            &fixture.receipt_sha256,
            &fixture.binary_sha256,
            &"c".repeat(64),
            &"e".repeat(64),
        );
        assert!(
            validate_official_config_with(
                &recipe_mismatch,
                COMMIT,
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert_eq!(fixture.io.events(), vec!["compile-identity"]);

        fixture.io.clear_events();
        fixture
            .io
            .set_compile_identity(Ok(BuildCompileIdentity::Hosted(compile())));
        assert!(
            validate_official_config_with(
                &fixture.config,
                COMMIT,
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert_eq!(fixture.io.events(), vec!["compile-identity"]);
    }

    #[test]
    fn official_checked_config_rejects_binary_binding_and_current_executable_mismatch() {
        let fixture = official_fixture();
        let binary_mismatch = official_process_config(
            COMMIT,
            &fixture.receipt_sha256,
            &"e".repeat(64),
            &"c".repeat(64),
            &"d".repeat(64),
        );
        assert!(
            validate_official_config_with(
                &binary_mismatch,
                COMMIT,
                &fixture.receipt_sha256,
                &fixture.io,
            )
            .is_err()
        );
        assert_eq!(
            fixture.io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read",
                "executable-hash",
            ]
        );

        fixture.io.clear_events();
        let mut path_mismatch = official_fixture();
        path_mismatch
            .io
            .set_current_executable("/usr/local/bin/other-runner");
        assert!(
            validate_official_config_with(
                &path_mismatch.config,
                COMMIT,
                &path_mismatch.receipt_sha256,
                &path_mismatch.io,
            )
            .is_err()
        );
        assert_eq!(
            path_mismatch.io.events(),
            vec!["compile-identity", "artifact-root", "current-executable"]
        );

        let executable_mismatch = official_fixture();
        executable_mismatch
            .io
            .set_executable_sha256(&"e".repeat(64));
        assert!(
            validate_official_config_with(
                &executable_mismatch.config,
                COMMIT,
                &executable_mismatch.receipt_sha256,
                &executable_mismatch.io,
            )
            .is_err()
        );
        assert_eq!(
            executable_mismatch.io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read",
                "executable-hash",
            ]
        );
    }

    #[test]
    fn official_checked_config_rejects_raw_receipt_mutation_and_invalid_receipt_schema() {
        let raw_mutation = official_fixture();
        let mut changed_bytes = raw_mutation.receipt_bytes.clone();
        changed_bytes.push(b' ');
        raw_mutation
            .io
            .set_file(raw_mutation.config.receipt_path(), changed_bytes);
        assert!(
            validate_official_config_with(
                &raw_mutation.config,
                COMMIT,
                &raw_mutation.receipt_sha256,
                &raw_mutation.io,
            )
            .is_err()
        );
        assert_eq!(
            raw_mutation.io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read"
            ]
        );

        let mut value: serde_json::Value =
            serde_json::from_slice(&raw_mutation.receipt_bytes).expect("fixture receipt parses");
        value["cargo_success"] = serde_json::Value::Bool(false);
        let invalid_receipt = serde_json::to_vec(&value).expect("invalid receipt serializes");
        let invalid_receipt_sha256 = sha256_hex(&invalid_receipt);
        let invalid_config = official_process_config(
            COMMIT,
            &invalid_receipt_sha256,
            &raw_mutation.binary_sha256,
            &"c".repeat(64),
            &"d".repeat(64),
        );
        let invalid_io = FakeOfficialProvenanceIo::new(
            &invalid_config,
            Ok(official_compile()),
            invalid_receipt,
            format!("{invalid_receipt_sha256}\n").into_bytes(),
            &raw_mutation.binary_sha256,
        );
        assert!(
            validate_official_config_with(
                &invalid_config,
                COMMIT,
                &invalid_receipt_sha256,
                &invalid_io,
            )
            .is_err()
        );
        assert_eq!(
            invalid_io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read"
            ]
        );

        let mut env_value: serde_json::Value = serde_json::from_slice(&raw_mutation.receipt_bytes)
            .expect("fixture receipt parses for compile environment mutation");
        env_value["compile_env"]["R4_SIMULATION_BUILD_RECIPE_SHA256"] =
            serde_json::Value::String("e".repeat(64));
        let invalid_env_receipt =
            serde_json::to_vec(&env_value).expect("invalid compile environment serializes");
        let invalid_env_sha256 = sha256_hex(&invalid_env_receipt);
        let invalid_env_config = official_process_config(
            COMMIT,
            &invalid_env_sha256,
            &raw_mutation.binary_sha256,
            &"c".repeat(64),
            &"d".repeat(64),
        );
        let invalid_env_io = FakeOfficialProvenanceIo::new(
            &invalid_env_config,
            Ok(official_compile()),
            invalid_env_receipt,
            format!("{invalid_env_sha256}\n").into_bytes(),
            &raw_mutation.binary_sha256,
        );
        assert!(
            validate_official_config_with(
                &invalid_env_config,
                COMMIT,
                &invalid_env_sha256,
                &invalid_env_io,
            )
            .is_err()
        );
        assert_eq!(
            invalid_env_io.events(),
            vec![
                "compile-identity",
                "artifact-root",
                "current-executable",
                "receipt-read"
            ]
        );
    }

    #[test]
    fn official_checked_config_requires_exact_bounded_complete_marker() {
        let baseline = official_fixture();
        let digest = baseline.receipt_sha256.clone();
        let uppercase_digest = digest.to_uppercase();
        assert_ne!(uppercase_digest, digest);
        let baseline_validated = validate_official_config_with(
            &baseline.config,
            COMMIT,
            &baseline.receipt_sha256,
            &baseline.io,
        )
        .expect("valid baseline reaches receipt, executable hash, and COMPLETE");
        let expected_events = vec![
            "compile-identity",
            "artifact-root",
            "current-executable",
            "receipt-read",
            "executable-hash",
            "complete-read",
        ];
        assert_eq!(baseline.io.events(), expected_events);
        assert_eq!(baseline_validated.expected_receipt_sha256, digest);

        let invalid_complete_values = [
            ("empty", Vec::new()),
            ("missing final LF", digest.as_bytes().to_vec()),
            ("CRLF", format!("{digest}\r\n").into_bytes()),
            ("wrong digest", format!("{}\n", "f".repeat(64)).into_bytes()),
            (
                "uppercase digest",
                format!("{uppercase_digest}\n").into_bytes(),
            ),
            ("cap plus one", format!("{digest}\nX").into_bytes()),
        ];
        for (case, complete_bytes) in invalid_complete_values {
            let fixture = official_fixture();
            fixture
                .io
                .set_file(fixture.config.complete_path(), complete_bytes);
            assert!(
                validate_official_config_with(
                    &fixture.config,
                    COMMIT,
                    &fixture.receipt_sha256,
                    &fixture.io,
                )
                .is_err(),
                "invalid COMPLETE case {case} is rejected"
            );
            let events = fixture.io.events();
            assert_eq!(
                events, expected_events,
                "invalid COMPLETE case {case} follows the valid baseline through the read"
            );
            assert_eq!(events.last(), Some(&"complete-read"));
        }

        let missing = official_fixture();
        missing.io.remove_file(missing.config.complete_path());
        assert!(
            validate_official_config_with(
                &missing.config,
                COMMIT,
                &missing.receipt_sha256,
                &missing.io,
            )
            .is_err()
        );
        let events = missing.io.events();
        assert_eq!(
            events, expected_events,
            "missing COMPLETE reaches its read attempt after receipt and executable checks"
        );
        assert_eq!(events.last(), Some(&"complete-read"));
    }

    #[test]
    fn retained_official_binding_revalidates_the_same_config_and_original_pins() {
        let mut fixture = official_fixture();
        let validated = validate_official_config_with(
            &fixture.config,
            COMMIT,
            &fixture.receipt_sha256,
            &fixture.io,
        )
        .expect("official fixture is admitted before retained revalidation");
        let runtime = runtime_from_official_validation(
            &fixture.config,
            COMMIT,
            &fixture.receipt_sha256,
            validated,
        );
        assert!(revalidate_official_runtime_with(&runtime, &fixture.io).is_ok());
        assert_eq!(
            runtime
                .official_process_binding
                .as_ref()
                .expect("official binding is retained")
                .expected_receipt_sha256,
            fixture.receipt_sha256
        );
        assert!(!fixture.io.events().contains(&"config-read"));

        fixture.io.clear_events();
        fixture.io.set_file(
            fixture.config.complete_path(),
            format!("{}\r\n", fixture.receipt_sha256).into_bytes(),
        );
        assert!(revalidate_official_runtime_with(&runtime, &fixture.io).is_err());
        assert!(fixture.io.events().contains(&"complete-read"));

        fixture.io.clear_events();
        fixture.io.set_file(
            fixture.config.complete_path(),
            fixture.complete_bytes.clone(),
        );
        let mut changed_receipt = fixture.receipt_bytes.clone();
        changed_receipt.push(b' ');
        fixture
            .io
            .set_file(fixture.config.receipt_path(), changed_receipt);
        assert!(revalidate_official_runtime_with(&runtime, &fixture.io).is_err());
        assert!(fixture.io.events().contains(&"receipt-read"));
        assert!(!fixture.io.events().contains(&"complete-read"));

        fixture.io.clear_events();
        fixture
            .io
            .set_file(fixture.config.receipt_path(), fixture.receipt_bytes.clone());
        fixture.io.set_executable_sha256(&"e".repeat(64));
        assert!(revalidate_official_runtime_with(&runtime, &fixture.io).is_err());
        assert!(fixture.io.events().contains(&"executable-hash"));
        assert!(!fixture.io.events().contains(&"complete-read"));

        fixture.io.clear_events();
        fixture.io.set_executable_sha256(&fixture.binary_sha256);
        fixture
            .io
            .set_compile_identity(Ok(BuildCompileIdentity::Official(
                official::CompileIdentity::new(
                    COMMIT,
                    official::TOOLCHAIN,
                    official::TARGET,
                    official::PROFILE,
                    &"c".repeat(64),
                    &"e".repeat(64),
                )
                .expect("changed compile fixture remains syntactically official"),
            )));
        assert!(revalidate_official_runtime_with(&runtime, &fixture.io).is_err());
        assert_eq!(fixture.io.events(), vec!["compile-identity"]);
    }
}
